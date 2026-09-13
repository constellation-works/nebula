//! What the app holds between commands.

use crate::session;
use nebula_core::Corpus;
use notify::RecommendedWatcher;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// Managed by Tauri; every command borrows it.
///
/// The corpus is opened lazily and cached, so a root that did not exist at
/// launch is picked up by the next command (or the "reload" button) without a
/// restart. The watcher is held here only so it lives as long as the app.
pub struct AppState {
    /// Where the corpus was looked for, resolved once at startup.
    pub corpus_root: PathBuf,
    corpus: Mutex<Option<Corpus>>,
    watcher: Mutex<Option<RecommendedWatcher>>,
}

impl AppState {
    /// Resolve the root the way the CLI does and try to open it. A failure is
    /// not fatal: it is reported by every command until it is fixed.
    pub fn new() -> Self {
        // `resolve_root` fails only when neither `NEBULA_ROOT` nor `HOME` is
        // set, which a GUI launched from a login session never sees. Keep the
        // path the user would expect so the error on screen names it.
        let corpus_root = session::resolve_root().unwrap_or_else(|_| PathBuf::from("~/.nebula"));
        let corpus = session::open(&corpus_root).ok();
        Self {
            corpus_root,
            corpus: Mutex::new(corpus),
            watcher: Mutex::new(None),
        }
    }

    /// The open corpus, opening it now if the last attempt failed. The error
    /// is the library's own message, which names the path it tried.
    pub fn corpus(&self) -> Result<Corpus, String> {
        let mut slot = self.corpus.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(c) = slot.as_ref() {
            return Ok(c.clone());
        }
        let c = session::open(&self.corpus_root).map_err(|e| e.to_string())?;
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
