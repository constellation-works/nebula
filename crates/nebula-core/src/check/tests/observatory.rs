//! Unit tests for `check/observatory.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant an Observatory tree directly"
)]

use crate::check_impl::{is_observatory_id, resolve_observatory};

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
    std::fs::create_dir_all(root.join("research").join("R012-arc")).unwrap();
    std::fs::write(root.join("questions").join("Q002-a-question.md"), "").unwrap();
    std::fs::write(root.join("questions").join("Q0021-not-it.md"), "").unwrap();
    std::fs::write(root.join("questions").join("README.md"), "").unwrap();

    assert_eq!(
        resolve_observatory(root, "Q002").unwrap(),
        Some(root.join("questions").join("Q002-a-question.md"))
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
