//! Nodes written directly: `new`, and the kill condition's `sharpen` and
//! `confirm_kill`.

use super::{Created, NewNode};
use crate::error::{Error, Result};
use crate::graph_impl as graph;
use crate::id::{is_slug, slugify};
use crate::model::{self, Doc, Edge, EdgeType, Node, Status};
use crate::stamp;
use crate::store::Corpus;

/// Create a node directly, without going through the inbox.
///
/// Naming a kill condition starts it as a hypothesis; without one it is a seed.
///
/// Every edge is written with the node, under the invariants [`link`](fn@super::link) would
/// apply to it afterwards: each end must exist, no edge is written twice, a
/// genealogy edge may not close a loop, and a `contradicts` edge is recorded
/// on the other node too. Every refusal comes before anything is written.
pub fn new_node(corpus: &Corpus, args: &NewNode) -> Result<Created> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(args.by.as_deref()))?;
    if args.kill.as_ref().is_some_and(|k| k.trim().is_empty()) {
        return Err(Error::EmptyKill);
    }
    let _lock = corpus.lock()?;
    let status = if args.kill.is_some() {
        Status::Hypothesis
    } else {
        Status::Seed
    };
    let doc = build(corpus, args, status, &args.body)?;
    // Read before the node is written, so a contradicted node that will not
    // parse refuses the whole verb rather than leaving a one-sided edge.
    let mut contradicted = args
        .contradicts
        .iter()
        .map(|id| corpus.load(id))
        .collect::<Result<Vec<_>>>()?;
    corpus.create(&doc)?;
    // `contradicts` is a claim about both nodes, so record it on both.
    let by = model::author(args.by.as_deref())?;
    for other in &mut contradicted {
        if !other.node.has_edge(EdgeType::Contradicts, &doc.node.id) {
            other.node.edges.push(Edge {
                kind: EdgeType::Contradicts,
                to: doc.node.id.clone(),
                by: by.clone(),
            });
            corpus.save(other)?;
        }
    }
    Ok(Created {
        path: corpus.node_path(&doc.node.id)?,
        doc,
        near: Vec::new(),
    })
}

/// A fresh node, with its id, dates and edges filled in.
///
/// Refuses, before anything is written, an id that is taken, an edge to a
/// node that is not there, the same edge twice, a parent that is also the
/// node this reopens, and a genealogy edge that would close a loop.
pub(super) fn build(corpus: &Corpus, spec: &NewNode, status: Status, body: &str) -> Result<Doc> {
    let id = match &spec.id {
        Some(id) => {
            if !is_slug(id) {
                return Err(Error::InvalidId(id.clone()));
            }
            id.clone()
        }
        None => slugify(&spec.title),
    };
    if id.is_empty() {
        return Err(Error::UnusableTitle(spec.title.clone()));
    }
    // Refused here as well as in `Corpus::create`, so the cycle check below
    // never has to reason about a node that is already on disk.
    if corpus.node_path(&id)?.exists() {
        return Err(Error::NodeExists(id));
    }
    for p in &spec.parents {
        if !corpus.node_path(p)?.exists() {
            return Err(Error::MissingParent(p.clone()));
        }
    }
    // Not parents, so a missing one is the refusal `link` gives.
    for other in spec.reopens.iter().chain(&spec.contradicts) {
        if !corpus.node_path(other)?.exists() {
            return Err(Error::NoSuchNode(other.clone()));
        }
    }
    // `reopens` is already genealogy: a `derives-from` beside it would be a
    // second, parallel claim of the same descent.
    if let Some(reopened) = spec.reopens.as_ref().filter(|r| spec.parents.contains(r)) {
        return Err(Error::ParentAndReopens(reopened.clone()));
    }
    let now = stamp::today();
    let by = model::author(spec.by.as_deref())?;
    let wanted = spec
        .parents
        .iter()
        .map(|p| (EdgeType::DerivesFrom, p))
        .chain(spec.reopens.iter().map(|r| (EdgeType::Reopens, r)))
        .chain(spec.contradicts.iter().map(|c| (EdgeType::Contradicts, c)));
    let mut edges: Vec<Edge> = Vec::new();
    for (kind, to) in wanted {
        if edges.iter().any(|e| e.kind == kind && e.to == *to) {
            return Err(Error::DuplicateEdge {
                from: id,
                kind,
                to: to.clone(),
            });
        }
        edges.push(Edge {
            kind,
            to: to.clone(),
            by: by.clone(),
        });
    }
    let doc = Doc {
        node: Node {
            id,
            title: spec.title.clone(),
            title_by: by.clone(),
            status,
            created: now.clone(),
            updated: now,
            kill: spec.kill.clone(),
            // A node with no kill condition has nobody to credit for one.
            kill_by: spec.kill.as_ref().and(by),
            tags: model::normalize_tags(&spec.tags),
            edges,
            references: vec![],
            closed: None,
            origin: spec.origin.clone(),
        },
        body: body.trim().to_string(),
    };
    refuse_cycle(corpus, &doc)?;
    Ok(doc)
}

