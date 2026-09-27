//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy. The rule IDs below are the source for the
//! tables in the v0.2 spec, the lineage spec, and the agent skill.
//!
//! The checker reports; it never fixes and never prints. What an error costs
//! the caller — an exit code, a red badge — is the caller's business.
//!
//! - `rules`       the rules judged one node at a time
//! - `references`  the reference kind vocabulary, and what a local URI is
//! - `observatory` where an Observatory record resolves, and rule 9
//! - `recovery`    rule 17: writes that did not finish
//!
//! This file holds the rule ids, the report, and the rules judged across
//! the whole corpus.

mod observatory;
mod recovery;
mod references;
mod rules;

#[cfg(test)]
mod tests;

use observatory::checked_observatory_setting;
pub(crate) use observatory::{is_observatory_id, resolve_observatory};
use recovery::interrupted_writes;
pub use references::{OBSERVATORY, REFERENCE_KINDS, normalize_reference_kind};
pub(crate) use references::{is_absolute_local, is_local_path, is_reference_kind, resolve_local};
use rules::{date_rules, edge_rules, lifecycle_rules, reference_rules, status_rules};

use crate::config::{OBSERVATORY_ROOT_ENV, ObservatorySource};
use crate::error::Result;
use crate::graph_impl::Graph;
use crate::model::{Doc, EdgeType};
use crate::store::{Corpus, Scan, UnreadableNode};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Stable IDs for corpus invariants. Some rules are enforced before a graph
/// reaches `check`, at parsing or at the point of action.
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Rule {
    /// Genealogy edges must form an acyclic graph.
    AcyclicGenealogy = 1,
    /// Hypotheses must name a kill condition.
    HypothesisKill = 2,
    /// Every edge must point to a known node.
    EdgeTargets = 3,
    /// Contradiction edges must be reciprocal.
    MutualContradicts = 4,
    /// A refuted node must explain why it closed.
    RefutedReason = 5,
    /// A refuted node can only be revived through a new node.
    RefutedReopens = 6,
    /// References cannot carry a verdict.
    NoReferenceVerdict = 7,
    /// Local references must resolve in the corpus.
    LocalReference = 8,
    /// Observatory references must resolve in Observatory.
    ObservatoryReference = 9,
    /// References need a note explaining their relevance.
    ReferenceNote = 10,
    /// Tags should not differ only by case or a trailing `s`.
    TagDrift = 11,
    /// Open nodes cannot carry a closure record.
    OpenNodeClosed = 12,
    /// Seeds cannot already name a kill condition.
    SeedKill = 13,
    /// Recorded dates must be valid and ordered.
    Dates = 14,
    /// Node IDs must match their filenames and valid ID syntax.
    NodeId = 15,
    /// Reference kinds outside the documented vocabulary are reported.
    ReferenceKind = 16,
    /// An interrupted write must be resolved before further changes.
    InterruptedWrite = 17,
}

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
    /// The corpus root checked, with the spelling used to open it.
    pub root: PathBuf,
    /// Files and directory entries that could not be loaded.
    pub unreadable: Vec<UnreadableNode>,
    /// Everything found, errors first.
    pub findings: Vec<Finding>,
    /// How many nodes were examined.
    pub nodes: usize,
}

impl Report {
    fn push(
        &mut self,
        level: Severity,
        rule: Rule,
        node: Option<&str>,
        message: impl Into<String>,
    ) {
        self.findings.push(Finding {
            level,
            rule: rule as u8,
            node: node.map(String::from),
            message: message.into(),
        });
    }
}

/// Run every invariant over a graph.
///
/// The corpus is needed for rules 8 and 9, which ask the filesystem whether
/// local references and Observatory records resolve.
///
/// A reference carrying a `verdict` or `strength`, or a node with an unknown
/// field, fails deserialization. [`run_scanned`] reports it as unreadable and
/// checks the remaining documents.
pub fn run_scanned(scan: &Scan, corpus: &Corpus) -> Result<Report> {
    let graph = Graph::build(&scan.docs)?;
    let mut report = run(&graph, corpus)?;
    report.unreadable.clone_from(&scan.unreadable);
    Ok(report)
}

#[derive(Clone, Copy)]
enum ObservatoryState<'a> {
    Root(&'a Path),
    Unset,
    Broken,
}

