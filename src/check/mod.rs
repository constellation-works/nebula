//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy.

mod graph;
mod rules;

use crate::corpus::{EdgeType, Store};
use anyhow::Result;
use graph::find_cycle;
use rules::check_node;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
    /// Which invariant, by its number in `docs/spec.md`.
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
/// Some invariants never reach this function: a reference carrying a
/// `verdict`, or a node with an unknown field, fails to deserialize, so the
/// corpus load itself errors out.
pub fn run(store: &Store) -> Result<Report> {
    // Note that some invariants are enforced before this function is reached.
    // A reference carrying a verdict, or a node with an unknown field, fails to
    // deserialize, so `load_all` returns an error rather than a finding.
    let docs = store.load_all()?;
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report {
        nodes: docs.len(),
        ..Report::default()
    };

    for doc in &docs {
        check_node(doc, &docs, &ids, store, &mut r);
    }

    // 5. `contradicts` is a claim about both nodes, so a one-sided declaration
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
                    5,
                    Some(&doc.node.id),
                    format!("contradicts `{target}`, which does not contradict back"),
                );
            }
        }
    }

    // 1. Genealogy must be acyclic: an idea cannot be its own ancestor. This
    //    constrains genealogy alone. Diamonds are legal, and the evidence graph
    //    below is allowed to cycle.
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

    // 9. A cycle in `supports` is circular reasoning. Surfaced, not forbidden,
    //    because noticing it is the point and sometimes the honest answer is
    //    that two ideas really do lean on each other.
    let supports: HashMap<&str, Vec<&str>> = docs
        .iter()
        .map(|d| {
            (
                d.node.id.as_str(),
                d.node.edges_of(EdgeType::Supports).collect(),
            )
        })
        .collect();
    if let Some(cycle) = find_cycle(&supports) {
        r.push(
            Level::Warn,
            9,
            None,
            format!("circular reasoning in supports: {}", cycle.join(" -> ")),
        );
    }

    r.findings
        .sort_by_key(|f| (f.level != Level::Error, f.rule));
    Ok(r)
}
