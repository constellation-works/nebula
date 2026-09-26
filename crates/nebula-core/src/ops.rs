//! Everything that changes a corpus: capture, promote, drop, new, sharpen,
//! link, cite, note, status, handoff, tags, and the commit that may follow
//! any of them.
//!
//! Each op takes a [`Corpus`] and typed arguments, enforces the invariants
//! that belong at the point of action, writes, and returns what changed. A
//! caller cannot produce an invalid corpus through this module; `check` exists
//! to catch hand edits, not bugs in here.
//!
//! [`commit`] is its own op rather than the tail of every other one, so a
//! write's result reaches the caller even when git then refuses: the write is
//! never rolled back because of git, and the caller can tell the two apart.
//!
//! Every op here that writes takes the corpus write lock first and holds it
//! until it returns, because three processes share one corpus and none of
//! these is a single atomic file replacement: they are read-modify-write
//! across steps, sometimes across two files, sometimes followed by a commit.
//! A caller that wants one critical section over a verb *and* the [`commit`]
//! that records it takes [`Corpus::lock`] itself and holds it across both;
//! the lock is re-entrant on one thread, so the op still taking it underneath
//! costs nothing. Queries take nothing: [`suggest`], [`close_tags`] and
//! everything in [`crate::graph`] read a corpus that a writer may be part-way
//! through, and that is the trade the lock exists to make — writers wait,
//! readers never do.
//!
//! Which invariants live here rather than in `check` is a deliberate choice
//! per rule. See `docs/design/lineage-graph/specs/invariants.md`.

use crate::check::{
    self, OBSERVATORY, is_absolute_local, is_local_path, is_observatory_id, is_reference_kind,
    normalize_reference_kind, resolve_local, resolve_observatory,
};
use crate::config::{CommitSetting, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::fs::create_private_dir_all;
use crate::graph::{self, Graph, Neighbour};
use crate::lock::CorpusLock;
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use crate::store::{self, CommitOutcome, Corpus, InboxEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Existing nodes the new one reads closest to, best first, for whoever
    /// is deciding whether it has a parent. Filled by [`promote`] when no
    /// parent was named; empty when one was, and always empty from
    /// [`new_node`]. A suggestion only: nothing here becomes an edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Neighbour>,
}

/// A thought that has just gone into the inbox, and where it may belong.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Captured {
    /// The entry as written.
    pub entry: InboxEntry,
    /// Existing nodes the text reads closest to, best first, for the triage
    /// that comes later. Empty when nothing shares a word with it, or when
    /// the caller asked for none. A suggestion only: nothing here becomes
    /// an edge, and the capture is in the inbox whatever this holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Neighbour>,
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

/// A tag a write has just introduced that reads as a variant of one already
/// in use: the same label but for case or a trailing `s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CloseTag {
    /// The tag as the write stored it.
    pub tag: String,
    /// The tag already in use that it collides with, as written there.
    pub near: String,
    /// How many nodes carry `near`.
    pub nodes: usize,
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

/// A node handed off to an Observatory record: cited and closed in one write.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HandedOff {
    /// The node as written: `abandoned`, with the hand-off as its reason.
    pub doc: Doc,
    /// The id of the `observatory` reference that was added.
    pub reference: String,
    /// The record it went to, as stored: `H012`.
    pub record: String,
    /// Where the node was before.
    pub from: Status,
}

