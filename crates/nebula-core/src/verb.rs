//! The verbs, one call each: what `neb` and the desktop run for every
//! command that writes, and the reads a surface needs whole.
//!
//! A surface parses its input, opens a [`Corpus`] (or hands over the
//! [`Locations`] a verb resolves its own root from), makes one call here or
//! one query on the corpus, and renders what comes back (STD-02 §R2). So the
//! rules every writer shares have one definition, here, and no surface can
//! run them in another order:
//!
//! - **The write lock.** Each verb takes the corpus lock itself, before the
//!   write, with the wait and the holder label its [`WriteOptions`] give, and
//!   holds it across the write *and* the commit that records it, so no other
//!   writer's verb lands between the two. It is released before anything
//!   slow that is only advice — suggestions, close tags, a node view — is
//!   read (STD-03 §R1).
//! - **The commit.** When [`CommitPolicy::Configured`] and `config.yaml`
//!   asks for it, the write is committed under that lock as `neb <verb>
//!   <ids>`. The outcome is returned beside the value, never raised: a
//!   refused commit leaves the write on disk, and a caller that raised it
//!   would read as the write failing (STD-02 §R30).
//! - **Advice fails open.** What is read after the lock only costs itself
//!   when it cannot be read, never the write (STD-02 §R31).
//!
//! The ops in [`crate::ops`] stay public, one write each, for a caller that
//! composes its own; every surface in this repository goes through here.

use crate::check::{self, OBSERVATORY, Report};
use crate::config::{CommitSetting, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::graph::{self, Graph, Neighbour, NodeView};
use crate::locations::Locations;
use crate::lock::{CorpusLock, LOCK_WAIT};
use crate::model::{Doc, EdgeType, Status};
use crate::ops::{
    self, Citation, Cited, CloseTag, Created, DroppedLegacy, HandedOff, Handoff, Initialized,
    NewNode, Promotion, Retagged, StatusChange,
};
use crate::store::{self, CommitOutcome, Corpus, InboxEntry};
use crate::{migrate, migrate::MigrationReport};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Whether a verb commits what it wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommitPolicy {
    /// Commit when `config.yaml` says `commit: true`, which is the default.
    #[default]
    Configured,
    /// Do not commit this write, whatever the setting: `--no-commit`.
    Skip,
}

/// How a verb takes the lock and whether it commits.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// Whether the write is committed.
    pub commit: CommitPolicy,
    /// How long to wait for another writer: [`LOCK_WAIT`] unless a surface
    /// has to answer sooner, as the desktop does.
    pub wait: Duration,
    /// The holder label a waiting writer is told, for a process whose writers
    /// are not all one command (`desktop capture`). `None` keeps the
    /// process's own label.
    pub label: Option<String>,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            commit: CommitPolicy::Configured,
            wait: LOCK_WAIT,
            label: None,
        }
    }
}

impl WriteOptions {
    /// The corpus lock, taken the way these options say.
    pub(crate) fn lock(&self, corpus: &Corpus) -> Result<CorpusLock> {
        match &self.label {
            Some(label) => corpus.lock_as(self.wait, label),
            None => corpus.lock_within(self.wait),
        }
    }

    /// Commit what `verb` wrote, per the policy. `None` when the policy
    /// skips it. Only ever called under the lock [`Self::lock`] took.
    pub(crate) fn commit(
        &self,
        corpus: &Corpus,
        verb: &str,
        ids: &[&str],
    ) -> Option<Result<CommitOutcome>> {
        match self.commit {
            CommitPolicy::Skip => None,
            CommitPolicy::Configured => Some(corpus.commit(verb, ids)),
        }
    }
}

/// What a writing verb did: the value it wrote, the commit that followed,
/// and the advice read once the lock was released.
#[derive(Debug)]
#[must_use = "a refused commit is reported here, not raised"]
pub struct Written<T> {
    /// What the write produced.
    pub value: T,
    /// The commit that recorded it, when one was attempted: `None` when the
    /// policy skipped it, or when the verb wrote nothing a commit would
    /// record. An `Err` is a commit git refused; the write stands.
    pub commit: Option<Result<CommitOutcome>>,
    /// Tags the write just added that read as a variant of a tag already in
    /// use. Advice: empty when there are none, and when the corpus could
    /// not be read for the comparison, since `check` reports the same drift.
    pub close_tags: Vec<CloseTag>,
}

