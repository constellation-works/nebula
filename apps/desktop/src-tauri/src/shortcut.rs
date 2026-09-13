//! The global shortcut and the capture window it toggles.

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
    let parsed: Shortcut = shortcut.parse()?;
    app.global_shortcut().register(parsed)?;
    Ok(())
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