/// Everything that goes into a hand-off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Handoff {
    /// The Observatory record id the node becomes, such as `H012`. Case is
    /// normalised up, as `cite --kind observatory` does.
    pub record: String,
    /// Why it goes there: the note on the `observatory` reference.
    pub note: Option<String>,
    /// Who attached the reference and wrote the note. `None` is the human.
    pub by: Option<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Everything that goes into a node created directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewNode {
    /// One line naming the idea.
    pub title: String,
    /// The argument, sketch or thought itself.
    pub body: String,
    /// Ids this descends from. More than one is a merge.
    pub parents: Vec<String>,
    /// A refuted node this revives, written as a `reopens` edge. That edge is
    /// genealogy, so the same id is not also a parent.
    #[serde(default)]
    pub reopens: Option<String>,
    /// Nodes this contradicts. Recorded on both ends, as [`link`] does.
    #[serde(default)]
    pub contradicts: Vec<String>,
    /// What would falsify it. Naming one starts the node as a hypothesis.
    pub kill: Option<String>,
    /// Labels, normalised on the way in.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
    /// Explicit id, overriding the title's slug. Validated with the same
    /// rules a derived slug already follows, and refused on collision.
    pub id: Option<String>,
    /// Who wrote the title, the kill condition and the edges. `None` is the
    /// human, which is what an unattributed write means.
    pub by: Option<String>,
}

/// Everything that goes into a node promoted from the inbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Promotion {
    /// Node title. Defaults to the captured text.
    pub title: Option<String>,
    /// Prose appended after the captured text.
    pub body: String,
    /// Ids this descends from.
    pub parents: Vec<String>,
    /// Labels.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
    /// Explicit id, overriding the title's slug. Validated with the same
    /// rules a derived slug already follows, and refused on collision. With
    /// neither this nor `title`, the id is a shortened slug of the captured
    /// text rather than the slug of the whole sentence.
    pub id: Option<String>,
    /// Who wrote the title and chose the parents. `None` is the human.
    pub by: Option<String>,
}

/// Everything that goes into a reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Citation {
    /// Where it lives: a URL, DOI, path relative to `nodes/`, almanac
    /// wikilink, or — with kind `observatory` — a bare record id. Optional
    /// only for a discussion.
    pub uri: Option<String>,
    /// `paper`, `study`, `article`, `note`, `discussion`, `book`, `dataset`,
    /// `thread`, `observatory` or `other`, in any case: it is lowercased
    /// before it is checked.
    pub kind: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// Why this is attached. The only field that matters in a year.
    pub note: Option<String>,
    /// Who attached it and wrote the note. `None` is the human.
    pub by: Option<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Create an empty corpus, at `path` if given and the resolved root otherwise.
/// Optionally make it the machine-local default, refusing to replace a
/// different setting unless `force` is set.
///
/// With `set_root`, the machine-setting lock is held from before the check
/// until after the write, so two of these cannot both find the setting free
/// and then both write it. It is taken first, before anything is created,
/// and the corpus lock inside it: the order `lock.rs` documents.
pub fn init(
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Result<Initialized> {
    let target = Corpus::resolve_root(path.or(root))?;
    let _settings = set_root.then(Corpus::lock_machine_settings).transpose()?;
    if set_root {
        Corpus::check_root_config(&target, force)?;
    }
    // The lock lives inside the root, so the root has to exist before it can
    // be taken. `Corpus::init` would create it a moment later anyway.
    create_private_dir_all(&target)?;
    let _lock = CorpusLock::acquire(&target)?;
    Corpus::init(&target)?;
    if set_root {
        Corpus::write_root_config(&target, force)?;
    }
    Ok(Initialized { root: target })
}

/// The five-second path: append a thought to the inbox.
///
/// No parent, no title, no decisions. A capture step that requires decisions
/// is a capture step you will skip at the exact moment the idea arrives.
pub fn capture(corpus: &Corpus, text: &str) -> Result<InboxEntry> {
    let _lock = corpus.lock()?;
    corpus.capture(text)
}

/// [`capture`], then the `k` existing nodes the text reads closest to.
///
/// The capture lands first and is never undone: the suggestions are for the
/// triage that follows, and a corpus that cannot be read for them is an
/// error reported *after* the write, the same way a refused commit is. `k`
/// of zero skips the read altogether, which is what `--quiet` means. A
/// caller that wants the entry id out before that read happens runs
/// [`capture`] and then [`suggest`] itself.
pub fn capture_near(corpus: &Corpus, text: &str, k: usize) -> Result<Captured> {
    // Only the capture is locked. The suggestions are a read, and holding a
    // writer's lock over one would make every other writer wait on it.
    let entry = capture(corpus, text)?;
    let near = suggest(corpus, &entry.text, k)?;
    Ok(Captured { entry, near })
}

/// The `k` nodes closest to `text`, over the corpus as it is on disk now.
///
/// What [`capture_near`] and [`promote`] run, read before the node a caller
/// is about to write so a promotion is not its own nearest neighbour. It
/// reads `nodes/`, which a capture never needs, so a node file that will not
/// parse fails here and not the capture.
pub fn suggest(corpus: &Corpus, text: &str, k: usize) -> Result<Vec<Neighbour>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    let docs = corpus.load_all()?;
    Ok(graph::near(&Graph::build(&docs)?, text, k)?.0)
}

