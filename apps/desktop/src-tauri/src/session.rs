//! The corpus as the desktop reads it.
//!
//! One thin function per command, each a single `nebula-core` call, so the
//! `#[tauri::command]` wrappers in [`crate::commands`] carry no logic and this
//! file can be exercised by a test without a window. Nothing here shells out
//! to `neb`: the desktop links the library and sees exactly what the CLI sees.

use nebula_core::{Corpus, Graph, GraphExport, InboxEntry, NodeView, Result, graph, ops};
use std::path::{Path, PathBuf};

/// Where the corpus is expected: `NEBULA_ROOT`, else `~/.nebula`. The CLI's
/// rule with no `--root`, so the app and the terminal never disagree.
pub fn resolve_root() -> Result<PathBuf> {
    Corpus::resolve_root(None)
}

/// Open the corpus at `root`. Fails when there is none, or when it is at a
/// schema this build does not read; the desktop never creates one, because a
/// wrong `NEBULA_ROOT` should be seen, not papered over.
pub fn open(root: &Path) -> Result<Corpus> {
    Corpus::open(Some(root.to_path_buf()))
}

/// Append one line to this month's inbox, exactly as `neb capture` does.
pub fn capture(corpus: &Corpus, text: &str) -> Result<InboxEntry> {
    ops::capture(corpus, text)
}

/// Every capture not yet promoted or dropped.
pub fn inbox(corpus: &Corpus) -> Result<Vec<InboxEntry>> {
    Ok(corpus.inbox()?.0)
}

/// The whole corpus as nodes and edges, for the Graph view.
pub fn graph(corpus: &Corpus) -> Result<GraphExport> {
    let docs = corpus.load_all()?;
    graph::export(&Graph::build(&docs)?)
}

/// One node in full: the `neb show --json` shape, body trimmed and ready for
/// a markdown renderer.
pub fn node(corpus: &Corpus, id: &str) -> Result<NodeView> {
    let docs = corpus.load_all()?;
    graph::node(&Graph::build(&docs)?, id)
}

/// The file behind a node, for handing to the OS. Fails when the node does
/// not exist, so a typo does not open an empty editor.
pub fn node_file(corpus: &Corpus, id: &str) -> Result<PathBuf> {
    corpus.load(id)?;
    Ok(corpus.node_path(id))
}
