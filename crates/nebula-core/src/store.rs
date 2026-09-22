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
//! else. That is the whole of what this module knows about git: it never
//! pushes, never stages a path outside the root, and never undoes a write
//! because the commit failed.

use crate::config::{self, CommitSetting, Config, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::lock::CorpusLock;
use crate::model::{self, Doc};
use serde::Serialize;
use std::path::{Path, PathBuf};
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
        let parent = path
            .parent()
            .ok_or_else(|| Error::corpus("root configuration path has no parent"))?;
        std::fs::create_dir_all(parent)?;
        std::fs::write(&path, format!("{}\n", root.display()))?;
        Ok(path)
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

    /// Create an empty corpus.
    pub fn init(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("nodes"))?;
        std::fs::create_dir_all(root.join("inbox"))?;
        let config = Config::fresh(corpus_id(root));
        config.save(root)?;
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

    /// Where `observatory` references resolve: `observatory_root` in
    /// `config.yaml`, else `$OBSERVATORY_ROOT`, else nowhere.
    ///
    /// The path is used as given and never canonicalized, like every other
    /// path here.
    pub fn observatory_root(&self) -> ObservatoryRoot {
        self.config.observatory_root()
    }

    /// Record where the Observatory checkout is, in `config.yaml`.
    ///
    /// The file stays machine-written: this rewrites it whole, header and
    /// all, rather than editing a line. The directory is not required to
    /// exist yet; `check` says so when a reference fails to resolve under it.
    pub(crate) fn set_observatory_root(&mut self, dir: PathBuf) -> Result<()> {
        self.config.observatory_root = Some(dir);
        self.config.save(&self.root)
    }

    /// Whether a write is followed by a commit, per `config.yaml`.
    pub fn commit_setting(&self) -> CommitSetting {
        CommitSetting {
            enabled: self.config.commit,
        }
    }

    /// Record in `config.yaml` whether writes are committed. Rewrites the
    /// file whole, like every other setting.
    pub(crate) fn set_commit(&mut self, enabled: bool) -> Result<()> {
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
    pub(crate) fn commit(&self, verb: &str, ids: &[&str]) -> Result<Option<Committed>> {
        if !self.config.commit {
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
        let staged = git_ok(root, &["diff", "--cached", "--name-only", "--no-renames"])?;
        let outside: Vec<String> = staged
            .lines()
            .filter(|path| !is_corpus_path(prefix, path))
            .map(String::from)
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
    /// re-deriving the layout.
    pub fn node_path(&self, id: &str) -> PathBuf {
        self.root.join("nodes").join(format!("{id}.md"))
    }

    /// Read one node.
    pub fn load(&self, id: &str) -> Result<Doc> {
        let path = self.node_path(id);
        if !path.exists() {
            return Err(Error::NoSuchNode(id.to_string()));
        }
        model::read(&path)
    }

    /// Write one node, stamping `updated`.
    pub fn save(&self, doc: &mut Doc) -> Result<()> {
        doc.node.updated = today();
        model::write(&self.node_path(&doc.node.id.clone()), doc)
    }

    /// Write a node that must not already exist.
    pub fn create(&self, doc: &Doc) -> Result<()> {
        let path = self.node_path(&doc.node.id);
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
            out.push(model::read(&p)?);
        }
        Ok(out)
    }

    /// Append a capture to the current month's inbox file.
    pub fn capture(&self, text: &str) -> Result<InboxEntry> {
        use std::io::Write;
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
            .filter_map(|e| e.ok().map(|e| e.path()))
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

/// What a `neb` commit may contain, relative to the corpus root. Everything
/// else under the root, and everything outside it, is left alone — the write
/// lock's `.lock` included, which is why it is not listed here and never
/// will be: it is a fact about which process is writing right now, not about
/// the corpus, and it means nothing on another machine.
const COMMIT_PATHS: [&str; 3] = ["nodes", "inbox", config::FILE];

/// Whether a path from `git diff --name-only`, relative to the repository's
/// top level, is one a `neb` commit may contain. `prefix` is the root's own
/// position under that top level (`git rev-parse --show-prefix`), empty when
/// the corpus root is the repository.
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
