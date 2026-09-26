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

use crate::config::{self, CommitSetting, Config, Declared, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::fs::{append_private, create_private_dir_all, write_private_atomic};
use crate::git::{self, GitOutput};
use crate::lock::{CorpusLock, LOCK_FILE};
use crate::model::{self, Doc};
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use time::{
    Date, OffsetDateTime, PrimitiveDateTime, UtcOffset,
    format_description::well_known::{Iso8601, Rfc3339},
    macros::format_description,
};

/// A corpus on disk.
#[derive(Debug, Clone)]
pub struct Corpus {
    root: PathBuf,
    config: Config,
}

impl Corpus {
    /// Where a corpus would be, given `--root`, else `NEBULA_ROOT`, else the
    /// corpus the working directory is in, else the configured root, else
    /// `~/.nebula`.
    ///
    /// The working directory sits below the two settings that are typed on
    /// purpose and above the two that are standing machine defaults, the way
    /// git finds its repository: standing inside a corpus is itself a choice
    /// of corpus. See [`Self::discover`] for what counts as being inside one.
    pub fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
        if let Some(p) = explicit {
            if p.as_os_str().is_empty() {
                return Err(Error::EmptyRoot);
            }
            return Ok(p);
        }
        if let Some(p) = std::env::var_os("NEBULA_ROOT").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(p));
        }
        if let Some(p) = working_dir().and_then(|cwd| Self::discover(&cwd)) {
            return Ok(p);
        }
        if let Some(p) = Self::configured_root()? {
            return Ok(p);
        }
        Self::default_root()
    }

    /// The nearest directory at or above `start` that already holds a corpus:
    /// a `nodes/` directory beside a `config.yaml` that names a `corpus_id`.
    /// `None` when no ancestor does.
    ///
    /// Only a corpus that exists is ever found, so discovery can never lead
    /// `capture`, the one verb that creates a corpus, to create one: inside a
    /// corpus it writes there, and anywhere else resolution carries on to the
    /// configured root exactly as if discovery did not exist. Both halves of
    /// the marker are required, so neither a half-initialized directory nor a
    /// project that happens to have a `nodes/` folder and a `config.yaml` is
    /// mistaken for one.
    ///
    /// The nearest corpus wins, so from inside a corpus nested in another the
    /// inner one is found. The walk is lexical, parent by parent as `cd ..`
    /// goes, and never resolves a symlink: the root comes back spelled the
    /// way `start` spelled it. A `nodes/` that is itself a symlink still
    /// counts, so that [`Self::open`] refuses it by name instead of the walk
    /// quietly passing it by for some other corpus.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .filter(|dir| !dir.as_os_str().is_empty())
            .find(|dir| holds_corpus(dir))
            .map(Path::to_path_buf)
    }

    /// The root used when no command, environment, or machine setting names one.
    pub fn default_root() -> Result<PathBuf> {
        Ok(Self::home()?.join(".nebula"))
    }

    /// The machine-local file that records a non-default corpus root.
    pub fn root_config_path() -> Result<PathBuf> {
        Ok(Self::machine_settings_dir()?.join("root"))
    }

    /// Where this machine's settings live: `~/.config/nebula`.
    pub fn machine_settings_dir() -> Result<PathBuf> {
        Ok(Self::home()?.join(".config").join("nebula"))
    }

    /// Take the lock that serializes writes to this machine's settings,
    /// `~/.config/nebula/.lock`, creating the directory `0700` if needed.
    ///
    /// The same advisory lock a corpus write takes, on a different
    /// directory, so it waits the same bounded time and refuses with
    /// [`Error::Locked`] naming `~/.config/nebula`. A writer that checks a
    /// setting and then writes it holds this across both, so two of them
    /// cannot both pass the check. Taken before any corpus lock, never after
    /// one; see the lock order in `lock.rs`.
    pub fn lock_machine_settings() -> Result<CorpusLock> {
        let dir = Self::machine_settings_dir()?;
        create_private_dir_all(&dir)?;
        CorpusLock::acquire(&dir)
    }

    /// Read the configured corpus root, if this machine has one.
    pub fn configured_root() -> Result<Option<PathBuf>> {
        let path = Self::root_config_path()?;
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let root = raw.trim();
                if root.is_empty() {
                    return Err(Error::corpus("configured nebula root is empty"));
                }
                Ok(Some(PathBuf::from(root)))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The machine-local file that records where the Observatory checkout
    /// is, beside [`Self::root_config_path`]. A machine setting rather than a
    /// corpus one, because the corpus travels between machines and the
    /// checkout's path does not.
    pub fn observatory_root_config_path() -> Result<PathBuf> {
        Ok(Self::machine_settings_dir()?.join("observatory-root"))
    }

    /// Read this machine's Observatory checkout setting, if it has one.
    ///
    /// A machine with no `HOME` has no machine settings, which is not an
    /// error: `$OBSERVATORY_ROOT` and an explicit `--root` still work there.
    /// A file that is present but empty or relative is refused rather than
    /// skipped, because falling through to the legacy key would resolve
    /// records against another machine's path without a word.
    pub fn configured_observatory_root() -> Result<Option<PathBuf>> {
        let Ok(path) = Self::observatory_root_config_path() else {
            return Ok(None);
        };
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io_at("reading", &path, error)),
        };
        let root = PathBuf::from(raw.trim());
        if !root.is_absolute() {
            return Err(Error::RelativeObservatoryRoot {
                root,
                setting: Some(path),
            });
        }
        Ok(Some(root))
    }

    /// Make `dir` this machine's Observatory checkout, for every corpus.
    ///
    /// Writes `~/.config/nebula/observatory-root` and nothing under any
    /// corpus. The directory is not required to exist yet; `check` says so
    /// when a reference fails to resolve under it. It must be absolute, since
    /// the setting is read from whatever directory a command runs in.
    pub(crate) fn write_observatory_root_config(dir: &Path) -> Result<PathBuf> {
        if !dir.is_absolute() {
            return Err(Error::RelativeObservatoryRoot {
                root: dir.to_path_buf(),
                setting: None,
            });
        }
        let path = Self::observatory_root_config_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::corpus("observatory root configuration path has no parent"))?;
        create_private_dir_all(parent)?;
        write_private_atomic(&path, format!("{}\n", dir.display()))?;
        Ok(path)
    }

    /// The machine-local setting that a new non-default corpus will write.
    pub fn root_config_path_if_absent(root: &Path) -> Result<Option<PathBuf>> {
        if root == Self::default_root()? {
            return Ok(None);
        }
        let path = Self::root_config_path()?;
        if path.exists() {
            return Ok(None);
        }
        Ok(Some(path))
    }

    /// Make `root` the machine-local default corpus.
    ///
    /// Refuses to replace a different configured root unless `force` is set.
    /// Paths are compared as given, like every other corpus path. The check
    /// and the write happen under [`Self::lock_machine_settings`], and the
    /// file is replaced whole: a symlink at `root` is replaced by a regular
    /// file, and whatever it pointed at is left alone.
    pub fn write_root_config(root: &Path, force: bool) -> Result<PathBuf> {
        let _lock = Self::lock_machine_settings()?;
        let path = Self::root_config_path()?;
        Self::check_root_config(root, force)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::corpus("root configuration path has no parent"))?;
        create_private_dir_all(parent)?;
        write_private_atomic(&path, format!("{}\n", root.display()))?;
        Ok(path)
    }

    /// Refuse a conflicting machine-local default before initializing a
    /// corpus. The write is deliberately separate so a refused `--set-root`
    /// cannot create or rewrite the requested corpus first.
    pub(crate) fn check_root_config(root: &Path, force: bool) -> Result<()> {
        if !root.is_absolute() {
            return Err(Error::corpus(format!(
                "--set-root requires an absolute corpus path, not {}; rerun with an absolute path",
                root.display()
            )));
        }
        let path = Self::root_config_path()?;
        if let Some(configured) = Self::configured_root()?
            && configured != root
            && !force
        {
            return Err(Error::RootConfigConflict {
                path,
                configured,
                requested: root.to_path_buf(),
            });
        }
        Ok(())
    }

    /// A conflicting machine setting is worth naming before creating the
    /// legacy default corpus. The normal resolver cannot reach this state,
    /// but an explicit `--root ~/.nebula` can.
    pub fn warning_before_default_init(root: &Path) -> Result<Option<PathBuf>> {
        if root != Self::default_root()? || root.exists() {
            return Ok(None);
        }
        Ok(Self::configured_root()?.filter(|configured| configured != root))
    }

    /// A configured root that names a *different* directory than the one
    /// about to be initialized is worth naming even when the target is not
    /// the legacy default: `init` would otherwise succeed silently and every
    /// later command would keep resolving to the old corpus, leaving the new
    /// one orphaned. `None` when there is no configured root, the target
    /// already matches it, or the target is the default (covered by
    /// [`Self::warning_before_default_init`] instead).
    pub fn warning_before_shadowing_init(root: &Path) -> Result<Option<PathBuf>> {
        if root == Self::default_root()? {
            return Ok(None);
        }
        Ok(Self::configured_root()?.filter(|configured| configured != root))
    }

    fn home() -> Result<PathBuf> {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| Error::corpus("HOME is not set"))
    }

    /// Open the corpus named by `--root`, else `NEBULA_ROOT`, else the one
    /// the working directory is in, else the configured root, else
    /// `~/.nebula`.
    pub fn open(explicit: Option<PathBuf>) -> Result<Self> {
        let root = Self::resolve_root(explicit)?;
        refuse_nodes_symlink(&root)?;
        if !root.join("nodes").is_dir() {
            return Err(Error::NoCorpus(root));
        }
        // A read, so it never writes: a corpus at another schema, or with no
        // config at all, refuses to open until `neb migrate` has brought it
        // forward (STD-03 §R6).
        let config = Config::load(&root)?;
        Ok(Self { root, config })
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
    pub fn init(root: &Path) -> Result<Self> {
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
        })
    }

    /// The config `init` keeps, or `None` when it is to write a fresh one.
    ///
    /// A config that is there is validated before anything is created, so a
    /// malformed or incompatible one is refused without mutation, while a
    /// valid identity and settings survive an interrupted init.
    fn config_to_keep(root: &Path) -> Result<Option<Config>> {
        match config::declared(root)? {
            Declared::Current { config, .. } => Ok(Some(config)),
            Declared::Missing if holds_content(root)? => Err(Error::MissingConfig {
                path: root.join(config::FILE),
            }),
            Declared::Missing => Ok(None),
            Declared::Older { version, .. } | Declared::Newer { version } => {
                Err(config::schema_mismatch(root, version))
            }
        }
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
    pub fn open_or_init(explicit: Option<PathBuf>) -> Result<(Self, bool)> {
        let root = Self::resolve_root(explicit)?;
        match Self::open(Some(root.clone())) {
            Err(Error::NoCorpus(_)) => Self::init(&root).map(|corpus| (corpus, true)),
            other => other.map(|corpus| (corpus, false)),
        }
    }

    /// Where the corpus lives, as it was given.
    pub fn root(&self) -> &Path {
        &self.root
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
    pub fn lock(&self) -> Result<CorpusLock> {
        CorpusLock::acquire(&self.root)
    }

    /// Take the write lock with a caller-chosen wait bound. A responsive UI
    /// can refuse a busy writer sooner while keeping the same critical section.
    pub fn lock_within(&self, wait: std::time::Duration) -> Result<CorpusLock> {
        CorpusLock::acquire_within(&self.root, wait)
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

    /// Where `observatory` references resolve on this machine:
    /// `$OBSERVATORY_ROOT`, else this machine's
    /// `~/.config/nebula/observatory-root`, else the legacy `observatory_root`
    /// key in `config.yaml`, else nowhere. The result names which one
    /// answered, and carries the legacy key whenever the file has one.
    ///
    /// The paths are used as given and never canonicalized, like every other
    /// path here.
    ///
    /// The environment and the machine file are read now; the legacy key is
    /// the snapshot [`Self::open`] took, which is what a read wants: it
    /// answers without waiting on a writer. Fails only on a machine setting
    /// that is unreadable, empty or relative.
    pub fn observatory_root(&self) -> Result<ObservatoryRoot> {
        let env = std::env::var_os(config::OBSERVATORY_ROOT_ENV).filter(|v| !v.is_empty());
        // The environment outranks the machine file, which is then not read
        // at all: a machine whose file is broken still works while the
        // variable is exported.
        let machine = match env {
            Some(_) => None,
            None => Self::configured_observatory_root()?,
        };
        Ok(ObservatoryRoot::from_settings(
            env,
            machine,
            self.config.legacy_observatory_root().map(Path::to_path_buf),
        ))
    }

    /// Remove the legacy `observatory_root` key from `config.yaml`. When the
    /// file on disk no longer carries it, nothing is written.
    ///
    /// The file stays machine-written: this rewrites it whole, header and
    /// all, rather than editing a line — which is why the reload comes
    /// first: this removes one key and must carry every other key across as
    /// it stands under the caller's lock, not as it stood when the corpus was
    /// opened.
    pub(crate) fn drop_legacy_observatory_root(&mut self) -> Result<()> {
        self.reload_config()?;
        match self.config.observatory_root.take() {
            Some(_) => self.config.save(&self.root),
            None => Ok(()),
        }
    }

    /// Whether a write is followed by a commit, per `config.yaml`.
    ///
    /// The snapshot [`Self::open`] took, like the legacy key in
    /// [`Self::observatory_root`]: a read of the setting never waits on a
    /// writer.
    pub fn commit_setting(&self) -> CommitSetting {
        CommitSetting {
            enabled: self.config.commit,
        }
    }

    /// Record in `config.yaml` whether writes are committed. Rewrites the
    /// file whole, like every other setting, and so reloads first for the
    /// same reason [`Self::drop_legacy_observatory_root`] does.
    pub(crate) fn set_commit(&mut self, enabled: bool) -> Result<()> {
        self.reload_config()?;
        self.config.commit = enabled;
        self.config.save(&self.root)
    }

    /// Commit the corpus after a write, when `config.yaml` asks for it and
    /// the root is inside a git work tree.
    ///
    /// Stages `nodes/`, `inbox/`, `config.yaml` and the generated `.gitignore`
    /// under the root and nothing else — not `.lock`, which records nothing
    /// about the corpus — and commits exactly those paths as
    /// `neb <verb> <ids>`. The commit names them as its pathspec, so whatever
    /// else is staged in the repository, before this runs or while it does,
    /// is neither committed nor unstaged (STD-03 §R28). The write is on disk
    /// before this runs and stays there whatever git says. Never pushes.
    ///
    /// The setting is read from disk under the caller's lock rather than from
    /// the snapshot: whether this write is recorded is a question about the
    /// configuration in force, and a writer that waited its turn opened before
    /// the writer ahead of it had finished saying what that configuration is.
    pub(crate) fn commit(&self, verb: &str, ids: &[&str]) -> Result<CommitOutcome> {
        if !self.current_config()?.commit {
            return Ok(CommitOutcome::Disabled);
        }
        let root = &self.root;
        // Fail closed (integrity, STD-02 §R31): a repository git cannot read
        // is an error here, never a reason to leave the write unrecorded
        // without a word.
        if !inside_work_tree(root)? {
            return Ok(CommitOutcome::NotARepository);
        }
        refuse_ignored_corpus(root)?;
        let paths = commit_pathspec(root);
        if paths.is_empty() {
            return Ok(CommitOutcome::NothingToCommit);
        }
        let mut add = vec!["add", "-A", "--"];
        add.extend(&paths);
        git_ok(root, &add)?;
        let changed = staged_paths(root, &paths)?;
        if changed.is_empty() {
            // The write changed nothing git can see.
            return Ok(CommitOutcome::NothingToCommit);
        }
        let message = match ids {
            [] => format!("neb {verb}"),
            ids => format!("neb {verb} {}", ids.join(" ")),
        };
        // `--only` commits the named paths and nothing else the index holds,
        // so work someone else stages meanwhile stays staged and theirs.
        let mut commit = vec!["commit", "-q", "-m", &message, "--only", "--"];
        commit.extend(&changed);
        git_ok(root, &commit)?;
        let hash = git_ok(root, &["rev-parse", "HEAD"])?.trim().to_string();
        Ok(CommitOutcome::Committed(Committed { hash, message }))
    }

    /// Path of a node file, whether or not it exists.
    ///
    /// Public so a consumer that hands a node to something outside the
    /// corpus (the desktop's "open in editor") asks for the path rather than
    /// re-deriving the layout. Fallible because an id becomes a path here:
    /// one that is not [`is_path_safe_id`] is refused rather than joined, so
    /// no caller can be handed a path outside `nodes/`.
    pub fn node_path(&self, id: &str) -> Result<PathBuf> {
        // "Ids stay strings, checked where they become paths" (4_decisions.md, STD-02@2 §R14).
        if !is_path_safe_id(id) {
            return Err(Error::UnsafeId(id.to_string()));
        }
        refuse_nodes_symlink(&self.root)?;
        Ok(self.root.join("nodes").join(format!("{id}.md")))
    }

    /// Read one node.
    ///
    /// The file's name and the id it stores are one fact, so a file that
    /// stores a different id is refused rather than read: [`Self::save`]
    /// derives its destination from the stored id, and a node loaded from
    /// one file that claims to be another would be written to that other
    /// one. Refusing here is what keeps a hand edit from turning a later
    /// verb into an overwrite.
    pub fn load(&self, id: &str) -> Result<Doc> {
        let path = self.node_path(id)?;
        if !path.exists() {
            return Err(Error::NoSuchNode(id.to_string()));
        }
        let doc = self.read_node(&path)?;
        self.require_file_agrees(&path, &doc)?;
        Ok(doc)
    }

    /// Commits that changed one node, newest first.
    pub fn history(&self, id: &str) -> Result<Vec<HistoryEntry>> {
        // The id first, then the machine: an id that could not name a node
        // is refused whether or not the corpus happens to be under git.
        self.node_path(id)?;
        self.require_git()?;
        self.load(id)?;
        let path = format!("nodes/{id}.md");
        let raw = git_ok(
            &self.root,
            &[
                "log",
                "-z",
                "--follow",
                "--format=%H%x00%cs%x00%s",
                "--",
                &path,
            ],
        )?;
        let fields: Vec<&str> = raw.split('\0').filter(|field| !field.is_empty()).collect();
        let (records, remainder) = fields.as_chunks::<3>();
        if !remainder.is_empty() {
            return Err(Error::corpus("git log returned a malformed history record"));
        }
        Ok(records
            .iter()
            .map(|[hash, date, message]| HistoryEntry {
                hash: hash.to_string(),
                date: date.to_string(),
                message: message.to_string(),
            })
            .collect())
    }

    /// Read one node as it existed at a commit hash or at the end of a date.
    pub fn load_at(&self, id: &str, at: &str) -> Result<Doc> {
        // The id becomes half of a git pathspec here rather than a path on
        // disk, and `git show <rev>:nodes/../../x.md` reads outside the
        // corpus just as readily as an open would. Checked before the work
        // tree, so the refusal does not depend on the machine.
        let node_path = self.node_path(id)?;
        self.require_git()?;
        let path = format!("nodes/{id}.md");
        let revision = if model::is_iso_date(at) {
            let before = format!("{at} 23:59:59");
            git_ok(
                &self.root,
                &[
                    "rev-list",
                    "-1",
                    &format!("--before={before}"),
                    "HEAD",
                    "--",
                    &path,
                ],
            )?
            .trim()
            .to_string()
        } else if at.len() >= 4 && at.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            at.to_string()
        } else {
            return Err(Error::corpus(format!(
                "invalid --at value `{at}`: expected a YYYY-MM-DD date or git revision"
            )));
        };
        let absent = || Error::NoNodeAtRevision {
            node: id.to_string(),
            revision: at.to_string(),
        };
        if revision.is_empty() {
            return Err(absent());
        }

        // Three different answers, told apart by git's exit status rather
        // than its words: no such commit, no such node in that commit, and
        // git failing (STD-02 §R30).
        if !self.resolves(&format!("{revision}^{{commit}}"))? {
            return Err(Error::UnknownRevision {
                revision: at.to_string(),
            });
        }
        let prefix = git_ok(&self.root, &["rev-parse", "--show-prefix"])?;
        let object = format!("{revision}:{}nodes/{id}.md", prefix.trim());
        if !self.resolves(&object)? {
            return Err(absent());
        }
        let shown = git_ok(&self.root, &["show", "--no-ext-diff", "--format=", &object])?;
        let doc = model::parse(&shown).map_err(|e| match e {
            // Same reporting as a read from disk: an id that came out of a
            // file is a fact about that file.
            Error::UnsafeId(id) => Error::IdMismatch {
                path: node_path.clone(),
                id,
            },
            other => other,
        })?;
        // Same agreement as [`Self::load`], one revision back: a historical
        // file that stores another node's id is not this node's history.
        if doc.node.id != id {
            return Err(Error::IdMismatch {
                path: node_path,
                id: doc.node.id,
            });
        }
        Ok(doc)
    }

    /// Whether `object` names something in the corpus's repository:
    /// `false` when git says it does not, [`Error::Git`] when git failed.
    fn resolves(&self, object: &str) -> Result<bool> {
        let out = git(&self.root, &["rev-parse", "--verify", "--quiet", object])?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_failed(&self.root, "rev-parse", &out.stderr.text())),
        }
    }

    fn require_git(&self) -> Result<()> {
        if inside_work_tree(&self.root)? {
            Ok(())
        } else {
            Err(Error::NotGitWorkTree(self.root.clone()))
        }
    }

    /// Write one node, stamping `updated`.
    ///
    /// The destination comes from the stored id, so the id is checked before
    /// anything is written: a `Doc` reaching here holds an id that parsed
    /// ([`crate::model`] refuses an unsafe one) and that agreed with its file
    /// ([`Self::load`]), and this is the last of the three places that has to
    /// hold for a write to land where the node already lives.
    pub fn save(&self, doc: &mut Doc) -> Result<()> {
        let path = self.node_path(&doc.node.id)?;
        doc.node.updated = today();
        model::write(&path, doc)
    }

    /// Write a node that must not already exist.
    pub fn create(&self, doc: &Doc) -> Result<()> {
        let path = self.node_path(&doc.node.id)?;
        if path.exists() {
            return Err(Error::NodeExists(doc.node.id.clone()));
        }
        model::write(&path, doc)
    }

    /// Every node in the corpus, sorted by id.
    ///
    /// A full scan, deliberately. The corpus is small and writes are rare, so
    /// an index would be a second source of truth that could drift for no gain.
    pub fn load_all(&self) -> Result<Vec<Doc>> {
        let dir = self.root.join("nodes");
        let mut out = Vec::new();
        refuse_nodes_symlink(&self.root)?;
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_node_file_name(p))
            .collect();
        paths.sort();
        for p in paths {
            let doc = self.read_node(&p)?;
            self.require_file_agrees(&p, &doc)?;
            out.push(doc);
        }
        Ok(out)
    }

    /// Read one node file with the current model.
    ///
    /// A node that will not parse and reads as a v1 node is named as one
    /// ([`Error::V1NodeUnderCurrentSchema`]): most likely an older `neb`
    /// stamped this corpus's config over files from before it, and the
    /// parser's unknown-field complaint alone does not lead anyone to the
    /// repair.
    fn read_node(&self, path: &Path) -> Result<Doc> {
        model::read(path).map_err(|error| match error {
            Error::Yaml { .. } => crate::migrate::v1_node_under_current_schema(
                &self.root,
                path,
                self.config.schema_version,
            )
            .unwrap_or(error),
            other => other,
        })
    }

    /// Refuse a node file whose name is not the id it stores.
    ///
    /// Both doors come through here. [`Self::load`] arrives with the path the
    /// caller's id names, and a scan arrives with a path it found on disk;
    /// either way the question is the same, and answering it in one place is
    /// what keeps a file called one thing and claiming to be another from
    /// reading as the node it claims — or, through [`Self::save`], from
    /// becoming a write over that node.
    ///
    /// A node has one directory entry, and the names are compared by asking
    /// the filesystem whether they open that one entry. A volume may store a
    /// name in a different Unicode normalization than the id it was written
    /// from — `título` is two spellings of the same word — and a byte
    /// comparison would call a perfectly ordinary node a mismatch. What the
    /// fallback asks is [`is_same_entry`]: whether the path the id names *is*
    /// this entry. Nothing is canonicalized; both paths are used as given,
    /// which is what keeps a symlinked root answering the same way on either
    /// platform.
    ///
    /// Equal *text* is not that question and never was. Two distinct files
    /// hold equal text the moment one is copied over the other, so a
    /// comparison of contents let `nodes/safe.md` — a byte-for-byte copy of
    /// `nodes/victim.md` — authorize `victim` as the id `safe` had asked
    /// for, and the write that followed landed on the other node.
    ///
    /// Nor is one *file* under two entries. `nodes/safe.md` hard-linked to,
    /// or a symlink at, `nodes/victim.md` opens the same bytes, but it is a
    /// second name for the node, and neither door can keep it coherent: a
    /// scan reads the node twice, and [`write_private_atomic`] replaces the
    /// entry the id names with a new file, so a hard link keeps the old bytes
    /// under the other name and the next load refuses. So an alias is refused wherever
    /// it is met, and it is the alias that is named, since it is what has to
    /// go. A hard link is met even when the node is loaded through its own
    /// name, because that is the load whose write would split the pair; a
    /// symlink is not, because a write leaves it pointing at the new file.
    fn require_file_agrees(&self, path: &Path, doc: &Doc) -> Result<()> {
        let id = OsStr::new(doc.node.id.as_str());
        let refuse = |path: &Path| {
            Err(Error::IdMismatch {
                path: path.to_path_buf(),
                id: doc.node.id.clone(),
            })
        };
        let links = hard_links_beside(path)?;
        if links.len() > 1 {
            let alias = links.iter().find(|link| link.file_stem() != Some(id));
            return refuse(alias.map_or(path, PathBuf::as_path));
        }
        if path.file_stem() == Some(id) {
            return Ok(());
        }
        if is_same_entry(path, &self.node_path(&doc.node.id)?) {
            return Ok(());
        }
        refuse(path)
    }

    /// Append a capture to the current month's inbox file.
    ///
    /// Text that spans lines is joined onto one with [`capture_line`] rather
    /// than refused: the inbox holds one entry per line, and a refusal would
    /// lose the thought at the moment it arrived. Only text that is nothing
    /// but whitespace is refused.
    ///
    /// The line, with the newline that repairs a month file missing its last
    /// one, is built whole and handed to one append, so a crash can lose the
    /// capture but never leave half of it in the inbox.
    pub fn capture(&self, text: &str) -> Result<InboxEntry> {
        let text = capture_line(text);
        if text.is_empty() {
            return Err(Error::corpus("nothing to capture"));
        }
        let dir = self.root.join("inbox");
        refuse_inbox_symlink(&dir)?;
        create_private_dir_all(&dir)?;
        refuse_inbox_symlink(&dir)?;
        let now = stamp();
        let month = &now[..7];
        let path = dir.join(format!("{month}.md"));
        refuse_inbox_symlink(&path)?;
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let inbox = self.inbox()?;
        let id = unique_entry_id(&format!("{now}{text}"), &inbox)?;
        let line = existing.lines().count();
        refuse_inbox_symlink(&dir)?;
        refuse_inbox_symlink(&path)?;
        let repair = if !existing.is_empty() && !existing.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        append_private(&path, format!("{repair}- [{id}] {now} {text}\n").as_bytes())?;
        Ok(InboxEntry {
            id,
            at: now.clone(),
            stamp: now,
            text,
            file: path,
            line,
        })
    }

    /// Inbox entries that have not been promoted or dropped.
    pub fn inbox(&self) -> Result<Inbox> {
        let mut out = Vec::new();
        for file in self.inbox_files()? {
            for (lineno, line) in std::fs::read_to_string(&file)?.lines().enumerate() {
                if let Some(e) = InboxEntry::parse(line, &file, lineno) {
                    out.push(e);
                }
            }
        }
        Ok(Inbox(out))
    }

    /// The inbox's month files, oldest first. None when there is no inbox
    /// yet, which is a corpus nothing has been captured into.
    fn inbox_files(&self) -> Result<Vec<PathBuf>> {
        let dir = self.root.join("inbox");
        refuse_inbox_symlink(&dir)?;
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().ok()?.is_file()
                    || !is_inbox_month_filename(&entry.file_name())
                {
                    return None;
                }
                Some(entry.path())
            })
            .collect();
        files.sort();
        Ok(files)
    }

    /// Find one live inbox entry by id.
    ///
    /// An id that only a struck-through line carries is refused with how
    /// that line says it was settled, rather than as an id nobody captured:
    /// the record is there, so the refusal can point at what became of it.
    pub fn inbox_entry(&self, id: &str) -> Result<InboxEntry> {
        if let Some(entry) = self.inbox()?.0.into_iter().find(|e| e.id == id) {
            return Ok(entry);
        }
        Err(match self.settlement(id)? {
            Some(settlement) => Error::InboxEntrySettled {
                id: id.to_string(),
                settlement,
            },
            None => Error::NoSuchInboxEntry(id.to_string()),
        })
    }

    /// How the settled entry `id` was settled, if a struck-through line
    /// records it.
    ///
    /// Ids are unique among live entries only, so one id can be settled more
    /// than once over the life of a corpus. The latest line wins: it is the
    /// capture whose id was most recently on offer.
    fn settlement(&self, id: &str) -> Result<Option<Settlement>> {
        let mut found = None;
        for file in self.inbox_files()? {
            for line in std::fs::read_to_string(&file)?.lines() {
                if let Some((settled, settlement)) = Settlement::parse(line)
                    && settled == id
                {
                    found = Some(settlement);
                }
            }
        }
        Ok(found)
    }

    /// Settle an inbox entry by striking it through in place.
    ///
    /// The line is never removed. What an idea looked like before it had a name
    /// is part of its history, and a dropped capture is a record of a road not
    /// taken rather than a mistake to erase.
    pub fn settle_inbox(&self, entry: &InboxEntry, outcome: &str) -> Result<()> {
        // Guard against settling an entry that belongs to a different corpus,
        // which would silently strike a line in someone else's inbox.
        if !entry.file.starts_with(self.root.join("inbox")) {
            return Err(Error::corpus(format!(
                "inbox entry `{}` is not in this corpus",
                entry.id
            )));
        }
        refuse_inbox_symlink(&self.root.join("inbox"))?;
        refuse_inbox_symlink(&entry.file)?;
        let content = std::fs::read_to_string(&entry.file)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        let Some(slot) = lines.get_mut(entry.line) else {
            return Err(Error::corpus(
                "inbox entry moved underneath us; nothing written",
            ));
        };
        if !slot.starts_with(&format!("- [{}]", entry.id)) {
            return Err(Error::corpus(
                "inbox entry moved underneath us; nothing written",
            ));
        }
        // The stamp goes back as the line held it, so settling a legacy
        // entry never rewrites when it was captured (STD-02 §R16).
        *slot = format!(
            "- ~~[{}] {} {}~~ {outcome}",
            entry.id, entry.stamp, entry.text
        );
        refuse_inbox_symlink(&self.root.join("inbox"))?;
        refuse_inbox_symlink(&entry.file)?;
        write_private_atomic(&entry.file, lines.join("\n") + "\n")?;
        Ok(())
    }
}

