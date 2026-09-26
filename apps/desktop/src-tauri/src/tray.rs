//! The menu-bar item: the unsettled inbox count and app actions.

use crate::state::AppState;
use crate::{session, shortcut};
use tauri::menu::{IsMenuItem, Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

/// The tray's id, for finding it again from the watcher.
pub const ID: &str = "nebula";

/// Build the tray. Called once, from setup.
pub fn build(app: &AppHandle, warnings: &[String]) -> tauri::Result<()> {
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
    let mut menu_items: Vec<&dyn IsMenuItem<Wry>> = vec![&capture, &open, &settings];
    menu_items.extend(
        warning_items
            .iter()
            .map(|item| item as &dyn IsMenuItem<Wry>),
    );
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
                let _ = app.emit("show-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Recompute the count. Cheap enough to do on every corpus change: the inbox
/// is a handful of small files.
pub fn refresh(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(ID) {
        let _ = tray.set_title(Some(title(app)));
    }
}

/// Bring the main window forward, creating focus even though the app has no
/// dock icon to click.
pub fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// The unsettled count, or `!` when the corpus cannot be read so the error is
/// visible from the menu bar too.
fn title(app: &AppHandle) -> String {
    let state = app.state::<AppState>();
    match state
        .corpus()
        .and_then(|c| session::inbox(&c).map_err(|e| e.to_string()))
    {
        Ok(entries) => entries.len().to_string(),
        Err(_) => "!".to_string(),
    }
}
