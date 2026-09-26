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
use crate::{session, shortcut, tray, watcher};
use nebula_core::{Created, Error, GraphExport, InboxEntry, NodeView};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Append one line to this month's inbox file.
#[tauri::command]
pub async fn capture(app: AppHandle, text: String) -> Result<InboxEntry, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        session::capture(&corpus, &text).map_err(|e| match e {
            Error::Locked { .. } => "corpus busy".to_string(),
            other => err(other),
        })
    })
    .await
    .map_err(err)?
}

/// Every unsettled capture, oldest first.
#[tauri::command]
pub async fn inbox(app: AppHandle) -> Result<Vec<InboxEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        session::inbox(&corpus).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Drop one unsettled entry without blocking the webview on the corpus lock.
#[tauri::command]
pub async fn drop_entry(app: AppHandle, entry: String) -> Result<InboxEntry, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        let dropped = session::drop_entry(&corpus, &entry).map_err(err)?;
        tray::refresh(&app);
        Ok(dropped)
    })
    .await
    .map_err(err)?
}

/// Promote one entry as an unlinked root node.
#[tauri::command]
pub async fn promote_root(app: AppHandle, entry: String) -> Result<Created, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        let created = session::promote_root(&corpus, &entry).map_err(err)?;
        tray::refresh(&app);
        Ok(created)
    })
    .await
    .map_err(err)?
}

/// The whole corpus as nodes and edges.
#[tauri::command]
pub async fn graph(app: AppHandle) -> Result<GraphExport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        session::graph(&corpus).map_err(err)
    })
    .await
    .map_err(err)?
}

/// IDs matching id, title, body, or status in the current corpus.
#[tauri::command]
pub async fn graph_search(app: AppHandle, query: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        session::graph_search(&corpus, &query).map_err(err)
    })
    .await
    .map_err(err)?
}

/// The capture shortcut loaded and registered at startup.
#[tauri::command]
pub fn capture_shortcut(app: AppHandle) -> String {
    app.state::<AppState>().capture_shortcut()
}

/// Replace the global capture shortcut without restarting the app.
#[tauri::command]
pub async fn set_capture_shortcut(app: AppHandle, shortcut: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || shortcut::change(&app, &shortcut))
        .await
        .map_err(err)?
}

/// Read the OS login registration, which persists outside settings.json.
#[tauri::command]
pub async fn launch_at_login(app: AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || app.autolaunch().is_enabled().map_err(err))
        .await
        .map_err(err)?
}

/// Enable or disable OS login registration and return the resulting state.
#[tauri::command]
pub async fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(err)?;
        let actual = manager.is_enabled().map_err(err)?;
        if actual != enabled {
            return Err("The OS did not apply the launch-at-login change".to_string());
        }
        Ok(actual)
    })
    .await
    .map_err(err)?
}

/// One node in full.
#[tauri::command]
pub async fn node(app: AppHandle, id: String) -> Result<NodeView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        session::node(&corpus, &id).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Hand the node's file to whatever the OS opens `.md` with.
#[tauri::command]
pub async fn open_in_editor(app: AppHandle, id: String) -> Result<(), String> {
    let lookup = app.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        let corpus = lookup.state::<AppState>().corpus()?;
        session::node_file(&corpus, &id).map_err(err)
    })
    .await
    .map_err(err)??;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

/// Where the corpus was looked for, whether or not it was found.
#[tauri::command]
pub async fn corpus_path(app: AppHandle) -> String {
    app.state::<AppState>().corpus_root.display().to_string()
}

/// Startup issues captured before the webview opened, such as settings or
/// global-shortcut failures.
#[tauri::command]
pub async fn startup_warnings(app: AppHandle) -> Vec<String> {
    app.state::<AppState>().startup_warnings()
}

/// Try the corpus again after the user has fixed the path. Starts the watcher
/// if this is the first time the corpus could be opened.
#[tauri::command]
pub async fn reload(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        ensure_watching(&app, &state)?;
        tray::refresh(&app);
        Ok(())
    })
    .await
    .map_err(err)?
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