/// Check the nodes directory entry without resolving the corpus root. A root
/// reached through a symlink is supported, but `nodes/` must be a real
/// directory entry so node paths cannot reach a different tree.
pub(crate) fn refuse_nodes_symlink(root: &Path) -> Result<()> {
    let path = root.join("nodes");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::corpus(format!(
            "{} is a symlink; node operations require a real directory",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io_at("inspecting", &path, error)),
    }
}

/// The one inbox line a piece of captured text is stored as.
///
/// Every line break, `\n` or `\r` alike, becomes a single space, together
/// with the whitespace around it, and blank lines vanish, so text pasted or
/// piped in over several lines reads as the sentence it was. Whitespace
/// inside a line is the author's and stays; the ends are trimmed. The result
/// never holds a line break, which is what keeps the inbox at one entry per
/// line, and it is empty exactly when the text was nothing but whitespace.
///
/// [`Corpus::capture`] applies it, so the CLI, the desktop app and anything
/// else that captures store the same line for the same text.
pub fn capture_line(text: &str) -> String {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `root` holds anything a corpus is made of: a node file under
/// `nodes/` or a month file under `inbox/`. What tells a root an init was
/// interrupted in, which is still empty, from a corpus that has lost its
/// config.
fn holds_content(root: &Path) -> Result<bool> {
    for (dir, is_content) in [
        ("nodes", is_node_file_name as fn(&Path) -> bool),
        ("inbox", |path: &Path| {
            path.file_name().is_some_and(is_inbox_month_filename)
        }),
    ] {
        let dir = root.join(dir);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io_at("reading", &dir, error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| Error::io_at("reading", &dir, error))?;
            if is_content(&entry.path()) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Whether `dir` is a corpus root: [`Corpus::discover`]'s marker.
fn holds_corpus(dir: &Path) -> bool {
    dir.join("nodes").is_dir()
        && std::fs::read_to_string(dir.join(config::FILE))
            .is_ok_and(|raw| config::names_a_corpus(&raw))
}

/// The working directory as the shell names it.
///
/// `$PWD` when it is absolute, has no `.` or `..` in it, and is the same
/// directory as `.`, which is the rule `pwd -L` follows; otherwise what the
/// OS reports. A corpus reached through a symlink is then found under the
/// spelling the user typed rather than the one the kernel resolved, because
/// paths are used as given. `None` when there is no working directory to
/// speak of, as when it has been removed: there is nothing to discover from.
fn working_dir() -> Option<PathBuf> {
    let logical = std::env::var_os("PWD").map(PathBuf::from).filter(|pwd| {
        pwd.is_absolute()
            && !pwd
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
            && same_dir(pwd, Path::new("."))
    });
    logical.or_else(|| std::env::current_dir().ok())
}

/// Whether two paths name the same directory, by identity rather than by
/// spelling.
#[cfg(unix)]
fn same_dir(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Without inode identity there is no cheap way to tell, so `$PWD` is never
/// trusted and the OS's answer stands.
#[cfg(not(unix))]
fn same_dir(_: &Path, _: &Path) -> bool {
    false
}

/// Check the named inbox entry itself, without resolving the corpus root. A
/// symlinked root is a supported way to reach a corpus, but a symlink planted
/// at `inbox/` or a month file must not redirect an inbox write.
fn refuse_inbox_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::corpus(format!(
            "{} is a symlink; inbox operations require real paths",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io_at("inspecting", path, error)),
    }
}

/// Whether a file name is one of the inbox's `YYYY-MM.md` month files.
fn is_inbox_month_filename(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let bytes = name.as_bytes();
    bytes.len() == 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && matches!(&bytes[5..7], [b'0', b'1'..=b'9'] | [b'1', b'0'..=b'2'])
        && &bytes[7..] == b".md"
}

/// A commit `neb` made after a write.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Committed {
    /// The full commit hash.
    pub hash: String,
    /// The message, `neb <verb> <ids>`.
    pub message: String,
}

/// What the commit after a write did. The write is on disk before the commit
/// runs, so it stays there whichever this is.
#[must_use = "a write left uncommitted is a fact to report, not to drop"]
#[derive(Debug, Clone)]
pub enum CommitOutcome {
    /// The corpus paths were committed, as `neb <verb> <ids>`.
    Committed(Committed),
    /// `config.yaml` does not ask for commits.
    Disabled,
    /// Commits are on, but there is no git repository at or above the root,
    /// so the write was not recorded.
    NotARepository,
    /// The write left nothing git would record.
    NothingToCommit,
}

/// One commit that changed a node.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HistoryEntry {
    /// The full commit hash.
    pub hash: String,
    /// The commit date, `YYYY-MM-DD`.
    pub date: String,
    /// The commit's first-line message.
    pub message: String,
}

/// What a `neb` commit may contain, relative to the corpus root. Everything
/// else under the root, and everything outside it, is left alone — the write
/// lock's `.lock` included, which is why it is not listed here and never
/// will be: it is a fact about which process is writing right now, not about
/// the corpus, and it means nothing on another machine. `.gitignore` is
/// corpus setup metadata: `init` maintains it so ordinary git commands cannot
/// mistake the lock for corpus content.
const GITIGNORE_FILE: &str = ".gitignore";
const COMMIT_PATHS: [&str; 4] = ["nodes", "inbox", config::FILE, GITIGNORE_FILE];

/// The pathspec a `neb` commit stages and commits: the [`COMMIT_PATHS`] that
/// exist under `root`, relative to it. Only those, because `inbox/` appears
/// on the first capture and a pathspec that matches nothing is a git error.
fn commit_pathspec(root: &Path) -> Vec<&'static str> {
    COMMIT_PATHS
        .iter()
        .copied()
        .filter(|path| root.join(path).exists())
        .collect()
}

