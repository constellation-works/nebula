//! Corpus access.
//!
//! The corpus lives outside this repository, holds the nodes and the inbox, and
//! is the only thing here that is irreplaceable. Every write goes through this
//! module so the atomicity and never-delete rules hold in one place.

use crate::model::{self, Doc};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use time::{
    Date, OffsetDateTime, format_description::well_known::Iso8601, macros::format_description,
};

/// A corpus on disk.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open the corpus named by `--root`, else `NEBULA_ROOT`, else `~/.nebula`.
    pub fn open(explicit: Option<PathBuf>) -> Result<Self> {
        let root = if let Some(p) = explicit {
            p
        } else if let Ok(p) = std::env::var("NEBULA_ROOT") {
            PathBuf::from(p)
        } else {
            let home = std::env::var("HOME").context("HOME is not set")?;
            PathBuf::from(home).join(".nebula")
        };
        if !root.join("nodes").is_dir() {
            bail!(
                "no corpus at {}\n\nCreate one with:  neb init {}",
                root.display(),
                root.display()
            );
        }
        Ok(Self { root })
    }

    /// Create an empty corpus.
    pub fn init(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("nodes"))?;
        std::fs::create_dir_all(root.join("inbox"))?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Where the corpus lives.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of a node file, whether or not it exists.
    pub fn node_path(&self, id: &str) -> PathBuf {
        self.root.join("nodes").join(format!("{id}.md"))
    }

    /// Read one node.
    pub fn load(&self, id: &str) -> Result<Doc> {
        let path = self.node_path(id);
        if !path.exists() {
            bail!("no node `{id}`\n\nList what exists with:  neb list");
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
            bail!("node `{}` already exists", doc.node.id);
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
    pub fn capture(&self, text: &str) -> Result<String> {
        use std::io::Write;
        let dir = self.root.join("inbox");
        std::fs::create_dir_all(&dir)?;
        let now = stamp();
        let month = &now[..7];
        let path = dir.join(format!("{month}.md"));
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let id = unique_entry_id(&format!("{now}{text}"), &existing);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(f, "- [{id}] {now} {}", text.trim())?;
        Ok(id)
    }

    /// Inbox entries that have not been promoted or dropped.
    pub fn inbox(&self) -> Result<Vec<InboxEntry>> {
        let dir = self.root.join("inbox");
        let mut out = Vec::new();
        if !dir.is_dir() {
            return Ok(out);
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
        Ok(out)
    }

    /// Find one live inbox entry by id.
    pub fn inbox_entry(&self, id: &str) -> Result<InboxEntry> {
        self.inbox()?
            .into_iter()
            .find(|e| e.id == id)
            .with_context(|| format!("no open inbox entry `{id}`\n\nSee them with:  neb inbox"))
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
            bail!("inbox entry `{}` is not in this corpus", entry.id);
        }
        let content = std::fs::read_to_string(&entry.file)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        let Some(slot) = lines.get_mut(entry.line) else {
            bail!("inbox entry moved underneath us; nothing written");
        };
        *slot = format!("- ~~[{}] {} {}~~ {outcome}", entry.id, entry.at, entry.text);
        std::fs::write(&entry.file, lines.join("\n") + "\n")?;
        Ok(())
    }
}

/// One unprocessed capture.
#[derive(Debug, Clone)]
pub struct InboxEntry {
    /// Short id, unique within its file.
    pub id: String,
    /// Capture timestamp.
    pub at: String,
    /// What you wrote.
    pub text: String,
    /// Which inbox file it lives in.
    pub file: PathBuf,
    /// Zero-based line within that file.
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
pub fn today() -> String {
    now()
        .format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

/// Now, local, to the minute.
pub fn stamp() -> String {
    now()
        .format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
        .unwrap_or_default()
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

/// Whole days between a `YYYY-MM-DD` string and today, if it parses.
pub fn days_since(date: &str) -> Option<i64> {
    let d = Date::parse(date, &Iso8601::DATE).ok()?;
    Some((now().date() - d).whole_days())
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

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Turn a title into a node id.
pub fn slugify(s: &str) -> String {
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

/// Index every node by id, for traversal.
pub fn by_id(docs: &[Doc]) -> HashMap<&str, &Doc> {
    docs.iter().map(|d| (d.node.id.as_str(), d)).collect()
}
