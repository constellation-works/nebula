//! Nebula's menu-bar app: the corpus, read through `nebula-core` and drawn in
//! a webview. The library is linked; nothing here shells out to `neb`.
//!
//! - [`session`]  one function per command, each a single core call
//! - [`state`]    the corpus root and the lazily opened corpus
//! - [`commands`] the `#[tauri::command]` wrappers
//! - [`error`]    the typed error, and the `{ code, message }` the webview gets
//! - [`watcher`]  `corpus-changed` on `nodes/` and `inbox/` writes
//! - [`tray`]     the menu-bar item with the inbox count
//! - [`shortcut`] the global shortcut and the capture window it toggles
//! - [`settings`] the capture shortcut settings file
//!
//! The pieces that need a running app (tray, shortcut, watcher) are wired in
//! [`run`]; everything else is plain and tested without one.
//!
//! Diagnostics go through `tracing` (STD-02 §R15): [`run`] installs the one
//! subscriber, and nothing else here writes to a standard stream.

pub mod commands;
pub mod error;
pub mod session;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod watcher;

use state::AppState;
use std::fmt::Display;
use std::path::Path;
use tauri::ipc::Invoke;
use tauri::{Manager, Runtime, WindowEvent};

/// Every command the webview can call. One table, used by [`run`] and by
/// `tests/commands.rs` under the mock runtime, so the test calls exactly what
/// the app registers.
pub fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::capture,
        commands::inbox,
        commands::drop_entry,
        commands::promote_root,
        commands::graph,
        commands::graph_search,
        commands::capture_shortcut,
        commands::set_capture_shortcut,
        commands::launch_at_login,
        commands::set_launch_at_login,
        commands::node,
        commands::open_in_editor,
        commands::corpus_path,
        commands::startup_warnings,
        commands::reload,
    ]
}

/// Build and run the app. Returns when the user quits.
///
/// # Panics
///
/// When Tauri cannot start at all (no webview, no event loop); there is
/// nothing to show an error in at that point.
pub fn run() {
    // The one subscriber, and the one place allowed to name stderr
    // (STD-02 §R15): a menu-bar app has nowhere else to say what went wrong
    // outside its windows.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(shortcut::plugin())
        .manage(AppState::new());

    // Keep the embedded WebDriver transport behind an explicit debug-only test feature.
    #[cfg(all(debug_assertions, feature = "webdriver-test"))]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    builder
        .invoke_handler(handler())
        .setup(|app| {
            // A menu-bar app: no dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            let config_dir = app.path().app_config_dir()?;
            let settings_path = config_dir.join(settings::FILE_NAME);
            let (settings, warning) = settings::load(&config_dir);
            let mut setup_warnings = warning.into_iter().collect::<Vec<_>>();
            if let Err(e) = shortcut::register(handle, &settings.capture_shortcut) {
                setup_warnings.push(shortcut_warning(
                    &settings_path,
                    &settings.capture_shortcut,
                    &e,
                ));
            }

            let state = app.state::<AppState>();
            state.set_capture_shortcut(settings.capture_shortcut.clone());
            state.set_startup_warnings(setup_warnings);
            // Read back, so an unresolved corpus root is among them.
            let startup_warnings = state.startup_warnings();
            for warning in &startup_warnings {
                tracing::warn!("startup: {warning}");
            }
            tray::build(handle, &startup_warnings)?;

            // A missing corpus is reported by every command; the watcher
            // starts from `reload` once the user has put one there.
            if let Err(e) = commands::ensure_watching(handle, &state) {
                tracing::warn!("not watching: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the main window hides it; the tray keeps the app alive.
            // Fail open: a window that will not hide is logged, and the app
            // stays running either way.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                fail_open("hiding the main window", window.hide());
            }
            // The capture window is a launcher: it goes away when it loses
            // focus, so a stray click never leaves it floating. Fail open, as
            // above.
            WindowEvent::Focused(false) if window.label() == shortcut::CAPTURE_WINDOW => {
                fail_open("hiding the capture window", window.hide());
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("failed to start the Nebula desktop app");
}

/// Log a side channel's failure at `warn`, naming what failed and why, and
/// carry on: the value when there is one, `None` when there is not.
///
/// For the calls no user action waits on: showing, hiding and placing a
/// window, the tray title, the `corpus-changed` event, a lock holder's name
/// in an error message. Each caller declares the choice with a "fail open"
/// comment at the call (STD-02 §R31): such a failure is recorded, and never
/// fails the operation it rides along with.
pub(crate) fn fail_open<T, E: Display>(what: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!("{what} failed: {error}");
            None
        }
    }
}

fn shortcut_warning(
    settings_path: &Path,
    shortcut: &str,
    error: &shortcut::ShortcutError,
) -> String {
    format!(
        "could not register capture shortcut `{shortcut}` (settings: {}): {error}",
        settings_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::{fail_open, shortcut, shortcut_warning};
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex, PoisonError};

    /// What a test subscriber wrote.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Log {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
                .into_owned()
        }
    }

    #[test]
    fn side_channel_failures_are_logged_not_propagated() {
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();

        let (failed, passed) = tracing::subscriber::with_default(subscriber, || {
            (
                fail_open::<(), _>("hiding the capture window", Err("no such window")),
                fail_open::<u8, &str>("updating the tray count", Ok(7)),
            )
        });

        assert_eq!(failed, None);
        assert_eq!(passed, Some(7));
        let text = log.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1, "one event, for the failure only: {text}");
        assert!(lines[0].contains("WARN"), "{text}");
        assert!(
            lines[0].contains("hiding the capture window failed: no such window"),
            "{text}"
        );
    }

    #[test]
    fn shortcut_registration_warning_names_shortcut_and_settings_path() {
        let path = Path::new("/user/config/settings.json");
        let refused = shortcut::parse("CmdOrCtrl+Shift+NoSuchKey").unwrap_err();
        let warning = shortcut_warning(path, "CmdOrCtrl+Shift+NoSuchKey", &refused);

        assert!(warning.contains("CmdOrCtrl+Shift+NoSuchKey"));
        assert!(warning.contains("/user/config/settings.json"));
        assert!(warning.contains("invalid accelerator"), "{warning}");
    }
}
