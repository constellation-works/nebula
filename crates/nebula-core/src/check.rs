//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy. The numbering follows the table in
//! `docs/design/v0.2/1_spec.md`, "Invariants".
//!
//! The checker reports; it never fixes and never prints. What an error costs
//! the caller — an exit code, a red badge — is the caller's business.

use crate::error::Result;
use crate::graph::Graph;
use crate::model::{Doc, EdgeType, Status};
use crate::store::Corpus;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// How badly a finding breaks the corpus.
///
/// Serialized under the key `level`, which is the name `neb check --json` has
/// always used for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The corpus is inconsistent. `check` exits non-zero.
    Error,
    /// Worth your attention, not worth blocking on.
    Warn,
}

/// One violated invariant.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Finding {
    /// Error or warning.
    pub level: Severity,
    /// Which invariant, by its number in `docs/design/v0.2/1_spec.md`.
    pub rule: u8,
    /// The node it belongs to, where there is one.
    pub node: Option<String>,
    /// What is wrong.
    pub message: String,
}

/// The result of a full check.
#[derive(Debug, Default, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Report {
    /// Everything found, errors first.
    pub findings: Vec<Finding>,
    /// How many nodes were examined.
    pub nodes: usize,
}

impl Report {
    fn push(&mut self, level: Severity, rule: u8, node: Option<&str>, message: impl Into<String>) {
        self.findings.push(Finding {
            level,
            rule,
            node: node.map(String::from),
            message: message.into(),
        });
    }
}

/// Run every invariant over a graph.
///
/// The corpus is needed for rule 8 alone, which asks the filesystem whether a
/// local reference resolves, and where the Observatory checkout is.
///
/// Rule 7 never reaches this function: a reference carrying a `verdict` or
/// `strength`, or a node with an unknown field, fails to deserialize, so the
/// corpus load itself errors out rather than producing a finding.
pub fn run(graph: &Graph<'_>, corpus: &Corpus) -> Result<Report> {
    let docs = graph.docs();
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report {
        nodes: docs.len(),
        ..Report::default()
    };
    let observatory = corpus.observatory_root().root;

    for doc in docs {
        check_node(doc, &ids, corpus, observatory.as_deref(), &mut r);
    }

    // 4. `contradicts` is a claim about both nodes, so a one-sided declaration
    //    is a half-recorded fact that will read as settled later.
    for doc in docs {
        for target in doc.node.edges_of(EdgeType::Contradicts) {
            let mutual = graph.get(target).is_some_and(|d| {
                d.node
                    .edges_of(EdgeType::Contradicts)
                    .any(|b| b == doc.node.id)
            });
            if !mutual {
                r.push(
                    Severity::Error,
                    4,
                    Some(&doc.node.id),
                    format!("contradicts `{target}`, which does not contradict back"),
                );
            }
        }
    }

    // 1. Genealogy must be acyclic: an idea cannot be its own ancestor.
    //    Diamonds are legal; only a loop is not.
    if let Some(cycle) = graph.cycle() {
        r.push(
            Severity::Error,
            1,
            None,
            format!("genealogy cycle: {}", cycle.join(" -> ")),
        );
    }

    // 10. Two tags that differ only by case or a trailing `s` are one label
    //     drifting into two. Writes normalise case, so this mostly catches
    //     hand edits and plurals; a warning keeps drift visible without a
    //     declared list to maintain.
    let tags: BTreeSet<&str> = docs
        .iter()
        .flat_map(|d| d.node.tags.iter().map(String::as_str))
        .collect();
    let tags: Vec<&str> = tags.into_iter().collect();
    for (i, a) in tags.iter().enumerate() {
        for b in &tags[i + 1..] {
            if let Some(how) = tag_drift(a, b) {
                r.push(
                    Severity::Warn,
                    10,
                    None,
                    format!("tags `{a}` and `{b}` differ only by {how}"),
                );
            }
        }
    }

    r.findings
        .sort_by_key(|f| (f.level != Severity::Error, f.rule));
    Ok(r)
}

/// How two distinct tags collide, if they do.
fn tag_drift(a: &str, b: &str) -> Option<&'static str> {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a == b {
        return Some("case");
    }
    let plural = |x: &str, y: &str| x.strip_suffix('s') == Some(y);
    if plural(&a, &b) || plural(&b, &a) {
        return Some("a trailing `s`");
    }
    None
}

// ------------------------------------------------------------- node rules --

/// Whether a URI is a path to resolve against `nodes/`, as opposed to a URL,
/// a wikilink, or a scheme-prefixed handle such as `doi:` or `orbit:`.
pub fn is_local_path(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("[[") && !has_scheme(uri)
}

/// `scheme:` with at least two leading letters, so a Windows drive letter
/// does not count and `doi:`, `orbit:`, `neb:`, `mailto:` all do.
fn has_scheme(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    })
}

/// Where a local reference URI lands: relative to the `nodes/` directory,
/// used as given and never canonicalized (macOS temp dirs sit under a
/// symlink, and resolving would pass on one platform and fail on the other).
pub(crate) fn resolve_local(corpus: &Corpus, uri: &str) -> PathBuf {
    corpus.root().join("nodes").join(Path::new(uri))
}

/// The reference kind whose `uri` is a bare Observatory record id rather
/// than a location: `Q002`, `H007`, `T003`, `R012`. Where the record is on
/// this machine is the corpus's `observatory_root` setting, so the reference
/// itself carries nothing machine-specific.
pub const OBSERVATORY: &str = "observatory";

