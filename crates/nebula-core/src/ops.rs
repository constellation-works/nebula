//! Everything that changes a corpus: capture, promote, drop, new, sharpen,
//! link, cite, status, tags.
//!
//! Each op takes a [`Corpus`] and typed arguments, enforces the invariants
//! that belong at the point of action, writes, and returns what changed. A
//! caller cannot produce an invalid corpus through this module; `check` exists
//! to catch hand edits, not bugs in here.
//!
//! Which invariants live here rather than in `check` is a deliberate choice
//! per rule. See `docs/design/lineage-graph/specs/invariants.md`.

use crate::check::{is_local_path, resolve_local};
use crate::error::{Error, Result};
use crate::graph;
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use crate::store::{self, Corpus, InboxEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A corpus that has just been created.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Initialized {
    /// Where it is.
    pub root: PathBuf,
}

/// A node that has just been written for the first time.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Created {
    /// The node as written.
    pub doc: Doc,
    /// The file it went into.
    pub path: PathBuf,
}

/// A node with a reference newly attached.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Cited {
    /// The node as written.
    pub doc: Doc,
    /// The id of the reference that was added.
    pub reference: String,
}

/// A node that has moved along its lifecycle.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StatusChange {
    /// The node as written.
    pub doc: Doc,
    /// Where it was before.
    pub from: Status,
}

/// Everything that goes into a node created directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewNode {
    /// One line naming the idea.
    pub title: String,
    /// Ids this descends from. More than one is a merge.
    pub parents: Vec<String>,
    /// What would falsify it. Naming one starts the node as a hypothesis.
    pub kill: Option<String>,
    /// Labels, normalised on the way in.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Everything that goes into a node promoted from the inbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Promotion {
    /// Node title. Defaults to the captured text.
    pub title: Option<String>,
    /// Ids this descends from.
    pub parents: Vec<String>,
    /// Labels.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Everything that goes into a reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Citation {
    /// Where it lives: a URL, DOI, path, or almanac wikilink.
    pub uri: String,
    /// `paper`, `study`, `article`, `note`, `discussion`, `book`, `dataset`,
    /// `thread` or `other`.
    pub kind: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// Why this is attached. The only field that matters in a year.
    pub note: Option<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Create an empty corpus, at `path` if given and the resolved root otherwise.
pub fn init(root: Option<PathBuf>, path: Option<PathBuf>) -> Result<Initialized> {
    let target = Corpus::resolve_root(path.or(root))?;
    Corpus::init(&target)?;
    Ok(Initialized { root: target })
}

/// The five-second path: append a thought to the inbox.
///
/// No parent, no title, no decisions. A capture step that requires decisions
/// is a capture step you will skip at the exact moment the idea arrives.
pub fn capture(corpus: &Corpus, text: &str) -> Result<InboxEntry> {
    corpus.capture(text)
}

/// Discard a capture, struck through rather than deleted.
pub fn drop(corpus: &Corpus, entry: &str) -> Result<InboxEntry> {
    let e = corpus.inbox_entry(entry)?;
    corpus.settle_inbox(&e, "dropped")?;
    Ok(e)
}

/// Turn an inbox entry into a seed node.
///
/// Deliberately separate from capture. Most captures should never be
/// promoted, and dropping one is a normal outcome rather than a failure.
pub fn promote(corpus: &Corpus, entry: &str, args: &Promotion) -> Result<Created> {
    let e = corpus.inbox_entry(entry)?;
    let title = args.title.clone().unwrap_or_else(|| e.text.clone());
    let doc = build(
        corpus,
        &NewNode {
            title,
            parents: args.parents.clone(),
            kill: None,
            tags: args.tags.clone(),
            origin: args.origin.clone(),
        },
        Status::Seed,
        &e.text,
    )?;
    corpus.create(&doc)?;
    corpus.settle_inbox(&e, &format!("-> {}", doc.node.id))?;
    Ok(Created {
        path: corpus.node_path(&doc.node.id),
        doc,
    })
}

/// Create a node directly, without going through the inbox.
///
/// Naming a kill condition starts it as a hypothesis; without one it is a seed.
pub fn new_node(corpus: &Corpus, args: &NewNode) -> Result<Created> {
    if args.kill.as_ref().is_some_and(|k| k.trim().is_empty()) {
        return Err(Error::EmptyKill);
    }
    let status = if args.kill.is_some() {
        Status::Hypothesis
    } else {
        Status::Seed
    };
    let doc = build(corpus, args, status, "")?;
    corpus.create(&doc)?;
    Ok(Created {
        path: corpus.node_path(&doc.node.id),
        doc,
    })
}

/// A fresh node, with its id, dates and parent edges filled in.
fn build(corpus: &Corpus, spec: &NewNode, status: Status, body: &str) -> Result<Doc> {
    let id = store::slugify(&spec.title);
    if id.is_empty() {
        return Err(Error::UnusableTitle(spec.title.clone()));
    }
    for p in &spec.parents {
        if !corpus.node_path(p).exists() {
            return Err(Error::MissingParent(p.clone()));
        }
    }
    let now = store::today();
    Ok(Doc {
        node: Node {
            id,
            title: spec.title.clone(),
            status,
            created: now.clone(),
            updated: now,
            kill: spec.kill.clone(),
            tags: model::normalize_tags(&spec.tags),
            edges: spec
                .parents
                .iter()
                .map(|p| Edge {
                    kind: EdgeType::DerivesFrom,
                    to: p.clone(),
                })
                .collect(),
            references: vec![],
            closed: None,
            origin: spec.origin.clone(),
        },
        body: body.to_string(),
    })
}

