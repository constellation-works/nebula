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
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
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
    assert!(warn.contains("directory"), "{warn}");
    assert!(warn.contains(DEFAULT_CAPTURE_SHORTCUT));
}

#[cfg(unix)]
#[test]
fn final_symlink_settings_are_refused_without_touching_the_target() {
    use std::os::unix::fs::symlink;

    for target_exists in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("config");
        std::fs::create_dir(&dir).unwrap();
        let target = root.path().join("target");
        let content = r#"{ "capture_shortcut": "CmdOrCtrl+Shift+N" }"#;
        if target_exists {
            std::fs::write(&target, content).unwrap();
        }
        let path = dir.join(FILE_NAME);
        symlink(&target, &path).unwrap();

        let (settings, warning) = load(&dir);

        assert_eq!(settings, Settings::default());
        let warning = warning.unwrap();
        assert!(warning.contains(&path.display().to_string()), "{warning}");
        assert!(warning.contains("symlink"), "{warning}");
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(&target).ok().as_deref(),
            target_exists.then_some(content)
        );
        assert!(!dir.join("settings.lock").exists());
    }
}

#[cfg(unix)]
#[test]
fn socket_settings_are_refused_without_connecting_or_writing() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixListener;

    // Unix socket paths use a fixed-size sun_path (104 bytes on macOS).
    // Keep this fixture independent of TMPDIR, which hostile-env tests lengthen.
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join(FILE_NAME);
    assert!(
        path.as_os_str().as_bytes().len() < 104,
        "socket path exceeds macOS sun_path limit: {}",
        path.display()
    );
    let _listener = UnixListener::bind(&path).unwrap();

    let (settings, warning) = load(dir.path());

    assert_eq!(settings, Settings::default());
    let warning = warning.unwrap();
    assert!(warning.contains(&path.display().to_string()), "{warning}");
    assert!(warning.contains("socket"), "{warning}");
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_socket()
    );
    assert!(!dir.path().join("settings.lock").exists());
}

#[cfg(unix)]
#[ctor::ctor]
unsafe fn settings_fifo_load_child() {
    let Some(dir) = std::env::var_os("NEBULA_TEST_SETTINGS_FIFO_LOAD") else {
        return;
    };
    let dir = Path::new(&dir);
    write_private_atomic(&dir.join("ready"), b"entered load").unwrap();
    let (settings, warning) = load(dir);
    assert_eq!(settings, Settings::default());
    let warning = warning.expect("a FIFO must produce a startup warning");
    assert!(warning.contains(&dir.join(FILE_NAME).display().to_string()));
    assert!(warning.contains("FIFO"), "{warning}");
    assert!(!dir.join("settings.lock").exists());
    std::process::exit(0);
}

