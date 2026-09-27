//! Unit tests for `store`, one file per source file (STD-02 §R19). This file
//! holds the tests for `store/mod.rs` and the fixture the others share.

mod commit;
mod inbox;
mod root;

use super::{Corpus, collect_directory_entries};
use crate::error::Error;

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus =
        Corpus::init(&crate::Locations::default(), &dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

#[test]
fn a_failing_directory_entry_is_reported_by_every_listing() {
    let (_dir, corpus) = corpus();
    // load_all, inbox_files and migration's node listing share this helper.
    // An injected iterator failure is deterministic across filesystems.
    for (consumer, dir) in [
        ("load_all", corpus.root().join("nodes")),
        ("inbox_files", corpus.root().join("inbox")),
        ("migrate", corpus.root().join("nodes")),
    ] {
        let entries = std::iter::once(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected directory entry failure",
        )));
        let listing = collect_directory_entries(&dir, entries);
        assert!(listing.entries.is_empty());
        assert_eq!(listing.errors.len(), 1);
        let error = listing.into_strict().unwrap_err();
        assert!(
            matches!(error, Error::IoAt { path, .. } if path == dir),
            "{consumer}"
        );
    }
}