/// Refuse a corpus that the repository around it ignores: `commit` on would
/// otherwise record nothing, forever, without a word. A private repository at
/// the corpus root is the fix.
fn refuse_ignored_corpus(root: &Path) -> Result<()> {
    let ignored = git(root, &["check-ignore", "-q", "--", "nodes"])?;
    match ignored.status.code() {
        Some(0) => Err(Error::CorpusIgnored(root.to_path_buf())),
        Some(1) => Ok(()),
        _ => Err(git_failed(root, "check-ignore", &ignored.stderr.text())),
    }
}

/// The entries of `pathspec` under which the index holds a change `HEAD`
/// does not: what a `neb` commit names, since `git commit --only` refuses an
/// entry that matches nothing git knows, such as the empty `nodes/` of a new
/// corpus. Each entry is asked about alone and nothing else is: something
/// staged elsewhere is not the corpus's to commit, so it is not something to
/// commit either. The entries are paths, never `:(exclude)` magic, which
/// asked about alone would mean everything else: a file to keep out of a
/// commit is kept out of the `git add` before it, since `--only` commits only
/// paths git already tracks.
fn staged_paths<'a>(root: &Path, pathspec: &[&'a str]) -> Result<Vec<&'a str>> {
    let mut staged = Vec::new();
    for &path in pathspec {
        let diff = git(root, &["diff", "--cached", "--quiet", "--", path])?;
        match diff.status.code() {
            Some(0) => {}
            Some(1) => staged.push(path),
            _ => return Err(git_failed(root, "diff", &diff.stderr.text())),
        }
    }
    Ok(staged)
}

