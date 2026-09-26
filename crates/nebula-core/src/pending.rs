//! The pending-write record: how a write that spans two files survives a
//! crash between them.
//!
//! Every file is replaced atomically, but `promote` changes two: it writes
//! the node, and then it strikes the inbox line the node came from. A crash
//! between the two renames used to leave a live line whose node exists,
//! which nothing could tell apart from a second capture of the same text: a
//! re-run refused with "already exists", `check` was silent, and the only way
//! out, `drop`, recorded the entry wrongly as dropped.
//!
//! So before its first write `promote` records what it is about to do in
//! `<root>/.pending`, through the durable helper, and removes the record after
//! the strike. The record is one fact, not a log: writers are serialized by
//! the corpus lock, and every one of them settles a record it finds before it
//! does anything else, so there is never more than one.
//!
//! **Recovery** (STD-03 §R9) runs when a writer takes the corpus lock, before
//! its own op, and decides from the record and the files, never from age:
//!
//! - The node exists: it was written after the record, under the same lock
//!   that checked it did not exist yet, so the promotion is decided. The live
//!   line is struck `-> <node>` (if a hand edit has already settled or removed
//!   it, there is nothing left to strike), and the record goes.
//! - The node does not exist: nothing was decided. The record goes, and the
//!   entry stays waiting. No node is written by guesswork.
//! - The record cannot be read, or names an operation this build does not
//!   know: every writer refuses with [`Error::PendingWriteUnreadable`], and
//!   so do the reads that would present its entry, until a person has looked
//!   at it. Finishing a write it cannot read would be a guess.
//!
//! Lock-free reads never wait for recovery, so they apply its decision
//! instead: an entry whose recorded promotion has written its node is not
//! offered as waiting ([`Corpus::inbox`]), and `check` names the record.
//!
//! The record lives beside `.lock`, outside `nodes/` and `inbox/`, so nothing
//! that reads the corpus reads it as content. `init` git-ignores it and a
//! `neb` commit never stages it: it is a fact about a write in flight on one
//! machine, not about the corpus.

use crate::error::{Error, Result};
use crate::fs::{remove_private, write_private_atomic};
use crate::store::{Corpus, InboxEntry};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The record's name, directly under the corpus root.
pub(crate) const PENDING_FILE: &str = ".pending";

/// A write that spans files and has begun, as `<root>/.pending` records it.
///
/// A persisted shape (STD-02 §R16): `{"op": "promote", ...}`, one JSON
/// object. Fields a later build adds are ignored by this one rather than
/// refused, but an `op` this build does not know is refused, since it cannot
/// know how to finish it (STD-03 §R10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub(crate) enum PendingWrite {
    /// [`crate::ops::promote`]: `node` is about to be written from the inbox
    /// entry `entry`, captured at `stamp`, and the entry then struck.
    Promote {
        /// The inbox entry's id.
        entry: String,
        /// Its stamp as the inbox line holds it. Ids are unique only among
        /// live entries, so the pair is what names one capture.
        stamp: String,
        /// The id of the node being written.
        node: String,
    },
}

impl PendingWrite {
    /// Whether `entry` is the capture this record is about.
    fn names(&self, entry: &InboxEntry) -> bool {
        match self {
            Self::Promote {
                entry: id, stamp, ..
            } => entry.id == *id && entry.stamp == *stamp,
        }
    }
}

/// Where the record is for the corpus at `root`.
pub(crate) fn pending_path(root: &Path) -> PathBuf {
    root.join(PENDING_FILE)
}

/// The record at `root`, if a write is pending.
///
/// Fails closed: a record that is present but cannot be read or parsed is
/// [`Error::PendingWriteUnreadable`], never "nothing pending", because
/// reading it as absent would expose the half-written state it guards.
pub(crate) fn read(root: &Path) -> Result<Option<PendingWrite>> {
    let path = pending_path(root);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io_at("reading", &path, error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| Error::PendingWriteUnreadable {
            path,
            reason: error.to_string(),
        })
}

