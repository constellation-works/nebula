//! What the app holds between commands.

use crate::error::DesktopError;
use crate::{session, settings};
use nebula_core::Corpus;
use notify::RecommendedWatcher;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};

/// Managed by Tauri; every command borrows it.
///
/// The corpus is opened lazily and cached, so a root that did not exist at
/// launch is picked up by the next command (or the "reload" button) without a
/// restart. The watcher is held here only so it lives as long as the app.
pub struct AppState {
    /// Where the corpus is looked for, resolved once at startup, or why it
    /// could not be resolved.
    corpus_root: Result<PathBuf, Arc<nebula_core::Error>>,
    corpus: Mutex<Option<Corpus>>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    startup_warnings: Mutex<Vec<String>>,
    capture_shortcut: Mutex<String>,
    shortcut_change: Mutex<()>,
}

impl AppState {
    /// Resolve the root the way the CLI does and try to open it. A failure is
    /// not fatal: it is reported by every command until it is fixed.
    pub fn new() -> Self {
        Self::with_root(session::resolve_root())
    }

    /// Start from a root already resolved, or from the reason it could not
    /// be. An unresolved root stays unresolved: it is a startup warning and
    /// every corpus command's error, never a stand-in path (STD-02 §R26,
    /// §R29). Resolution fails on a missing `HOME` and equally on an empty or
    /// unreadable `~/.config/nebula/root`, so no single guess would be right.
    pub fn with_root(root: nebula_core::Result<PathBuf>) -> Self {
        let corpus_root = root.map_err(Arc::new);
        let corpus = corpus_root
            .as_ref()
            .ok()
            .and_then(|root| session::open(root).ok());
        Self {
            corpus_root,
            corpus: Mutex::new(corpus),
            watcher: Mutex::new(None),
            startup_warnings: Mutex::new(Vec::new()),
            capture_shortcut: Mutex::new(settings::DEFAULT_CAPTURE_SHORTCUT.to_string()),
            shortcut_change: Mutex::new(()),
        }
    }

    /// Retain setup warnings so the tray and a later-opened main window can
    /// report failures from before the webview started.
    pub fn set_startup_warnings(&self, warnings: Vec<String>) {
        *self
            .startup_warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = warnings;
    }

    /// The startup warnings: an unresolved corpus root first, then those
    /// captured during application setup.
    pub fn startup_warnings(&self) -> Vec<String> {
        let setup = self
            .startup_warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        self.corpus_root()
            .err()
            .map(|e| e.to_string())
            .into_iter()
            .chain(setup)
            .collect()
    }

    /// Where the corpus is looked for, or why that is unknown.
    pub fn corpus_root(&self) -> Result<&Path, DesktopError> {
        match &self.corpus_root {
            Ok(root) => Ok(root),
            Err(e) => Err(DesktopError::RootUnresolved(Arc::clone(e))),
        }
    }

    pub fn set_capture_shortcut(&self, shortcut: String) {
        *self
            .capture_shortcut
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = shortcut;
    }

    pub fn capture_shortcut(&self) -> String {
        self.capture_shortcut
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Only shortcut changes take this gate; readers never wait for the OS or
    /// filesystem calls needed to apply a change. `None` while another change
    /// holds it.
    pub fn begin_shortcut_change(&self) -> Option<MutexGuard<'_, ()>> {
        match self.shortcut_change.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(guard)) => Some(guard.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// The open corpus, opening it now if the last attempt failed. The error
    /// is the library's own, which names the path it tried, or the reason
    /// there is no path to try.
    pub fn corpus(&self) -> Result<Corpus, DesktopError> {
        let mut slot = self.corpus.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = slot.as_ref() {
            return Ok(c.clone());
        }
        let c = session::open(self.corpus_root()?)?;
        *slot = Some(c.clone());
        Ok(c)
    }

    /// Whether a watcher is already running.
    pub fn watching(&self) -> bool {
        self.watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Keep a watcher alive for the rest of the app's life.
    pub fn keep_watcher(&self, watcher: RecommendedWatcher) {
        *self.watcher.lock().unwrap_or_else(PoisonError::into_inner) = Some(watcher);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
