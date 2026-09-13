//! The IPC surface. Each command is one [`crate::session`] call with the
//! library's error turned into the string the frontend shows.
//!
//! The names and shapes are the ones `apps/desktop/src/api.ts` wraps; the
//! payload types are `nebula-core`'s own, so the TypeScript side imports the
//! generated bindings rather than restating them.

// Tauri injects `State` and `AppHandle` by value; that is the command
// signature, not a choice this module gets to make.
#![allow(clippy::needless_pass_by_value)]

use crate::state::AppState;
use crate::{session, tray, watcher};
use nebula_core::{GraphExport, InboxEntry, NodeView};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Append one line to this month's inbox file.
#[tauri::command]
pub fn capture(state: State<'_, AppState>, text: &str) -> Result<InboxEntry, String> {
    let corpus = state.corpus()?;
    session::capture(&corpus, text).map_err(err)
}

/// Every unsettled capture, oldest first.
#[tauri::command]
pub fn inbox(state: State<'_, AppState>) -> Result<Vec<InboxEntry>, String> {
    let corpus = state.corpus()?;
    session::inbox(&corpus).map_err(err)
}

/// The whole corpus as nodes and edges.
#[tauri::command]
pub fn graph(state: State<'_, AppState>) -> Result<GraphExport, String> {
    let corpus = state.corpus()?;
    session::graph(&corpus).map_err(err)
}

/// One node in full.
#[tauri::command]
pub fn node(state: State<'_, AppState>, id: &str) -> Result<NodeView, String> {
    let corpus = state.corpus()?;
    session::node(&corpus, id).map_err(err)
}

/// Hand the node's file to whatever the OS opens `.md` with.
#[tauri::command]
pub fn open_in_editor(app: AppHandle, state: State<'_, AppState>, id: &str) -> Result<(), String> {
    let corpus = state.corpus()?;
    let path = session::node_file(&corpus, id).map_err(err)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

/// Where the corpus was looked for, whether or not it was found.
#[tauri::command]
pub fn corpus_path(state: State<'_, AppState>) -> String {
    state.corpus_root.display().to_string()
}

/// Try the corpus again after the user has fixed the path. Starts the watcher
/// if this is the first time the corpus could be opened.
#[tauri::command]
pub fn reload(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    ensure_watching(&app, &state)?;
    tray::refresh(&app);
    Ok(())
}

/// Start the watcher once the corpus can be opened; idempotent. Opening
/// first matters: the watcher creates `inbox/` if it is missing, and that
/// must never happen under a root that is not a corpus.
pub fn ensure_watching(app: &AppHandle, state: &AppState) -> Result<(), String> {
    if state.watching() {
        return Ok(());
    }
    state.corpus()?;
    let watcher = watcher::start(app.clone(), &state.corpus_root).map_err(err)?;
    state.keep_watcher(watcher);
    Ok(())
}