/// Keep the process-local advisory lock out of the corpus repository.
///
/// Existing ignore content is preserved. Re-running `init` is idempotent when
/// the final effective rule is already ours; if the user later adds another
/// rule, a later `init` puts this root-specific rule last again. Git does not
/// follow a `.gitignore` symlink, so even a symlink whose target already ends
/// in our rule is replaced atomically with a regular file containing the same
/// bytes. The target itself is never changed.
fn ensure_lock_ignored(root: &Path) -> Result<()> {
    let path = root.join(GITIGNORE_FILE);
    let is_symlink = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(Error::io_at("inspecting", &path, error)),
    };
    let mut contents = match std::fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(Error::io_at("reading", &path, error)),
    };
    let expected = format!("/{LOCK_FILE}");
    let last_rule = contents
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .rfind(|line| !line.is_empty() && !line.starts_with(b"#"));
    if last_rule == Some(expected.as_bytes()) {
        return if is_symlink {
            write_private_atomic(&path, contents)
        } else {
            Ok(())
        };
    }

    if !contents.is_empty() && !contents.ends_with(b"\n") {
        contents.push(b'\n');
    }
    contents.extend_from_slice(expected.as_bytes());
    contents.push(b'\n');
    write_private_atomic(&path, contents)
}

/// Run git at the corpus root, through the one supervised runner. Git not
/// starting, running out of time or being stopped is what this reports;
/// whether the command succeeded is the caller's to judge, since a non-zero
/// exit is an answer for some of them.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<GitOutput> {
    git::run_git(root, args).map_err(|e| e.into_error(root, args.first().copied().unwrap_or("git")))
}

