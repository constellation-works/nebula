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
use crate::fs_impl::{remove_private, write_private_atomic};
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
    // A regular file only: a symlink or a FIFO planted here is refused
    // rather than followed or waited on (STD-05 §R7).
    let Some(bytes) = crate::fs_impl::read_regular_bytes(&path, crate::fs_impl::Links::Refuse)?
    else {
        return Ok(None);
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
