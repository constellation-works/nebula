//! Unit tests for `fs`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures are written directly so the helper under test is the only thing exercised"
)]

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::fs_impl::*;

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
#[test]
fn a_new_file_and_its_new_directories_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("a").join("b");
    create_private_dir_all(&deep).unwrap();
    let file = deep.join("f");
    write_private_atomic(&file, "x").unwrap();
    assert_eq!(mode(&dir.path().join("a")), PRIVATE_DIR_MODE);
    assert_eq!(mode(&deep), PRIVATE_DIR_MODE);
    assert_eq!(mode(&file), PRIVATE_FILE_MODE);
}

#[cfg(unix)]
#[test]
fn an_existing_directory_keeps_its_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("mine");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o750)).unwrap();
    create_private_dir_all(&existing.join("new")).unwrap();
    assert_eq!(
        mode(&existing),
        0o750,
        "an existing directory is not chmod'ed"
    );
    assert_eq!(mode(&existing.join("new")), PRIVATE_DIR_MODE);
}

#[cfg(unix)]
#[test]
fn a_rewrite_narrows_a_wide_file_and_never_widens_a_narrow_one() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    std::fs::write(&file, "old").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_private_atomic(&file, "new").unwrap();
    assert_eq!(mode(&file), PRIVATE_FILE_MODE);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_target_is_replaced_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let target = dir.path().join("setting");
    std::os::unix::fs::symlink(&outside, &target).unwrap();

    write_private_atomic(&target, "new").unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE"
    );
    assert!(
        std::fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_file()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
}

#[test]
fn a_new_file_is_flushed_owner_only_and_never_replaces_a_taken_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("kept.md");
    let (result, steps) = recording(|| create_private_new(&file, "the text"));
    result.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "the text");
    assert!(steps.contains(&Step::SyncAll(file.clone())), "{steps:?}");
    #[cfg(unix)]
    assert_eq!(mode(&file), PRIVATE_FILE_MODE);

    let taken = create_private_new(&file, "another text").expect_err("the name is taken");
    assert!(
        matches!(&taken, Error::IoAt { source, .. }
            if source.kind() == std::io::ErrorKind::AlreadyExists),
        "{taken:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "the text",
        "the first file is untouched"
    );
}

#[test]
fn an_append_creates_the_file_and_flushes_its_directory_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("log");
    let (result, first) = recording(|| append_private(&file, b"one\n"));
    result.unwrap();
    let (result, second) = recording(|| append_private(&file, b"two\n"));
    result.unwrap();

    assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
    let dir_syncs = |steps: &[Step]| {
        steps
            .iter()
            .filter(|step| matches!(step, Step::SyncDir(_)))
            .count()
    };
    assert_eq!(dir_syncs(&first), usize::from(cfg!(unix)), "{first:?}");
    assert_eq!(dir_syncs(&second), 0, "{second:?}");
    #[cfg(unix)]
    assert_eq!(mode(&file), PRIVATE_FILE_MODE);
}

#[test]
fn a_removal_flushes_its_directory_and_a_missing_file_is_already_removed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("record");
    std::fs::write(&file, "x").unwrap();
    let (result, steps) = recording(|| remove_private(&file));
    result.unwrap();
    assert!(!file.exists());
    let mut expected = vec![Step::Remove(file.clone())];
    if cfg!(unix) {
        expected.push(Step::SyncDir(dir.path().to_path_buf()));
    }
    assert_eq!(steps, expected);
    remove_private(&file).unwrap();
}

/// The failpoint stops at the step it names, after that step and before
/// the next, and never mistakes a real panic for its own stop.
#[test]
fn a_crash_stops_right_after_the_step_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("first"), dir.path().join("second"));
    let stop = first.clone();
    let crashed = crashing_at(
        move |step| matches!(step, Step::Rename { to, .. } if *to == stop),
        || {
            write_private_atomic(&first, "1").unwrap();
            write_private_atomic(&second, "2").unwrap();
        },
    );
    assert!(crashed.is_err());
    assert!(first.exists() && !second.exists());

    assert_eq!(crashing_at(|_| false, || 7), Ok(7));
    let real = std::panic::catch_unwind(|| crashing_at(|_| false, || panic!("a real bug")));
    assert!(real.is_err(), "a real panic carries on up");
}

#[test]
fn recording_is_off_unless_a_test_asks_for_it() {
    let dir = tempfile::tempdir().unwrap();
    write_private_atomic(&dir.path().join("f"), "x").unwrap();
    let ((), steps) = recording(|| ());
    assert!(steps.is_empty(), "{steps:?}");
}
