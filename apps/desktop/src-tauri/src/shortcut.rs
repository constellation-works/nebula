//! The global shortcut and the capture window it toggles.

use crate::settings::{self, SettingsError};
use crate::state::AppState;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, PhysicalPosition, Runtime, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// The frameless always-on-top window from `tauri.conf.json`.
pub const CAPTURE_WINDOW: &str = "capture";

/// Why a capture shortcut was not put in place. An accelerator that does not
/// parse is told apart from one the OS refused, so neither the startup
/// warning nor the Settings message has to guess from text (STD-02 §R10).
#[derive(Debug, thiserror::Error)]
pub enum ShortcutError {
    /// The text is not an accelerator the plugin understands.
    #[error("invalid accelerator `{value}`: {reason}")]
    Unparsable {
        /// The text as given.
        value: String,
        /// The parser's complaint.
        reason: String,
    },

    /// The accelerator is a bare key; a system-wide shortcut needs a
    /// modifier or it would swallow ordinary typing.
    #[error("invalid accelerator `{0}`: include a modifier such as Alt, Ctrl or Shift")]
    NoModifier(String),

    /// The accelerator parsed, and the OS or plugin would not register it.
    #[error("could not register `{shortcut}`: {source}")]
    Register {
        /// The accelerator.
        shortcut: String,
        /// The plugin's reason, for instance that another app holds it.
        source: tauri_plugin_global_shortcut::Error,
    },

    /// Another shortcut change has not finished.
    #[error("a shortcut change is already in progress")]
    ChangeInProgress,

    /// The app's config directory, where the setting lives, is unknown.
    #[error("could not locate the app config directory: {0}")]
    ConfigDir(#[source] tauri::Error),

    /// The new shortcut is registered but could not be saved, so it was
    /// unregistered again and the old one stays.
    #[error("could not save the shortcut: {0}")]
    Save(#[source] SettingsError),

    /// The old shortcut would not unregister; the new one was withdrawn and
    /// the saved setting restored, or not, as `rollback` says.
    #[error("could not replace `{previous}`: {source}; {}", rollback_outcome(.rollback.as_deref()))]
    Replace {
        /// The shortcut that stays.
        previous: String,
        /// The plugin's reason.
        source: tauri_plugin_global_shortcut::Error,
        /// Why the saved setting could not be restored, if it could not.
        rollback: Option<Box<SettingsError>>,
    },
}

/// How restoring the saved shortcut went, for [`ShortcutError::Replace`].
fn rollback_outcome(rollback: Option<&SettingsError>) -> String {
    match rollback {
        None => "the saved setting was restored".to_string(),
        Some(e) => format!("restoring the saved setting failed too: {e}"),
    }
}

impl ShortcutError {
    /// The stable machine name [`crate::error::IpcError`] carries.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unparsable { .. } => "shortcut_unparsable",
            Self::NoModifier(_) => "shortcut_no_modifier",
            Self::Register { .. } => "shortcut_register_refused",
            Self::ChangeInProgress => "shortcut_change_in_progress",
            Self::ConfigDir(_) => "config_dir_unavailable",
            Self::Save { .. } => "shortcut_save_failed",
            Self::Replace { .. } => "shortcut_replace_failed",
        }
    }
}

/// The plugin, with the one handler every registered shortcut shares.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_capture(app);
            }
        })
        .build()
}

/// Register `shortcut` (in the plugin's `Alt+Space` syntax) system-wide.
pub fn register<R: Runtime>(app: &AppHandle<R>, shortcut: &str) -> Result<(), ShortcutError> {
    let parsed = parse(shortcut)?;
    app.global_shortcut()
        .register(parsed)
        .map_err(|source| ShortcutError::Register {
            shortcut: shortcut.trim().to_string(),
            source,
        })
}

/// Parse an accelerator, refusing one without a modifier.
pub fn parse(value: &str) -> Result<Shortcut, ShortcutError> {
    let value = value.trim();
    let parsed: Shortcut = value.parse().map_err(|e| ShortcutError::Unparsable {
        value: value.to_string(),
        reason: format!("{e}"),
    })?;
    if parsed.mods.is_empty() {
        return Err(ShortcutError::NoModifier(value.to_string()));
    }
    Ok(parsed)
}

