//! Unit tests for `pending`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures read the corpus back directly, beside the code under test"
)]

use crate::error::{Error, Result};
use crate::fs_impl::{remove_private, write_private_atomic};
use crate::store::{Corpus, InboxEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::fs_impl::{Step, crashing_at, recording};
use crate::ops_impl::{self as ops, Promotion};
use crate::pending::*;

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus =
        Corpus::init(&crate::Locations::default(), &dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

const TEXT: &str = "an interrupted promotion idea";
const NODE: &str = "an-interrupted-promotion-idea";

/// The steps that change what a name holds, in order.
fn renames_and_removals(steps: &[Step]) -> Vec<Step> {
    steps
        .iter()
        .filter(|step| matches!(step, Step::Rename { .. } | Step::Remove(_)))
        .map(|step| match step {
            Step::Rename { to, .. } => Step::Rename {
                from: PathBuf::new(),
                to: to.clone(),
            },
            Step::Write { .. }
            | Step::SyncAll(_)
            | Step::SyncData(_)
            | Step::Remove(_)
            | Step::SyncDir(_) => step.clone(),
        })
        .collect()
}

fn renamed_onto(step: &Step, path: &Path) -> bool {
    matches!(step, Step::Rename { to, .. } if to == path)
}

fn line(entry: &InboxEntry) -> String {
    std::fs::read_to_string(&entry.file)
        .unwrap()
        .lines()
        .nth(entry.line)
        .unwrap()
        .to_string()
}

/// The record lands before the first file the promotion changes and goes
/// after the last, which is what makes every crash between them one the
/// next writer can settle.
#[test]
fn a_promotion_records_before_its_first_write_and_clears_after_its_last() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let (created, steps) = recording(|| ops::promote(&corpus, &entry.id, &Promotion::default(), 0));
    let created = created.unwrap();

    let record = pending_path(corpus.root());
    let rename = |to: &Path| Step::Rename {
        from: PathBuf::new(),
        to: to.to_path_buf(),
    };
    assert_eq!(
        renames_and_removals(&steps),
        [
            rename(&record),
            rename(&created.path),
            rename(&entry.file),
            Step::Remove(record.clone()),
        ]
    );
    assert!(!record.exists());
}

/// Killed between the node and the strike: the entry is not offered as
/// waiting, and the next writer strikes it rather than leaving it to be
/// dropped or promoted twice.
#[test]
fn a_crash_after_the_node_is_finished_by_the_next_writer() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let node = corpus.node_path(NODE).unwrap();
    let crashed = crashing_at(
        {
            let node = node.clone();
            move |step| renamed_onto(step, &node)
        },
        || ops::promote(&corpus, &entry.id, &Promotion::default(), 0),
    );
    assert!(crashed.is_err(), "the crash point was reached");
    assert!(node.exists() && pending_path(corpus.root()).exists());
    assert!(line(&entry).starts_with("- ["), "the strike never ran");
    assert!(corpus.inbox().unwrap().0.is_empty(), "{:?}", corpus.inbox());

    ops::capture(&corpus, "the next thought").unwrap();

    assert!(
        line(&entry).ends_with(&format!("~~ -> {NODE}")),
        "{}",
        line(&entry)
    );
    assert!(!pending_path(corpus.root()).exists());
    assert_eq!(corpus.load_all().unwrap().len(), 1);
}

/// Killed after the record and before the node: nothing was decided, so
/// the entry is still waiting and the record goes.
#[test]
fn a_crash_before_the_node_leaves_the_entry_waiting() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let record = pending_path(corpus.root());
    let crashed = crashing_at(
        {
            let record = record.clone();
            move |step| renamed_onto(step, &record)
        },
        || ops::promote(&corpus, &entry.id, &Promotion::default(), 0),
    );
    assert!(crashed.is_err());
    assert!(record.exists());
    assert_eq!(corpus.inbox().unwrap().0.len(), 1, "still waiting");

    let _lock = corpus.lock().unwrap();
    assert!(!record.exists(), "the next writer discards it");
    assert!(
        !corpus.node_path(NODE).unwrap().exists(),
        "no node by guesswork"
    );
    assert!(line(&entry).starts_with("- ["));
}

/// Killed after the strike and before the record went: only the record
/// is left to remove, and the line is not struck twice.
#[test]
fn a_crash_after_the_strike_only_removes_the_record() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let month = entry.file.clone();
    let crashed = crashing_at(
        move |step| renamed_onto(step, &month),
        || ops::promote(&corpus, &entry.id, &Promotion::default(), 0),
    );
    assert!(crashed.is_err());
    let struck = line(&entry);

    let _lock = corpus.lock().unwrap();
    assert!(!pending_path(corpus.root()).exists());
    assert_eq!(line(&entry), struck);
    assert!(struck.ends_with(&format!("~~ -> {NODE}")), "{struck}");
}

/// Settling happens once per critical section, on the way in. A verb
/// already inside it re-enters the lock without settling its own record
/// out from under itself.
#[test]
fn only_the_outermost_lock_settles_a_record() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let outer = corpus.lock().unwrap();
    let pending = Pending::begin(
        &corpus,
        &PendingWrite::Promote {
            entry: entry.id.clone(),
            stamp: entry.stamp.clone(),
            node: NODE.into(),
        },
    )
    .unwrap();
    let inner = corpus.lock().unwrap();
    assert!(
        pending_path(corpus.root()).exists(),
        "re-entry settled nothing"
    );
    drop(inner);
    pending.finished().unwrap();
    drop(outer);
    assert!(!pending_path(corpus.root()).exists());
}

/// An error return, unlike a crash, settles the record at once from the
/// files, so `check` does not report a promotion that failed outright as
/// an interrupted one.
#[test]
fn a_record_dropped_unfinished_is_settled_from_the_files() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, TEXT).unwrap();
    let _lock = corpus.lock().unwrap();
    drop(
        Pending::begin(
            &corpus,
            &PendingWrite::Promote {
                entry: entry.id.clone(),
                stamp: entry.stamp.clone(),
                node: NODE.into(),
            },
        )
        .unwrap(),
    );
    assert!(!pending_path(corpus.root()).exists());
    assert!(
        line(&entry).starts_with("- ["),
        "no node, so nothing struck"
    );
}

#[test]
fn the_record_round_trips_in_its_documented_shape() {
    let write = PendingWrite::Promote {
        entry: "1f2e".into(),
        stamp: "2026-09-26T08:11:05+02:00".into(),
        node: "an-idea".into(),
    };
    let json = serde_json::to_value(&write).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "op": "promote",
            "entry": "1f2e",
            "stamp": "2026-09-26T08:11:05+02:00",
            "node": "an-idea",
        })
    );
    assert_eq!(serde_json::from_value::<PendingWrite>(json).unwrap(), write);
}
