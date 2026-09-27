//! Corpus access.
//!
//! The corpus lives outside this repository, holds the nodes and the inbox, and
//! is the only thing here that is irreplaceable. Every write goes through this
//! module so the atomicity and never-delete rules hold in one place.
//!
//! Nothing here prints. A caller that wants to tell someone what happened gets
//! back the data and says it in its own voice.
//!
//! The corpus may sit inside a git work tree, and when `config.yaml` says
//! `commit: true` a write ends with a commit of the corpus paths and nothing
//! else. Read-only history queries use those commits as the record of how a
//! node changed. This module never pushes, never checks a historical tree out,
//! never stages a path outside the root, and never undoes a write because the
//! commit failed.
//!
//! [`Corpus`] is one type whose methods are spread over this module's files
//! by what they are about:
//!
//! - `root`    where a corpus is, and this machine's settings beside it
//! - `nodes`   reading and writing node files
//! - `history` a node as git recorded it, read-only
//! - `inbox`   captures, and settling them
//! - `commit`  the commit after a write, and every git call the store makes
//! - `links`   whether two names in `nodes/` are one directory entry
//!
//! This file holds the type, opening and creating a corpus, and the write
//! lock.

mod commit;
mod history;
mod inbox;
mod links;
mod nodes;
mod root;

#[cfg(test)]
mod tests;

pub use commit::{CommitOutcome, Committed};
pub(crate) use commit::{GITIGNORE_FILE, git, inside_work_tree};
pub use history::HistoryEntry;
pub(crate) use inbox::validate_capture;
pub use inbox::{Inbox, InboxEntry, Settlement};
pub(crate) use root::refuse_nodes_symlink;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::fs_impl::create_private_dir_all;
use crate::git::GitAt;
use crate::id::corpus_id;
use crate::locations::Locations;
use crate::lock::CorpusLock;
use crate::model::Doc;
use commit::ensure_lock_ignored;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A corpus on disk.
///
/// Writers that do not take the corpus lock are crate-private. A consumer
/// outside this crate cannot name the store module or call them.
///
/// ```compile_fail
/// fn save_is_crate_private(corpus: &nebula_core::Corpus, doc: &mut nebula_core::Doc) {
///     corpus.save(doc).ok();
/// }
/// ```
///
/// ```compile_fail
/// fn settle_inbox_is_crate_private(
///     corpus: &nebula_core::Corpus,
///     entry: &nebula_core::InboxEntry,
/// ) {
///     corpus.settle_inbox(entry, "dropped").ok();
/// }
/// ```
///
/// ```compile_fail
/// fn the_store_module_is_private() {
///     let _ = std::any::type_name::<nebula_core::store::Corpus>();
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Corpus {
    root: PathBuf,
    config: Config,
    /// The resolved environment this corpus was opened with: where the
    /// machine settings are, and the ceilings every git child is given.
    locations: Locations,
}

/// A full node scan. Bad files remain visible while the good documents can
/// still be checked; graph queries use [`Corpus::load_all`] instead.
#[derive(Debug)]
pub struct Scan {
    /// Node documents that could be read and parsed.
    pub docs: Vec<Doc>,
    /// Entries that could not be read or parsed, with their reasons.
    pub unreadable: Vec<UnreadableNode>,
}

/// One file (or directory entry whose name could not be obtained) a scan
/// could not read. The path is kept as supplied by the corpus root.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct UnreadableNode {
    // Keep the TypeScript binding's existing shape and comments stable: ts-rs
    // copies Rust field docs into generated source, while this lint documents
    // the Rust API. The field is hidden from Rustdoc only during TS export.
    #[cfg_attr(
        not(feature = "ts"),
        doc = "Path of the unreadable entry as supplied by the corpus root."
    )]
    #[cfg_attr(feature = "ts", doc(hidden))]
    pub path: PathBuf,
    #[cfg_attr(not(feature = "ts"), doc = "Stable error code for the failure.")]
    #[cfg_attr(feature = "ts", doc(hidden))]
    pub code: String,
    #[cfg_attr(
        not(feature = "ts"),
        doc = "Human-readable description of the failure."
    )]
    #[cfg_attr(feature = "ts", doc(hidden))]
    pub message: String,
}

