//! The one setting there is: which global shortcut opens capture.
//!
//! Lives in `settings.json` under the app's config directory
//! (`~/Library/Application Support/works.constellation.nebula` on macOS).
//! Written with its defaults on first launch so there is a file to edit.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The shortcut registered when the settings file names none.
pub const DEFAULT_CAPTURE_SHORTCUT: &str = "Alt+Space";

/// Filename under the app config directory.
pub const FILE_NAME: &str = "settings.json";

/// Everything the user can change without a rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The global shortcut that toggles the capture window, in the
    /// `tauri-plugin-global-shortcut` syntax, e.g. `Alt+Space` or
    /// `CmdOrCtrl+Shift+N`.
    pub capture_shortcut: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_shortcut: DEFAULT_CAPTURE_SHORTCUT.to_string(),
        }
    }
}

/// Read `settings.json` from `dir`, writing the defaults there when it is
/// missing. A file that will not parse falls back to the defaults rather than
/// taking the shortcut away; the error is returned alongside so the caller
/// can show it in the tray and main window.
pub fn load(dir: &Path) -> (Settings, Option<String>) {
    let path = dir.join(FILE_NAME);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return write_defaults(dir, &path);
        }
        Err(e) => {
            return (
                Settings::default(),
                Some(format!(
                    "could not read {}: {e}; using default capture shortcut `{}`",
                    path.display(),
                    DEFAULT_CAPTURE_SHORTCUT
                )),
            );
        }
    };
    match serde_json::from_str(&raw) {
        Ok(s) => (s, None),
        Err(e) => (
            Settings::default(),
            Some(format!(
                "invalid {}: {e}; using default capture shortcut `{}`",
                path.display(),
                DEFAULT_CAPTURE_SHORTCUT
            )),
        ),
    }
}

fn write_defaults(dir: &Path, path: &Path) -> (Settings, Option<String>) {
    let s = Settings::default();
    let written = std::fs::create_dir_all(dir).and_then(|()| {
        let json = serde_json::to_string_pretty(&s).unwrap_or_default();
        std::fs::write(path, json + "\n")
    });
    let warn = written
        .err()
        .map(|e| format!("could not write {}: {e}", path.display()));
    (s, warn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_load_writes_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (s, warn) = load(dir.path());
        assert_eq!(s, Settings::default());
        assert_eq!(warn, None);
        let on_disk = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(on_disk.contains("\"capture_shortcut\": \"Alt+Space\""));
    }

    #[test]
    fn an_edited_file_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE_NAME),
            r#"{ "capture_shortcut": "CmdOrCtrl+Shift+N" }"#,
        )
        .unwrap();
        let (s, warn) = load(dir.path());
        assert_eq!(s.capture_shortcut, "CmdOrCtrl+Shift+N");
        assert_eq!(warn, None);
    }

    #[test]
    fn a_broken_file_keeps_the_default_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        let (s, warn) = load(dir.path());
        assert_eq!(s.capture_shortcut, DEFAULT_CAPTURE_SHORTCUT);
        let warn = warn.unwrap();
        assert!(warn.contains(&dir.path().join(FILE_NAME).display().to_string()));
        assert!(warn.contains(DEFAULT_CAPTURE_SHORTCUT));
    }

    #[test]
    fn an_unreadable_settings_file_keeps_the_default_shortcut_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let settings_file = dir.path().join(FILE_NAME);
        std::fs::create_dir(&settings_file).unwrap();

        let (s, warn) = load(dir.path());

        assert_eq!(s.capture_shortcut, DEFAULT_CAPTURE_SHORTCUT);
        let warn = warn.unwrap();
        assert!(warn.contains(&settings_file.display().to_string()));
        assert!(warn.contains(DEFAULT_CAPTURE_SHORTCUT));
    }
}
