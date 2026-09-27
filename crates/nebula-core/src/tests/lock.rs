//! Unit tests for `lock`.

use crate::error::{Error, Result};
use crate::fs_impl::{Links, open_regular, overwrite_in_place, private_open_options, read_capped};
use fs4::{FileExt, TryLockError};
use std::collections::HashMap;
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::lock::*;

fn root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn the_lock_file_is_created_under_the_root_and_outlives_the_guard() {
    let dir = root();
    let lock = CorpusLock::acquire(dir.path()).expect("first acquire");
    assert!(dir.path().join(LOCK_FILE).exists());
    drop(lock);
    assert!(
        dir.path().join(LOCK_FILE).exists(),
        "the file stays; deleting it would race a process holding the old inode"
    );
    drop(CorpusLock::acquire(dir.path()).expect("the next writer gets in"));
}

#[cfg(unix)]
#[test]
fn the_lock_file_is_created_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = root();
    drop(CorpusLock::acquire(dir.path()).expect("acquire"));
    let mode = std::fs::metadata(dir.path().join(LOCK_FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_lock_file_that_cannot_be_opened_is_named() {
    let dir = root();
    let missing = dir.path().join("missing");
    let error = CorpusLock::acquire(&missing).expect_err("no directory to hold the lock");
    assert!(
        matches!(
            &error,
            Error::IoAt { action: "opening", path, .. } if *path == missing.join(LOCK_FILE)
        ),
        "expected the lock path in {error:?}"
    );
}

/// The CLI holds the lock across a verb and the commit that records it,
/// and the op it calls takes the same lock again. Without re-entry that
/// second take would wait on the first for five seconds and then refuse.
#[test]
fn one_thread_may_take_the_same_lock_again() {
    let dir = root();
    let outer = CorpusLock::acquire(dir.path()).expect("outer");
    let inner = CorpusLock::acquire_within(dir.path(), Duration::ZERO).expect("re-entry");
    assert!(outer.is_outermost() && !inner.is_outermost());
    drop(inner);

    // The outer guard still holds it: only the last drop releases.
    let gate = gate_for(dir.path());
    assert_eq!(held(&gate).as_ref().map(|h| h.depth), Some(1));
    drop(outer);
    assert!(held(&gate).is_none());
}

#[test]
fn held_by_this_thread_reports_only_the_holding_thread() {
    let dir = root();
    assert!(!held_by_this_thread(dir.path()), "nobody holds it yet");

    let outer = CorpusLock::acquire(dir.path()).expect("outer");
    let inner = CorpusLock::acquire(dir.path()).expect("re-entry");
    assert!(held_by_this_thread(dir.path()));
    let path = dir.path().to_path_buf();
    let elsewhere = std::thread::spawn(move || held_by_this_thread(&path))
        .join()
        .expect("the other thread did not panic");
    assert!(!elsewhere, "another thread does not hold it");

    // Still held until the last guard goes.
    drop(inner);
    assert!(held_by_this_thread(dir.path()));
    drop(outer);
    assert!(!held_by_this_thread(dir.path()));

    let other = root();
    let _held = CorpusLock::acquire(dir.path()).expect("again");
    assert!(
        !held_by_this_thread(other.path()),
        "holding one root is not holding another"
    );
}

/// Hold `root`'s lock as `label` on another thread, run `waiter` on
/// this one while it is held, then let the holder go.
fn while_held_elsewhere<T>(root: &Path, label: &str, waiter: impl FnOnce() -> T) -> T {
    let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(0);
    std::thread::scope(|scope| {
        let holder = scope.spawn(move || {
            let lock = CorpusLock::acquire_as(root, LOCK_WAIT, label).expect("the holder");
            held_tx.send(()).expect("the waiter is listening");
            // Until the waiter is done, or gone.
            let _ = done_rx.recv();
            drop(lock);
        });
        held_rx.recv().expect("the holder took the lock");
        let out = waiter();
        done_tx.send(()).expect("the holder is waiting");
        holder.join().expect("the holder did not panic");
        out
    })
}

/// Replace the lock file's contents by hand, the way a torn write, a
/// crash or a stranger would leave them.
fn scribble(root: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(root.join(LOCK_FILE))
        .and_then(|mut file| file.write_all(bytes))
        .expect("scribbling on the lock file");
}

#[test]
fn a_timeout_names_the_holder() {
    let dir = root();
    let refused = while_held_elsewhere(dir.path(), "test holder", || {
        CorpusLock::acquire_within(dir.path(), Duration::from_millis(50)).map(|_| ())
    });
    let Err(Error::Locked {
        root,
        holder: Some(holder),
    }) = &refused
    else {
        panic!("expected a named holder, got {refused:?}");
    };
    assert_eq!(root, dir.path());
    assert_eq!(holder.pid, std::process::id());
    assert_eq!(holder.label, "test holder");
    assert!(holder.since <= SystemTime::now(), "{holder:?}");
    assert!(
        SystemTime::now()
            .duration_since(holder.since)
            .is_ok_and(|age| age < Duration::from_secs(60)),
        "taken just now: {holder:?}"
    );
    assert!(
        refused
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("`test holder`, pid "),
        "the message names it: {refused:?}"
    );
}

/// The record only explains a wait. Whatever the file holds, a lock
/// somebody has is refused as held — never taken as free because its
/// record could not be read.
#[test]
fn an_unreadable_holder_record_is_still_contention() {
    let oversized = format!(
        r#"{{"pid":1,"since":"2026-09-26T00:00:00Z","label":"{}"}}"#,
        "x".repeat(RECORD_CAP)
    );
    let cases: [(&str, &[u8]); 6] = [
        ("garbage", b"\xff\x00 not a record at all"),
        ("empty", b""),
        ("truncated", br#"{"pid":1,"since":"2026-09-"#),
        (
            "a bad time",
            br#"{"pid":1,"since":"yesterday","label":"neb"}"#,
        ),
        (
            "a control character",
            b"{\"pid\":1,\"since\":\"2026-09-26T00:00:00Z\",\"label\":\"\\u001b[2J\"}",
        ),
        ("oversized", oversized.as_bytes()),
    ];
    for (case, bytes) in cases {
        let dir = root();
        let refused = while_held_elsewhere(dir.path(), "test holder", || {
            scribble(dir.path(), bytes);
            CorpusLock::acquire_within(dir.path(), Duration::from_millis(50)).map(|_| ())
        });
        assert!(
            matches!(&refused, Err(Error::Locked { holder: None, .. })),
            "{case}: got {refused:?}"
        );
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .contains("(an unidentified writer)"),
            "{case}"
        );
    }
}

#[test]
fn the_record_is_cleared_on_release_and_not_rewritten_on_reentry() {
    let dir = root();
    let path = dir.path().join(LOCK_FILE);
    let outer = CorpusLock::acquire_as(dir.path(), Duration::ZERO, "outer").expect("outer");
    let recorded = read_holder(&path).expect("the holder recorded itself");
    assert_eq!(recorded.label, "outer");
    assert_eq!(recorded.pid, std::process::id());

    let inner = CorpusLock::acquire_as(dir.path(), Duration::ZERO, "inner").expect("re-entry");
    assert_eq!(read_holder(&path), Some(recorded.clone()), "re-entry wrote");
    drop(inner);
    assert_eq!(
        read_holder(&path),
        Some(recorded),
        "an inner release cleared the outer holder's record"
    );

    drop(outer);
    assert_eq!(std::fs::metadata(&path).expect("the file stays").len(), 0);
}

/// The record is written into the file the lock opened, so a symlink at
/// `.lock` would have it empty the link's target. It is refused instead,
/// and the target is left as it was.
#[cfg(unix)]
#[test]
fn a_symlinked_lock_file_is_refused_and_its_target_untouched() {
    let dir = root();
    let target = dir.path().join("precious");
    scribble_new(&target, b"keep me");
    std::os::unix::fs::symlink(&target, dir.path().join(LOCK_FILE)).expect("symlink");

    let error = CorpusLock::acquire(dir.path()).expect_err("a symlinked lock file");
    assert!(
        matches!(
            &error,
            Error::NotRegularFile {
                path,
                found: crate::fs_impl::EntryKind::Symlink,
            } if *path == dir.path().join(LOCK_FILE)
        ),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&target).expect("target"), b"keep me");
}

#[cfg(unix)]
fn scribble_new(path: &Path, bytes: &[u8]) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .expect("writing a fixture");
}

