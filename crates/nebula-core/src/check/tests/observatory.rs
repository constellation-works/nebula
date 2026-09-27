//! Unit tests for `check/observatory.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant an Observatory tree directly"
)]

use crate::check_impl::{is_observatory_id, resolve_observatory};
use crate::error::Error;

#[test]
fn an_observatory_id_is_one_letter_of_four_then_digits() {
    for id in ["Q002", "H7", "T003", "R012"] {
        assert!(is_observatory_id(id), "{id}");
    }
    for id in ["", "Q", "q002", "X002", "Q002-slug", "Q 2", "/abs/Q002.md"] {
        assert!(!is_observatory_id(id), "{id}");
    }
}

/// The record is found by its id alone, whatever slug follows it, and a
/// longer id that merely starts with the same digits is not a match.
#[test]
fn a_record_resolves_by_id_prefix_in_its_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("questions")).unwrap();
    std::fs::create_dir_all(root.join("hypotheses")).unwrap();
    std::fs::create_dir_all(root.join("theories")).unwrap();
    std::fs::create_dir_all(root.join("research").join("R012-arc")).unwrap();
    std::fs::write(root.join("questions").join("Q002-a-question.md"), "").unwrap();
    std::fs::write(root.join("hypotheses").join("H012-an-idea.md"), "").unwrap();
    std::fs::write(root.join("theories").join("T003-a-theory.md"), "").unwrap();
    std::fs::write(root.join("questions").join("Q0021-not-it.md"), "").unwrap();
    std::fs::write(root.join("questions").join("README.md"), "").unwrap();

    assert_eq!(
        resolve_observatory(root, "Q002").unwrap(),
        Some(root.join("questions").join("Q002-a-question.md"))
    );
    assert_eq!(
        resolve_observatory(root, "H012").unwrap(),
        Some(root.join("hypotheses").join("H012-an-idea.md"))
    );
    assert_eq!(
        resolve_observatory(root, "T003").unwrap(),
        Some(root.join("theories").join("T003-a-theory.md"))
    );
    assert_eq!(
        resolve_observatory(root, "R012").unwrap(),
        Some(root.join("research").join("R012-arc"))
    );
    assert_eq!(resolve_observatory(root, "Q003").unwrap(), None);
    assert_eq!(
        resolve_observatory(root, "H001").unwrap(),
        None,
        "no hypotheses/ at all"
    );
    assert_eq!(resolve_observatory(root, "Q002-a-question").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn a_record_target_must_exist_and_remains_a_lexical_path() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("observatory");
    std::fs::create_dir_all(root.join("hypotheses")).unwrap();
    symlink("missing.md", root.join("hypotheses/H012-dangling.md")).unwrap();
    assert_eq!(resolve_observatory(&root, "H012").unwrap(), None);

    std::fs::write(root.join("hypotheses/H012-real.md"), "# H012\n").unwrap();
    let alias = dir.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert_eq!(
        resolve_observatory(&alias, "H012").unwrap(),
        Some(alias.join("hypotheses/H012-real.md"))
    );
}

#[cfg(unix)]
#[test]
fn an_inaccessible_matching_target_is_an_io_error() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("hypotheses")).unwrap();
    let loop_path = root.join("hypotheses/H012-loop.md");
    symlink("H012-loop.md", &loop_path).unwrap();
    assert!(
        matches!(resolve_observatory(root, "H012"), Err(Error::IoAt { path, .. })
            if path == loop_path)
    );
}