/// A recorded write in progress: the record is on disk until this is
/// [`finished`](Self::finished).
///
/// The guard STD-03 §R4 asks a pending-write marker to have. Dropped without
/// being finished, on an error return, it settles the record at once from
/// the files, exactly as the next writer would: a promotion whose node was
/// written is struck, and one whose node was not is discarded. That cleanup
/// is best-effort by design: if it fails, the record stays, and the next
/// writer settles it before doing anything else, so nothing is lost by the
/// failure. Dropped while unwinding it does nothing and leaves the record,
/// as a process that died there would.
#[must_use = "the record stays on disk until the write is finished"]
pub(crate) struct Pending<'a> {
    corpus: &'a Corpus,
    finished: bool,
}

impl<'a> Pending<'a> {
    /// Record `write` before its first file is touched. The caller holds the
    /// corpus lock.
    pub(crate) fn begin(corpus: &'a Corpus, write: &PendingWrite) -> Result<Self> {
        let json = serde_json::to_vec(write).map_err(|source| Error::Json {
            context: "writing the pending-write record".into(),
            source,
        })?;
        write_private_atomic(&pending_path(corpus.root()), json)?;
        Ok(Self {
            corpus,
            finished: false,
        })
    }

    /// Every file is written: remove the record.
    pub(crate) fn finished(mut self) -> Result<()> {
        self.finished = true;
        remove_private(&pending_path(self.corpus.root()))
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if self.finished || std::thread::panicking() {
            return;
        }
        // Fail open, and only here: the record is the durable obligation, so
        // a failure to settle it now leaves it for the next writer, which
        // settles it before its own op or refuses (STD-02 §R31).
        let _ = self.corpus.finish_pending();
    }
}

impl Corpus {
    /// Settle a pending write, if one is recorded: finish it when its files
    /// say it was decided, discard the record when they say it never began.
    ///
    /// Called with the corpus lock held, on the take that entered the
    /// critical section ([`Corpus::lock`]), so it runs before every writer's
    /// own op and never underneath one. A record that cannot be read is
    /// refused rather than settled, and stays: the store stays closed until
    /// a person has looked at it (STD-03 §R9). Removing the record is safe
    /// because it has just been read back and parsed as one this build
    /// writes (STD-03 §R29).
    pub(crate) fn finish_pending(&self) -> Result<()> {
        let Some(write) = read(self.root())? else {
            return Ok(());
        };
        if self.decided(&write)? {
            match &write {
                PendingWrite::Promote { node, .. } => {
                    // Found by id and stamp rather than by line number, and
                    // only among live lines: one a hand edit already settled
                    // or removed leaves nothing to strike.
                    if let Some(entry) = self.live_entries()?.into_iter().find(|e| write.names(e)) {
                        self.settle_inbox(&entry, &format!("-> {node}"))?;
                    }
                }
            }
        }
        remove_private(&pending_path(self.root()))
    }

    /// `entries` less the one a recorded promotion has already written its
    /// node for. That entry is settled in all but its strike, which the next
    /// writer makes, so a lock-free read that offered it as waiting would
    /// expose the half-written state recovery exists to hide.
    pub(crate) fn waiting(&self, mut entries: Vec<InboxEntry>) -> Result<Vec<InboxEntry>> {
        if let Some(write) = read(self.root())?
            && self.decided(&write)?
        {
            entries.retain(|entry| !write.names(entry));
        }
        Ok(entries)
    }

    /// Whether the files say the recorded write passed its point of no
    /// return. For a promotion that is the node: it is written after the
    /// record, under the lock that checked it did not exist yet.
    fn decided(&self, write: &PendingWrite) -> Result<bool> {
        match write {
            PendingWrite::Promote { node, .. } => Ok(self.node_path(node)?.exists()),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures read the corpus back directly, beside the code under test"
)]
mod tests {
    use super::*;
    use crate::fs::{Step, crashing_at, recording};
    use crate::ops::{self, Promotion};

    fn corpus() -> (tempfile::TempDir, Corpus) {
        let dir = tempfile::tempdir().expect("tempdir");
        let corpus = Corpus::init(&dir.path().join("corpus")).expect("init");
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
                other => other.clone(),
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
        let (created, steps) =
            recording(|| ops::promote(&corpus, &entry.id, &Promotion::default(), 0));
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
}