impl UnreadableNode {
    fn from_error(path: PathBuf, error: &Error) -> Self {
        Self {
            path,
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

/// A directory listing keeps entry errors beside entries, so a tolerant scan
/// can continue and a strict consumer can refuse before using a partial list.
pub(crate) struct DirectoryEntries {
    pub(crate) entries: Vec<std::fs::DirEntry>,
    pub(crate) errors: Vec<Error>,
}

impl DirectoryEntries {
    pub(crate) fn into_strict(self) -> Result<Vec<std::fs::DirEntry>> {
        if let Some(error) = self.errors.into_iter().next() {
            return Err(error);
        }
        Ok(self.entries)
    }
}

pub(crate) fn collect_directory_entries(
    dir: &Path,
    entries: impl IntoIterator<Item = std::io::Result<std::fs::DirEntry>>,
) -> DirectoryEntries {
    let mut found = DirectoryEntries {
        entries: Vec::new(),
        errors: Vec::new(),
    };
    for entry in entries {
        match entry {
            Ok(entry) => found.entries.push(entry),
            Err(error) => found
                .errors
                .push(Error::io_at("reading directory entry in", dir, error)),
        }
    }
    found
}

pub(crate) fn list_directory(dir: &Path) -> Result<DirectoryEntries> {
    let entries = std::fs::read_dir(dir).map_err(|error| Error::io_at("listing", dir, error))?;
    Ok(collect_directory_entries(dir, entries))
}

impl Corpus {
    /// Open the corpus named by `--root`, else `NEBULA_ROOT`, else the one
    /// the working directory is in, else the configured root, else
    /// `~/.nebula`, all as `locations` resolves them. The corpus keeps
    /// `locations` for the machine settings and git runs it makes later.
    pub fn open(locations: &Locations, explicit: Option<PathBuf>) -> Result<Self> {
        let root = Self::resolve_root(locations, explicit)?;
        refuse_nodes_symlink(&root)?;
        if !root.join("nodes").is_dir() {
            return Err(Error::NoCorpus(root));
        }
        // A read, so it never writes: a corpus at another schema, or with no
        // config at all, refuses to open until `neb migrate` has brought it
        // forward (STD-03 §R6).
        let config = Config::load(&root)?;
        Ok(Self {
            root,
            config,
            locations: locations.clone(),
        })
    }

    /// Create an empty corpus, or open an existing one without resetting it.
    ///
    /// Completes a root an earlier init was interrupted in: one with no
    /// config yet and no content, or with a config and some directories
    /// missing. A root that holds nodes or captures but no `config.yaml` is
    /// refused with [`Error::MissingConfig`] and left as it is, since minting
    /// a config there would give an existing corpus an id and settings it
    /// never had.
    ///
    /// Takes the corpus lock before it creates anything inside the root, so
    /// no two initializers write `config.yaml` at once and every path that
    /// creates one holds the lock (STD-03 §R6).
    pub fn init(locations: &Locations, root: &Path) -> Result<Self> {
        locations.write_gate(crate::locations::WriteIntent::Ordinary)?;
        refuse_nodes_symlink(root)?;
        // Decided before anything is created, not even the lock file, so a
        // refusal leaves the root exactly as it was.
        Self::config_to_keep(root)?;
        // Each directory is named when it cannot be made: `capture` creates
        // corpora unasked, so a bare "Permission denied" would leave the
        // reader guessing which root it was aimed at.
        create_private_dir_all(root)?;
        let _lock = CorpusLock::acquire(root)?;
        // Decided again under the lock: another initializer may have written
        // the config in between, and its identity is the one to keep.
        let config = Self::config_to_keep(root)?;
        for dir in [root.join("nodes"), root.join("inbox")] {
            create_private_dir_all(&dir)?;
        }
        let config = if let Some(config) = config {
            config
        } else {
            let config = Config::fresh(corpus_id(root));
            config.save(root)?;
            config
        };
        // The one setup repair re-running init makes on an existing corpus.
        ensure_lock_ignored(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            config,
            locations: locations.clone(),
        })
    }

    /// Open the corpus, creating it when there is none, and say whether this
    /// call created it.
    ///
    /// Capture is the reason this exists: being told to run a setup command is
    /// precisely the friction that loses the thought. A corpus that exists but
    /// is at the wrong schema, or has lost its config, still refuses, because
    /// rewriting it blind would be worse than the friction. The create is
    /// [`Self::init`], so it runs under the corpus lock.
    ///
    /// The flag is `true` when there was no corpus at the root, so the caller
    /// can say where it made one: a mistyped root would otherwise split the
    /// corpus with nothing to show for it.
    pub fn open_or_init(locations: &Locations, explicit: Option<PathBuf>) -> Result<(Self, bool)> {
        let root = Self::resolve_root(locations, explicit)?;
        match Self::open(locations, Some(root.clone())) {
            Err(Error::NoCorpus(_)) => Self::init(locations, &root).map(|corpus| (corpus, true)),
            other => other.map(|corpus| (corpus, false)),
        }
    }

    /// Where the corpus lives, as it was given.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The resolved environment this corpus was opened with.
    pub fn locations(&self) -> &Locations {
        &self.locations
    }

    /// Run one query over the graph of every node as it is on disk now.
    ///
    /// A [`Graph`](crate::Graph) borrows the nodes it indexes, so it cannot
    /// be handed back; this loads them, builds it, and gives it to `query`,
    /// which is how a read is `open` plus one call. Takes no lock: a reader
    /// never waits on a writer.
    pub fn query<T>(&self, query: impl FnOnce(&crate::Graph<'_>) -> Result<T>) -> Result<T> {
        let docs = self.load_all()?;
        query(&crate::Graph::build(&docs)?)
    }

    /// Where this corpus's git commands run, and where git's repository
    /// discovery stops for them.
    pub(crate) fn git_at(&self) -> GitAt<'_> {
        self.locations.git_at(&self.root)
    }

    /// Take this corpus's write lock, waiting up to [`crate::LOCK_WAIT`] for
    /// whoever holds it, and hold it until the returned guard drops.
    ///
    /// Every op that writes takes this, so a caller gets it without asking.
    /// A caller asks for it directly to widen the critical section over more
    /// than one call — which is what the CLI does, so a verb and the commit
    /// that records it cannot have another writer's verb between them.
    /// Re-entrant on one thread, so the op taking it underneath is free.
    ///
    /// Deliberately not taken by [`Self::open`]: a read-only verb and the
    /// desktop's file watcher must never wait on a writer.
    ///
    /// The take that enters the critical section first settles a write an
    /// earlier writer recorded and did not finish — a promotion interrupted
    /// between its node and its strike — so every writer starts from the
    /// outcome that write decided (STD-03 §R9; see `pending.rs`). A record
    /// that cannot be read refuses the lock, as
    /// [`Error::PendingWriteUnreadable`], and nothing is written.
    #[must_use = "the corpus lock is released when this guard drops"]
    pub fn lock(&self) -> Result<CorpusLock> {
        self.lock_within(crate::LOCK_WAIT)
    }

    /// Take the write lock with a caller-chosen wait bound. A responsive UI
    /// can refuse a busy writer sooner while keeping the same critical section.
    #[must_use = "the corpus lock is released when this guard drops"]
    pub fn lock_within(&self, wait: std::time::Duration) -> Result<CorpusLock> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        self.settled(CorpusLock::acquire_within(&self.root, wait)?)
    }

    /// Take the write lock with a wait bound and a label for its holder
    /// record, for a process whose writers are not all one command: the
    /// desktop's capture box says `desktop capture`, its inbox
    /// `desktop drop <entry>`. A waiter that times out is told the label.
    /// Otherwise [`Self::lock_within`], pending write and all.
    #[must_use = "the corpus lock is released when this guard drops"]
    pub fn lock_as(&self, wait: std::time::Duration, label: &str) -> Result<CorpusLock> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        self.settled(CorpusLock::acquire_as(&self.root, wait, label)?)
    }

