//! Unit tests for `shortcut`.

use crate::fail_open;
use crate::settings::{self, SettingsError};
use crate::state::AppState;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, PhysicalPosition, Runtime, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::shortcut::{ShortcutError, parse};

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
