//! `store`, through its crate-internal seams.
//!
//! A capture's stamp picks its month file and seeds its id, so these tests
//! capture through [`Corpus::capture_at`] at fixed stamps: every run takes
//! the same ids, and no run depends on where the wall clock is.

use std::collections::HashSet;

use crate::fs::{create_private_dir_all, write_private_atomic};
use crate::ops::{self, Promotion};
use crate::store::Corpus;

/// A capture instant one second before a minute, an hour and a local day
/// turn over: where a test that read the wall clock was most likely to race.
const STAMP: &str = "2026-09-26T23:59:59+02:00";

/// How many ids one stamp and text hash to before an id is looked for
/// elsewhere. The chain can repeat itself, so it holds at most this many
/// distinct ids.
const HASH_CANDIDATES: usize = 64;

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = Corpus::init(&dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

#[test]
fn capture_at_uses_a_free_id_after_the_hash_candidates_collide() {
    const TEXT: &str = "the intended new thought";
    let (_dir, corpus) = corpus();

    // Every capture of one text at one stamp walks the same hash chain and
    // takes the first id on it still free. Once this many hold the chain,
    // whatever it holds, the next capture has no hash candidate left.
    let occupied: Vec<String> = (0..HASH_CANDIDATES)
        .map(|_| corpus.capture_at(TEXT, STAMP).unwrap().id)
        .collect();
    let distinct: HashSet<&String> = occupied.iter().collect();
    assert_eq!(distinct.len(), HASH_CANDIDATES, "{occupied:?}");

    let captured = corpus.capture_at(TEXT, STAMP).unwrap();

    assert!(
        !occupied.contains(&captured.id),
        "{} is already taken",
        captured.id
    );
    assert_eq!(captured.at, STAMP);
    let resolved = corpus.inbox_entry(&captured.id).unwrap();
    assert_eq!(
        resolved.line, HASH_CANDIDATES,
        "the id resolves to the new line"
    );
    assert_eq!(resolved.text, TEXT);

    let promoted = ops::promote(
        &corpus,
        &captured.id,
        &Promotion {
            title: Some("Intended new thought".into()),
            ..Promotion::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(promoted.doc.body.trim(), TEXT);
    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), HASH_CANDIDATES);
    assert!(
        inbox.0.iter().all(|entry| entry.id != captured.id),
        "only the new capture was settled"
    );
}

#[test]
fn capture_at_ids_are_unique_across_month_files_and_older_entries_still_resolve() {
    const TEXT: &str = "the same thought";
    const OLDER: &str = "2000-01-15T09:30:00+00:00";

    // The id a capture of TEXT at STAMP takes in an empty inbox.
    let (_scratch, scratch) = corpus();
    let taken = scratch.capture_at(TEXT, STAMP).unwrap().id;

    // An older month's file already holds that id.
    let (dir, corpus) = corpus();
    let inbox = dir.path().join("corpus").join("inbox");
    create_private_dir_all(&inbox).unwrap();
    let older_file = inbox.join("2000-01.md");
    write_private_atomic(&older_file, format!("- [{taken}] {OLDER} {TEXT}\n")).unwrap();

    let second = corpus.capture_at(TEXT, STAMP).unwrap();

    assert_ne!(second.id, taken, "ids are unique across month files");
    assert_eq!(second.file, inbox.join("2026-09.md"));
    let older = corpus.inbox_entry(&taken).unwrap();
    assert_eq!(older.file, older_file);
    assert_eq!(older.at, OLDER);
    assert_eq!(older.text, TEXT);
    let newer = corpus.inbox_entry(&second.id).unwrap();
    assert_eq!(newer.file, second.file);
    assert_eq!(newer.at, STAMP);
}