/// Discard a capture, struck through rather than deleted.
pub fn drop(corpus: &Corpus, entry: &str) -> Result<InboxEntry> {
    let _lock = corpus.lock()?;
    let e = corpus.inbox_entry(entry)?;
    corpus.settle_inbox(&e, "dropped")?;
    Ok(e)
}

/// Turn an inbox entry into a seed node.
///
/// Deliberately separate from capture. Most captures should never be
/// promoted, and dropping one is a normal outcome rather than a failure.
///
/// Without a parent, `near` on the result names the existing nodes the
/// title and captured text read closest to, so the human can see whether
/// a parent was defensible. The node is written as a root either way: the
/// suggestion never blocks the promotion and never becomes an edge. `near_k`
/// is how many to look for; zero looks for none.
pub fn promote(corpus: &Corpus, entry: &str, args: &Promotion, near_k: usize) -> Result<Created> {
    // Held across the whole verb: the node is written and *then* the inbox
    // line is struck, and a capture landing between the two would shift the
    // line this entry was found at.
    let _lock = corpus.lock()?;
    let e = corpus.inbox_entry(entry)?;
    let title = args.title.clone().unwrap_or_else(|| e.text.clone());
    // An explicit id or title decides the id as it always has. Only an id
    // minted from the raw capture is shortened; see `store::capture_ids`.
    let id = match (&args.id, &args.title) {
        (Some(id), _) => Some(id.clone()),
        (None, Some(_)) => None,
        (None, None) => free_capture_id(corpus, &e.text)?,
    };
    // Read before the write, so the new node is not among its own neighbours.
    let near = if args.parents.is_empty() {
        suggest(corpus, &format!("{title}\n{}", e.text), near_k)?
    } else {
        Vec::new()
    };
    let body = if args.body.trim().is_empty() {
        e.text.clone()
    } else {
        format!("{}\n\n{}", e.text, args.body.trim())
    };
    let mut doc = build(
        corpus,
        &NewNode {
            title,
            body: String::new(),
            parents: args.parents.clone(),
            reopens: None,
            contradicts: Vec::new(),
            kill: None,
            tags: args.tags.clone(),
            origin: args.origin.clone(),
            id,
            by: args.by.clone(),
        },
        Status::Seed,
        &body,
    )?;
    // A capture promoted as it was captured is titled in the human's own
    // words. Whoever ran the verb authored the edges, not that sentence.
    if args.title.is_none() {
        doc.node.title_by = None;
    }
    corpus.create(&doc)?;
    corpus.settle_inbox(&e, &format!("-> {}", doc.node.id))?;
    Ok(Created {
        path: corpus.node_path(&doc.node.id)?,
        doc,
        near,
    })
}

/// The id for a capture promoted with neither a title nor an id: the first
/// of [`store::capture_ids`] no node has taken.
///
/// A candidate is passed over only when the node holding it is a different
/// idea. One titled with this very text is the same thought already
/// promoted, so its id is returned and the create refuses it as
/// [`Error::NodeExists`], exactly as before ids were shortened: a duplicate
/// node in a lineage graph is worse than a refusal. When every candidate is
/// another idea's, the last — the full slug — is returned and refused the
/// same way. `None` when the text reduces to no id, which the build refuses
/// as [`Error::UnusableTitle`]. Called under the corpus lock, so nothing can
/// take the id between this check and the write.
fn free_capture_id(corpus: &Corpus, text: &str) -> Result<Option<String>> {
    let candidates = store::capture_ids(text);
    for id in &candidates {
        if !corpus.node_path(id)?.exists() {
            return Ok(Some(id.clone()));
        }
        if corpus.load(id)?.node.title.trim() == text.trim() {
            return Ok(Some(id.clone()));
        }
    }
    Ok(candidates.last().cloned())
}

