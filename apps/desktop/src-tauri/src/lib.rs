//! Nebula's menu-bar app: the corpus, read through `nebula-core` and drawn in
//! a webview. The library is linked; nothing here shells out to `neb`.
//!
//! - [`session`]  one function per command, each a single core call
//! - [`state`]    the corpus root and the lazily opened corpus
//! - [`commands`] the `#[tauri::command]` wrappers
//! - [`watcher`]  `corpus-changed` on `nodes/` and `inbox/` writes
//! - [`tray`]     the menu-bar item with the inbox count
//! - [`shortcut`] the global shortcut and the capture window it toggles
//! - [`settings`] the one settings file, for the shortcut
//!
//! The pieces that need a running app (tray, shortcut, watcher) are wired in
//! [`run`]; everything else is plain and tested without one.

pub mod commands;
pub mod session;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod watcher;

use state::AppState;
use tauri::{Manager, WindowEvent};

/// Build and run the app. Returns when the user quits.
///
/// # Panics
///
/// When Tauri cannot start at all (no webview, no event loop); there is
/// nothing to show an error in at that point.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(shortcut::plugin())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::capture,
            commands::inbox,
            commands::graph,
            commands::node,
            commands::open_in_editor,
            commands::corpus_path,
            commands::reload,
        ])
        .setup(|app| {
            // A menu-bar app: no dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            tray::build(handle)?;

            let (settings, warning) = settings::load(&app.path().app_config_dir()?);
            if let Some(w) = warning {
                eprintln!("settings: {w}");
            }
            if let Err(e) = shortcut::register(handle, &settings.capture_shortcut) {
                eprintln!(
                    "could not register `{}` as the capture shortcut: {e}",
                    settings.capture_shortcut
                );
            }

            // A missing corpus is reported by every command; the watcher
            // starts from `reload` once the user has put one there.
            let state = app.state::<AppState>();
            if let Err(e) = commands::ensure_watching(handle, &state) {
                eprintln!("not watching: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the main window hides it; the tray keeps the app alive.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // The capture window is a launcher: it goes away when it loses
            // focus, so a stray click never leaves it floating.
            WindowEvent::Focused(false) if window.label() == shortcut::CAPTURE_WINDOW => {
                let _ = window.hide();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("failed to start the Nebula desktop app");
}
