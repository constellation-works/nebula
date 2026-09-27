//! Unit tests for `check`, one file per source file (STD-02 §R19). This
//! file holds the tests for `check/mod.rs`.

mod observatory;
mod references;

use crate::check_impl::{Rule, tag_drift};

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
            include_str!("../../../../../docs/design/v0.2/1_spec.md"),
        ),
        (
            "skill",
            include_str!("../../../../../skills/nebula/references/invariants.md"),
        ),
        (
            "lineage spec",
            include_str!("../../../../../docs/design/lineage-graph/specs/invariants.md"),
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