/// Run git at the corpus root and require it to succeed; all of stdout as
/// text.
fn git_ok(root: &Path, args: &[&str]) -> Result<String> {
    let context = args.first().copied().unwrap_or("git");
    let out = git(root, args)?;
    if !out.status.success() {
        return Err(git_failed(root, context, &out.stderr.text()));
    }
    Ok(String::from_utf8_lossy(out.stdout_whole(root, context)?).into_owned())
}

fn git_failed(root: &Path, context: &str, stderr: &str) -> Error {
    Error::Git {
        root: root.to_path_buf(),
        context: context.to_string(),
        stderr: stderr.trim().to_string(),
    }
}

/// Whether the root is inside a git work tree.
///
/// `false` when there is no repository to find
/// ([`git::repository_expected`]), without running git, or when git found
/// one and says the root is not in its work tree. With a repository there, git
/// failing — not starting, or unable to read it — is [`Error::Git`], never
/// "not a work tree" (STD-02 §R29).
pub(crate) fn inside_work_tree(root: &Path) -> Result<bool> {
    if !git::repository_expected(root) {
        return Ok(false);
    }
    let out = git(root, &["rev-parse", "--is-inside-work-tree"])?;
    if !out.status.success() {
        return Err(git_failed(root, "rev-parse", &out.stderr.text()));
    }
    Ok(String::from_utf8_lossy(out.stdout_whole(root, "rev-parse")?).trim() == "true")
}

/// Every capture still waiting to be promoted or dropped.
///
/// Serializes as the bare list, which is the shape `neb inbox --json` has
/// always had.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Inbox(pub Vec<InboxEntry>);

impl Inbox {
    /// The earliest other waiting entry that says what `entry` says.
    ///
    /// "Says" is equality after folding case and collapsing whitespace, so a
    /// thought typed twice with different capitals or spacing is caught and a
    /// reworded one is not: that is `near`'s job, and it searches nodes. Only
    /// live entries are here, so a settled capture never counts.
    pub fn same_as(&self, entry: &InboxEntry) -> Option<&InboxEntry> {
        let said = fold(&entry.text);
        self.0
            .iter()
            .find(|other| other.id != entry.id && fold(&other.text) == said)
    }
}

/// Text as the duplicate check compares it: lowercase, one space between words.
fn fold(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What became of a settled inbox entry, as its struck-through line records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    /// Promoted into the node with this id.
    Promoted(String),
    /// Dropped without becoming a node.
    Dropped,
}

impl Settlement {
    /// The id and outcome of a line [`Corpus::settle_inbox`] wrote, which is
    /// `- ~~[<id>] <at> <text>~~ <outcome>`. A struck line whose outcome is
    /// neither of the two that verb writes was edited by hand, and is not
    /// read as either.
    fn parse(line: &str) -> Option<(&str, Self)> {
        let rest = line.strip_prefix("- ~~[")?;
        let (id, rest) = rest.split_once("] ")?;
        let (_, outcome) = rest.rsplit_once("~~")?;
        let outcome = outcome.trim();
        if outcome == "dropped" {
            return Some((id, Self::Dropped));
        }
        let node = outcome.strip_prefix("->")?.trim();
        (!node.is_empty()).then(|| (id, Self::Promoted(node.to_string())))
    }
}

impl std::fmt::Display for Settlement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Promoted(node) => write!(f, "promoted to `{node}`"),
            Self::Dropped => f.write_str("dropped"),
        }
    }
}

/// One unprocessed capture.
///
/// Where the line lives is how [`Corpus::settle_inbox`] finds it again, and is
/// a detail of this corpus rather than part of the entry, so it stays out of
/// the serialized form.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct InboxEntry {
    /// Short id, unique among the corpus's live inbox entries.
    pub id: String,
    /// When it was captured: RFC 3339 with the offset the stamp was taken
    /// at, ending in `Z` when the local offset could not be read. A legacy
    /// `YYYY-MM-DDTHH:MM` stamp reads as local time with the offset this
    /// machine has for that instant. A stamp that is neither, edited in by
    /// hand, is passed through as written.
    pub at: String,
    /// The stamp as the inbox line holds it, which settling writes back
    /// unchanged.
    #[serde(skip)]
    pub(crate) stamp: String,
    /// What you wrote.
    pub text: String,
    /// Which inbox file it lives in.
    #[serde(skip)]
    pub file: PathBuf,
    /// Zero-based line within that file.
    #[serde(skip)]
    pub line: usize,
}

impl InboxEntry {
    fn parse(line: &str, file: &Path, lineno: usize) -> Option<Self> {
        // `- ~~...~~` is a settled entry: promoted or dropped, kept for the record.
        let rest = line.strip_prefix("- [")?;
        let (id, rest) = rest.split_once("] ")?;
        let (stamp, text) = rest.split_once(' ')?;
        Some(Self {
            id: id.to_string(),
            at: rfc3339_stamp(stamp).unwrap_or_else(|| stamp.to_string()),
            stamp: stamp.to_string(),
            text: text.trim().to_string(),
            file: file.to_path_buf(),
            line: lineno,
        })
    }
}

