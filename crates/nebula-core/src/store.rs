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

use crate::config::{self, CommitSetting, Config, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::lock::{CorpusLock, LOCK_FILE};
use crate::model::{self, Doc};
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use time::{
    Date, OffsetDateTime, format_description::well_known::Iso8601, macros::format_description,
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
        Ok(Self::home()?.join(".config").join("nebula").join("root"))
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
        Ok(Self::home()?
            .join(".config")
            .join("nebula")
            .join("observatory-root"))
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
        std::fs::create_dir_all(parent).map_err(|error| Error::io_at("creating", parent, error))?;
        write_atomic(&path, format!("{}\n", dir.display()))?;
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
    /// Paths are compared as given, like every other corpus path.
    pub fn write_root_config(root: &Path, force: bool) -> Result<PathBuf> {
        let path = Self::root_config_path()?;
        Self::check_root_config(root, force)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::corpus("root configuration path has no parent"))?;
        std::fs::create_dir_all(parent)?;
        std::fs::write(&path, format!("{}\n", root.display()))?;
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
        // A corpus at an older schema refuses to open until `neb migrate`
        // has brought it forward.
        let config = Config::load(&root, || corpus_id(&root))?;
        Ok(Self { root, config })
    }

    /// Create an empty corpus, or open an existing one without resetting it.
    pub fn init(root: &Path) -> Result<Self> {
        refuse_nodes_symlink(root)?;
        // Re-running init on a corpus is an open, not a reset. In particular,
        // opening first validates an existing config before any directory or
        // file can be created. The one setup repair it may make afterwards is
        // adding the runtime lock to `.gitignore`.
        if root.join("nodes").is_dir() {
            let corpus = Self::open(Some(root.to_path_buf()))?;
            ensure_lock_ignored(root)?;
            return Ok(corpus);
        }

        // A config can survive an interrupted or partial initialization. Read
        // it before creating anything so malformed and incompatible files are
        // refused without mutation, while a valid identity and settings are
        // preserved as the missing directories are completed.
        let existing_config = root.join(config::FILE).exists();
        let config = existing_config
            .then(|| Config::load(root, || corpus_id(root)))
            .transpose()?;
        // Each directory is named when it cannot be made: `capture` creates
        // corpora unasked, so a bare "Permission denied" would leave the
        // reader guessing which root it was aimed at.
        for dir in [root.to_path_buf(), root.join("nodes"), root.join("inbox")] {
            std::fs::create_dir_all(&dir).map_err(|error| Error::io_at("creating", &dir, error))?;
        }
        let config = if let Some(config) = config {
            config
        } else {
            let config = Config::fresh(corpus_id(root));
            config.save(root)?;
            config
        };
        ensure_lock_ignored(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            config,
        })
    }

    /// Open the corpus, creating it when there is none, and say whether this
    /// call created it.
    ///
    /// Capture is the reason this exists: being told to run a setup command is
    /// precisely the friction that loses the thought. A corpus that exists but
    /// is at the wrong schema still refuses, because rewriting it blind would
    /// be worse than the friction.
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

    /// Where the corpus lives.
    pub(crate) fn root(&self) -> &Path {
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
    /// The corpus id in hand is the fallback rather than a freshly synthesized
    /// one, so a config deleted underneath a live corpus comes back with the
    /// identity it had instead of a new one.
    fn current_config(&self) -> Result<Config> {
        let id = self.config.corpus_id.clone();
        Config::load(&self.root, || id)
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
    /// about the corpus — and commits as `neb <verb> <ids>`. `None` when the
    /// setting is off, the root is not under git, or the write left nothing to
    /// record. Refused, as [`Error::StagedElsewhere`], when the index already
    /// holds something outside the corpus: a `neb` commit is exactly the
    /// corpus, and folding a stranger's staged work into one would misfile it.
    /// The write is on disk before this runs and stays there whatever git says.
    /// Never pushes.
    ///
    /// The setting is read from disk under the caller's lock rather than from
    /// the snapshot: whether this write is recorded is a question about the
    /// configuration in force, and a writer that waited its turn opened before
    /// the writer ahead of it had finished saying what that configuration is.
    pub(crate) fn commit(&self, verb: &str, ids: &[&str]) -> Result<Option<Committed>> {
        if !self.current_config()?.commit {
            return Ok(None);
        }
        let root = &self.root;
        if !inside_work_tree(root).map_err(|e| git_unavailable(root, &e))? {
            return Ok(None);
        }
        // The corpus is gitignored by the repository around it, which is
        // exactly the setup a private repository at the corpus root fixes.
        let ignored = git(root, &["check-ignore", "-q", "--", "nodes"])?;
        match ignored.status.code() {
            Some(0) => return Err(Error::CorpusIgnored(root.clone())),
            Some(1) => {}
            _ => {
                return Err(git_failed(
                    root,
                    "check-ignore",
                    &String::from_utf8_lossy(&ignored.stderr),
                ));
            }
        }
        let prefix = git_ok(root, &["rev-parse", "--show-prefix"])?;
        let prefix = prefix.trim();
        // The whole index, not just the part under the root: a path staged
        // elsewhere in the repository is exactly what the refusal is for.
        // Paths come back relative to the top level, hence the prefix.
        // `-z` both separates names unambiguously and disables Git's
        // `core.quotePath` quoting. Newlines therefore stay inside one record,
        // and non-ASCII names are checked as the paths they actually name.
        let staged = git(
            root,
            &["diff", "--cached", "--name-only", "--no-renames", "-z"],
        )?;
        if !staged.status.success() {
            return Err(git_failed(
                root,
                "diff",
                &String::from_utf8_lossy(&staged.stderr),
            ));
        }
        let outside: Vec<String> = staged
            .stdout
            .split(|byte| *byte == b'\0')
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .filter(|path| !is_corpus_path(prefix, path))
            .collect();
        if !outside.is_empty() {
            return Err(Error::StagedElsewhere {
                root: root.clone(),
                paths: outside,
            });
        }

        // Only paths that exist can be named: `inbox/` appears on the first
        // capture and a pathspec that matches nothing is a git error.
        let present: Vec<&str> = COMMIT_PATHS
            .iter()
            .copied()
            .filter(|p| root.join(p).exists())
            .collect();
        if present.is_empty() {
            return Ok(None);
        }
        let mut add = vec!["add", "-A", "--"];
        add.extend(present);
        git_ok(root, &add)?;
        let staged = git(root, &["diff", "--cached", "--quiet"])?;
        match staged.status.code() {
            Some(0) => return Ok(None), // the write changed nothing git can see
            Some(1) => {}
            _ => {
                return Err(git_failed(
                    root,
                    "diff",
                    &String::from_utf8_lossy(&staged.stderr),
                ));
            }
        }
        let message = match ids {
            [] => format!("neb {verb}"),
            ids => format!("neb {verb} {}", ids.join(" ")),
        };
        git_ok(root, &["commit", "-q", "-m", &message])?;
        let hash = git_ok(root, &["rev-parse", "HEAD"])?.trim().to_string();
        Ok(Some(Committed { hash, message }))
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
        let doc = model::read(&path)?;
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
        if revision.is_empty() {
            return Err(Error::NoNodeAtRevision {
                node: id.to_string(),
                revision: at.to_string(),
            });
        }

        let prefix = git_ok(&self.root, &["rev-parse", "--show-prefix"])?;
        let object = format!("{revision}:{}nodes/{id}.md", prefix.trim());
        let shown = git(&self.root, &["show", "--no-ext-diff", "--format=", &object])?;
        if !shown.status.success() {
            return Err(Error::NoNodeAtRevision {
                node: id.to_string(),
                revision: at.to_string(),
            });
        }
        let doc = model::parse(&String::from_utf8_lossy(&shown.stdout)).map_err(|e| match e {
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

    fn require_git(&self) -> Result<()> {
        if inside_work_tree(&self.root).map_err(|error| git_unavailable(&self.root, &error))? {
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
            let doc = model::read(&p)?;
            self.require_file_agrees(&p, &doc)?;
            out.push(doc);
        }
        Ok(out)
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
    /// scan reads the node twice, and [`write_atomic`] replaces the entry the
    /// id names with a new file, so a hard link keeps the old bytes under the
    /// other name and the next load refuses. So an alias is refused wherever
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
    pub fn capture(&self, text: &str) -> Result<InboxEntry> {
        use std::io::Write;
        let text = capture_line(text);
        if text.is_empty() {
            return Err(Error::corpus("nothing to capture"));
        }
        let dir = self.root.join("inbox");
        refuse_inbox_symlink(&dir)?;
        std::fs::create_dir_all(&dir).map_err(|error| Error::io_at("writing", &dir, error))?;
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
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| Error::io_at("writing", &path, error))?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(f).map_err(|error| Error::io_at("writing", &path, error))?;
        }
        writeln!(f, "- [{id}] {now} {text}")
            .map_err(|error| Error::io_at("writing", &path, error))?;
        Ok(InboxEntry {
            id,
            at: now,
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
        *slot = format!("- ~~[{}] {} {}~~ {outcome}", entry.id, entry.at, entry.text);
        refuse_inbox_symlink(&self.root.join("inbox"))?;
        refuse_inbox_symlink(&entry.file)?;
        write_atomic(&entry.file, lines.join("\n") + "\n")?;
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

/// Replace a file through a sibling temporary file.
///
/// The temporary file is created, never opened by name a second time, so the
/// bytes go to the file this call made and to nothing else. Writing by name
/// would follow whatever already answers to it: a symlink planted at the
/// temporary path is a write straight through the corpus wall, and the rename
/// that follows would then install the symlink as the node.
///
/// The temporary file is removed when either writing or renaming fails, so a
/// failed write does not leave debris that could be mistaken for corpus data.
pub(crate) fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let (tmp, file) = create_temporary_sibling(path)?;
    let result =
        write_and_close(file, contents.as_ref()).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(error) = result {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => {
                return Err(Error::corpus(format!(
                    "atomic write to {} failed: {error}; removing {} failed: {cleanup}",
                    path.display(),
                    tmp.display()
                )));
            }
        }
        return Err(Error::io_at("writing", path, error));
    }
    Ok(())
}

/// Write the whole of `contents` and close the file, so the rename that
/// follows moves a file nobody still holds open.
fn write_and_close(mut file: std::fs::File, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    file.write_all(contents)
}

/// How many names a single write tries before giving up. Two names collide
/// only when another writer picks the same counter in the same nanosecond
/// under the same process id, or when something is planting files under each
/// name as fast as they are tried. A few attempts cover the first and a bound
/// keeps the second from spinning.
const TEMPORARY_NAME_ATTEMPTS: u32 = 8;

/// Create a temporary file beside `path` and hand back its name and its open
/// handle.
///
/// `create_new` is the guard: it opens with `O_CREAT | O_EXCL`, which refuses
/// a name that already exists instead of following it, so a symlink sitting at
/// the temporary path is a refusal rather than a write to its target. The
/// handle comes back with the name because reopening by name afterwards would
/// hand the same opening back to whoever won the race.
///
/// The name carries a nonce, and not only for the race. A fixed name that
/// `O_EXCL` refuses would wedge every later write to that file behind one
/// stale temporary left by a killed process, and nothing here deletes what it
/// did not create. `.tmp` stays the extension so a temporary that does outlive
/// a crash stays invisible to `load_all` and to the inbox, which both match on
/// the name.
///
/// The name is built by appending to `path` as given, so a corpus reached
/// through a symlinked root writes beside the file the caller named. Nothing
/// is resolved or canonicalized.
fn create_temporary_sibling(path: &Path) -> Result<(PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    for _ in 0..TEMPORARY_NAME_ATTEMPTS {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{:x}-{count:x}-{nanos:x}.tmp", std::process::id()));
        let tmp = PathBuf::from(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Error::io_at("writing", path, error)),
        }
    }
    Err(Error::corpus(format!(
        "no free temporary name beside {} after {TEMPORARY_NAME_ATTEMPTS} tries; \
         something is creating files under them",
        path.display()
    )))
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

/// Whether a path from a NUL-delimited `git diff --name-only -z`, relative to
/// the repository's top level, is one a `neb` commit may contain. `prefix` is
/// the root's own position under that top level (`git rev-parse
/// --show-prefix`), empty when the corpus root is the repository.
fn is_corpus_path(prefix: &str, path: &str) -> bool {
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    rest == config::FILE
        || rest == GITIGNORE_FILE
        || rest.starts_with("nodes/")
        || rest.starts_with("inbox/")
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
            write_atomic(&path, contents)
        } else {
            Ok(())
        };
    }

    if !contents.is_empty() && !contents.ends_with(b"\n") {
        contents.push(b'\n');
    }
    contents.extend_from_slice(expected.as_bytes());
    contents.push(b'\n');
    write_atomic(&path, contents)
}

/// Run git at the corpus root. The process not starting at all is the one
/// failure this reports; whether the command succeeded is the caller's to
/// judge, since a non-zero exit is an answer for some of them.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| git_unavailable(root, &e))
}

/// Run git at the corpus root and require it to succeed; stdout as text.
fn git_ok(root: &Path, args: &[&str]) -> Result<String> {
    let out = git(root, args)?;
    if !out.status.success() {
        return Err(git_failed(
            root,
            args.first().copied().unwrap_or("git"),
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_failed(root: &Path, context: &str, stderr: &str) -> Error {
    Error::Git {
        root: root.to_path_buf(),
        context: context.to_string(),
        stderr: stderr.trim().to_string(),
    }
}

fn git_unavailable(root: &Path, e: &std::io::Error) -> Error {
    git_failed(root, "start", &e.to_string())
}

/// Whether the root is inside a git work tree. `Err` only when git itself
/// could not be run, which a caller that merely wants to know may treat as
/// "no".
pub(crate) fn inside_work_tree(root: &Path) -> std::io::Result<bool> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()?;
    Ok(out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
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
    /// Capture timestamp.
    pub at: String,
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
        let (at, text) = rest.split_once(' ')?;
        Some(Self {
            id: id.to_string(),
            at: at.to_string(),
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

/// Now, local, to the minute.
pub(crate) fn stamp() -> String {
    now()
        .format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
        .unwrap_or_default()
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

/// Whole days between a `YYYY-MM-DD` string and today, if it parses.
pub(crate) fn days_since(date: &str) -> Option<i64> {
    let d = Date::parse(date, &Iso8601::DATE).ok()?;
    Some((now().date() - d).whole_days())
}

/// Whole days between the date in a `YYYY-MM-DDTHH:MM` stamp and today.
pub(crate) fn days_since_stamp(stamp: &str) -> Option<i64> {
    days_since(stamp.get(..10)?)
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
mod tests {
    use super::*;

    #[test]
    fn atomic_write_removes_its_temporary_file_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("destination");
        std::fs::create_dir(&destination).unwrap();

        let error = write_atomic(&destination, "replacement").unwrap_err();
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

        write_atomic(&destination, "replacement").unwrap();

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
