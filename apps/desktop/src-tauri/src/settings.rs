//! The capture shortcut persisted in the app config directory. Launch at
//! login is persisted by the operating system through the autostart plugin.
//!
//! Lives in `settings.json` under the app's config directory
//! (`~/Library/Application Support/works.constellation.nebula` on macOS).
//! Written with its defaults on first launch.

use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    let written = with_lock(dir, || {
        if path.exists() {
            return Ok(false);
        }
        write_file(dir, &s)?;
        Ok(true)
    });
    if matches!(written, Ok(false)) {
        return load(dir);
    }
    let warn = written
        .err()
        .map(|e| format!("could not write {}: {e}", path.display()));
    (s, warn)
}

/// Replace the settings file atomically so a crash cannot leave partial JSON.
pub fn save(dir: &Path, settings: &Settings) -> Result<(), String> {
    with_lock(dir, || write_file(dir, settings))
}

fn write_file(dir: &Path, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    let mut json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    json.push(b'\n');
    temporary.write_all(&json).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist(dir.join(FILE_NAME))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// A process-level lock keeps separate app instances from writing settings at
/// once. Its persistent holder text makes a bounded wait actionable.
fn with_lock<T>(dir: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let lock_path = dir.join("settings.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&lock_path).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match FileExt::try_lock(&file) {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(TryLockError::WouldBlock) => {
                let mut holder = String::new();
                let _ = file
                    .seek(SeekFrom::Start(0))
                    .and_then(|_| file.read_to_string(&mut holder));
                return Err(format!(
                    "settings busy ({}; holder: {})",
                    lock_path.display(),
                    holder.trim()
                ));
            }
            Err(TryLockError::Error(e)) => {
                return Err(format!("could not lock {}: {e}", lock_path.display()));
            }
        }
    }
    file.set_len(0).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs());
    write!(
        file,
        "pid={} at={} label=nebula-settings",
        std::process::id(),
        since_epoch
    )
    .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    // Closing the handle releases the advisory lock on every return path.
    f()
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
}
