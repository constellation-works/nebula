//! The corpus as the desktop reads it.
//!
//! One thin function per command, each a single `nebula-core` call, so the
//! `#[tauri::command]` wrappers in [`crate::commands`] carry no logic and this
//! file can be exercised by a test without a window. Nothing here shells out
//! to `neb`: the desktop links the library and sees exactly what the CLI sees.

use crate::error::IpcError;
use nebula_core::{
    CommitOutcome, Committed, Corpus, Created, Graph, GraphExport, InboxEntry, NodeView, Result,
    graph, ops,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A desktop write should tell the UI promptly when another writer is busy.
pub const WRITE_LOCK_WAIT: Duration = Duration::from_millis(150);

/// Where the corpus is expected: `$NEBULA_ROOT`, else the corpus the working
/// directory is in, else `~/.config/nebula/root`, else `~/.nebula`. The CLI's
/// rule without `--root`, so the app and the terminal never disagree.
pub fn resolve_root() -> Result<PathBuf> {
    Corpus::resolve_root(None)
}

/// Open the corpus at `root`. Fails when there is none, or when it is at a
/// schema this build does not read; the desktop never creates one, because a
/// wrong `NEBULA_ROOT` should be seen, not papered over.
pub fn open(root: &Path) -> Result<Corpus> {
    Corpus::open(Some(root.to_path_buf()))
}

/// What a write command returns once the write is on disk: the written value,
/// and what the commit after it did. Declared again as `Written<T>` in
/// `apps/desktop/src/api.ts`.
///
/// The commit is reported, never raised (STD-02 §R30): a refused commit
/// leaves the write in place, so an error here would tell the webview that a
/// write failed when it did not, and invite a retry that writes it twice.
#[derive(Debug, Clone, Serialize)]
pub struct Written<T> {
    /// What the write produced: the inbox entry, the settled entry, the node.
    pub value: T,
    /// Whether the write was committed, and why not when it was not.
    pub commit: CommitReport,
}

/// What the commit after a desktop write did, as the webview reads it:
/// core's [`CommitOutcome`], or the refusal, tagged by `status`. Declared
/// again as `CommitReport` in `apps/desktop/src/api.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommitReport {
    /// The write was committed.
    Committed {
        /// The commit made.
        commit: Committed,
    },
    /// `config.yaml` does not ask for commits.
    Disabled,
    /// Commits are on, but the corpus is not in a git repository.
    NotARepository,
    /// The write left nothing git would record.
    NothingToCommit,
    /// Commits are on and the commit was refused or failed. The write it was
    /// meant to record is on disk regardless.
    Refused {
        /// Why, with the code `neb --json` reports for the same refusal.
        error: IpcError,
    },
}

impl CommitReport {
    /// Report the commit that followed a write. A refusal is logged as well:
    /// the write stands, and the corpus now holds an uncommitted change.
    fn of(outcome: Result<CommitOutcome>) -> Self {
        match outcome {
            Ok(CommitOutcome::Committed(commit)) => Self::Committed { commit },
            Ok(CommitOutcome::Disabled) => Self::Disabled,
            Ok(CommitOutcome::NotARepository) => Self::NotARepository,
            Ok(CommitOutcome::NothingToCommit) => Self::NothingToCommit,
            Err(error) => {
                tracing::warn!("the write landed, but its commit was refused: {error}");
                Self::Refused {
                    error: error.into(),
                }
            }
        }
    }
}

/// Append and, when configured, commit one line just as `neb capture` does.
/// Fails only when nothing was written; a refused commit is in the report.
pub fn capture(corpus: &Corpus, text: &str) -> Result<Written<InboxEntry>> {
    // Hold the lock across the write and commit, just as the CLI does. Both
    // core operations re-enter it on this thread without waiting again, and
    // leave this label as the one a waiting writer is told about.
    let _lock = corpus.lock_as(WRITE_LOCK_WAIT, "desktop capture")?;
    let entry = ops::capture(corpus, text)?;
    let commit = CommitReport::of(ops::commit(corpus, "capture", &[&entry.id]));
    Ok(Written {
        value: entry,
        commit,
    })
}

/// Every capture not yet promoted or dropped.
pub fn inbox(corpus: &Corpus) -> Result<Vec<InboxEntry>> {
    Ok(corpus.inbox()?.0)
}