/// Create a node directly, without going through the inbox.
///
/// Naming a kill condition starts it as a hypothesis; without one it is a seed.
///
/// Every edge is written with the node, under the invariants [`link`] would
/// apply to it afterwards: each end must exist, no edge is written twice, a
/// genealogy edge may not close a loop, and a `contradicts` edge is recorded
/// on the other node too. Every refusal comes before anything is written.
pub fn new_node(corpus: &Corpus, args: &NewNode) -> Result<Created> {
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
fn build(corpus: &Corpus, spec: &NewNode, status: Status, body: &str) -> Result<Doc> {
    let id = match &spec.id {
        Some(id) => {
            if !store::is_slug(id) {
                return Err(Error::InvalidId(id.clone()));
            }
            id.clone()
        }
        None => store::slugify(&spec.title),
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
    let now = store::today();
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
            return Err(Error::DuplicateEdge);
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
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    if doc.node.status.is_closed_by_verdict() {
        return Err(Error::RefutedCannotReopen);
    }
    if doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        return Err(Error::corpus(
            "there is no kill condition to confirm; name one with --kill",
        ));
    }
    doc.node.kill_by = None;
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Add a typed edge between two nodes.
///
/// Returns every node that changed: a `contradicts` edge is a claim about both
/// ends, so it is recorded on both.
///
/// `by` is whoever claims the relation; `None` is the human.
pub fn link(
    corpus: &Corpus,
    from: &str,
    kind: EdgeType,
    to: &str,
    by: Option<&str>,
) -> Result<Vec<Doc>> {
    if from == to {
        return Err(Error::SelfLoop);
    }
    // Held across both ends. A `contradicts` edge is saved on `from` and then
    // on `to`, and the cycle check reads every node before saving one; either
    // gap is where a second writer leaves a one-sided edge or closes a loop
    // this check did not see.
    let _lock = corpus.lock()?;
    let by = model::author(by)?;
    let mut doc = corpus.load(from)?;
    corpus.load(to)?;
    if doc.node.has_edge(kind, to) {
        return Err(Error::DuplicateEdge);
    }
    doc.node.edges.push(Edge {
        kind,
        to: to.to_string(),
        by: by.clone(),
    });

    // Genealogy must stay acyclic, so refuse the edge that would close a
    // loop rather than leaving `check` to find it later.
    if kind.is_genealogy() {
        // "The cycle check runs under the write lock" (4_decisions.md, STD-03@2 §R1).
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
        if !other.node.has_edge(kind, from) {
            other.node.edges.push(Edge {
                kind,
                to: from.to_string(),
                by,
            });
            corpus.save(&mut other)?;
            changed.push(other);
        }
    }
    Ok(changed)
}

/// Append a dated paragraph of reasoning to a node body.
///
/// Creates a `## Notes` section at the end of the body if needed, then
/// appends `- YYYY-MM-DD: <text>`. Earlier body text, status, edges and tags
/// are left as they are. `updated` is stamped by [`Corpus::save`].
///
/// `by` is whoever wrote the paragraph; `None` is the human, whose line
/// carries no attribution.
pub fn note(corpus: &Corpus, id: &str, text: &str, by: Option<&str>) -> Result<Doc> {
    let text = text
        .trim()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Err(Error::corpus("a note cannot be empty"));
    }
    let _lock = corpus.lock()?;
    let by = model::author(by)?;
    let mut doc = corpus.load(id)?;
    doc.body = model::append_note(&doc.body, &store::today(), &text, by.as_deref());
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Replace a node's prose body, leaving its structured fields alone.
///
/// `by` is validated for consistency with other authored writes, but is not
/// stored: bodies do not yet carry per-field authorship. `updated` is stamped
/// by [`Corpus::save`]. Callers that expose free-form editing must preserve
/// append-only sections before calling this operation.
pub fn set_body(corpus: &Corpus, id: &str, body: &str, by: Option<&str>) -> Result<Doc> {
    let _lock = corpus.lock()?;
    model::author(by)?;
    let mut doc = corpus.load(id)?;
    doc.body = body.trim().to_string();
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Attach context to a node. The note is the field that matters.
///
/// An `observatory` reference stores a bare record id rather than a
/// location, so the citation says the same thing on every machine. Whether
/// that record is on *this* machine is a question for `check`, which warns
/// rather than refuses: a checkout that is not there yet is not a broken
/// citation.
///
/// A local reference is a path relative to `nodes/`, for the same reason:
/// an absolute path or a `file:` URI is refused as [`Error::AbsoluteUri`]
/// even when it exists here, and a relative one that does not resolve as
/// [`Error::UnresolvedUri`].
pub fn cite(corpus: &Corpus, id: &str, args: &Citation) -> Result<Cited> {
    let _lock = corpus.lock()?;
    let by = model::author(args.by.as_deref())?;
    let mut doc = corpus.load(id)?;
    let reference = attach(corpus, &mut doc.node, args, by)?;
    corpus.save(&mut doc)?;
    Ok(Cited { doc, reference })
}

/// Validate a citation and add it to `node` as its next reference, without
/// saving. Returns the new reference's id. Shared by [`cite`] and
/// [`handoff`], so a hand-off's reference is held to exactly the rules a
/// citation is.
fn attach(corpus: &Corpus, node: &mut Node, args: &Citation, by: Option<String>) -> Result<String> {
    // Case is normalised the way tags are: `Paper` is a spelling of `paper`,
    // not a new kind. Anything still outside the vocabulary is refused.
    let kind = normalize_reference_kind(&args.kind);
    let uri = args
        .uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .map(str::to_owned);
    if uri.is_none() && kind != "discussion" {
        return Err(Error::corpus(
            "--uri is required unless --kind is discussion",
        ));
    }
    if !is_reference_kind(&kind) {
        return Err(Error::UnknownReferenceKind(args.kind.clone()));
    }
    // An Observatory record id is checked for its shape at the point of
    // action, because a path or a slug stored here would never resolve and
    // the mistake is obvious now and cryptic later.
    let uri = match (kind.as_str(), uri.as_deref()) {
        (OBSERVATORY, Some(record)) => Some(observatory_record(record)?),
        _ => uri,
    };
    // Rule 8, before resolving: an absolute path or a `file:` URI may well
    // exist on this machine, and that is the trap. The corpus is synced, so
    // on every other machine it names nothing, and it leaks this one's
    // layout into the corpus. Refused whether or not it resolves here.
    if let Some(uri) = uri.as_deref().filter(|uri| is_absolute_local(uri)) {
        return Err(Error::AbsoluteUri(uri.to_string()));
    }
    // Rule 8 at the point of action: a local path that does not resolve is a
    // citation to nothing, and refusing it here is cheaper than finding it
    // in `check` after the context of why it was attached has gone.
    if let Some(uri) = uri
        .as_deref()
        .filter(|_| kind != OBSERVATORY)
        .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
    {
        return Err(Error::UnresolvedUri {
            uri: uri.to_string(),
            from: corpus.root().join("nodes"),
        });
    }
    let reference = node.next_reference_id();
    node.references.push(Reference {
        id: reference.clone(),
        kind,
        uri,
        title: args.title.clone(),
        note: args.note.clone(),
        added: store::today(),
        by,
        origin: args.origin.clone(),
    });
    Ok(reference)
}

/// An Observatory record id as it is stored: trimmed and upper-cased, or
/// [`Error::InvalidObservatoryId`] when it is not one.
fn observatory_record(raw: &str) -> Result<String> {
    let record = raw.trim().to_ascii_uppercase();
    if !is_observatory_id(&record) {
        return Err(Error::InvalidObservatoryId(record));
    }
    Ok(record)
}

/// Hand a node off to an Observatory record: cite the record and close the
/// node as [`Status::Abandoned`] with `closed.why` reading
/// `handed off to <record>`, in one save.
///
/// What was two verbs (`cite --kind observatory`, then `status abandoned`)
/// is one write here, so no reader, and no failure between the two, ever
/// sees a node that is cited but open or closed but uncited.
///
/// `observatory` is the root this machine resolves records under, as
/// [`Corpus::observatory_root`] reports it; the caller reads it, as it does
/// for [`crate::graph::NodeView::with_observatory`], and so can tell the
/// human where the record is. With a root, the record must resolve under
/// it, or the hand-off is refused as [`Error::UnresolvedObservatoryRecord`]:
/// closing a node for a record that is not there would point its lineage at
/// nothing. With none, the id is accepted on its shape alone, as `cite`
/// accepts it, and `check` keeps warning until this machine has a root.
///
/// Refuses, before anything is written, an unknown node, one that is
/// already closed ([`Error::AlreadyClosed`]: refuted is a verdict, and an
/// abandoned node's reason would be replaced), and a record id of the wrong
/// shape.
pub fn handoff(
    corpus: &Corpus,
    id: &str,
    args: &Handoff,
    observatory: Option<&Path>,
) -> Result<HandedOff> {
    let _lock = corpus.lock()?;
    let by = model::author(args.by.as_deref())?;
    let mut doc = corpus.load(id)?;
    let from = doc.node.status;
    if !from.is_open() {
        return Err(Error::AlreadyClosed {
            id: id.to_string(),
            status: from,
        });
    }
    let record = observatory_record(&args.record)?;
    if let Some(root) = observatory.filter(|root| resolve_observatory(root, &record).is_none()) {
        return Err(Error::UnresolvedObservatoryRecord {
            record,
            root: root.to_path_buf(),
        });
    }
    let reference = attach(
        corpus,
        &mut doc.node,
        &Citation {
            uri: Some(record.clone()),
            kind: OBSERVATORY.to_string(),
            title: None,
            note: args.note.clone(),
            by: None,
            origin: args.origin.clone(),
        },
        by,
    )?;
    // Abandoned asks for no kill condition, and an open node has no verdict
    // to protect, so this is the move `set_status` would allow.
    doc.node.status = Status::Abandoned;
    doc.node.closed = Some(Closed {
        why: model::handoff_why(&record),
        at: store::today(),
    });
    corpus.save(&mut doc)?;
    Ok(HandedOff {
        doc,
        reference,
        record,
        from,
    })
}

/// Record where the Observatory checkout is on this machine, in
/// `~/.config/nebula/observatory-root`. Nothing under the corpus changes, so
/// no corpus lock is taken: the checkout's path belongs to the machine, and
/// the corpus travels between machines. The machine-setting lock is, so this
/// write is serialized with every other one to `~/.config/nebula`.
///
/// Returns the setting as `corpus` now resolves it, which is the machine
/// setting unless `$OBSERVATORY_ROOT` outranks it.
pub fn set_observatory_root(corpus: &Corpus, dir: &Path) -> Result<ObservatoryRoot> {
    let _settings = Corpus::lock_machine_settings()?;
    Corpus::write_observatory_root_config(dir)?;
    corpus.observatory_root()
}

/// Remove the legacy `observatory_root` key from the corpus's `config.yaml`,
/// the one place a machine path was ever stored in the corpus. A no-op when
/// the file does not carry it.
///
/// Returns the setting as it now resolves, without the key.
pub fn drop_legacy_observatory_root(corpus: &mut Corpus) -> Result<ObservatoryRoot> {
    let _lock = corpus.lock()?;
    corpus.drop_legacy_observatory_root()?;
    corpus.observatory_root()
}

/// Record in the corpus's `config.yaml` whether each write is committed.
///
/// Returns the setting as it now stands.
pub fn set_commit(corpus: &mut Corpus, enabled: bool) -> Result<CommitSetting> {
    let _lock = corpus.lock()?;
    corpus.set_commit(enabled)?;
    Ok(corpus.commit_setting())
}

/// Commit the corpus after a successful write, as `neb <verb> <ids>`.
///
/// Does nothing unless `commit: true` is set in `config.yaml` and the root is
/// inside a git work tree, and the [`CommitOutcome`] says which of those it
/// was. Stages and commits only `nodes/`, `inbox/`, `config.yaml` and the
/// generated `.gitignore` under the root, by pathspec, so anything else staged
/// in the repository stays staged and out of the commit. Never pushes. A
/// repository git cannot read is [`Error::Git`], not a skipped commit. Called
/// after the write it records, which stays on disk whatever happens here.
pub fn commit(corpus: &Corpus, verb: &str, ids: &[&str]) -> Result<CommitOutcome> {
    // Two commits racing would race on git's index. Taking the lock here
    // covers a caller that commits on its own; a caller that already holds it
    // from the write this records re-enters, which is the point.
    let _lock = corpus.lock()?;
    corpus.commit(verb, ids)
}

/// Move a node to a new status, with the transition guards applied.
///
/// `why` is required for [`Status::Refuted`], optional for
/// [`Status::Abandoned`], and meaningless on an open status. A node that
/// names a kill condition cannot move to [`Status::Seed`]; it reopens as a
/// [`Status::Hypothesis`] instead.
pub fn set_status(
    corpus: &Corpus,
    id: &str,
    status: Status,
    why: Option<&str>,
) -> Result<StatusChange> {
    let _lock = corpus.lock()?;
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
    // Rule 6: a ruled-out idea cannot quietly come back, and a verdict is
    // part of the record: even refuted -> refuted is refused, because it
    // would silently replace `closed.why` and its date rather than leaving
    // them as the recorded verdict. Reviving one takes a new node with a
    // `reopens` edge, so the fact that it was once dead stays visible.
    if from.is_closed_by_verdict() {
        return Err(Error::RefutedCannotReopen);
    }
    // Rule 13 at the point of action: nothing is deleted, so a move to seed
    // would keep the kill condition, and a seed carrying one is what `check`
    // reads as a hand edit. A node that already names its falsifier is open
    // as a hypothesis, which is the honest way back.
    if status == Status::Seed && doc.node.kill.is_some() {
        return Err(Error::SeedWithKill);
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

/// Which of `tags` on node `id` read as a variant of a tag already in use.
///
/// A tag counts only when this write introduced it to the corpus: no node
/// but `id` carries it. It is then compared with every other tag in the
/// corpus, by the rule `check` warns on (case, or a trailing `s`), and each
/// collision is returned with the number of nodes carrying the existing
/// variant. `tags` are normalised first, so a caller passes what it was
/// given.
///
/// Run after the write, over the corpus as it now is. A read, not a guard:
/// there is no declared list to refuse against, so nothing here stops a
/// write, and an empty result says only that nothing collided.
pub fn close_tags(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Vec<CloseTag>> {
    let tags = model::normalize_tags(tags);
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    let docs = corpus.load_all()?;
    let carriers = check::tag_carriers(&docs);
    let mut close = Vec::new();
    for tag in &tags {
        let introduced = carriers
            .get(tag.as_str())
            .is_none_or(|ids| ids.iter().all(|carrier| *carrier == id));
        if !introduced {
            continue;
        }
        for (existing, ids) in &carriers {
            if *existing != tag.as_str() && check::tag_drift(tag, existing).is_some() {
                close.push(CloseTag {
                    tag: tag.clone(),
                    near: (*existing).to_string(),
                    nodes: ids.len(),
                });
            }
        }
    }
    Ok(close)
}

fn edit_tags(corpus: &Corpus, id: &str, add: &[String], remove: &[String]) -> Result<Doc> {
    // Both [`tag_add`] and [`tag_remove`] come through here, so the lock does
    // too. Without it the load and the save are two moments, and a second
    // process editing the same node between them loses one edit entirely.
    let _lock = corpus.lock()?;
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
