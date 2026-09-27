//! A node as the corpus's git history recorded it. Read-only: nothing here
//! checks a historical tree out or writes to the repository.

use super::Corpus;
use super::commit::{git, git_failed, git_ok, inside_work_tree};
use crate::error::{Error, Result};
use crate::model::{self, Doc};
use serde::Serialize;

impl Corpus {
    /// Commits that changed one node, newest first.
    pub fn history(&self, id: &str) -> Result<Vec<HistoryEntry>> {
        // The id first, then the machine: an id that could not name a node
        // is refused whether or not the corpus happens to be under git.
        self.node_path(id)?;
        self.require_git()?;
        self.load(id)?;
        let path = format!("nodes/{id}.md");
        // The path is the node's identity. Following Git's similarity-based
        // rename/copy detection can cross into a different node's commits.
        let raw = git_ok(
            self.git_at(),
            &["log", "-z", "--format=%H%x00%cs%x00%s", "--", &path],
        )?;
        // `-z` adds a trailing NUL per record. Strip only that framing
        // delimiter; empty subjects are meaningful third fields.
        let fields: Vec<&str> = raw.split_terminator('\0').collect();
        let (records, remainder) = fields.as_chunks::<3>();
        if !remainder.is_empty() {
            return Err(Error::MalformedHistory {
                root: self.root.clone(),
                command: "log",
                pathspec: path,
            });
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

    /// Read one node as it existed at a commit hash, or on a date: after the
    /// last commit that day, dated the way [`Self::history`] dates it.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "all other parse errors pass through unchanged"
    )]
    pub fn load_at(&self, id: &str, at: &str) -> Result<Doc> {
        // The id becomes half of a git pathspec here rather than a path on
        // disk, and `git show <rev>:nodes/../../x.md` reads outside the
        // corpus just as readily as an open would. Checked before the work
        // tree, so the refusal does not depend on the machine.
        let node_path = self.node_path(id)?;
        self.require_git()?;
        let path = format!("nodes/{id}.md");
        let revision = if model::is_iso_date(at) {
            self.last_commit_on_or_before(&path, at)?
                .unwrap_or_default()
        } else if at.len() >= 4 && at.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            at.to_string()
        } else {
            return Err(Error::InvalidAt(at.to_string()));
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
        let prefix = git_ok(self.git_at(), &["rev-parse", "--show-prefix"])?;
        let object = format!("{revision}:{}nodes/{id}.md", prefix.trim());
        if !self.resolves(&object)? {
            return Err(absent());
        }
        let shown = git_ok(
            self.git_at(),
            &["show", "--no-ext-diff", "--format=", &object],
        )?;
        let doc = model::parse(&shown, &node_path).map_err(|e| match e {
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
        let out = git(self.git_at(), &["rev-parse", "--verify", "--quiet", object])?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_failed(&self.root, "rev-parse", &out.stderr.text())),
        }
    }

    /// The newest commit reachable from `HEAD` that touched `path` on or
    /// before `date`, taking each commit's date as [`Self::history`] prints
    /// it: the calendar day in the offset the commit was recorded with.
    ///
    /// Not `rev-list --before=<date> 23:59:59`: git reads a date without an
    /// offset in the reader's timezone, so the same corpus answered the same
    /// date differently by where it was read. At UTC+14 the end of a day falls
    /// before noon UTC on it, and a commit `log` dated that day was missed.
    /// The commit's own day is fixed when it is written, so every reader gets
    /// the revision that `log` shows for the date.
    fn last_commit_on_or_before(&self, path: &str, date: &str) -> Result<Option<String>> {
        let listed = git_ok(
            self.git_at(),
            &[
                "rev-list",
                "--no-commit-header",
                "--format=%H %cs",
                "HEAD",
                "--",
                path,
            ],
        )?;
        for line in listed.lines() {
            let Some((hash, day)) = line.split_once(' ') else {
                return Err(Error::MalformedHistory {
                    root: self.root.clone(),
                    command: "rev-list",
                    pathspec: path.to_string(),
                });
            };
            // Both sides are `YYYY-MM-DD`, so text order is date order.
            if day <= date {
                return Ok(Some(hash.to_string()));
            }
        }
        Ok(None)
    }

    fn require_git(&self) -> Result<()> {
        if inside_work_tree(self.git_at())? {
            Ok(())
        } else {
            Err(Error::NotGitWorkTree(self.root.clone()))
        }
    }
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