impl<T> Written<T> {
    pub(crate) fn new(value: T, commit: Option<Result<CommitOutcome>>) -> Self {
        Self {
            value,
            commit,
            close_tags: Vec::new(),
        }
    }

    fn with_close_tags(self, corpus: &Corpus, id: &str, tags: &[String]) -> Self {
        Self {
            close_tags: close_tags(corpus, id, tags),
            ..self
        }
    }
}

/// [`ops::close_tags`], failing open: the write has landed, and a corpus
/// that cannot be read for the comparison costs the note, not the command.
/// Run with the lock released, because it reads every node.
fn close_tags(corpus: &Corpus, id: &str, tags: &[String]) -> Vec<CloseTag> {
    if tags.is_empty() {
        return Vec::new();
    }
    ops::close_tags(corpus, id, tags).unwrap_or_default()
}

// ------------------------------------------------------------------ init --

/// A machine setting worth naming before a corpus is created, because every
/// later command would otherwise keep resolving somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootWarning {
    /// `~/.nebula` is being created while the machine setting names another
    /// corpus.
    DefaultWhileConfigured {
        /// The machine setting, `~/.config/nebula/root`.
        setting: PathBuf,
        /// The corpus it names.
        configured: PathBuf,
    },
    /// A corpus is being created somewhere other than the one the machine
    /// setting names, so commands will not find it.
    Shadowed {
        /// The machine setting, `~/.config/nebula/root`.
        setting: PathBuf,
        /// The corpus it names.
        configured: PathBuf,
    },
}

/// `init`'s result and what a person should be told about it.
#[derive(Debug, Clone)]
pub struct InitReport {
    /// The corpus, as `--json` reports it.
    pub initialized: Initialized,
    /// The machine setting that names the default corpus: written when
    /// `set_root` was given.
    pub setting: PathBuf,
    /// Whether the machine has no default yet and this corpus is not
    /// `~/.nebula`, so `--set-root` is worth suggesting.
    pub suggest_set_root: bool,
    /// Settings that point commands elsewhere; empty with `set_root`.
    pub warnings: Vec<RootWarning>,
}

