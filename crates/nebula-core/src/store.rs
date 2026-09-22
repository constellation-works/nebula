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
use crate::lock::CorpusLock;
use crate::model::{self, Doc};
use serde::Serialize;
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
    /// configured root, else `~/.nebula`.
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
        if let Some(p) = Self::configured_root()? {
            return Ok(p);
        }
        Self::default_root()
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

    /// Open the corpus named by `--root`, else `NEBULA_ROOT`, else the
    /// configured root, else `~/.nebula`.
    pub fn open(explicit: Option<PathBuf>) -> Result<Self> {
        let root = Self::resolve_root(explicit)?;
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
        // Re-running init on a corpus is an open, not a reset. In particular,
        // opening first validates an existing config before any directory or
        // file can be created, and leaves its spelling and bytes untouched.
        if root.join("nodes").is_dir() {
            return Self::open(Some(root.to_path_buf()));
        }

        // A config can survive an interrupted or partial initialization. Read
        // it before creating anything so malformed and incompatible files are
        // refused without mutation, while a valid identity and settings are
        // preserved as the missing directories are completed.
        let existing_config = root.join(config::FILE).exists();
        let config = existing_config
            .then(|| Config::load(root, || corpus_id(root)))
            .transpose()?;
        std::fs::create_dir_all(root.join("nodes"))?;
        std::fs::create_dir_all(root.join("inbox"))?;
        let config = if let Some(config) = config {
            config
        } else {
            let config = Config::fresh(corpus_id(root));
            config.save(root)?;
            config
        };
        Ok(Self {
            root: root.to_path_buf(),
            config,
        })
    }

    /// Open the corpus, creating it when there is none.
    ///
    /// Capture is the reason this exists: being told to run a setup command is
    /// precisely the friction that loses the thought. A corpus that exists but
    /// is at the wrong schema still refuses, because rewriting it blind would
    /// be worse than the friction.
    pub fn open_or_init(explicit: Option<PathBuf>) -> Result<Self> {
        let root = Self::resolve_root(explicit)?;
        match Self::open(Some(root.clone())) {
            Err(Error::NoCorpus(_)) => Self::init(&root),
            other => other,
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

    /// Where `observatory` references resolve: `observatory_root` in
    /// `config.yaml`, else `$OBSERVATORY_ROOT`, else nowhere.
    ///
    /// The path is used as given and never canonicalized, like every other
    /// path here.
    ///
    /// The snapshot [`Self::open`] took, which is what a read wants: it
    /// answers without waiting on a writer.
    pub fn observatory_root(&self) -> ObservatoryRoot {
        self.config.observatory_root()
    }

    /// Record where the Observatory checkout is, in `config.yaml`.
    ///
    /// The file stays machine-written: this rewrites it whole, header and
    /// all, rather than editing a line. The directory is not required to
    /// exist yet; `check` says so when a reference fails to resolve under it.
    ///
    /// Rewriting it whole is why the reload comes first: this sets one key
    /// and must carry every other key across as it stands under the caller's
    /// lock, not as it stood when the corpus was opened.
    pub(crate) fn set_observatory_root(&mut self, dir: PathBuf) -> Result<()> {
        self.reload_config()?;
        self.config.observatory_root = Some(dir);
        self.config.save(&self.root)
    }

    /// Whether a write is followed by a commit, per `config.yaml`.
    ///
    /// The snapshot [`Self::open`] took, like [`Self::observatory_root`]: a
    /// read of the setting never waits on a writer.
    pub fn commit_setting(&self) -> CommitSetting {
        CommitSetting {
            enabled: self.config.commit,
        }
    }

    /// Record in `config.yaml` whether writes are committed. Rewrites the
    /// file whole, like every other setting, and so reloads first for the
    /// same reason [`Self::set_observatory_root`] does.
    pub(crate) fn set_commit(&mut self, enabled: bool) -> Result<()> {
        self.reload_config()?;
        self.config.commit = enabled;
        self.config.save(&self.root)
    }

    /// Commit the corpus after a write, when `config.yaml` asks for it and
    /// the root is inside a git work tree.
    ///
    /// Stages `nodes/`, `inbox/` and `config.yaml` under the root and
    /// nothing else — not `.lock`, which records nothing about the corpus —
    /// and commits as `neb <verb> <ids>`. `None` when the setting is off,
    /// the root is not under git, or the write left nothing to record. Refused, as [`Error::StagedElsewhere`], when the index
    /// already holds something outside the corpus: a `neb` commit is exactly
    /// the corpus, and folding a stranger's staged work into one would misfile
    /// it. The write is on disk before this runs and stays there whatever
    /// git says. Never pushes.
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
        if !is_path_safe_id(id) {
            return Err(Error::UnsafeId(id.to_string()));
        }
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
        let revision = if Date::parse(at, &Iso8601::DATE).is_ok() {
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
            String::new()
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
        if !dir.is_dir() {
            return Ok(out);
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
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
    /// The names are compared by asking the filesystem rather than by
    /// comparing bytes. A volume may store a name in a different Unicode
    /// normalization than the id it was written from — `título` is two
    /// spellings of the same word — and a byte comparison would call a
    /// perfectly ordinary node a mismatch. Nothing is canonicalized: the
    /// question asked is whether the path the id names holds this same file's
    /// text, which a symlinked root answers the same way on either platform.
    fn require_file_agrees(&self, path: &Path, doc: &Doc) -> Result<()> {
        if path.file_stem() == Some(OsStr::new(doc.node.id.as_str())) {
            return Ok(());
        }
        let declared = self.node_path(&doc.node.id)?;
        let same = std::fs::read_to_string(&declared)
            .ok()
            .zip(std::fs::read_to_string(path).ok())
            .is_some_and(|(declared, found)| declared == found);
        if same {
            return Ok(());
        }
        Err(Error::IdMismatch {
            path: path.to_path_buf(),
            id: doc.node.id.clone(),
        })
    }

    /// Append a capture to the current month's inbox file.
    pub fn capture(&self, text: &str) -> Result<InboxEntry> {
        use std::io::Write;
        if text.contains(['\n', '\r']) {
            return Err(Error::corpus("capture text must fit on one line"));
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::corpus("nothing to capture"));
        }
        let dir = self.root.join("inbox");
        std::fs::create_dir_all(&dir)?;
        let now = stamp();
        let month = &now[..7];
        let path = dir.join(format!("{month}.md"));
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let inbox = self.inbox()?;
        let id = unique_entry_id(&format!("{now}{text}"), &inbox);
        let line = existing.lines().count();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(f)?;
        }
        writeln!(f, "- [{id}] {now} {text}")?;
        Ok(InboxEntry {
            id,
            at: now,
            text: text.to_string(),
            file: path,
            line,
        })
    }

    /// Inbox entries that have not been promoted or dropped.
    pub fn inbox(&self) -> Result<Inbox> {
        let dir = self.root.join("inbox");
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(Inbox(out));
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
        for file in files {
            for (lineno, line) in std::fs::read_to_string(&file)?.lines().enumerate() {
                if let Some(e) = InboxEntry::parse(line, &file, lineno) {
                    out.push(e);
                }
            }
        }
        Ok(Inbox(out))
    }

    /// Find one live inbox entry by id.
    pub fn inbox_entry(&self, id: &str) -> Result<InboxEntry> {
        self.inbox()?
            .0
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::NoSuchInboxEntry(id.to_string()))
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
        write_atomic(&entry.file, lines.join("\n") + "\n")?;
        Ok(())
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
/// The temporary file is removed when either writing or renaming fails, so a
/// failed write does not leave debris that could be mistaken for corpus data.
pub(crate) fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path));
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
        return Err(error.into());
    }
    Ok(())
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
/// the corpus, and it means nothing on another machine.
const COMMIT_PATHS: [&str; 3] = ["nodes", "inbox", config::FILE];

