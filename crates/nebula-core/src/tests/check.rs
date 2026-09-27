//! Unit tests for `check`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant an Observatory tree directly"
)]

use crate::config::{self, OBSERVATORY_ROOT_ENV, ObservatoryRoot, ObservatorySource};
use crate::error::{Error, Result};
use crate::graph_impl::Graph;
use crate::model::{Doc, EdgeType, Reference, Status, is_iso_date};
use crate::pending::{self, PENDING_FILE, PendingWrite};
use crate::store::GITIGNORE_FILE;
use crate::store::{Corpus, Scan, UnreadableNode};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::check_impl::{
    Rule, is_absolute_local, is_local_path, is_observatory_id, resolve_observatory, tag_drift,
};

// The canonical labels and IDs for the published invariant tables. Keep
// this beside the code that emits findings, so documentation drift fails
// a core test rather than silently changing the meaning of `[N]`.
const RULES: &[(Rule, &str)] = &[
    (Rule::AcyclicGenealogy, "Genealogy is acyclic"),
    (
        Rule::HypothesisKill,
        "`hypothesis` names a non-empty `kill`",
    ),
    (Rule::EdgeTargets, "Every edge target exists; no self-loop"),
    (Rule::MutualContradicts, "`contradicts` is mutual"),
    (Rule::RefutedReason, "`refuted` carries `closed.why`"),
    (
        Rule::RefutedReopens,
        "`refuted` leaves only via a new node's `reopens` edge",
    ),
    (
        Rule::NoReferenceVerdict,
        "A reference carries no `verdict`/`strength`",
    ),
    (
        Rule::LocalReference,
        "Non-discussion references have a URI; local URIs resolve relative to `nodes/` and are never absolute",
    ),
    (
        Rule::ObservatoryReference,
        "An `observatory` reference's record resolves under the configured root",
    ),
    (Rule::ReferenceNote, "Every reference has a note"),
    (
        Rule::TagDrift,
        "No two tags differ only by case or a trailing `s`",
    ),
    (
        Rule::OpenNodeClosed,
        "`closed` is set only on a `refuted`/`abandoned` node, never an open one",
    ),
    (Rule::SeedKill, "A `seed` does not carry a `kill` condition"),
    (
        Rule::Dates,
        "`created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created`",
    ),
    (
        Rule::NodeId,
        "A node's `id` names one file under `nodes/`, and is the id its file name names",
    ),
    (
        Rule::ReferenceKind,
        "Every reference kind belongs to the documented vocabulary",
    ),
    (
        Rule::InterruptedWrite,
        "No write is left half-done: no stray temporary file and no pending-write record",
    ),
];

fn table_rules(markdown: &str) -> Vec<(u8, &str)> {
    markdown
        .lines()
        .skip_while(|line| !line.starts_with("| # |"))
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let mut cells = line.split('|').map(str::trim);
            assert_eq!(cells.next(), Some(""), "table row begins with `|`");
            let number = cells.next().unwrap().parse::<u8>().unwrap();
            (number, cells.next().unwrap())
        })
        .collect()
}

// "The invariant-table guard stays a Rust test" (4_decisions.md, STD-02@2 §R21).
#[test]
fn published_invariant_tables_match_checker_rules() {
    let expected: Vec<_> = RULES
        .iter()
        .map(|(rule, label)| (*rule as u8, *label))
        .collect();
    assert!(
        expected.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "checker rule IDs must be distinct and ordered"
    );
    for (name, markdown) in [
        (
            "v0.2 spec",
            include_str!("../../../../docs/design/v0.2/1_spec.md"),
        ),
        (
            "skill",
            include_str!("../../../../skills/nebula/references/invariants.md"),
        ),
        (
            "lineage spec",
            include_str!("../../../../docs/design/lineage-graph/specs/invariants.md"),
        ),
    ] {
        assert_eq!(table_rules(markdown), expected, "{name} invariant table");
    }
}

#[test]
fn tag_drift_catches_case_and_plurals_only() {
    assert_eq!(tag_drift("Physics", "physics"), Some("case"));
    assert_eq!(tag_drift("sim", "sims"), Some("a trailing `s`"));
    assert_eq!(tag_drift("Sims", "sim"), Some("a trailing `s`"));
    assert_eq!(tag_drift("physics", "orrery"), None);
    assert_eq!(tag_drift("sim", "simulation"), None);
}

#[test]
fn schemes_and_urls_are_not_local_paths() {
    for uri in [
        "https://example.org",
        "doi:10.1000/x",
        "orbit:DANI-10345",
        "neb:some-node",
        "[[almanac/page]]",
    ] {
        assert!(!is_local_path(uri), "{uri}");
    }
    for uri in ["./notes/x.md", "notes/x.md", "C:/x.md", "x"] {
        assert!(is_local_path(uri), "{uri}");
    }
}

/// Absolute on any platform is absolute on all of them, and nothing a
/// scheme, a URL or a path under `nodes/` looks like is caught with it.
#[test]
fn absolute_paths_and_file_uris_are_absolute_on_every_platform() {
    for uri in [
        "/etc/hostname",
        "\\\\server\\share\\x.md",
        "C:/x.md",
        "c:\\x.md",
        "C:x.md",
        "file:///etc/hostname",
        "FILE://host/x.md",
        "file:/etc/hostname",
    ] {
        assert!(is_absolute_local(uri), "{uri}");
    }
    for uri in [
        "./notes/x.md",
        "notes/x.md",
        "../../studies/x.md",
        "x",
        "https://example.org",
        "http://example.org/file:///x",
        "mailto:someone@example.org",
        "orbit:DANI-10345",
        "neb:some-node",
        "doi:10.1000/x",
        "[[almanac/page]]",
        "files/x.md",
    ] {
        assert!(!is_absolute_local(uri), "{uri}");
    }
}

#[test]
fn an_observatory_id_is_one_letter_of_four_then_digits() {
    for id in ["Q002", "H7", "T003", "R012"] {
        assert!(is_observatory_id(id), "{id}");
    }
    for id in ["", "Q", "q002", "X002", "Q002-slug", "Q 2", "/abs/Q002.md"] {
        assert!(!is_observatory_id(id), "{id}");
    }
}

/// The record is found by its id alone, whatever slug follows it, and a
/// longer id that merely starts with the same digits is not a match.
#[test]
fn a_record_resolves_by_id_prefix_in_its_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("questions")).unwrap();
    std::fs::create_dir_all(root.join("research").join("R012-arc")).unwrap();
    std::fs::write(root.join("questions").join("Q002-a-question.md"), "").unwrap();
    std::fs::write(root.join("questions").join("Q0021-not-it.md"), "").unwrap();
    std::fs::write(root.join("questions").join("README.md"), "").unwrap();

    assert_eq!(
        resolve_observatory(root, "Q002").unwrap(),
        Some(root.join("questions").join("Q002-a-question.md"))
    );
    assert_eq!(
        resolve_observatory(root, "R012").unwrap(),
        Some(root.join("research").join("R012-arc"))
    );
    assert_eq!(resolve_observatory(root, "Q003").unwrap(), None);
    assert_eq!(
        resolve_observatory(root, "H001").unwrap(),
        None,
        "no hypotheses/ at all"
    );
    assert_eq!(resolve_observatory(root, "Q002-a-question").unwrap(), None);
}
