//! The IPC surface. Each command is one [`crate::session`] call, and every
//! failure reaches the webview as an [`IpcError`] built by its one translator,
//! so the frontend branches on `code` and shows `message`.
//!
//! The names and shapes are the ones `apps/desktop/src/api.ts` wraps; the
//! payload types are `nebula-core`'s own, so the TypeScript side imports the
//! generated bindings rather than restating them. Every command is generic
//! over the runtime so `tests/commands.rs` drives this same table through
//! Tauri's mock runtime.

// Tauri injects `State` and `AppHandle` by value; that is the command
// signature, not a choice this module gets to make.
#![allow(clippy::needless_pass_by_value)]

use crate::error::{DesktopError, IpcError};
use crate::state::AppState;
use crate::{session, shortcut, tray, watcher};
use nebula_core::{Created, GraphExport, InboxEntry, NodeView};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

/// Run `work` off the webview's thread and translate its failure, or the
/// worker's, once.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DesktopError> + Send + 'static,
) -> Result<T, IpcError> {
    Ok(tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(DesktopError::Worker)??)
}

/// Append one line to this month's inbox file. A held corpus lock is the
/// `locked` code, which the capture box retries.
#[tauri::command]
pub async fn capture<R: Runtime>(app: AppHandle<R>, text: String) -> Result<InboxEntry, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        Ok(session::capture(&corpus, &text)?)
    })
    .await
}

/// Every unsettled capture, oldest first.
#[tauri::command]
pub async fn inbox<R: Runtime>(app: AppHandle<R>) -> Result<Vec<InboxEntry>, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        Ok(session::inbox(&corpus)?)
    })
    .await
}

/// Drop one unsettled entry without blocking the webview on the corpus lock.
#[tauri::command]
pub async fn drop_entry<R: Runtime>(
    app: AppHandle<R>,
    entry: String,
) -> Result<InboxEntry, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        let dropped = session::drop_entry(&corpus, &entry)?;
        tray::refresh(&app);
        Ok(dropped)
    })
    .await
}

/// Promote one entry as an unlinked root node.
#[tauri::command]
pub async fn promote_root<R: Runtime>(
    app: AppHandle<R>,
    entry: String,
) -> Result<Created, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        let created = session::promote_root(&corpus, &entry)?;
        tray::refresh(&app);
        Ok(created)
    })
    .await
}

/// The whole corpus as nodes and edges.
#[tauri::command]
pub async fn graph<R: Runtime>(app: AppHandle<R>) -> Result<GraphExport, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        Ok(session::graph(&corpus)?)
    })
    .await
}

/// IDs matching id, title, body, or status in the current corpus.
#[tauri::command]
pub async fn graph_search<R: Runtime>(
    app: AppHandle<R>,
    query: String,
) -> Result<Vec<String>, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        Ok(session::graph_search(&corpus, &query)?)
    })
    .await
}

/// The capture shortcut loaded and registered at startup.
#[tauri::command]
pub fn capture_shortcut<R: Runtime>(app: AppHandle<R>) -> String {
    app.state::<AppState>().capture_shortcut()
}

/// Replace the global capture shortcut without restarting the app.
#[tauri::command]
pub async fn set_capture_shortcut<R: Runtime>(
    app: AppHandle<R>,
    shortcut: String,
) -> Result<String, IpcError> {
    blocking(move || Ok(shortcut::change(&app, &shortcut)?)).await
}

/// Read the OS login registration, which persists outside settings.json.
#[tauri::command]
pub async fn launch_at_login<R: Runtime>(app: AppHandle<R>) -> Result<bool, IpcError> {
    blocking(move || {
        app.autolaunch()
            .is_enabled()
            .map_err(DesktopError::LaunchAtLogin)
    })
    .await
}

/// Enable or disable OS login registration and return the resulting state.
#[tauri::command]
pub async fn set_launch_at_login<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<bool, IpcError> {
    blocking(move || {
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(DesktopError::LaunchAtLogin)?;
        let actual = manager.is_enabled().map_err(DesktopError::LaunchAtLogin)?;
        if actual != enabled {
            return Err(DesktopError::LaunchAtLoginNotApplied { actual });
        }
        Ok(actual)
    })
    .await
}

/// One node in full.
#[tauri::command]
pub async fn node<R: Runtime>(app: AppHandle<R>, id: String) -> Result<NodeView, IpcError> {
    blocking(move || {
        let corpus = app.state::<AppState>().corpus()?;
        Ok(session::node(&corpus, &id)?)
    })
    .await
}

/// Hand the node's file to whatever the OS opens `.md` with.
#[tauri::command]
pub async fn open_in_editor<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), IpcError> {
    let lookup = app.clone();
    let path = blocking(move || {
        let corpus = lookup.state::<AppState>().corpus()?;
        Ok(session::node_file(&corpus, &id)?)
    })
    .await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|source| DesktopError::Open { path, source }.into())
}

/// Where the corpus is looked for, whether or not it was found; `null` when
/// the root could not be resolved, whose reason `startup_warnings` and every
/// corpus command report.
#[tauri::command]
pub async fn corpus_path<R: Runtime>(app: AppHandle<R>) -> Option<String> {
    app.state::<AppState>()
        .corpus_root()
        .ok()
        .map(|root| root.display().to_string())
}

/// Startup issues captured before the webview opened, such as an unresolved
/// corpus root or settings and global-shortcut failures.
#[tauri::command]
pub async fn startup_warnings<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    app.state::<AppState>().startup_warnings()
}

/// Try the corpus again after the user has fixed the path. Starts the watcher
/// if this is the first time the corpus could be opened.
#[tauri::command]
pub async fn reload<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    blocking(move || {
        let state = app.state::<AppState>();
        ensure_watching(&app, &state)?;
        tray::refresh(&app);
        Ok(())
    })
    .await
}

/// Start the watcher once the corpus can be opened; idempotent. Opening
/// first matters: the watcher creates `inbox/` if it is missing, and that
/// must never happen under a root that is not a corpus.
pub fn ensure_watching<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<(), DesktopError> {
    if state.watching() {
        return Ok(());
    }
    state.corpus()?;
    let root = state.corpus_root()?;
    let watcher = watcher::start(app.clone(), root).map_err(|source| DesktopError::Watch {
        root: root.to_path_buf(),
        source,
    })?;
    state.keep_watcher(watcher);
    Ok(())
}
