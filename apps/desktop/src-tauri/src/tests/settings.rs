//! Unit tests for `settings`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures are planted directly, beside the settings code under test"
)]

use crate::fail_open;
use fs4::{FileExt, TryLockError};
use nebula_core::fs::{create_private_dir_all, private_open_options, write_private_atomic};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::settings::*;

#[test]
fn first_load_writes_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let (s, warn) = load(dir.path());
    assert_eq!(s, Settings::default());
    assert_eq!(warn, None);
    let on_disk = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(on_disk.contains("\"capture_shortcut\": \"Alt+Space\""));
}

/// A first run creates the config directory itself, so it is the
/// owner's alone, and the defaults land through the durable helper: no
/// temporary file is left beside them.
#[cfg(unix)]
#[test]
fn first_load_writes_owner_only_defaults_atomically() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("app-config");

    let (s, warn) = load(&dir);

    assert_eq!((s, warn), (Settings::default(), None));
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join(FILE_NAME)), 0o600);
    let debris: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(debris.is_empty(), "temporary files left behind: {debris:?}");
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
fn saving_shortcut_persists_across_loads() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings {
        capture_shortcut: "CmdOrCtrl+Shift+N".into(),
    };
    save(dir.path(), &settings).unwrap();
    assert_eq!(load(dir.path()), (settings, None));
}

#[cfg(unix)]
#[test]
fn settings_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("config");
    save(&dir, &Settings::default()).unwrap();
    for path in [&dir, &dir.join(FILE_NAME), &dir.join("settings.lock")] {
        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & 0o077, 0, "{}", path.display());
    }
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
