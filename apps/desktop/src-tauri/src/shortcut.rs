//! The global shortcut and the capture window it toggles.

use crate::{settings, state::AppState};
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// The frameless always-on-top window from `tauri.conf.json`.
pub const CAPTURE_WINDOW: &str = "capture";

/// The plugin, with the one handler every registered shortcut shares.
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_capture(app);
            }
        })
        .build()
}

/// Register `shortcut` (in the plugin's `Alt+Space` syntax) system-wide.
pub fn register(app: &AppHandle, shortcut: &str) -> Result<(), Box<dyn std::error::Error>> {
    let parsed = parse(shortcut).map_err(std::io::Error::other)?;
    app.global_shortcut().register(parsed)?;
    Ok(())
}

fn parse(value: &str) -> Result<Shortcut, String> {
    let parsed: Shortcut = value
        .trim()
        .parse()
        .map_err(|e| format!("Invalid accelerator: {e}"))?;
    if parsed.mods.is_empty() {
        return Err("Invalid accelerator: include a modifier such as Alt, Ctrl or Shift".into());
    }
    Ok(parsed)
}

/// Activate a new shortcut immediately and persist it only after registration
/// succeeds. The old shortcut remains active if registration or saving fails.
pub fn change(app: &AppHandle, value: &str) -> Result<String, String> {
    let next_text = value.trim();
    let next = parse(next_text)?;
    let state = app.state::<AppState>();
    let _change = state.begin_shortcut_change()?;
    let current = state.capture_shortcut();
    let manager = app.global_shortcut();
    let previous = current
        .parse::<Shortcut>()
        .ok()
        .filter(|old| manager.is_registered(*old));
    if previous == Some(next) {
        return Ok(current);
    }
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    manager
        .register(next)
        .map_err(|e| format!("Could not register `{next_text}`: {e}"))?;
    let updated = settings::Settings {
        capture_shortcut: next_text.to_string(),
    };
    if let Err(e) = settings::save(&config_dir, &updated) {
        let _ = manager.unregister(next);
        return Err(format!("Could not save shortcut: {e}"));
    }
    if let Some(old) = previous
        && let Err(e) = manager.unregister(old)
    {
        let restored = settings::Settings {
            capture_shortcut: current.clone(),
        };
        let rollback = settings::save(&config_dir, &restored);
        let _ = manager.unregister(next);
        return Err(format!(
            "Could not replace shortcut: {e}; restoring settings: {rollback:?}"
        ));
    }
    state.set_capture_shortcut(next_text.to_string());
    Ok(next_text.to_string())
}

/// Show the capture window on the screen the cursor is on, or hide it if it
/// is already up. Focus goes with it, so typing can start at once.
pub fn toggle_capture(app: &AppHandle) {
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
fn place_on_active_screen(win: &WebviewWindow) {
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
    use super::parse;

    #[test]
    fn invalid_accelerators_are_refused() {
        for value in ["", "NotAKey", "Space", "Alt+"] {
            assert!(parse(value).is_err(), "{value}");
        }
        assert!(parse("CmdOrCtrl+Shift+N").is_ok());
    }
}
