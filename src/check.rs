//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy.

use crate::model::{Doc, EdgeType, Status, Verdict};
use crate::store::Store;
use anyhow::Result;
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

fn is_task_id(s: &str) -> bool {
    s.strip_prefix("ORB-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn is_local_path(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("[[") && !uri.starts_with("doi:")
}

/// Every invariant that can be judged from a single node plus the id set.
fn check_node(doc: &Doc, docs: &[Doc], ids: &HashSet<&str>, store: &Store, r: &mut Report) {
    status_rules(doc, r);
    edge_rules(doc, docs, ids, r);
    attachment_rules(doc, r);
    provenance_rules(doc, store, r);
}

/// Rules about where a node sits in its lifecycle and what that costs.
fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before the
    //    evidence arrives. Without this, step four is retroactive
    //    rationalisation rather than a verdict.
    if n.status.needs_kill() && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Level::Error,
            2,
            id,
            format!("status is {} but no kill condition is named", n.status),
        );
    }

    // 3. A verdict has to rest on something.
    match n.status {
        Status::Supported if !n.has_verdict(Verdict::Supports) => {
            r.push(
                Level::Error,
                3,
                id,
                "status supported with no supporting evidence",
            );
        }
        Status::Refuted if !n.has_verdict(Verdict::Undermines) => {
            r.push(
                Level::Error,
                3,
                id,
                "status refuted with no undermining evidence",
            );
        }
        Status::Testing if n.evidence.is_empty() => {
            r.push(
                Level::Error,
                3,
                id,
                "status testing with no evidence at all",
            );
        }
        _ => {}
    }
}

/// Rules about the links a node declares.
fn edge_rules(doc: &Doc, docs: &[Doc], ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 4. Dangling edges. A lineage that points into nothing is worse than
    //    no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Level::Error,
                4,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Level::Error,
                4,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }

    // 7. A refuted node stays refuted unless something explicitly reopens
    //    it, which keeps the fact that it once died visible.
    if n.status.is_open() {
        let revived = docs
            .iter()
            .any(|d| d.node.edges_of(EdgeType::Reopens).any(|t| t == n.id));
        let was_refuted = n.evidence.iter().any(|e| e.verdict == Verdict::Undermines)
            && n.status == Status::Supported;
        if was_refuted && !revived {
            r.push(
                Level::Warn,
                7,
                id,
                "supported despite undermining evidence; say why, or reopen properly",
            );
        }
    }

    // 8. Graduating without saying where breaks the lineage at exactly the
    //    boundary it was supposed to cross.
    if n.status == Status::Graduated && n.graduated_to.is_none() {
        r.push(Level::Error, 8, id, "graduated but graduated_to is empty");
    }
}

/// Rules about evidence and references hanging off a node.
fn attachment_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 12. Ids are unique within a node and never reused.
    let mut seen = HashSet::new();
    for f in &n.references {
        if !seen.insert(f.id.as_str()) {
            r.push(
                Level::Error,
                12,
                id,
                format!("duplicate reference id `{}`", f.id),
            );
        }
        // 11. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Level::Warn,
                11,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
    }
    let mut seen = HashSet::new();
    for e in &n.evidence {
        if !seen.insert(e.id.as_str()) {
            r.push(
                Level::Error,
                12,
                id,
                format!("duplicate evidence id `{}`", e.id),
            );
        }
    }
}

/// Rules about where a node and its attachments came from.
fn provenance_rules(doc: &Doc, store: &Store, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 13. A malformed task id is provenance that cannot be resolved, which
    //     is indistinguishable from no provenance at the moment you need it.
    let tasks = n.tasks.iter().map(|t| t.id.as_str());
    let origins = n.origin.as_ref().and_then(|o| o.task.as_deref());
    for t in tasks.chain(origins) {
        if !is_task_id(t) {
            r.push(
                Level::Error,
                13,
                id,
                format!("malformed Orbit task id `{t}` (expected ORB-nnnnn)"),
            );
        }
    }

    // 14. A local path that does not resolve is a citation to nothing.
    //     External URLs are not fetched; `check` stays offline and fast.
    let local = n
        .evidence
        .iter()
        .map(|e| (e.id.as_str(), e.source.as_str()))
        .chain(n.references.iter().map(|f| (f.id.as_str(), f.uri.as_str())));
    for (eid, uri) in local {
        if is_local_path(uri) {
            let base = store.node_path(&n.id);
            let resolved = base.parent().map(|p| p.join(uri));
            if resolved.is_some_and(|p| !p.exists()) {
                r.push(
                    Level::Warn,
                    14,
                    id,
                    format!("`{eid}` points at a path that does not resolve: {uri}"),
                );
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    OnStack,
    Done,
}

fn dfs<'a>(
    at: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    state: &mut HashMap<&'a str, Mark>,
    stack: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    state.insert(at, Mark::OnStack);
    stack.push(at);
    for next in graph.get(at).map(Vec::as_slice).unwrap_or_default() {
        match state.get(next) {
            Some(Mark::OnStack) => {
                let from = stack.iter().position(|s| s == next).unwrap_or(0);
                let mut c: Vec<String> = stack[from..].iter().map(ToString::to_string).collect();
                c.push((*next).to_string());
                return Some(c);
            }
            None => {
                if let Some(c) = dfs(next, graph, state, stack) {
                    return Some(c);
                }
            }
            Some(Mark::Done) => {}
        }
    }
    stack.pop();
    state.insert(at, Mark::Done);
    None
}

/// First cycle in a directed graph, as a readable path.
fn find_cycle(graph: &HashMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    let mut state: HashMap<&str, Mark> = HashMap::new();
    let mut stack: Vec<&str> = Vec::new();
    // Sorted so the reported cycle does not change between runs on the same
    // corpus, which matters when the output goes into a review file.
    let mut roots: Vec<&&str> = graph.keys().collect();
    roots.sort_unstable();
    for start in roots {
        if !state.contains_key(*start) {
            if let Some(c) = dfs(start, graph, &mut state, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}