/// Create an empty corpus with [`ops::init`], and say what a person should
/// know about the machine setting: the checks run before anything is
/// created, so they describe the machine as `init` found it.
pub fn init(
    locations: &Locations,
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Result<InitReport> {
    // Before any work: a `--root` that names another directory than the
    // path is refused here, with neither created (STD-01 §R28).
    let target = ops::init_target(locations, root.clone(), path.clone())?;
    let absent = Corpus::root_config_path_if_absent(locations, &target)?;
    let mut warnings = Vec::new();
    if !set_root {
        warnings.extend(default_root_warning(locations, &target)?);
        if let Some(configured) = Corpus::warning_before_shadowing_init(locations, &target)? {
            warnings.push(RootWarning::Shadowed {
                setting: Corpus::root_config_path(locations)?,
                configured,
            });
        }
    }
    let initialized = ops::init(locations, root, path, set_root, force)?;
    Ok(InitReport {
        initialized,
        setting: Corpus::root_config_path(locations)?,
        suggest_set_root: !set_root && absent.is_some(),
        warnings,
    })
}

/// The one warning before `~/.nebula` is created while the machine setting
/// names another corpus, shared by `init` and `capture`.
fn default_root_warning(locations: &Locations, root: &Path) -> Result<Option<RootWarning>> {
    Corpus::warning_before_default_init(locations, root)?
        .map(|configured| {
            Ok(RootWarning::DefaultWhileConfigured {
                setting: Corpus::root_config_path(locations)?,
                configured,
            })
        })
        .transpose()
}

// --------------------------------------------------------------- migrate --

/// A corpus brought forward, and where it is.
#[derive(Debug, Clone)]
pub struct Migrated {
    /// The root it resolved to.
    pub root: PathBuf,
    /// What changed.
    pub report: MigrationReport,
}

/// [`migrate::run`], committed under the lock the migration holds, so no
/// other writer lands between the rewrite and the commit that records it.
pub fn migrate(
    locations: &Locations,
    root: Option<PathBuf>,
    options: &WriteOptions,
) -> Result<Written<Migrated>> {
    let (root, report, commit) = migrate::run_then(locations, root, |root| {
        // The corpus is at this build's schema now, so it opens; the
        // migration lands as its own commit when the setting is on.
        match options.commit {
            CommitPolicy::Skip => None,
            CommitPolicy::Configured => Some(
                Corpus::open(locations, Some(root.to_path_buf()))
                    .and_then(|corpus| corpus.commit("migrate", &[])),
            ),
        }
    })?;
    Ok(Written::new(Migrated { root, report }, commit))
}

// ---------------------------------------------------------------- config --

/// The observatory setting after a change, and what `--drop-legacy` did.
#[derive(Debug, Clone)]
pub struct ObservatoryUpdate {
    /// The setting as it now resolves.
    pub setting: ObservatoryRoot,
    /// What removing the legacy key did, when it was asked for.
    pub dropped: Option<DroppedLegacy>,
}

/// Make `dir` this machine's Observatory checkout, and with `drop_legacy`
/// remove the legacy key from `config.yaml`.
///
/// Setting the root writes this machine's file and nothing in the corpus,
/// so it takes the machine-setting lock, and only `drop_legacy` takes the
/// corpus lock. Both are taken before either write, in the order `lock.rs`
/// documents, so a busy lock refuses with nothing written. Only a removed
/// key is committed: the machine file is not the corpus's.
pub fn set_observatory_root(
    corpus: &mut Corpus,
    dir: Option<&Path>,
    drop_legacy: bool,
    options: &WriteOptions,
) -> Result<Written<ObservatoryUpdate>> {
    let _settings = dir
        .is_some()
        .then(|| Corpus::lock_machine_settings(corpus.locations()))
        .transpose()?;
    let _lock = drop_legacy.then(|| options.lock(corpus)).transpose()?;
    if let Some(dir) = dir {
        ops::set_observatory_root(corpus, dir)?;
    }
    let dropped = drop_legacy
        .then(|| ops::drop_legacy_observatory_root(corpus))
        .transpose()?;
    let setting = corpus.observatory_root()?;
    let commit = dropped
        .as_ref()
        .is_some_and(|dropped| dropped.removed.is_some())
        .then(|| options.commit(corpus, "config", &["observatory-root"]))
        .flatten();
    Ok(Written::new(ObservatoryUpdate { setting, dropped }, commit))
}

/// Record whether writes are committed. Turning it on records itself;
/// turning it off leaves the file for the next commit made by hand,
/// because off means off: the commit reports [`CommitOutcome::Disabled`].
pub fn set_commit(
    corpus: &mut Corpus,
    enabled: bool,
    options: &WriteOptions,
) -> Result<Written<CommitSetting>> {
    let _lock = options.lock(corpus)?;
    let setting = ops::set_commit(corpus, enabled)?;
    let commit = options.commit(corpus, "config", &["commit"]);
    Ok(Written::new(setting, commit))
}

// ----------------------------------------------------------------- inbox --

/// Append one line to the inbox and commit it.
pub fn capture(corpus: &Corpus, text: &str, options: &WriteOptions) -> Result<Written<InboxEntry>> {
    let _lock = options.lock(corpus)?;
    let entry = ops::capture(corpus, text)?;
    let commit = options.commit(corpus, "capture", &[&entry.id]);
    Ok(Written::new(entry, commit))
}

/// A capture made wherever the root resolves, creating the corpus there if
/// there is none, with what the triage after it can use.
#[derive(Debug)]
pub struct CapturedAt {
    /// The root the capture resolved to, as given.
    pub root: PathBuf,
    /// Whether this call created the corpus there.
    pub created: bool,
    /// Said when the corpus created is `~/.nebula` while the machine setting
    /// names another.
    pub warning: Option<RootWarning>,
    /// The entry as written.
    pub entry: InboxEntry,
    /// An earlier entry still waiting that says the same thing, when there
    /// is one; `Err` when the inbox could not be read back to tell.
    pub same_as: Result<Option<InboxEntry>>,
    /// The nodes the text reads closest to; `Err` when `nodes/` could not be
    /// read for them.
    pub near: Result<Vec<Neighbour>>,
}

/// `neb capture`: the five-second path, on a corpus that may not exist yet.
///
/// The text is refused by core's own emptiness rule before a corpus can be
/// created for it. Being told to run a setup command is precisely the
/// friction that loses the thought, so the corpus is created when there is
/// none; a root found from the working directory already holds one, so only
/// an explicit or configured root is ever created here. The capture and its
/// commit are the critical section; the repeat check and the `near_k`
/// suggestions are reads, made once the lock is released, and fail open.
pub fn capture_at(
    locations: &Locations,
    root: Option<PathBuf>,
    text: &str,
    near_k: usize,
    options: &WriteOptions,
) -> Result<Written<CapturedAt>> {
    store::validate_capture(text)?;
    let root = Corpus::resolve_root(locations, root)?;
    let warning = default_root_warning(locations, &root)?;
    let (corpus, created) = Corpus::open_or_init(locations, Some(root.clone()))?;
    let Written {
        value: entry,
        commit,
        ..
    } = capture(&corpus, text, options)?;
    let same_as = corpus.inbox().map(|inbox| inbox.same_as(&entry).cloned());
    let near = ops::suggest(&corpus, &entry.text, near_k);
    Ok(Written::new(
        CapturedAt {
            root,
            created,
            warning,
            entry,
            same_as,
            near,
        },
        commit,
    ))
}

/// Settle a capture as dropped, struck through and never deleted.
pub fn drop(corpus: &Corpus, entry: &str, options: &WriteOptions) -> Result<Written<InboxEntry>> {
    let _lock = options.lock(corpus)?;
    let dropped = ops::drop(corpus, entry)?;
    let commit = options.commit(corpus, "drop", &[entry]);
    Ok(Written::new(dropped, commit))
}

/// Turn an inbox entry into a seed node. The `near_k` suggestions are read
/// before the lock, so the new node is not its own neighbour and no writer
/// waits on the scan.
pub fn promote(
    corpus: &Corpus,
    entry: &str,
    promotion: &Promotion,
    near_k: usize,
    options: &WriteOptions,
) -> Result<Written<Created>> {
    let near = ops::promotion_near(corpus, entry, promotion, near_k)?;
    let lock = options.lock(corpus)?;
    let created = ops::promote_with(corpus, entry, promotion, near)?;
    let commit = options.commit(corpus, "promote", &[entry, &created.doc.node.id]);
    drop_lock(lock);
    let id = created.doc.node.id.clone();
    let tags = created.doc.node.tags.clone();
    Ok(Written::new(created, commit).with_close_tags(corpus, &id, &tags))
}

// ----------------------------------------------------------------- nodes --

/// Create a node directly. A contradicted node changes too, so the commit
/// names it.
pub fn new_node(
    corpus: &Corpus,
    spec: &NewNode,
    options: &WriteOptions,
) -> Result<Written<Created>> {
    let lock = options.lock(corpus)?;
    let created = ops::new_node(corpus, spec)?;
    let node = &created.doc.node;
    let ids: Vec<&str> = std::iter::once(node.id.as_str())
        .chain(node.edges_of(EdgeType::Contradicts))
        .collect();
    let commit = options.commit(corpus, "new", &ids);
    drop_lock(lock);
    let id = node.id.clone();
    let tags = node.tags.clone();
    Ok(Written::new(created, commit).with_close_tags(corpus, &id, &tags))
}

/// A body saved, and the node as `show --json` would print it afterwards.
#[derive(Debug)]
pub struct Saved {
    /// Whether anything was written: an edit that changed nothing is not.
    pub changed: bool,
    /// The node over the corpus as it is now, when it was asked for; read
    /// with the lock released. `Err` when it could not be read; the write
    /// stands.
    pub view: Option<Result<NodeView>>,
}

/// Save a body edited without the lock — in `$EDITOR` — over `before`, the
/// body it was loaded as.
///
/// Nothing typed is nothing to write: no lock, no save, no commit, and
/// `updated` stays as it was (STD-01 §R30). Otherwise the append-only notes
/// are checked before the lock is waited for, and again under it by
/// [`ops::set_body_if`], which also refuses a body another writer changed
/// meanwhile. With `view`, the node is read back once the lock is released.
pub fn edit(
    corpus: &Corpus,
    id: &str,
    before: &str,
    after: &str,
    view: bool,
    options: &WriteOptions,
) -> Result<Written<Saved>> {
    if ops::body_unchanged(before, after) {
        let saved = Saved {
            changed: false,
            view: view.then(|| node_view(corpus, id)),
        };
        return Ok(Written::new(saved, None));
    }
    ops::refuse_rewritten_notes(before, after)?;
    let lock = options.lock(corpus)?;
    ops::set_body_if(corpus, id, before, after)?;
    let commit = options.commit(corpus, "edit", &[id]);
    drop_lock(lock);
    let saved = Saved {
        changed: true,
        view: view.then(|| node_view(corpus, id)),
    };
    Ok(Written::new(saved, commit))
}

/// Append a dated line to the node's notes, and with `view` read the node
/// back once the lock is released.
pub fn note(
    corpus: &Corpus,
    id: &str,
    text: &str,
    by: Option<&str>,
    view: bool,
    options: &WriteOptions,
) -> Result<Written<Saved>> {
    let lock = options.lock(corpus)?;
    ops::note(corpus, id, text, by)?;
    let commit = options.commit(corpus, "note", &[id]);
    drop_lock(lock);
    let saved = Saved {
        changed: true,
        view: view.then(|| node_view(corpus, id)),
    };
    Ok(Written::new(saved, commit))
}

/// A node sharpened, and what it was before.
#[derive(Debug, Clone)]
pub struct Sharpened {
    /// The node as written.
    pub doc: Doc,
    /// Its status before.
    pub from: Status,
    /// Whether it had a kill condition before, which this one replaced.
    pub replaced: bool,
}

/// Name what would kill a node, which makes a seed a hypothesis.
pub fn sharpen(
    corpus: &Corpus,
    id: &str,
    kill: &str,
    by: Option<&str>,
    options: &WriteOptions,
) -> Result<Written<Sharpened>> {
    let _lock = options.lock(corpus)?;
    let before = corpus.load(id)?;
    let doc = ops::sharpen(corpus, id, kill, by)?;
    let commit = options.commit(corpus, "sharpen", &[id]);
    let sharpened = Sharpened {
        doc,
        from: before.node.status,
        replaced: before.node.kill.is_some(),
    };
    Ok(Written::new(sharpened, commit))
}

/// Confirm an agent's kill condition as the human's.
pub fn confirm_kill(corpus: &Corpus, id: &str, options: &WriteOptions) -> Result<Written<Doc>> {
    let _lock = options.lock(corpus)?;
    let doc = ops::confirm_kill(corpus, id)?;
    let commit = options.commit(corpus, "sharpen", &[id]);
    Ok(Written::new(doc, commit))
}

/// Move a node along its lifecycle, with the transition guards applied.
pub fn set_status(
    corpus: &Corpus,
    id: &str,
    status: Status,
    why: Option<&str>,
    options: &WriteOptions,
) -> Result<Written<StatusChange>> {
    let _lock = options.lock(corpus)?;
    let changed = ops::set_status(corpus, id, status, why)?;
    let commit = options.commit(corpus, "status", &[id]);
    Ok(Written::new(changed, commit))
}

/// Add an edge; returns every node the write changed.
pub fn link(
    corpus: &Corpus,
    from: &str,
    kind: EdgeType,
    to: &str,
    by: Option<&str>,
    options: &WriteOptions,
) -> Result<Written<Vec<Doc>>> {
    let _lock = options.lock(corpus)?;
    let changed = ops::link(corpus, from, kind, to, by)?;
    let commit = options.commit(corpus, "link", &[from, to]);
    Ok(Written::new(changed, commit))
}

/// Add and remove tags in one write. A node whose tags come out as they
/// were is neither written nor committed (STD-01 §R30), and has no close
/// tags to report.
pub fn retag(
    corpus: &Corpus,
    id: &str,
    add: &[String],
    remove: &[String],
    options: &WriteOptions,
) -> Result<Written<Retagged>> {
    let lock = options.lock(corpus)?;
    let done = ops::retag(corpus, id, add, remove)?;
    if !done.written {
        return Ok(Written::new(done, None));
    }
    let commit = options.commit(corpus, "tag", &[id]);
    drop_lock(lock);
    Ok(Written::new(done, commit).with_close_tags(corpus, id, add))
}

/// A citation, and the observatory setting it was located under.
#[derive(Debug, Clone)]
pub struct CiteReport {
    /// The node and the reference added.
    pub cited: Cited,
    /// The observatory setting, read before the write, for an `observatory`
    /// reference; `None` for every other kind.
    pub setting: Option<ObservatoryRoot>,
}

/// Attach a reference. For an `observatory` one the setting is read before
/// the write, so a broken machine setting refuses the cite rather than
/// failing it after the reference has landed. The kind is compared as the
/// write stores it, so `Observatory` counts.
pub fn cite(
    corpus: &Corpus,
    id: &str,
    citation: &Citation,
    options: &WriteOptions,
) -> Result<Written<CiteReport>> {
    let _lock = options.lock(corpus)?;
    let setting = (check::normalize_reference_kind(&citation.kind) == OBSERVATORY)
        .then(|| corpus.observatory_root())
        .transpose()?;
    let root = setting.as_ref().and_then(|s| s.root.as_deref());
    let cited = ops::cite_with_observatory(corpus, id, citation, root)?;
    let commit = options.commit(corpus, "cite", &[id, &cited.reference]);
    Ok(Written::new(CiteReport { cited, setting }, commit))
}

/// A hand-off, and the observatory setting its record resolved under.
#[derive(Debug, Clone)]
pub struct HandoffReport {
    /// What the hand-off wrote.
    pub done: HandedOff,
    /// The observatory setting, read before the write.
    pub setting: ObservatoryRoot,
}

/// Cite an Observatory record and close the node in one write. The setting
/// is read first: the record has to resolve under it, and a broken machine
/// setting refuses the hand-off before the node closes.
pub fn handoff(
    corpus: &Corpus,
    id: &str,
    handoff: &Handoff,
    options: &WriteOptions,
) -> Result<Written<HandoffReport>> {
    let _lock = options.lock(corpus)?;
    let setting = corpus.observatory_root()?;
    let done = ops::handoff(corpus, id, handoff, setting.root.as_deref())?;
    let commit = options.commit(corpus, "handoff", &[id, &done.record]);
    Ok(Written::new(HandoffReport { done, setting }, commit))
}

// ----------------------------------------------------------------- reads --

/// One node in full, and the observatory setting its references were
/// located under.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The node, with its observatory references located.
    pub view: NodeView,
    /// The setting they were located under.
    pub setting: ObservatoryRoot,
}