/// Refuse a new node whose genealogy would make it its own ancestor.
///
/// Nothing points at a node that does not exist yet, except an edge somebody
/// wrote by hand ahead of it: that dangling edge is what one of these closes
/// into a loop. Each edge is tried alone, so the refusal names the one that
/// does.
fn refuse_cycle(corpus: &Corpus, doc: &Doc) -> Result<()> {
    let mut genealogy = doc.node.edges.iter().filter(|e| e.kind.is_genealogy());
    let Some(first) = genealogy.next() else {
        return Ok(());
    };
    // "The cycle check runs under the write lock" (4_decisions.md, STD-03@2 §R1).
    let mut docs = corpus.load_all()?;
    let mut trial = doc.clone();
    for edge in std::iter::once(first).chain(genealogy) {
        trial.node.edges = vec![edge.clone()];
        docs.push(trial.clone());
        if graph::creates_cycle(&docs, &doc.node.id) {
            return Err(Error::Cycle {
                from: doc.node.id.clone(),
                to: edge.to.clone(),
            });
        }
        docs.pop();
    }
    Ok(())
}

/// Sharpen a seed into a hypothesis by naming what would kill it.
///
/// `by` is whoever wrote the kill condition; `None` is the human.
/// An open node's existing kill is content rather than a replaceable field,
/// so a different falsifier needs a new node. A refuted node's kill is part
/// of the recorded verdict, so rewriting it is refused the same way a status
/// change is: the idea stays dead, and a new node with a `reopens` edge is the
/// way back.
pub fn sharpen(corpus: &Corpus, id: &str, kill: &str, by: Option<&str>) -> Result<Doc> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(by))?;
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    if doc.node.status.is_closed_by_verdict() {
        return Err(Error::RefutedCannotReopen);
    }
    if kill.trim().is_empty() {
        return Err(Error::EmptyKill);
    }
    if doc.node.status.is_open()
        && let Some(existing) = doc
            .node
            .kill
            .as_ref()
            .filter(|kill| !kill.trim().is_empty())
    {
        return Err(Error::KillAlreadySet(existing.clone()));
    }
    doc.node.kill_by = model::author(by)?;
    doc.node.kill = Some(kill.to_string());
    if doc.node.status == Status::Seed {
        doc.node.status = Status::Hypothesis;
    }
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Adopt an existing kill condition as the human's own.
///
/// The text is not touched and nothing is appended: this records that the
/// human read what somebody else proposed and now stands behind it, which is
/// the only thing that takes the node off `review`'s unconfirmed list.
pub fn confirm_kill(corpus: &Corpus, id: &str) -> Result<Doc> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::ConfirmKill)?;
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    if doc.node.status.is_closed_by_verdict() {
        return Err(Error::RefutedCannotReopen);
    }
    if doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        return Err(Error::NoKillToConfirm(id.to_string()));
    }
    doc.node.kill_by = None;
    corpus.save(&mut doc)?;
    Ok(doc)
}