/// Whether a path from a NUL-delimited `git diff --name-only -z`, relative to
/// the repository's top level, is one a `neb` commit may contain. `prefix` is
/// the root's own position under that top level (`git rev-parse
/// --show-prefix`), empty when the corpus root is the repository.
fn is_corpus_path(prefix: &str, path: &str) -> bool {
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    rest == config::FILE || rest.starts_with("nodes/") || rest.starts_with("inbox/")
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

/// A short id for an inbox entry, retried until it is unique in the live inbox.
fn unique_entry_id(seed: &str, inbox: &Inbox) -> String {
    let mut h = fnv(seed);
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if inbox.0.iter().all(|entry| entry.id != id) {
            return id;
        }
        h = fnv(&format!("{h}"));
    }
    format!("{:04x}", (fnv(seed) & 0xffff) as u16)
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
    let out = out.trim_end_matches('-');
    if out.chars().count() <= 60 {
        return out.to_string();
    }
    let cut: String = out.chars().take(60).collect();
    match cut.rfind('-') {
        Some(i) => cut[..i].to_string(),
        None => String::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_removes_its_temporary_file_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("destination");
        std::fs::create_dir(&destination).unwrap();

        assert!(write_atomic(&destination, "replacement").is_err());
        assert!(
            destination.is_dir(),
            "the failed rename left the target alone"
        );
        assert!(
            !dir.path().join("destination.tmp").exists(),
            "the failed rename cleaned up its temporary file"
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