/// `neb show`: one node over the corpus as it is now, or with `at`, as that
/// node was at a commit or on a date, among the rest as they are now.
pub fn show(corpus: &Corpus, id: &str, at: Option<&str>) -> Result<Shown> {
    let mut docs = corpus.load_all()?;
    if let Some(at) = at {
        let historical = corpus.load_at(id, at)?;
        let current = docs
            .iter_mut()
            .find(|doc| doc.node.id == id)
            .ok_or_else(|| Error::NoSuchNode(id.to_string()))?;
        *current = historical;
    }
    let setting = corpus.observatory_root()?;
    let view = graph::node(&Graph::build(&docs)?, id)?.with_observatory(setting.root.as_deref())?;
    Ok(Shown { view, setting })
}

/// Every invariant over the whole corpus: an unreadable node file is a
/// finding, not a refusal.
pub fn check(corpus: &Corpus) -> Result<Report> {
    check::run_scanned(&corpus.scan()?, corpus)
}

/// The node as `show --json` prints it, without locating observatory
/// records: what `edit` and `note` print once their write is committed.
fn node_view(corpus: &Corpus, id: &str) -> Result<NodeView> {
    corpus.query(|graph| graph::node(graph, id))
}

/// Release the lock before advice is read. A named step, so every verb that
/// reads after its write says where the critical section ends.
fn drop_lock(lock: CorpusLock) {
    std::mem::drop(lock);
}
