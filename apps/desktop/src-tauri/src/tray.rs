//! The menu-bar item: the unsettled inbox count and app actions.

use crate::error::DesktopError;
use crate::state::AppState;
use crate::{fail_open, session, shortcut};
use tauri::menu::{IsMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The tray's id, for finding it again from the watcher.
pub const ID: &str = "nebula";

/// Build the tray. Called once, from setup.
pub fn build<R: Runtime>(app: &AppHandle<R>, warnings: &[String]) -> tauri::Result<()> {
    let capture = MenuItem::with_id(app, "capture", "Capture", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Nebula", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let warning_items = warnings
        .iter()
        .enumerate()
        .map(|(index, warning)| {
            MenuItem::with_id(
                app,
                format!("startup-warning-{index}"),
                format!("Startup warning: {warning}"),
                false,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let mut menu_items: Vec<&dyn IsMenuItem<R>> = vec![&capture, &open, &settings];
    menu_items.extend(warning_items.iter().map(|item| item as &dyn IsMenuItem<R>));
    menu_items.push(&quit);
    let menu = Menu::with_items(app, &menu_items)?;

    TrayIconBuilder::with_id(ID)
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(true)
        .tooltip("Nebula")
        .title(title(app))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "capture" => shortcut::toggle_capture(app),
            "open" => show_main(app),
            "settings" => {
                show_main(app);
                // Fail open: a missed event is logged; the window is up for
                // the user to open Settings in by hand.
                fail_open(
                    "asking the window to show Settings",
                    app.emit("show-settings", ()),
                );
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Recompute the count. Cheap enough to do on every corpus change: the inbox
/// is a handful of small files.
///
/// Fail open: the count is a side channel of whatever changed the corpus,
/// so a title that will not update is logged and never fails that change.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tray) = app.tray_by_id(ID) {
        fail_open("updating the tray count", tray.set_title(Some(title(app))));
    }
}

/// Bring the main window forward, creating focus even though the app has no
/// dock icon to click.
///
/// Fail open: a window the OS will not show or focus is logged; a menu click
/// has no caller to hand the failure to.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        fail_open("showing the main window", win.show());
        fail_open("focusing the main window", win.set_focus());
    }
}

/// The unsettled count, or `!` when the corpus cannot be read so the error is
/// visible from the menu bar too.
///
/// Fail open: the reason is logged, and the window's own commands report it
/// in full.
fn title<R: Runtime>(app: &AppHandle<R>) -> String {
    let state = app.state::<AppState>();
    let entries = state
        .corpus()
        .and_then(|c| session::inbox(&c).map_err(DesktopError::from));
    match fail_open("reading the inbox for the tray count", entries) {
        Some(entries) => entries.len().to_string(),
        None => "!".to_string(),
    }
}