/// Today, local, as `YYYY-MM-DD`.
pub(crate) fn today() -> String {
    now()
        .format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

/// Now, as an inbox stamp: RFC 3339 to the second, with the local offset,
/// or in UTC marked `Z` when the local offset cannot be read. The first seven
/// characters are the month, which names the inbox file.
pub(crate) fn stamp() -> String {
    let now = OffsetDateTime::now_utc();
    format_stamp(now, UtcOffset::local_offset_at(now).ok())
}

/// `instant` as an inbox stamp at `offset`, or in UTC marked `Z` when there
/// is none.
///
/// `Z` and `+00:00` are the same instant, and the difference is kept on
/// purpose: `+00:00` is a local offset of zero that was read, `Z` is a
/// fallback, so a stamp never passes a guess off as local time
/// (STD-01 §R11).
fn format_stamp(instant: OffsetDateTime, offset: Option<UtcOffset>) -> String {
    match offset {
        Some(offset) => instant.to_offset(offset).format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
        )),
        None => instant
            .to_offset(UtcOffset::UTC)
            .format(format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z")),
    }
    .unwrap_or_default()
}

/// When an inbox stamp says a capture happened, in either form it may be in.
///
/// RFC 3339 is read as written. The legacy `YYYY-MM-DDTHH:MM` form, which
/// carries no offset, is local time with the offset this machine has for
/// that instant, or UTC when there is none: the reading recorded in
/// `docs/design/lineage-graph/4_decisions.md`.
pub(crate) fn parse_stamp(stamp: &str) -> Option<OffsetDateTime> {
    if let Ok(at) = OffsetDateTime::parse(stamp, &Rfc3339) {
        return Some(at);
    }
    let (wall, offset) = legacy_stamp(stamp)?;
    Some(wall.assume_offset(offset.unwrap_or(UtcOffset::UTC)))
}

/// A stamp as RFC 3339: itself when it already is, the legacy form read as
/// [`parse_stamp`] reads it, and `None` when it is neither.
fn rfc3339_stamp(stamp: &str) -> Option<String> {
    if OffsetDateTime::parse(stamp, &Rfc3339).is_ok() {
        return Some(stamp.to_string());
    }
    let (wall, offset) = legacy_stamp(stamp)?;
    Some(format_stamp(
        wall.assume_offset(offset.unwrap_or(UtcOffset::UTC)),
        offset,
    ))
}

/// A legacy `YYYY-MM-DDTHH:MM` stamp's wall-clock time, and the local
/// offset for it, if this machine can say.
///
/// The offset is looked up twice because it depends on the instant, which
/// depends on the offset: the second lookup settles a wall-clock time within
/// a few hours of a daylight-saving change.
fn legacy_stamp(stamp: &str) -> Option<(PrimitiveDateTime, Option<UtcOffset>)> {
    let wall = PrimitiveDateTime::parse(
        stamp,
        format_description!("[year]-[month]-[day]T[hour]:[minute]"),
    )
    .ok()?;
    let offset = UtcOffset::local_offset_at(wall.assume_utc())
        .ok()
        .map(|guess| UtcOffset::local_offset_at(wall.assume_offset(guess)).unwrap_or(guess));
    Some((wall, offset))
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

/// Whole days between a `YYYY-MM-DD` string and today, if it parses.
pub(crate) fn days_since(date: &str) -> Option<i64> {
    let d = Date::parse(date, &Iso8601::DATE).ok()?;
    Some((now().date() - d).whole_days())
}

/// Whole days between the date an inbox stamp was taken on, as its own
/// offset has it, and today. Either stamp form; `None` when it parses as
/// neither.
pub(crate) fn days_since_stamp(stamp: &str) -> Option<i64> {
    Some((now().date() - parse_stamp(stamp)?.date()).whole_days())
}

/// A short id unique in the live inbox, or a refusal when all ids are occupied.
fn unique_entry_id(seed: &str, inbox: &Inbox) -> Result<String> {
    let used: HashSet<&str> = inbox.0.iter().map(|entry| entry.id.as_str()).collect();
    let mut h = fnv(seed);
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if !used.contains(id.as_str()) {
            return Ok(id);
        }
        h = fnv(&format!("{h}"));
    }

    // Hashing keeps the ordinary path short and makes ids hard to predict from
    // their position. Once that bounded path collides, walk the finite id space
    // rather than returning an occupied candidate. A full inbox is unusual but
    // valid input, and refusing it is the only unambiguous result.
    for candidate in 0..=u16::MAX {
        let id = format!("{candidate:04x}");
        if !used.contains(id.as_str()) {
            return Ok(id);
        }
    }
    Err(Error::corpus(
        "inbox id namespace exhausted; nothing captured",
    ))
}

/// A stable id for a corpus, derived from where it was created and when.
/// Opaque by design: it identifies, it does not describe.
pub(crate) fn corpus_id(root: &Path) -> String {
    let seed = format!("{}{}", root.display(), stamp());
    format!("neb-{:06x}", fnv(&seed) & 0xff_ffff)
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Turn a title into a node id.
///
/// A slug over 60 characters is cut at the last `-` at or before the limit,
/// never mid-word. A title with no dash in its first 60 characters (one long
/// word) cuts to nothing; the empty-id check at the call site turns that into
/// a refusal rather than a truncated word standing in for the whole title.
pub(crate) fn slugify(s: &str) -> String {
    cap(&dashed(s))
}

/// Lowercase Unicode letters and digits, every other run a single `-`, with
/// no leading or trailing dash. A slug before [`cap`] is applied.
fn dashed(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase().filter(|lower| lower.is_alphanumeric()));
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Cut a dashed slug to 60 characters at the last `-` at or before the limit.
fn cap(dashed: &str) -> String {
    if dashed.chars().count() <= 60 {
        return dashed.to_string();
    }
    let cut: String = dashed.chars().take(60).collect();
    match cut.rfind('-') {
        Some(i) => cut[..i].to_string(),
        None => String::new(),
    }
}

/// How many words an id minted from a raw capture keeps.
pub(crate) const CAPTURE_ID_WORDS: usize = 5;

/// Words that carry grammar rather than the idea, dropped from an id minted
/// from a raw capture. Negations (`not`, `no`, `never`, `without`) and
/// quantifiers (`all`, `only`, `every`) are deliberately absent: dropping one
/// would name the opposite idea.
const STOP_WORDS: &[&str] = &[
    "a", "about", "am", "an", "and", "any", "are", "as", "at", "be", "been", "being", "but", "by",
    "can", "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "may", "maybe", "might", "must", "of", "on",
    "onto", "or", "perhaps", "really", "s", "shall", "she", "should", "so", "some", "than", "that",
    "the", "their", "then", "there", "these", "they", "this", "those", "to", "very", "was", "we",
    "were", "what", "when", "where", "which", "who", "why", "will", "with", "would", "you",
];

/// The ids a capture promoted without `--title` or `--id` may take, most
/// preferred first. Empty when the text reduces to no usable id.
///
/// The title is then the whole captured sentence, and an id is permanent, so
/// the sentence's slug would be carried by every trace, link and edge. A
/// capture of [`CAPTURE_ID_WORDS`] words or fewer keeps exactly that slug.
/// A longer one drops [`STOP_WORDS`] and keeps the first
/// [`CAPTURE_ID_WORDS`] words left (or the first words of the sentence, when
/// every word is a stop word). The later candidates are fallbacks for a
/// collision, still derived from the text: one more significant word at a
/// time, then the full slug [`slugify`] would give. Every candidate follows
/// the slug rules, so it passes [`is_slug`].
pub(crate) fn capture_ids(text: &str) -> Vec<String> {
    let full = dashed(text);
    let words: Vec<&str> = full.split('-').filter(|w| !w.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    if words.len() > CAPTURE_ID_WORDS {
        let significant: Vec<&str> = words
            .iter()
            .copied()
            .filter(|w| !STOP_WORDS.contains(w))
            .collect();
        let significant = if significant.is_empty() {
            &words
        } else {
            &significant
        };
        for n in CAPTURE_ID_WORDS.min(significant.len())..=significant.len() {
            let id = cap(&significant[..n].join("-"));
            if !id.is_empty() && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    let full = cap(&full);
    if !full.is_empty() && !out.contains(&full) {
        out.push(full);
    }
    out
}

/// Whether `s` is already exactly what [`slugify`] would turn it into:
/// lowercase words joined by single dashes, no leading, trailing, or doubled
/// dash, 60 characters or fewer. Used to validate a user-supplied `--id`
/// against the same rule a derived id already has to follow.
pub(crate) fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 60 && slugify(s) == s
}

/// Whether `id` can name a node file and nothing else.
///
/// Every id becomes a path — `nodes/<id>.md` — so it has to be exactly one
/// ordinary file name: no separator, no `.` or `..`, no root or drive
/// prefix, no control character, and no surrounding whitespace that would
/// make two ids look like one name. `../../escaped` and `/etc/passwd` are
/// what this refuses; `ünïcode-título-ok` and `시간은-프레임의-수다` are
/// ordinary ids and stay valid, because the rule is about path structure
/// rather than about which alphabet an idea was named in.
///
/// Distinct from [`is_slug`], which is the stricter shape a *new* id has to
/// take. This is the weaker rule every id must satisfy, including one read
/// back out of a file somebody edited by hand, so tightening the id a verb
/// creates never silently makes an existing corpus unreadable.
///
/// Asked before any read or write derives a path, and nothing is
/// canonicalized: the components are judged as written, which is what keeps
/// the answer the same under a symlinked root.
pub(crate) fn is_path_safe_id(id: &str) -> bool {
    if id.is_empty() || id.trim() != id {
        return false;
    }
    if id
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\' || c == std::path::MAIN_SEPARATOR)
    {
        return false;
    }
    // One `Normal` component spelled exactly as the id: `.`, `..`, a root and
    // a Windows prefix are each their own component kind, and an id that
    // parses to anything else is not a file name.
    let mut components = Path::new(id).components();
    let single =
        matches!(components.next(), Some(Component::Normal(name)) if name == OsStr::new(id));
    single && components.next().is_none()
}

/// Whether a name in `nodes/` is one a scan reads as a node.
fn is_node_file_name(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "md")
}

/// Whether two paths name the same directory entry.
///
/// Identity, because that is the only reason two different spellings may name
/// one node: a volume may store a file name in a different Unicode
/// normalization than the id it was written from, and the filesystem is the
/// one that knows the two are one entry. A device and inode pair is that
/// answer, read without following the last component, so a symlink is its
/// own entry rather than the file it points at. Both paths are otherwise used
/// as given — nothing is resolved or canonicalized — so a corpus reached
/// through a symlinked root, which is every corpus under a macOS temporary
/// directory, answers this the same way a corpus reached directly does.
///
/// A hard link is the one case the pair cannot tell apart from a respelling,
/// which is why [`hard_links_beside`] is asked first.
///
/// A path that cannot be read is not the same entry as anything, including
/// itself: the caller is deciding whether to trust a mismatched name, and an
/// unanswered question is not a yes.
#[cfg(unix)]
fn is_same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Elsewhere the question does not arise. A Windows volume compares file
/// names case-insensitively but not across Unicode normalizations, so a name
/// that differs from the id it stores differs for some other reason, and the
/// byte comparison the caller already made is the whole answer.
#[cfg(not(unix))]
fn is_same_entry(_a: &Path, _b: &Path) -> bool {
    false
}

/// The node file names in `path`'s directory that are hard links to it,
/// itself included, sorted.
///
/// Empty for the ordinary file with one link, which is answered from its own
/// metadata without listing the directory. A link count above one sends the
/// question to the directory, because only links a scan would read as nodes
/// are aliases: a backup hard-linked from outside the corpus is not a second
/// name for the node, and one Unicode respelling of a name is one entry
/// however it is typed.
#[cfg(unix)]
fn hard_links_beside(path: &Path) -> Result<Vec<PathBuf>> {
    use std::os::unix::fs::MetadataExt;
    let Ok(file) = std::fs::symlink_metadata(path) else {
        return Ok(Vec::new());
    };
    if file.nlink() <= 1 {
        return Ok(Vec::new());
    }
    let Some(dir) = path.parent() else {
        return Ok(Vec::new());
    };
    let entries = std::fs::read_dir(dir).map_err(|error| Error::io_at("reading", dir, error))?;
    let mut links = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io_at("reading", dir, error))?;
        let link = entry.path();
        if !is_node_file_name(&link) {
            continue;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&link)
            && metadata.dev() == file.dev()
            && metadata.ino() == file.ino()
        {
            links.push(link);
        }
    }
    links.sort();
    Ok(links)
}

