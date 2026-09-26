//! What a command can fail with, and the one shape the webview receives.
//!
//! [`DesktopError`] is the desktop's own typed error: `nebula-core`'s error
//! unchanged, plus the failures only an app has (a worker thread, the opener,
//! the watcher, the global shortcut, the OS login registration, a root that
//! could not be resolved). [`IpcError`] is what every `#[tauri::command]`
//! returns, and `From<DesktopError>` is the only place one becomes the other
//! (STD-02 §R12), so the frontend branches on `code` and never on a message.

use crate::shortcut::ShortcutError;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything a command can fail with.
#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    /// A `nebula-core` failure, reported with core's own code and message,
    /// the ones `neb --json` reports for the same refusal.
    #[error(transparent)]
    Core(#[from] nebula_core::Error),

    /// The corpus root could not be resolved when the app started, so there
    /// is no path to open. Kept from startup and reported by every corpus
    /// command, with the resolution error's own code, rather than replaced
    /// by a path nobody checked.
    #[error("could not resolve the corpus root: {0}")]
    RootUnresolved(Arc<nebula_core::Error>),

    /// The worker thread a blocking command ran on did not return a result.
    #[error("the command's worker thread failed: {0}")]
    Worker(#[source] tauri::Error),

    /// The OS would not hand a node's file to its default application.
    #[error("could not open {}: {source}", .path.display())]
    Open {
        /// The node file.
        path: PathBuf,
        /// The opener's reason.
        source: tauri_plugin_opener::Error,
    },

    /// The watcher on `nodes/` and `inbox/` could not start.
    #[error("could not watch {} for changes: {source}", .root.display())]
    Watch {
        /// The corpus root.
        root: PathBuf,
        /// The watcher's reason.
        source: notify::Error,
    },

    /// The capture shortcut could not be parsed, registered or saved.
    /// Boxed: it is the largest payload, and every command's error would
    /// otherwise carry its size (STD-02 §R11).
    #[error(transparent)]
    Shortcut(Box<ShortcutError>),

    /// The OS login registration could not be read or changed.
    #[error("launch at login: {0}")]
    LaunchAtLogin(#[source] tauri_plugin_autostart::Error),

    /// The OS accepted a launch-at-login change but reports the old state.
    #[error("the OS did not apply the launch-at-login change (still {})", if *.actual { "on" } else { "off" })]
    LaunchAtLoginNotApplied {
        /// What the OS reports after the change.
        actual: bool,
    },
}

impl DesktopError {
    /// The stable machine name the frontend matches. A core error keeps
    /// core's code, and so does an unresolved root: the webview learns what
    /// went wrong from the same code `neb --json` would print.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Core(e) => e.code(),
            Self::RootUnresolved(e) => e.code(),
            Self::Worker(_) => "worker_failed",
            Self::Open { .. } => "open_failed",
            Self::Watch { .. } => "watch_failed",
            Self::Shortcut(e) => e.code(),
            Self::LaunchAtLogin(_) => "launch_at_login_failed",
            Self::LaunchAtLoginNotApplied { .. } => "launch_at_login_not_applied",
        }
    }
}

impl From<ShortcutError> for DesktopError {
    fn from(e: ShortcutError) -> Self {
        Self::Shortcut(Box::new(e))
    }
}

/// The one error every command returns: `{ code, message }` in the webview,
/// declared again as `IpcError` in `apps/desktop/src/ipcError.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpcError {
    /// [`DesktopError::code`]: what the frontend branches on.
    pub code: String,
    /// The error's own message: what the frontend shows.
    pub message: String,
}

/// The translator (STD-02 §R12): every command error reaches the webview
/// through this and nothing else.
impl From<DesktopError> for IpcError {
    fn from(e: DesktopError) -> Self {
        Self {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

/// A core error goes through the same translator as every other.
impl From<nebula_core::Error> for IpcError {
    fn from(e: nebula_core::Error) -> Self {
        DesktopError::from(e).into()
    }
}