/// Whether `id` has the shape of an Observatory record id: one of `Q`, `H`,
/// `T`, `R` followed by digits. The shape is the whole contract; how many
/// digits Observatory uses is its business.
pub fn is_observatory_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| observatory_dir(c).is_some())
        && !chars.as_str().is_empty()
        && chars.all(|c| c.is_ascii_digit())
}

/// The Observatory directory a record id's letter files it under. Research
/// layout v2: questions, hypotheses and theories are files, research
/// records are directories.
fn observatory_dir(letter: char) -> Option<&'static str> {
    match letter {
        'Q' => Some("questions"),
        'H' => Some("hypotheses"),
        'T' => Some("theories"),
        'R' => Some("research"),
        _ => None,
    }
}

/// Where an Observatory record is under `root`: the entry of the id's
/// directory whose name is the id, or the id followed by `-` or `.`, so
/// `Q002` finds `questions/Q002-is-proper-time-a-count.md` without the
/// reference having to know the slug. `None` when the id has the wrong
/// shape, the directory is unreadable, or nothing there starts with it.
///
/// Matched by prefix in a listing rather than by resolving a path, and used
/// as given: nothing is canonicalized. Ties (two records claiming one id)
/// go to the first in name order, which `check` in Observatory is the place
/// to catch.
pub fn resolve_observatory(root: &Path, id: &str) -> Option<PathBuf> {
    if !is_observatory_id(id) {
        return None;
    }
    let dir = root.join(observatory_dir(id.chars().next()?)?);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
        })
        .collect();
    names.sort();
    names.into_iter().next().map(|name| dir.join(name))
}

/// Every invariant that can be judged from a single node plus the id set.
fn check_node(
    doc: &Doc,
    ids: &HashSet<&str>,
    corpus: &Corpus,
    observatory: Option<&Path>,
    r: &mut Report,
) {
    status_rules(doc, r);
    edge_rules(doc, ids, r);
    reference_rules(doc, corpus, observatory, r);
}

/// Rules about where a node sits in its lifecycle and what that costs.
fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before anything
    //    is read. Without this, the verdict is retroactive rationalisation.
    if n.status == Status::Hypothesis && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Severity::Error,
            2,
            id,
            "status is hypothesis but no kill condition is named",
        );
    }

    // 5. A refuted node says how its kill condition fired. The reference
    //    that convinced you goes in `references`; the sentence goes here.
    if n.status == Status::Refuted && n.closed.as_ref().is_none_or(|c| c.why.trim().is_empty()) {
        r.push(
            Severity::Error,
            5,
            id,
            "status is refuted but closed.why is empty; say what fired the kill condition",
        );
    }
}

/// Rules about the links a node declares.
fn edge_rules(doc: &Doc, ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 3. Dangling edges and self-loops. A lineage that points into nothing
    //    is worse than no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Severity::Error,
                3,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Severity::Error,
                3,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }
}

/// Rules about the references hanging off a node.
fn reference_rules(doc: &Doc, corpus: &Corpus, observatory: Option<&Path>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    for f in &n.references {
        if f.uri.as_deref().is_none_or(|uri| uri.trim().is_empty()) && f.kind != "discussion" {
            r.push(
                Severity::Error,
                8,
                id,
                format!(
                    "reference `{}` has kind `{}` but no URI; only discussions may omit it",
                    f.id, f.kind
                ),
            );
        }
        // 9. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Severity::Warn,
                9,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
        // 8, for an Observatory record: the id is the reference, and where
        //    it is on this machine is a setting. Not finding it here says
        //    the setting is missing or the checkout is behind, not that the
        //    record is gone, so the finding is a warning rather than an
        //    error and the reference stays valid on a machine that has it.
        if f.kind == OBSERVATORY {
            if let Some(record) = f.uri.as_deref().filter(|record| !record.trim().is_empty()) {
                match observatory {
                    None => r.push(
                        Severity::Warn,
                        8,
                        id,
                        format!(
                            "reference `{}` names Observatory record `{record}` but no \
                             observatory root is set (config observatory_root or \
                             $OBSERVATORY_ROOT)",
                            f.id
                        ),
                    ),
                    Some(root) if resolve_observatory(root, record).is_none() => r.push(
                        Severity::Warn,
                        8,
                        id,
                        format!(
                            "reference `{}` names Observatory record `{record}`, which does \
                             not resolve under {}",
                            f.id,
                            root.display()
                        ),
                    ),
                    Some(_) => {}
                }
            }
            continue;
        }
        // 8. A local path that does not resolve is a citation to nothing.
        //    External URLs are not fetched; `check` stays offline and fast.
        if let Some(uri) = f
            .uri
            .as_deref()
            .filter(|uri| !uri.trim().is_empty())
            .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
        {
            r.push(
                Severity::Error,
                8,
                id,
                format!(
                    "reference `{}` points at a path that does not resolve: {}",
                    f.id, uri
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_local_path, is_observatory_id, resolve_observatory, tag_drift};

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
            resolve_observatory(root, "Q002"),
            Some(root.join("questions").join("Q002-a-question.md"))
        );
        assert_eq!(
            resolve_observatory(root, "R012"),
            Some(root.join("research").join("R012-arc"))
        );
        assert_eq!(resolve_observatory(root, "Q003"), None);
        assert_eq!(
            resolve_observatory(root, "H001"),
            None,
            "no hypotheses/ at all"
        );
        assert_eq!(resolve_observatory(root, "Q002-a-question"), None);
    }
}