/// Elsewhere a hard link is not an alias this module can see, and the byte
/// comparison stands alone.
#[cfg(not(unix))]
fn hard_links_beside(_path: &Path) -> Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures are planted directly, beside the helper under test"
)]
mod tests {
    use super::*;
    use crate::fs::{Step, create_temporary_sibling, recording};

    #[test]
    fn atomic_write_removes_its_temporary_file_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("destination");
        std::fs::create_dir(&destination).unwrap();

        let error = write_private_atomic(&destination, "replacement").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("writing"), "{message}");
        assert!(
            message.contains(&destination.display().to_string()),
            "{message}"
        );
        assert!(
            matches!(error, Error::IoAt { source, .. } if source.raw_os_error().is_some()),
            "the OS cause was not preserved: {message}"
        );
        assert!(
            destination.is_dir(),
            "the failed rename left the target alone"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name != OsStr::new("destination"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the failed rename left temporary files: {leftovers:?}"
        );
    }

    /// The reported break: a symlink planted at the temporary path turned an
    /// ordinary note into a write outside the corpus, and then became the node.
    #[cfg(unix)]
    #[test]
    fn atomic_write_never_writes_through_a_temporary_planted_as_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside-sentinel.txt");
        std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
        let destination = dir.path().join("destination.md");
        std::fs::write(&destination, "original").unwrap();
        let planted = dir.path().join("destination.md.tmp");
        std::os::unix::fs::symlink(&outside, &planted).unwrap();

        write_private_atomic(&destination, "replacement").unwrap();

        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "IRREPLACEABLE FIXTURE",
            "the write reached a file outside the corpus"
        );
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "replacement"
        );
        assert!(
            std::fs::symlink_metadata(&destination)
                .unwrap()
                .file_type()
                .is_file(),
            "the planted symlink was renamed onto the destination"
        );
        assert!(
            std::fs::symlink_metadata(&planted)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the planted path is not ours to delete"
        );
    }

    /// The order is the durability: bytes flushed before the rename, so the
    /// name never points at a file the device has not got, and the directory
    /// flushed after it, so the rename itself is not lost to a crash.
    #[test]
    fn write_atomic_syncs_the_file_before_rename_and_the_parent_after() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("node.md");

        let (result, steps) = recording(|| write_private_atomic(&destination, "contents"));
        result.unwrap();

        let Some(Step::Write { path: tmp, bytes }) = steps.first() else {
            panic!("the first step is not the write: {steps:?}");
        };
        assert_eq!(bytes, b"contents");
        assert_eq!(tmp.parent(), Some(dir.path()), "{}", tmp.display());
        let mut expected = vec![
            Step::Write {
                path: tmp.clone(),
                bytes: b"contents".to_vec(),
            },
            Step::SyncAll(tmp.clone()),
            Step::Rename {
                from: tmp.clone(),
                to: destination.clone(),
            },
        ];
        if cfg!(unix) {
            expected.push(Step::SyncDir(dir.path().to_path_buf()));
        }
        assert_eq!(steps, expected);
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "contents");
    }

    /// Seven `write(2)` calls for one capture could be cut anywhere by a
    /// crash. One buffer, one `write_all`, then the data flushed.
    #[test]
    fn a_capture_line_is_built_whole_and_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = Corpus::init(&dir.path().join("corpus")).unwrap();
        let month = dir
            .path()
            .join("corpus")
            .join("inbox")
            .join(format!("{}.md", &stamp()[..7]));
        // A month file whose last line lost its newline to a hand edit.
        std::fs::write(&month, "- [0001] 2026-09-01T00:00 an earlier thought").unwrap();

        let (entry, steps) = recording(|| corpus.capture("a thought, torn nowhere"));
        let entry = entry.unwrap();

        let line = format!("\n- [{}] {} a thought, torn nowhere\n", entry.id, entry.at);
        assert_eq!(
            steps,
            [
                Step::Write {
                    path: month.clone(),
                    bytes: line.clone().into_bytes(),
                },
                Step::SyncData(month.clone()),
            ],
            "the repair newline and the line are one write, then synced"
        );
        assert_eq!(
            std::fs::read_to_string(&month).unwrap(),
            format!("- [0001] 2026-09-01T00:00 an earlier thought{line}")
        );

        // The next capture needs no repair, and is still one write.
        let (entry, steps) = recording(|| corpus.capture("and another"));
        let entry = entry.unwrap();
        assert_eq!(
            steps,
            [
                Step::Write {
                    path: month.clone(),
                    bytes: format!("- [{}] {} and another\n", entry.id, entry.at).into_bytes(),
                },
                Step::SyncData(month.clone()),
            ]
        );
    }

    #[test]
    fn a_temporary_sibling_is_fresh_each_time_and_hides_from_corpus_listings() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("safe.md");
        let (first, _handle) = create_temporary_sibling(&node).unwrap();
        let (second, _handle) = create_temporary_sibling(&node).unwrap();

        assert_ne!(
            first, second,
            "a stale temporary must not wedge the next write"
        );
        for tmp in [&first, &second] {
            assert_eq!(tmp.parent(), node.parent(), "the temporary is a sibling");
            assert!(
                tmp.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("safe.md."),
                "the temporary does not name its destination: {}",
                tmp.display()
            );
            assert_eq!(
                tmp.extension(),
                Some(OsStr::new("tmp")),
                "`load_all` reads every `.md` in `nodes`, so a temporary may not be one"
            );
        }

        let (month, _handle) = create_temporary_sibling(&dir.path().join("2026-09.md")).unwrap();
        assert!(
            !is_inbox_month_filename(month.file_name().unwrap()),
            "the inbox would read this temporary as a month file: {}",
            month.display()
        );
    }

    #[test]
    fn stamp_names_the_offset_it_used() {
        let instant = time::macros::datetime!(2026-09-26 06:11:05.25 UTC);

        let fallback = format_stamp(instant, None);
        assert_eq!(fallback, "2026-09-26T06:11:05Z");
        let local = format_stamp(instant, Some(time::macros::offset!(+2)));
        assert_eq!(local, "2026-09-26T08:11:05+02:00");
        let zero = format_stamp(instant, Some(UtcOffset::UTC));
        assert_eq!(zero, "2026-09-26T06:11:05+00:00");
        for stamp in [fallback, local, zero, stamp()] {
            let parsed = OffsetDateTime::parse(&stamp, &Rfc3339)
                .unwrap_or_else(|error| panic!("{stamp}: {error}"));
            assert_eq!(parse_stamp(&stamp), Some(parsed), "{stamp}");
            assert!(
                stamp.ends_with('Z') || stamp[stamp.len() - 6..].starts_with(['+', '-']),
                "{stamp} does not name its offset"
            );
            assert_eq!(stamp.find('.'), None, "{stamp} is to the second");
        }
    }

    #[test]
    fn a_legacy_stamp_reads_as_local_time_and_rfc3339_as_written() {
        let legacy = rfc3339_stamp("2026-09-01T08:00").unwrap();
        assert!(legacy.starts_with("2026-09-01T08:00:00"), "{legacy}");
        let parsed = OffsetDateTime::parse(&legacy, &Rfc3339).unwrap();
        assert_eq!(parse_stamp("2026-09-01T08:00"), Some(parsed));

        for written in ["2026-09-01T08:00:00+02:00", "2026-09-01T08:00:00.5-05:30"] {
            assert_eq!(rfc3339_stamp(written).as_deref(), Some(written));
        }
        for unreadable in ["someday", "2026-09-01", "2026-09-01T08:00:00", ""] {
            assert_eq!(rfc3339_stamp(unreadable), None, "{unreadable}");
            assert_eq!(parse_stamp(unreadable), None, "{unreadable}");
        }
    }

    #[test]
    fn a_short_title_slugifies_whole() {
        assert_eq!(slugify("Tags beat domains"), "tags-beat-domains");
    }

    #[test]
    fn unicode_titles_slugify_to_stable_valid_ids() {
        for (title, expected) in [
            ("Ünïcode título → ok", "ünïcode-título-ok"),
            ("시간은 프레임의 수다", "시간은-프레임의-수다"),
            ("Tags beat domains", "tags-beat-domains"),
        ] {
            let slug = slugify(title);
            assert_eq!(slug, expected);
            assert!(is_slug(&slug), "derived id is not a valid slug: {slug}");
        }
    }

    #[test]
    fn a_slug_over_the_limit_never_ends_mid_word() {
        let word = "abcdefg"; // 7 chars, so units of 8 with the joining dash
        let title = [word; 9].join(" ");
        let slug = slugify(&title);
        assert!(slug.chars().count() <= 60, "slug is over the limit: {slug}");
        assert!(!slug.is_empty());
        assert!(
            slug.split('-').all(|w| w == word),
            "slug has a partial word: {slug}"
        );
    }

    /// The title behind the frozen id in the bug report: the naive
    /// `.chars().take(60)` cut landed on a dash-adjacent boundary here by
    /// coincidence, but the fixed rule (cut at the last dash at or before 60)
    /// still applies and drops the trailing word rather than keeping a slug
    /// that happens to look intact.
    #[test]
    fn every_surviving_word_is_whole() {
        let title = "Self-authored structure is a paved path, imposed structure is rigidity";
        let slug = slugify(title);
        assert!(slug.chars().count() <= 60);
        assert!(!slug.is_empty());
        assert!(!slug.ends_with('-'));
        let words: Vec<String> = title
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        for part in slug.split('-') {
            assert!(
                words.iter().any(|w| w == part),
                "fragment `{part}` is not a whole word from the title"
            );
        }
    }

    /// The capture from the v0.2 evaluation: a 60-character sentence slug
    /// becomes five significant words, with longer fallbacks behind it.
    #[test]
    fn a_long_capture_mints_a_short_id_from_its_significant_words() {
        let text = "gravity might be a scarcity gradient in some shared resource";
        let ids = capture_ids(text);
        assert_eq!(
            ids,
            [
                "gravity-scarcity-gradient-shared-resource",
                "gravity-might-be-a-scarcity-gradient-in-some-shared-resource",
            ]
        );
        assert_eq!(ids.last().unwrap(), &slugify(text), "the full slug is last");
        for id in &ids {
            assert!(is_slug(id) && is_path_safe_id(id), "{id}");
        }
    }

    #[test]
    fn collision_fallbacks_add_one_significant_word_at_a_time() {
        let ids = capture_ids("the map is not the territory but the atlas is a map of maps");
        assert_eq!(
            ids,
            [
                "map-not-territory-atlas-map",
                "map-not-territory-atlas-map-maps",
                "the-map-is-not-the-territory-but-the-atlas-is-a-map-of-maps",
            ]
        );
        assert!(
            ids[0].split('-').count() <= CAPTURE_ID_WORDS,
            "the first choice is within the bound: {}",
            ids[0]
        );
    }

    #[test]
    fn a_short_capture_keeps_the_slug_it_always_had() {
        for text in [
            "search ranking decays with age",
            "the human's own words",
            "a thought",
            "시간은 프레임의 수다",
        ] {
            assert_eq!(capture_ids(text), [slugify(text)], "{text}");
        }
    }

    /// Words past the 60-character cut of the full slug still count: the
    /// short id is built from the whole sentence, then capped.
    #[test]
    fn a_capture_id_draws_on_words_past_the_full_slugs_cut() {
        let text = "it is what it is and it was what it was and so it goes on and on forever";
        // Every word but `goes` and `forever` is a stop word, so those two
        // and nothing else carry the id.
        assert_eq!(capture_ids(text)[0], "goes-forever");
        assert!(!slugify(text).contains("forever"), "{}", slugify(text));

        let stop_only = "it is what it is and so it was";
        assert_eq!(capture_ids(stop_only)[0], "it-is-what-it-is");
    }

    #[test]
    fn a_capture_that_reduces_to_nothing_has_no_id() {
        assert!(capture_ids("→ … !!").is_empty());
        assert!(capture_ids(&"a".repeat(61)).is_empty());
    }

    #[test]
    fn a_title_with_no_dash_in_the_first_60_chars_reduces_to_empty() {
        // One long run with no separator: there is no dash to cut at, so the
        // whole thing reduces to nothing rather than a truncated fragment
        // standing in for the title.
        let title = "a".repeat(61);
        assert_eq!(slugify(&title), "");
    }

    /// The rule an id has to satisfy before it is joined into a path. Every
    /// spelling here that escapes `nodes/` reached a file outside the corpus
    /// before this existed.
    #[test]
    fn an_id_that_is_not_one_file_name_is_refused() {
        for id in [
            "../../escaped",
            "../escaped",
            "..",
            ".",
            "./escaped",
            "nodes/other",
            "a\\b",
            "/etc/passwd",
            "/absolute",
            "",
            " ",
            " leading",
            "trailing ",
            "new\nline",
            "nul\0byte",
        ] {
            assert!(!is_path_safe_id(id), "accepted `{id}`");
        }
    }

    /// The rule is about path structure, not about which alphabet an idea was
    /// named in: every id a verb has ever derived stays valid.
    #[test]
    fn an_ordinary_id_including_a_unicode_one_is_accepted() {
        for id in [
            "safe",
            "self-authored-structure",
            "ünïcode-título-ok",
            "시간은-프레임의-수다",
            "..leading-dots",
            "a",
        ] {
            assert!(is_path_safe_id(id), "refused `{id}`");
            assert_eq!(
                Path::new(id).components().count(),
                1,
                "`{id}` is more than one component"
            );
        }
        // Everything `slugify` produces satisfies the weaker rule, which is
        // what keeps the two checks from ever disagreeing about a new node.
        for title in [
            "Tags beat domains",
            "Ünïcode título → ok",
            "시간은 프레임의 수다",
        ] {
            let slug = slugify(title);
            assert!(is_slug(&slug) && is_path_safe_id(&slug), "{slug}");
        }
    }

    #[test]
    fn is_slug_matches_what_slugify_would_produce() {
        assert!(is_slug("self-authored-structure"));
        assert!(is_slug(&"a".repeat(60)));
        assert!(!is_slug(""));
        assert!(!is_slug("Has-Capitals"));
        assert!(!is_slug("trailing-"));
        assert!(!is_slug("-leading"));
        assert!(!is_slug("double--dash"));
        assert!(!is_slug("has space"));
        assert!(!is_slug(&"a".repeat(61)));
    }
}