    /// `lock`, once the take that entered the critical section first has
    /// settled any write an earlier writer left pending.
    fn settled(&self, lock: CorpusLock) -> Result<CorpusLock> {
        if lock.is_outermost() {
            self.finish_pending()?;
        }
        Ok(lock)
    }

    /// `config.yaml` as it stands on disk, rather than the snapshot
    /// [`Self::open`] took.
    ///
    /// Only meaningful under the write lock, and every caller here holds one.
    /// `open` takes no lock on purpose, so by the time a writer gets in its
    /// snapshot can be arbitrarily old: a cooperating writer that was ahead in
    /// the queue may have changed a setting in between. Rewriting the whole
    /// file from the stale copy would erase that change, and deciding from it
    /// would decide on a configuration nobody holds any more.
    ///
    /// Never writes. A config deleted underneath a live corpus is
    /// [`Error::MissingConfig`], not a file brought back from the snapshot:
    /// the snapshot is exactly what may be stale, and a writer that put it
    /// back would decide on settings nobody holds.
    fn current_config(&self) -> Result<Config> {
        Config::load(&self.root)
    }

    /// Re-read `config.yaml`, so the rewrite that follows starts from the
    /// settings in force rather than the ones this corpus opened with.
    fn reload_config(&mut self) -> Result<()> {
        self.config = self.current_config()?;
        Ok(())
    }
}