/// Sharpen a seed into a hypothesis by naming what would kill it.
pub fn sharpen(corpus: &Corpus, id: &str, kill: &str) -> Result<Doc> {
    let mut doc = corpus.load(id)?;
    if kill.trim().is_empty() {
        return Err(Error::EmptyKill);
    }
    doc.node.kill = Some(kill.to_string());
    if doc.node.status == Status::Seed {
        doc.node.status = Status::Hypothesis;
    }
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Add a typed edge between two nodes.
///
/// Returns every node that changed: a `contradicts` edge is a claim about both
/// ends, so it is recorded on both.
pub fn link(corpus: &Corpus, from: &str, kind: EdgeType, to: &str) -> Result<Vec<Doc>> {
    if from == to {
        return Err(Error::SelfLoop);
    }
    let mut doc = corpus.load(from)?;
    corpus.load(to)?;
    let edge = Edge {
        kind,
        to: to.to_string(),
    };
    if doc.node.edges.contains(&edge) {
        return Err(Error::DuplicateEdge);
    }
    doc.node.edges.push(edge);

    // Genealogy must stay acyclic, so refuse the edge that would close a
    // loop rather than leaving `check` to find it later.
    if kind.is_genealogy() {
        let mut docs = corpus.load_all()?;
        docs.retain(|d| d.node.id != doc.node.id);
        docs.push(doc.clone());
        if graph::creates_cycle(&docs, from) {
            return Err(Error::Cycle {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
    }
    corpus.save(&mut doc)?;
    let mut changed = vec![doc];

    // `contradicts` is a claim about both nodes, so record it on both.
    if kind == EdgeType::Contradicts {
        let mut other = corpus.load(to)?;
        let back = Edge {
            kind,
            to: from.to_string(),
        };
        if !other.node.edges.contains(&back) {
            other.node.edges.push(back);
            corpus.save(&mut other)?;
            changed.push(other);
        }
    }
    Ok(changed)
}

/// Attach context to a node. The note is the field that matters.
pub fn cite(corpus: &Corpus, id: &str, args: &Citation) -> Result<Cited> {
    let mut doc = corpus.load(id)?;
    // Rule 8 at the point of action: a local path that does not resolve is a
    // citation to nothing, and refusing it here is cheaper than finding it
    // in `check` after the context of why it was attached has gone.
    if is_local_path(&args.uri) && !resolve_local(corpus, &args.uri).exists() {
        return Err(Error::UnresolvedUri {
            uri: args.uri.clone(),
            from: corpus.root().join("nodes"),
        });
    }
    let reference = doc.node.next_reference_id();
    doc.node.references.push(Reference {
        id: reference.clone(),
        kind: args.kind.clone(),
        uri: args.uri.clone(),
        title: args.title.clone(),
        note: args.note.clone(),
        added: store::today(),
        origin: args.origin.clone(),
    });
    corpus.save(&mut doc)?;
    Ok(Cited { doc, reference })
}

/// Move a node to a new status, with the transition guards applied.
///
/// `why` is required for [`Status::Refuted`], optional for
/// [`Status::Abandoned`], and meaningless on an open status.
pub fn set_status(
    corpus: &Corpus,
    id: &str,
    status: Status,
    why: Option<&str>,
) -> Result<StatusChange> {
    let mut doc = corpus.load(id)?;
    let from = doc.node.status;
    let why = why.map(str::trim).filter(|w| !w.is_empty());

    // An open status is not a closing, so there is nothing for a reason to be
    // the reason for.
    if status.is_open() && why.is_some() {
        return Err(Error::corpus(
            "a reason only applies to refuted or abandoned",
        ));
    }
    // Rule 5 at the point of action: refuting is asserting the kill
    // condition fired, and that assertion has to be written down.
    if status == Status::Refuted && why.is_none() {
        return Err(Error::RefutedNeedsWhy);
    }
    if status.needs_kill() && doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        return Err(Error::NeedsKill(status));
    }
    // Rule 6: a ruled-out idea cannot quietly come back. Reviving one takes
    // a new node with a `reopens` edge, so the fact that it was once dead
    // stays visible.
    if from.is_closed_by_verdict() && status != from {
        return Err(Error::RefutedCannotReopen);
    }
    doc.node.status = status;
    doc.node.closed = match status {
        Status::Refuted => why.map(|why| Closed {
            why: why.to_string(),
            at: store::today(),
        }),
        Status::Abandoned => why
            .map(|why| Closed {
                why: why.to_string(),
                at: store::today(),
            })
            .or_else(|| doc.node.closed.take()),
        Status::Seed | Status::Hypothesis => None,
    };
    corpus.save(&mut doc)?;
    Ok(StatusChange { doc, from })
}

/// Add tags to a node, normalised to lowercase kebab-case on the way in.
pub fn tag_add(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Doc> {
    edit_tags(corpus, id, &model::normalize_tags(tags), &[])
}

/// Remove tags from a node. `--remove Physics` and `--remove physics` name
/// the same label, because both are normalised first.
pub fn tag_remove(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Doc> {
    edit_tags(corpus, id, &[], &model::normalize_tags(tags))
}

fn edit_tags(corpus: &Corpus, id: &str, add: &[String], remove: &[String]) -> Result<Doc> {
    let mut doc = corpus.load(id)?;
    let mut tags = model::normalize_tags(&doc.node.tags);
    tags.retain(|t| !remove.contains(t));
    for t in add {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    doc.node.tags = tags;
    corpus.save(&mut doc)?;
    Ok(doc)
}
