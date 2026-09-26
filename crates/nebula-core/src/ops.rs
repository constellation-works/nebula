//! Everything that changes a corpus: capture, promote, drop, new, sharpen,
//! link, cite, note, status, tags, and the commit that may follow any of them.
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
//! costs nothing. Queries take nothing: [`suggest`] and everything in
//! [`crate::graph`] read a corpus that a writer may be part-way through, and
//! that is the trade the lock exists to make — writers wait, readers never do.
//!
//! Which invariants live here rather than in `check` is a deliberate choice
//! per rule. See `docs/design/lineage-graph/specs/invariants.md`.

use crate::check::{
    OBSERVATORY, REFERENCE_KINDS, is_absolute_local, is_local_path, is_observatory_id,
    is_reference_kind, resolve_local,
};
use crate::config::{CommitSetting, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::graph::{self, Graph, Neighbour};
use crate::lock::CorpusLock;
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use crate::store::{self, Committed, Corpus, InboxEntry};
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
    /// The argument, sketch or thought itself.
    pub body: String,
    /// Ids this descends from. More than one is a merge.
    pub parents: Vec<String>,
    /// What would falsify it. Naming one starts the node as a hypothesis.
    pub kill: Option<String>,
    /// Labels, normalised on the way in.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
    /// Explicit id, overriding the title's slug. Validated with the same
    /// rules a derived slug already follows, and refused on collision.
    pub id: Option<String>,
    /// Who wrote the title, the kill condition and the parent edges. `None`
    /// is the human, which is what an unattributed write means.
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
    /// rules a derived slug already follows, and refused on collision.
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
    /// `thread`, `observatory` or `other`.
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
pub fn init(
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Result<Initialized> {
    let target = Corpus::resolve_root(path.or(root))?;
    if set_root {
        Corpus::check_root_config(&target, force)?;
    }
    // The lock lives inside the root, so the root has to exist before it can
    // be taken. `Corpus::init` would create it a moment later anyway.
    std::fs::create_dir_all(&target).map_err(|error| Error::io_at("creating", &target, error))?;
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
            kill: None,
            tags: args.tags.clone(),
            origin: args.origin.clone(),
            id: args.id.clone(),
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

/// Create a node directly, without going through the inbox.
///
/// Naming a kill condition starts it as a hypothesis; without one it is a seed.
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
    corpus.create(&doc)?;
    Ok(Created {
        path: corpus.node_path(&doc.node.id)?,
        doc,
        near: Vec::new(),
    })
}

/// A fresh node, with its id, dates and parent edges filled in.
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
    for p in &spec.parents {
        if !corpus.node_path(p)?.exists() {
            return Err(Error::MissingParent(p.clone()));
        }
    }
    let now = store::today();
    let by = model::author(spec.by.as_deref())?;
    Ok(Doc {
        node: Node {
            id,
            title: spec.title.clone(),
            title_by: by.clone(),
            status,
            created: now.clone(),
            updated: now,
            kill: spec.kill.clone(),
            // A node with no kill condition has nobody to credit for one.
            kill_by: spec.kill.as_ref().and(by.clone()),
            tags: model::normalize_tags(&spec.tags),
            edges: spec
                .parents
                .iter()
                .map(|p| Edge {
                    kind: EdgeType::DerivesFrom,
                    to: p.clone(),
                    by: by.clone(),
                })
                .collect(),
            references: vec![],
            closed: None,
            origin: spec.origin.clone(),
        },
        body: body.trim().to_string(),
    })
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
    let uri = args
        .uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .map(str::to_owned);
    if uri.is_none() && args.kind != "discussion" {
        return Err(Error::corpus(
            "--uri is required unless --kind is discussion",
        ));
    }
    if !is_reference_kind(&args.kind) {
        return Err(Error::corpus(format!(
            "`{}` is not an accepted reference kind; accepted kinds: {}",
            args.kind,
            REFERENCE_KINDS.join(", ")
        )));
    }
    // An Observatory record id is checked for its shape at the point of
    // action, because a path or a slug stored here would never resolve and
    // the mistake is obvious now and cryptic later.
    let uri = match (args.kind.as_str(), uri.as_deref()) {
        (OBSERVATORY, Some(record)) => {
            let record = record.trim().to_ascii_uppercase();
            if !is_observatory_id(&record) {
                return Err(Error::InvalidObservatoryId(record));
            }
            Some(record)
        }
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
        .filter(|_| args.kind != OBSERVATORY)
        .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
    {
        return Err(Error::UnresolvedUri {
            uri: uri.to_string(),
            from: corpus.root().join("nodes"),
        });
    }
    let reference = doc.node.next_reference_id();
    doc.node.references.push(Reference {
        id: reference.clone(),
        kind: args.kind.clone(),
        uri,
        title: args.title.clone(),
        note: args.note.clone(),
        added: store::today(),
        by,
        origin: args.origin.clone(),
    });
    corpus.save(&mut doc)?;
    Ok(Cited { doc, reference })
}

/// Record where the Observatory checkout is, in the corpus's `config.yaml`.
///
/// Returns the setting as it now resolves, which is the config's value: a
/// root written here takes precedence over `$OBSERVATORY_ROOT`.
pub fn set_observatory_root(corpus: &mut Corpus, dir: PathBuf) -> Result<ObservatoryRoot> {
    let _lock = corpus.lock()?;
    corpus.set_observatory_root(dir)?;
    Ok(corpus.observatory_root())
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
/// Does nothing, and says so with `None`, unless `commit: true` is set in
/// `config.yaml` and the root is inside a git work tree. Stages only
/// `nodes/`, `inbox/`, `config.yaml` and the generated `.gitignore` under the
/// root, never pushes, and refuses with [`Error::StagedElsewhere`] rather than
/// sweep up something staged outside the corpus. Called after the write it
/// records, which stays on disk whatever happens here.
pub fn commit(corpus: &Corpus, verb: &str, ids: &[&str]) -> Result<Option<Committed>> {
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
