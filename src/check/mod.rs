//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy. The numbering follows the table in
//! `docs/design/v0.2/1_spec.md`, "Invariants".

mod graph;
mod rules;

use crate::corpus::{EdgeType, Store};
use anyhow::Result;
use graph::find_cycle;
use rules::check_node;
pub use rules::{is_local_path, resolve_local};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};

/// How badly a finding breaks the corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The corpus is inconsistent. `check` exits non-zero.
    Error,
    /// Worth your attention, not worth blocking on.
    Warn,
}

/// One violated invariant.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Error or warning.
    pub level: Level,
    /// Which invariant, by its number in `docs/design/v0.2/1_spec.md`.
    pub rule: u8,
    /// The node it belongs to, where there is one.
    pub node: Option<String>,
    /// What is wrong.
    pub message: String,
}

/// The result of a full check.
#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// Everything found, errors first.
    pub findings: Vec<Finding>,
    /// How many nodes were examined.
    pub nodes: usize,
}

impl Report {
    fn push(&mut self, level: Level, rule: u8, node: Option<&str>, message: impl Into<String>) {
        self.findings.push(Finding {
            level,
            rule,
            node: node.map(String::from),
            message: message.into(),
        });
    }

    /// Findings that make the corpus inconsistent.
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.level == Level::Error)
            .count()
    }

    /// Findings worth attention but not blocking.
    pub fn warnings(&self) -> usize {
        self.findings.len() - self.errors()
    }
}

/// Run every invariant over the corpus.
///
/// Rule 7 never reaches this function: a reference carrying a `verdict` or
/// `strength`, or a node with an unknown field, fails to deserialize, so the
/// corpus load itself errors out rather than producing a finding.
pub fn run(store: &Store) -> Result<Report> {
    let docs = store.load_all()?;
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report {
        nodes: docs.len(),
        ..Report::default()
    };

    for doc in &docs {
        check_node(doc, &ids, store, &mut r);
    }

    // 4. `contradicts` is a claim about both nodes, so a one-sided declaration
    //    is a half-recorded fact that will read as settled later.
    for doc in &docs {
        for target in doc.node.edges_of(EdgeType::Contradicts) {
            let mutual = docs.iter().find(|d| d.node.id == target).is_some_and(|d| {
                d.node
                    .edges_of(EdgeType::Contradicts)
                    .any(|b| b == doc.node.id)
            });
            if !mutual {
                r.push(
                    Level::Error,
                    4,
                    Some(&doc.node.id),
                    format!("contradicts `{target}`, which does not contradict back"),
                );
            }
        }
    }

    // 1. Genealogy must be acyclic: an idea cannot be its own ancestor.
    //    Diamonds are legal; only a loop is not.
    let genealogy: HashMap<&str, Vec<&str>> = docs
        .iter()
        .map(|d| (d.node.id.as_str(), d.node.parents().collect()))
        .collect();
    if let Some(cycle) = find_cycle(&genealogy) {
        r.push(
            Level::Error,
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
                    Level::Warn,
                    10,
                    None,
                    format!("tags `{a}` and `{b}` differ only by {how}"),
                );
            }
        }
    }

    r.findings
        .sort_by_key(|f| (f.level != Level::Error, f.rule));
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

#[cfg(test)]
mod tests {
    use super::tag_drift;

    #[test]
    fn tag_drift_catches_case_and_plurals_only() {
        assert_eq!(tag_drift("Physics", "physics"), Some("case"));
        assert_eq!(tag_drift("sim", "sims"), Some("a trailing `s`"));
        assert_eq!(tag_drift("Sims", "sim"), Some("a trailing `s`"));
        assert_eq!(tag_drift("physics", "orrery"), None);
        assert_eq!(tag_drift("sim", "simulation"), None);
    }
}