/// Settle one entry through the same core op and commit as `neb drop`. As
/// with [`capture`], a refused commit is reported, not raised.
pub fn drop_entry(corpus: &Corpus, entry: &str) -> Result<Written<InboxEntry>> {
    let _lock = corpus.lock_as(WRITE_LOCK_WAIT, &format!("desktop drop {entry}"))?;
    let dropped = ops::drop(corpus, entry)?;
    let commit = CommitReport::of(ops::commit(corpus, "drop", &[entry]));
    Ok(Written {
        value: dropped,
        commit,
    })
}

/// Promote the captured text as an unlinked root, like `neb promote --quiet`.
/// As with [`capture`], a refused commit is reported, not raised.
pub fn promote_root(corpus: &Corpus, entry: &str) -> Result<Written<Created>> {
    let _lock = corpus.lock_as(WRITE_LOCK_WAIT, &format!("desktop promote {entry}"))?;
    let created = ops::promote(corpus, entry, &ops::Promotion::default(), 0)?;
    let commit = CommitReport::of(ops::commit(
        corpus,
        "promote",
        &[entry, &created.doc.node.id],
    ));
    Ok(Written {
        value: created,
        commit,
    })
}

/// The whole corpus as nodes and edges, for the Graph view.
pub fn graph(corpus: &Corpus) -> Result<GraphExport> {
    let docs = corpus.load_all()?;
    graph::export(&Graph::build(&docs)?)
}

/// Match graph nodes without transferring every body to the webview. One
/// corpus read per debounced query keeps search linear in corpus size.
pub fn graph_search(corpus: &Corpus, query: &str) -> Result<Vec<String>> {
    let needle = query.trim().to_lowercase();
    Ok(corpus
        .load_all()?
        .into_iter()
        .filter(|doc| {
            matches_graph_query(
                &doc.node.id,
                &doc.node.title,
                &doc.body,
                &doc.node.status.to_string(),
                &needle,
            )
        })
        .map(|doc| doc.node.id)
        .collect())
}

fn matches_graph_query(id: &str, title: &str, body: &str, status: &str, needle: &str) -> bool {
    [id, title, body, status]
        .iter()
        .any(|value| value.to_lowercase().contains(needle))
}

/// One node in full, body trimmed and ready for a markdown renderer.
/// Observatory references are located the same way `neb show` locates them,
/// so the panel and the terminal agree on what they say.
///
/// The fields are `neb show --json`'s, but not its shape: this is core's
/// [`NodeView`] as serde writes it, which leaves an absent value out (`kill`,
/// `closed`, empty `notes`, …), where the CLI's `--json` view states every
/// key, as `null` or `[]` (and every author label, as `"human"` where the
/// file omits it). The generated `NodeView.ts` marks those fields optional
/// to match.
pub fn node(corpus: &Corpus, id: &str) -> Result<NodeView> {
    let docs = corpus.load_all()?;
    let observatory = corpus.observatory_root()?.root;
    graph::node(&Graph::build(&docs)?, id)?.with_observatory(observatory.as_deref())
}

/// The file behind a node, for handing to the OS. Fails when the node does
/// not exist, so a typo does not open an empty editor.
pub fn node_file(corpus: &Corpus, id: &str) -> Result<PathBuf> {
    corpus.load(id)?;
    corpus.node_path(id)
}

#[cfg(test)]
mod commit_report_tests {
    use super::CommitReport;
    use std::path::PathBuf;

    /// Git failing under commit-on is a refusal, never one of the outcomes
    /// that mean nothing was asked of git.
    #[test]
    fn a_failed_git_is_refused_not_skipped() {
        let failed = nebula_core::Error::Git {
            root: PathBuf::from("/corpus"),
            context: "commit".into(),
            stderr: "fatal: unable to write new index file".into(),
        };
        let report = CommitReport::of(Err(failed));
        assert!(
            matches!(&report, CommitReport::Refused { error } if error.code == "git"),
            "{report:?}"
        );
    }
}

#[cfg(test)]
mod graph_search_tests {
    use super::matches_graph_query;

    #[test]
    fn matches_every_requested_field_case_insensitively() {
        for query in ["n42", "IDEA", "evidence", "REFUTED"] {
            assert!(matches_graph_query(
                "n42",
                "An idea",
                "new evidence",
                "refuted",
                &query.to_lowercase()
            ));
        }
        assert!(!matches_graph_query(
            "n42",
            "An idea",
            "new evidence",
            "refuted",
            "absent"
        ));
    }
}