/// Activate a new shortcut immediately and persist it only after registration
/// succeeds. The old shortcut remains active if registration or saving fails.
pub fn change<R: Runtime>(app: &AppHandle<R>, value: &str) -> Result<String, ShortcutError> {
    let next_text = value.trim();
    let next = parse(next_text)?;
    let state = app.state::<AppState>();
    let _change = state
        .begin_shortcut_change()
        .ok_or(ShortcutError::ChangeInProgress)?;
    let current = state.capture_shortcut();
    let manager = app.global_shortcut();
    let previous = current
        .parse::<Shortcut>()
        .ok()
        .filter(|old| manager.is_registered(*old));
    if previous == Some(next) {
        return Ok(current);
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(ShortcutError::ConfigDir)?;
    manager
        .register(next)
        .map_err(|source| ShortcutError::Register {
            shortcut: next_text.to_string(),
            source,
        })?;
    let updated = settings::Settings {
        capture_shortcut: next_text.to_string(),
    };
    if let Err(e) = settings::save(&config_dir, &updated) {
        let _ = manager.unregister(next);
        return Err(ShortcutError::Save(e));
    }
    if let Some(old) = previous
        && let Err(source) = manager.unregister(old)
    {
        let restored = settings::Settings {
            capture_shortcut: current.clone(),
        };
        let rollback = settings::save(&config_dir, &restored);
        let _ = manager.unregister(next);
        return Err(ShortcutError::Replace {
            previous: current,
            source,
            rollback: rollback.err().map(Box::new),
        });
    }
    state.set_capture_shortcut(next_text.to_string());
    Ok(next_text.to_string())
}

/// Show the capture window on the screen the cursor is on, or hide it if it
/// is already up. Focus goes with it, so typing can start at once.
pub fn toggle_capture<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window(CAPTURE_WINDOW) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }
    place_on_active_screen(&win);
    let _ = win.show();
    let _ = win.set_focus();
}

/// Centre horizontally on the cursor's monitor, a third of the way down,
/// where a launcher is expected. Falls back to the primary screen's centre
/// when the cursor cannot be located.
fn place_on_active_screen<R: Runtime>(win: &WebviewWindow<R>) {
    let monitor = win
        .cursor_position()
        .ok()
        .and_then(|p| win.monitor_from_point(p.x, p.y).ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, win.outer_size()) else {
        let _ = win.center();
        return;
    };
    let area = monitor.work_area();
    let width = i32::try_from(area.size.width).unwrap_or(i32::MAX);
    let height = i32::try_from(area.size.height).unwrap_or(i32::MAX);
    let own_width = i32::try_from(size.width).unwrap_or(0);
    let x = area.position.x + (width - own_width) / 2;
    let y = area.position.y + height / 3;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

#[cfg(test)]
mod tests {
    use super::{ShortcutError, parse};

    #[test]
    fn invalid_accelerators_are_refused() {
        for value in ["", "NotAKey", "Space", "Alt+"] {
            assert!(parse(value).is_err(), "{value}");
        }
        assert!(parse("CmdOrCtrl+Shift+N").is_ok());
    }

    #[test]
    fn an_unparsable_accelerator_is_a_parse_error_not_a_registration_error() {
        let refused = parse("Alt+NoSuchKey").unwrap_err();
        assert!(
            matches!(&refused, ShortcutError::Unparsable { value, .. } if value == "Alt+NoSuchKey"),
            "{refused:?}"
        );
        assert_eq!(refused.code(), "shortcut_unparsable");
        assert!(refused.to_string().contains("`Alt+NoSuchKey`"), "{refused}");
    }

    #[test]
    fn a_bare_key_is_refused_for_its_missing_modifier() {
        let refused = parse(" Space ").unwrap_err();
        assert!(
            matches!(&refused, ShortcutError::NoModifier(value) if value == "Space"),
            "{refused:?}"
        );
        assert_eq!(refused.code(), "shortcut_no_modifier");
    }
}
