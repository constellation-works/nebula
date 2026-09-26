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
use std::path::Path;
use tauri::{Manager, WindowEvent};

/// Build and run the app. Returns when the user quits.
///
/// # Panics
///
/// When Tauri cannot start at all (no webview, no event loop); there is
/// nothing to show an error in at that point.
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(shortcut::plugin())
        .manage(AppState::new());

    // Keep the embedded WebDriver transport behind an explicit debug-only test feature.
    #[cfg(all(debug_assertions, feature = "webdriver-test"))]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    builder
        .invoke_handler(tauri::generate_handler![
            commands::capture,
            commands::inbox,
            commands::drop_entry,
            commands::promote_root,
            commands::graph,
            commands::node,
            commands::open_in_editor,
            commands::corpus_path,
            commands::startup_warnings,
            commands::reload,
        ])
        .setup(|app| {
            // A menu-bar app: no dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            let config_dir = app.path().app_config_dir()?;
            let settings_path = config_dir.join(settings::FILE_NAME);
            let (settings, warning) = settings::load(&config_dir);
            let mut startup_warnings = warning.into_iter().collect::<Vec<_>>();
            if let Err(e) = shortcut::register(handle, &settings.capture_shortcut) {
                startup_warnings.push(shortcut_warning(
                    &settings_path,
                    &settings.capture_shortcut,
                    e,
                ));
            }
            for warning in &startup_warnings {
                eprintln!("startup: {warning}");
            }

            let state = app.state::<AppState>();
            state.set_startup_warnings(startup_warnings.clone());
            tray::build(handle, &startup_warnings)?;

            // A missing corpus is reported by every command; the watcher
            // starts from `reload` once the user has put one there.
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

fn shortcut_warning(settings_path: &Path, shortcut: &str, error: impl std::fmt::Display) -> String {
    format!(
        "could not register capture shortcut `{shortcut}` (settings: {}): {error}",
        settings_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::shortcut_warning;
    use std::path::Path;

    #[test]
    fn shortcut_registration_warning_names_shortcut_and_settings_path() {
        let path = Path::new("/user/config/settings.json");
        let warning = shortcut_warning(path, "CmdOrCtrl+Shift+N", "already registered");

        assert!(warning.contains("CmdOrCtrl+Shift+N"));
        assert!(warning.contains("/user/config/settings.json"));
        assert!(warning.contains("already registered"));
    }
}
