//! The corpus as the desktop reads it.
//!
//! One thin function per command, each a single `nebula-core` call, so the
//! `#[tauri::command]` wrappers in [`crate::commands`] carry no logic and this
//! file can be exercised by a test without a window. Nothing here shells out
//! to `neb`: the desktop links the library and sees exactly what the CLI sees.

use nebula_core::{Corpus, Created, Graph, GraphExport, InboxEntry, NodeView, Result, graph, ops};
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

/// Append and, when configured, commit one line just as `neb capture` does.
pub fn capture(corpus: &Corpus, text: &str) -> Result<InboxEntry> {
    // Hold the lock across the write and commit, just as the CLI does. Both
    // core operations re-enter it on this thread without waiting again.
    let _lock = corpus.lock_within(WRITE_LOCK_WAIT)?;
    let entry = ops::capture(corpus, text)?;
    ops::commit(corpus, "capture", &[&entry.id])?;
    Ok(entry)
}

/// Every capture not yet promoted or dropped.
pub fn inbox(corpus: &Corpus) -> Result<Vec<InboxEntry>> {
    Ok(corpus.inbox()?.0)
}

/// Settle one entry through the same core op and commit as `neb drop`.
pub fn drop_entry(corpus: &Corpus, entry: &str) -> Result<InboxEntry> {
    let _lock = corpus.lock_within(WRITE_LOCK_WAIT)?;
    let dropped = ops::drop(corpus, entry)?;
    ops::commit(corpus, "drop", &[entry])?;
    Ok(dropped)
}

/// Promote the captured text as an unlinked root, like `neb promote --quiet`.
pub fn promote_root(corpus: &Corpus, entry: &str) -> Result<Created> {
    let _lock = corpus.lock_within(WRITE_LOCK_WAIT)?;
    let created = ops::promote(corpus, entry, &ops::Promotion::default(), 0)?;
    ops::commit(corpus, "promote", &[entry, &created.doc.node.id])?;
    Ok(created)
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

/// One node in full: the `neb show --json` shape, body trimmed and ready for
/// a markdown renderer. Observatory references are located the same way
/// `neb show` locates them, so the panel and the terminal agree.
pub fn node(corpus: &Corpus, id: &str) -> Result<NodeView> {
    let docs = corpus.load_all()?;
    let observatory = corpus.observatory_root()?.root;
    Ok(graph::node(&Graph::build(&docs)?, id)?.with_observatory(observatory.as_deref()))
}

/// The file behind a node, for handing to the OS. Fails when the node does
/// not exist, so a typo does not open an empty editor.
pub fn node_file(corpus: &Corpus, id: &str) -> Result<PathBuf> {
    corpus.load(id)?;
    corpus.node_path(id)
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