#[cfg(unix)]
#[test]
fn fifo_settings_are_refused_before_startup_can_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let output = support::output(
        support::command("mkfifo", support::home()).arg(&path),
        support::DEADLINE,
    )
    .unwrap();
    assert!(output.status.success());

    let mut child = support::ChildGuard::spawn(
        support::command(std::env::current_exe().unwrap(), support::home())
            .env("NEBULA_TEST_SETTINGS_FIFO_LOAD", dir.path()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("ready").exists() {
        assert!(
            child.try_wait().is_none(),
            "load child exited before readiness"
        );
        assert!(Instant::now() < deadline, "load child never became ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = child.wait(Duration::from_secs(2));
    assert!(
        status.as_ref().is_ok_and(std::process::ExitStatus::success),
        "{status:?}"
    );
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_fifo()
    );
}

#[path = "../../../../../crates/nebula-core/tests/support/mod.rs"]
mod support;

#[cfg(unix)]
fn aliased_lock_is_refused(hard_link: bool, first_launch: bool) {
    use std::os::unix::fs::{MetadataExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("config");
    std::fs::create_dir(&dir).unwrap();
    let sentinel = root.path().join("sentinel");
    std::fs::write(&sentinel, "KEEP ME").unwrap();
    let lock = dir.join("settings.lock");
    if hard_link {
        std::fs::hard_link(&sentinel, &lock).unwrap();
    } else {
        symlink(&sentinel, &lock).unwrap();
    }
    let inode = std::fs::symlink_metadata(&lock).unwrap().ino();
    let error = if first_launch {
        let (settings, warning) = load(&dir);
        assert_eq!(settings, Settings::default());
        warning
    } else {
        save(&dir, &Settings::default())
            .err()
            .map(|e| e.to_string())
    };
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "KEEP ME");
    let error = error.expect("an aliased settings lock must be refused");
    assert!(error.contains(&lock.display().to_string()), "{error}");
    assert!(!dir.join(FILE_NAME).exists());
    assert_eq!(std::fs::symlink_metadata(&lock).unwrap().ino(), inode);
}

#[cfg(unix)]
#[test]
fn save_refuses_symlinked_lock() {
    aliased_lock_is_refused(false, false);
}

#[cfg(unix)]
#[test]
fn first_launch_refuses_symlinked_lock() {
    aliased_lock_is_refused(false, true);
}

#[cfg(unix)]
#[test]
fn save_refuses_hardlinked_lock() {
    aliased_lock_is_refused(true, false);
}

#[cfg(unix)]
#[test]
fn first_launch_refuses_hardlinked_lock() {
    aliased_lock_is_refused(true, true);
}

#[cfg(unix)]
#[test]
fn non_regular_locks_are_refused_promptly() {
    for kind in ["directory", "fifo"] {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("settings.lock");
        match kind {
            "directory" => {
                std::fs::create_dir(&lock).unwrap();
            }
            "fifo" => {
                let output = support::output(
                    support::command("mkfifo", support::home()).arg(&lock),
                    support::DEADLINE,
                )
                .unwrap();
                assert!(output.status.success());
            }
            _ => unreachable!(),
        }
        let start = Instant::now();
        let error = save(dir.path(), &Settings::default()).unwrap_err();
        assert!(error.to_string().contains(&lock.display().to_string()));
        let (_, warning) = load(dir.path());
        assert!(warning.unwrap().contains(&lock.display().to_string()));
        assert!(start.elapsed() < Duration::from_secs(2), "{kind}");
        assert!(!dir.path().join(FILE_NAME).exists());
    }
}

/// Explicit test substitute, before libtest: hold a real settings lock until
/// the parent closes stdin. No production self-reexec path is involved.
#[ctor::ctor]
unsafe fn settings_lock_holder_child() {
    let Some(dir) = std::env::var_os("NEBULA_TEST_SETTINGS_LOCK") else {
        return;
    };
    let dir = Path::new(&dir);
    save(dir, &Settings::default()).unwrap();
    let file = private_open_options()
        .read(true)
        .write(true)
        .open(dir.join("settings.lock"))
        .unwrap();
    FileExt::try_lock(&file).unwrap();
    write_private_atomic(&dir.join("ready"), b"held").unwrap();
    // A bounded channel makes stdin closure observable with a deadline even
    // if a broken parent never closes it. Process exit closes the descriptor.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = std::io::stdin().read(&mut [0]);
        let _ = send.send(result);
    });
    let result = receive.recv_timeout(Duration::from_secs(30));
    drop(file);
    std::process::exit(i32::from(!matches!(result, Ok(Ok(0)))));
}

#[test]
fn settings_lock_excludes_another_process_and_recovers_after_release() {
    use std::process::Stdio;
    let dir = tempfile::tempdir().unwrap();
    let mut child = support::ChildGuard::spawn(
        support::command(std::env::current_exe().unwrap(), support::home())
            .env("NEBULA_TEST_SETTINGS_LOCK", dir.path())
            .stdin(Stdio::piped()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("ready").exists() {
        assert!(child.try_wait().is_none(), "holder exited before readiness");
        assert!(Instant::now() < deadline, "holder never became ready");
        std::thread::sleep(Duration::from_millis(10));
    }
    let lock = dir.path().join("settings.lock");
    let holder = std::fs::read(&lock).unwrap();
    let prior = std::fs::read(dir.path().join(FILE_NAME)).unwrap();
    let changed = Settings {
        capture_shortcut: "CmdOrCtrl+Shift+N".into(),
    };
    let error = save(dir.path(), &changed).unwrap_err();
    assert!(matches!(error, SettingsError::Busy { .. }), "{error}");
    assert!(
        error.to_string().contains(&child.id().to_string()),
        "{error}"
    );
    assert_eq!(std::fs::read(&lock).unwrap(), holder);
    assert_eq!(std::fs::read(dir.path().join(FILE_NAME)).unwrap(), prior);
    drop(child.take_stdin());
    assert!(child.wait(Duration::from_secs(10)).unwrap().success());
    save(dir.path(), &changed).unwrap();
    assert_eq!(load(dir.path()), (changed, None));
}

#[test]
fn settings_test_children_use_the_isolating_builder() {
    let strays = support::commands_outside(include_str!("settings.rs"), &[]);
    assert!(
        strays.is_empty(),
        "settings tests must contain every child: {strays:?}"
    );
}
