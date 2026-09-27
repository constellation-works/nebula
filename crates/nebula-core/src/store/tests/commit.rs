//! Unit tests for `store/commit.rs`.

use crate::fs_impl::create_temporary_sibling;
use crate::lock::LOCK_FILE;
use crate::pending::PENDING_FILE;
use crate::store::commit::{NEVER_STAGED, ignore_rules};
use std::ffi::OsStr;

/// Crash debris and the pending record stay out of every commit, and
/// out of a person's `git add -A` once `init` has run.
#[test]
fn commits_and_ignore_rules_cover_debris_and_the_pending_record() {
    assert_eq!(
        NEVER_STAGED,
        [
            ":(exclude,glob)**/*.tmp".to_string(),
            format!(":(exclude){PENDING_FILE}")
        ]
    );
    // What the exclusion and the ignore rule both match on is the name
    // every temporary the write helper makes ends with.
    let dir = tempfile::tempdir().unwrap();
    let (tmp, _handle) = create_temporary_sibling(&dir.path().join("a.md")).unwrap();
    assert_eq!(
        tmp.extension(),
        Some(OsStr::new("tmp")),
        "{}",
        tmp.display()
    );
    assert_eq!(
        ignore_rules(),
        [
            format!("/{LOCK_FILE}"),
            format!("/{PENDING_FILE}"),
            "*.tmp".to_string()
        ]
    );
}
