//! Corpus access.
//!
//! The corpus lives outside this repository, holds the nodes and the inbox, and
//! is the only thing here that is irreplaceable. Every write goes through this
//! module so the atomicity and never-delete rules hold in one place.
//!
//! Nothing here prints. A caller that wants to tell someone what happened gets
//! back the data and says it in its own voice.

use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{self, Doc};
use serde::Serialize;
use std::path::{Path, PathBuf};
use time::{
    Date, OffsetDateTime, format_description::well_known::Iso8601, macros::format_description,
};

/// A corpus on disk.
#[derive(Debug, Clone)]
pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    /// Where a corpus would be, given `--root`, else `NEBULA_ROOT`, else `~/.nebula`.
    pub fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
        if let Some(p) = explicit {
            return Ok(p);
        }
        if let Ok(p) = std::env::var("NEBULA_ROOT") {
            return Ok(PathBuf::from(p));
        }
        let home = std::env::var("HOME").map_err(|_| Error::corpus("HOME is not set"))?;
        Ok(PathBuf::from(home).join(".nebula"))
    }

    /// Open the corpus named by `--root`, else `NEBULA_ROOT`, else `~/.nebula`.
    pub fn open(explicit: Option<PathBuf>) -> Result<Self> {
        let root = Self::resolve_root(explicit)?;
        if !root.join("nodes").is_dir() {
            return Err(Error::NoCorpus(root));
        }
        // Loaded for its schema check alone: a corpus at an older schema
        // refuses to open until `neb migrate` has brought it forward.
        Config::load(&root, || corpus_id(&root))?;
        Ok(Self { root })
    }

    /// Create an empty corpus.
    pub fn init(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("nodes"))?;
        std::fs::create_dir_all(root.join("inbox"))?;
        Config::fresh(corpus_id(root)).save(root)?;
        Ok(Self {
            root: root.to_path_buf(),
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

    /// Path of a node file, whether or not it exists.
    pub(crate) fn node_path(&self, id: &str) -> PathBuf {
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
        let id = unique_entry_id(&format!("{now}{text}"), &existing);
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
        *slot = format!("- ~~[{}] {} {}~~ {outcome}", entry.id, entry.at, entry.text);
        std::fs::write(&entry.file, lines.join("\n") + "\n")?;
        Ok(())
    }
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
    /// Short id, unique within its file.
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

/// A short id for an inbox entry, retried until it is unique in the file.
fn unique_entry_id(seed: &str, existing: &str) -> String {
    let mut h = fnv(seed);
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if !existing.contains(&format!("[{id}]")) {
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
pub(crate) fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').chars().take(60).collect()
}