/// Run invariant checks over loaded documents. Call [`run_scanned`] when a
/// corpus scan may also contain unreadable files.
pub fn run(graph: &Graph<'_>, corpus: &Corpus) -> Result<Report> {
    let docs = graph.docs();
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report {
        root: corpus.root().to_path_buf(),
        nodes: docs.len(),
        ..Report::default()
    };
    let observatory = checked_observatory_setting(corpus, &mut r);
    let observatory_state = match observatory.as_ref() {
        Some(setting) => match setting.root.as_deref() {
            Some(root) => ObservatoryState::Root(root),
            None => ObservatoryState::Unset,
        },
        None => ObservatoryState::Broken,
    };

    for doc in docs {
        check_node(doc, &ids, corpus, observatory_state, &mut r);
    }

    // 9, for the Observatory setting itself: an `observatory_root` key in
    //    `config.yaml` is one machine's path in a file every machine shares.
    //    Still honoured as a fallback, so a warning rather than an error, and
    //    one that says whether it is the value in force here.
    if let Some(observatory) = &observatory
        && let Some(legacy) = &observatory.legacy
    {
        let shadowed = |by: &str| {
            format!(
                "config.yaml still carries the machine-specific observatory_root {}, ignored \
                 here in favour of {by}; once every machine has its own setting, remove it \
                 with `neb config observatory-root --drop-legacy`",
                legacy.display()
            )
        };
        let message = match observatory.source {
            ObservatorySource::Env => shadowed(&format!("${OBSERVATORY_ROOT_ENV}")),
            ObservatorySource::Machine => shadowed("this machine's setting"),
            ObservatorySource::Config | ObservatorySource::Unset => format!(
                "config.yaml sets observatory_root to {}, a machine-specific path in a file \
                 every machine shares; give this machine its own with `neb config \
                 observatory-root <DIR>` or ${OBSERVATORY_ROOT_ENV}",
                legacy.display()
            ),
        };
        r.push(Severity::Warn, Rule::ObservatoryReference, None, message);
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
            // Re-running the link writes the missing half and nothing else,
            // which is also how a crash between its two saves is finished.
            if !mutual {
                r.push(
                    Severity::Error,
                    Rule::MutualContradicts,
                    Some(&doc.node.id),
                    format!(
                        "contradicts `{target}`, which does not contradict back; record the \
                         other half with `neb link {} contradicts {target}`",
                        doc.node.id
                    ),
                );
            }
        }
    }

    interrupted_writes(corpus, &mut r);

    // 1. Genealogy must be acyclic: an idea cannot be its own ancestor.
    //    Diamonds are legal; only a loop is not.
    if let Some(cycle) = graph.cycle() {
        r.push(
            Severity::Error,
            Rule::AcyclicGenealogy,
            None,
            format!("genealogy cycle: {}", cycle.join(" -> ")),
        );
    }

    // 11. Two tags that differ only by case or a trailing `s` are one label
    //     drifting into two. Writes normalise case, so this mostly catches
    //     hand edits and plurals; a warning keeps drift visible without a
    //     declared list to maintain. `ops::close_tags` notes the same drift
    //     at the write that introduces it. Each variant names the nodes that
    //     carry it, since those are the files to retag.
    let carriers = tag_carriers(docs);
    let tags: Vec<(&str, &Vec<&str>)> = carriers.iter().map(|(t, ids)| (*t, ids)).collect();
    for (i, (a, on_a)) in tags.iter().enumerate() {
        for (b, on_b) in &tags[i + 1..] {
            if let Some(how) = tag_drift(a, b) {
                r.push(
                    Severity::Warn,
                    Rule::TagDrift,
                    None,
                    format!(
                        "tags `{a}` (on {}) and `{b}` (on {}) differ only by {how}",
                        on_a.join(", "),
                        on_b.join(", ")
                    ),
                );
            }
        }
    }

    r.findings
        .sort_by_key(|f| (f.level != Severity::Error, f.rule));
    Ok(r)
}

/// Every tag in the corpus, as written, with the ids of the nodes carrying
/// it in id order.
pub(crate) fn tag_carriers(docs: &[Doc]) -> BTreeMap<&str, Vec<&str>> {
    let mut carriers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for d in docs {
        for t in &d.node.tags {
            carriers
                .entry(t.as_str())
                .or_default()
                .push(d.node.id.as_str());
        }
    }
    for ids in carriers.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    carriers
}

/// How two distinct tags collide, if they do.
pub(crate) fn tag_drift(a: &str, b: &str) -> Option<&'static str> {
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

/// Every invariant that can be judged from a single node plus the id set.
fn check_node(
    doc: &Doc,
    ids: &HashSet<&str>,
    corpus: &Corpus,
    observatory: ObservatoryState<'_>,
    r: &mut Report,
) {
    status_rules(doc, r);
    lifecycle_rules(doc, r);
    date_rules(doc, r);
    edge_rules(doc, ids, r);
    reference_rules(doc, corpus, observatory, r);
}