#[test]
fn a_label_is_recorded_without_control_characters_and_within_bounds() {
    assert_eq!(clean_label("neb edit a-node"), "neb edit a-node");
    assert_eq!(clean_label("neb edit \u{1b}[2J"), "neb edit ?[2J");
    assert_eq!(clean_label("  "), DEFAULT_LABEL);
    assert_eq!(clean_label(&"x".repeat(1000)).chars().count(), LABEL_CAP);
}

#[test]
fn a_second_thread_waits_and_then_refuses_with_the_root_it_wanted() {
    let dir = root();
    let held_by_us = CorpusLock::acquire(dir.path()).expect("held");
    let path = dir.path().to_path_buf();

    let refused = std::thread::spawn(move || {
        CorpusLock::acquire_within(&path, Duration::from_millis(50)).map(|_| ())
    })
    .join()
    .expect("the waiter did not panic");

    assert!(
        matches!(&refused, Err(Error::Locked { root, .. }) if root == dir.path()),
        "got {refused:?}"
    );
    drop(held_by_us);

    // And once it is free, another thread gets in. The guard is not
    // `Send`, so it is taken and dropped over there rather than handed
    // back.
    let path = dir.path().to_path_buf();
    std::thread::spawn(move || {
        CorpusLock::acquire_within(&path, Duration::from_millis(50)).map(|_| ())
    })
    .join()
    .expect("no panic")
    .expect("the lock is free again");
}
