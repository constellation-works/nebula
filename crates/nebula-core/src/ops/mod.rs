//! Everything that changes a corpus: capture, promote, drop, new, sharpen,
//! link, cite, note, status, handoff, tags, and the commit that may follow
//! any of them.
//!
//! Each op takes a [`Corpus`](crate::Corpus) and typed arguments, enforces the invariants
//! that belong at the point of action, writes, and returns what changed. A
//! caller cannot produce an invalid corpus through this module; `check` exists
//! to catch hand edits, not bugs in here.
//!
//! [`commit`] is its own op rather than the tail of every other one, so a
//! write's result reaches the caller even when git then refuses: the write is
//! never rolled back because of git, and the caller can tell the two apart.
//! The surfaces do not compose these themselves: [`crate::verb`] runs each
//! write and its commit as one call, under one lock.
//!
//! Every op here that writes takes the corpus write lock first and holds it
//! until it returns, because three processes share one corpus and none of
//! these is a single atomic file replacement: they are read-modify-write
//! across steps, sometimes across two files, sometimes followed by a commit.
//! A verb that wants one critical section over a write *and* the [`commit`]
//! that records it takes [`Corpus::lock`](crate::Corpus::lock) itself and holds it across both;
//! the lock is re-entrant on one thread, so the op still taking it underneath
//! costs nothing. Queries take nothing: [`suggest`], [`close_tags`] and
//! everything in [`crate::graph`] read a corpus that a writer may be part-way
//! through, and that is the trade the lock exists to make — writers wait,
//! readers never do.
//!
//! Nor is the lock held across anything slow (STD-03 §R1). [`suggest`] and
//! [`close_tags`] scan every node for advice, so they run before a writer
//! takes the lock or after it drops, and say so in debug builds;
//! [`promotion_near`] is [`promote`]'s suggestions read ahead of its lock. A
//! body edited in `$EDITOR` is saved with [`set_body_if`], which re-checks
//! under the lock that nobody changed it while the person typed.
//!
//! Which invariants live here rather than in `check` is a deliberate choice
//! per rule. See `docs/design/lineage-graph/specs/invariants.md`.
//!
//! The ops are spread over this module's files by verb family:
//!
//! - `capture`  `init`, and the inbox: capture, suggest, drop, promote
//! - `node`     `new`, `sharpen` and `confirm_kill`
//! - `link`     typed edges between two nodes
//! - `body`     dated notes, and the body replaced whole
//! - `cite`     references, and the hand-off to an Observatory record
//! - `status`   status moves and tags
//! - `settings` the machine's and the corpus's settings, and `commit`
//!
//! This file holds the arguments and results they share, and the bounded
//! read of standard input.

mod body;
mod capture;
mod cite;
mod link;
mod node;
mod settings;
mod status;

pub use body::{body_unchanged, note, refuse_rewritten_notes, set_body, set_body_if};
pub(crate) use capture::{capture, init_with_effect};
pub use capture::{
    capture_locked, capture_near, drop, init, init_target, promote, promote_with, promotion_near,
    suggest,
};
pub use cite::{cite, cite_with_observatory, handoff};
pub use link::link;
pub use node::{confirm_kill, new_node, sharpen};
pub use settings::{commit, drop_legacy_observatory_root, set_commit, set_observatory_root};
pub use status::{close_tags, retag, set_status, tag_add, tag_remove};

use crate::check_impl::resolve_observatory;
use crate::config::ObservatoryRoot;
use crate::error::{Error, Result};
use crate::graph_impl::{Neighbour, ObservatoryLink};
use crate::model::{Doc, Origin, Status};
use crate::store::InboxEntry;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A corpus that has just been created.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Initialized {
    /// Where it is.
    pub root: PathBuf,
}

/// A node that has just been written for the first time.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Created {
    /// The node as written.
    pub doc: Doc,
    /// The file it went into.
    pub path: PathBuf,
    /// Existing nodes the new one reads closest to, best first, for whoever
    /// is deciding whether it has a parent. Filled by [`promote`] when no
    /// parent was named; empty when one was, and always empty from
    /// [`new_node`]. A suggestion only: nothing here becomes an edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Neighbour>,
}

/// A thought that has just gone into the inbox, and where it may belong.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Captured {
    /// The entry as written.
    pub entry: InboxEntry,
    /// Existing nodes the text reads closest to, best first, for the triage
    /// that comes later. Empty when nothing shares a word with it, or when
    /// the caller asked for none. A suggestion only: nothing here becomes
    /// an edge, and the capture is in the inbox whatever this holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near: Vec<Neighbour>,
}

/// A node with a reference newly attached.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Cited {
    /// The node as written.
    pub doc: Doc,
    /// The id of the reference that was added.
    pub reference: String,
    /// Where the reference's record is on this machine, for an `observatory`
    /// reference, and absent for every other kind. Its `path` is filled in by
    /// [`Cited::with_observatory`], and is absent until a caller asks, as on
    /// [`crate::graph::NodeView`], or when the record does not resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observatory: Option<ObservatoryLink>,
}

impl Cited {
    /// Locate the cited record under `root`, when this is an `observatory`
    /// citation: the step [`crate::graph::NodeView::with_observatory`] is
    /// for `show`, and a caller takes it for the same reason.
    pub fn with_observatory(mut self, root: Option<&Path>) -> Result<Self> {
        if let Some(link) = &mut self.observatory {
            link.path = root
                .map(|root| resolve_observatory(root, &link.record))
                .transpose()?
                .flatten();
        }
        Ok(self)
    }
}

/// A tag a write has just introduced that reads as a variant of one already
/// in use: the same label but for case or a trailing `s`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CloseTag {
    /// The tag as the write stored it.
    pub tag: String,
    /// The tag already in use that it collides with, as written there.
    pub near: String,
    /// How many nodes carry `near`.
    pub nodes: usize,
}

/// A node that has moved along its lifecycle.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct StatusChange {
    /// The node as written.
    pub doc: Doc,
    /// Where it was before.
    pub from: Status,
}

/// A node handed off to an Observatory record: cited and closed in one write.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct HandedOff {
    /// The node as written: `abandoned`, with the hand-off as its reason.
    pub doc: Doc,
    /// The id of the `observatory` reference that was added.
    pub reference: String,
    /// The record it went to, as stored: `H012`.
    pub record: String,
    /// Where the node was before.
    pub from: Status,
    /// Where the record is on this machine: its `path` is absent when no
    /// observatory root was given.
    pub observatory: ObservatoryLink,
}

/// Everything that goes into a hand-off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Handoff {
    /// The Observatory record id the node becomes, such as `H012`. Case is
    /// normalised up, as `cite --kind observatory` does.
    pub record: String,
    /// Why it goes there: the note on the `observatory` reference.
    pub note: Option<String>,
    /// Who attached the reference and wrote the note. `None` is the human.
    pub by: Option<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// Everything that goes into a node created directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewNode {
    /// One line naming the idea.
    pub title: String,
    /// The argument, sketch or thought itself.
    pub body: String,
    /// Ids this descends from. More than one is a merge.
    pub parents: Vec<String>,
    /// A refuted node this revives, written as a `reopens` edge. That edge is
    /// genealogy, so the same id is not also a parent.
    #[serde(default)]
    pub reopens: Option<String>,
    /// Nodes this contradicts. Recorded on both ends, as [`link`](fn@link) does.
    #[serde(default)]
    pub contradicts: Vec<String>,
    /// What would falsify it. Naming one starts the node as a hypothesis.
    pub kill: Option<String>,
    /// Labels, normalised on the way in.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
    /// Explicit id, overriding the title's slug. Validated with the same
    /// rules a derived slug already follows, and refused on collision.
    pub id: Option<String>,
    /// Who wrote the title, the kill condition and the edges. `None` is the
    /// human, which is what an unattributed write means.
    pub by: Option<String>,
}

/// Everything that goes into a node promoted from the inbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Promotion {
    /// Node title. Defaults to the captured text.
    pub title: Option<String>,
    /// Prose appended after the captured text.
    pub body: String,
    /// Whether the caller explicitly supplied a body, even an empty one.
    /// This is invocation context and never part of a stored document.
    #[serde(skip)]
    pub body_supplied: bool,
    /// Ids this descends from.
    pub parents: Vec<String>,
    /// Labels.
    pub tags: Vec<String>,
    /// What produced it.
    pub origin: Option<Origin>,
    /// Explicit id, overriding the title's slug. Validated with the same
    /// rules a derived slug already follows, and refused on collision. With
    /// neither this nor `title`, the id is a shortened slug of the captured
    /// text rather than the slug of the whole sentence.
    pub id: Option<String>,
    /// Who wrote the title and chose the parents. `None` is the human.
    pub by: Option<String>,
}

/// Everything that goes into a reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Citation {
    /// Where it lives: a URL, DOI, path relative to `nodes/`, almanac
    /// wikilink, or — with kind `observatory` — a bare record id. Optional
    /// only for a discussion.
    pub uri: Option<String>,
    /// `paper`, `study`, `article`, `note`, `discussion`, `book`, `dataset`,
    /// `thread`, `observatory` or `other`, in any case: it is lowercased
    /// before it is checked.
    pub kind: String,
    /// Human-readable name.
    pub title: Option<String>,
    /// Why this is attached. The only field that matters in a year.
    pub note: Option<String>,
    /// Who attached it and wrote the note. `None` is the human.
    pub by: Option<String>,
    /// What produced it.
    pub origin: Option<Origin>,
}

/// The most a capture read from standard input may be: 64 KiB.
///
/// A capture is one thought on one inbox line, and every later read of the
/// inbox — the CLI's, the tray's, the desktop's — loads that line whole.
/// Prose longer than this belongs in a node's body.
pub const CAPTURE_INPUT_LIMIT: usize = 64 * 1024;

/// The most a node body read from standard input may be: 1 MiB.
pub const BODY_INPUT_LIMIT: usize = 1024 * 1024;

/// Read `input` to its end as text, refusing it as [`Error::InputTooLarge`]
/// when it is more than `limit` bytes. `what` names the text in that refusal.
///
/// At most `limit + 1` bytes are ever read, so an endless or enormous pipe
/// costs a bounded read rather than the machine's memory (STD-03 §R22). A
/// caller reads before it opens the corpus for writing, so a refusal has
/// written nothing and held no lock while the pipe drained.
pub fn read_bounded(input: impl std::io::Read, what: &'static str, limit: usize) -> Result<String> {
    use std::io::Read as _;

    let ceiling = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    input
        .take(ceiling)
        .read_to_end(&mut bytes)
        .map_err(|source| Error::IoStdin { what, source })?;
    if bytes.len() > limit {
        return Err(Error::InputTooLarge { what, limit });
    }
    String::from_utf8(bytes).map_err(|_| Error::StdinNotUtf8 { what })
}

/// What [`drop_legacy_observatory_root`] did.
#[derive(Debug, Clone)]
pub struct DroppedLegacy {
    /// The path the key held, now gone from `config.yaml`; `None` when the
    /// file carried no key, and so was not written.
    pub removed: Option<PathBuf>,
    /// The setting as it now resolves, without the key.
    pub setting: ObservatoryRoot,
}

/// A node's tags after [`retag`], and the part of the request that changed
/// nothing.
#[derive(Debug, Clone)]
pub struct Retagged {
    /// The node as it now stands, written or not.
    pub doc: Doc,
    /// Tags asked for that the node already carried, normalised.
    pub already: Vec<String>,
    /// Tags asked to be removed that the node did not carry, normalised.
    pub absent: Vec<String>,
    /// Whether the node was written. `false` when its tags came out exactly
    /// as they were stored: then `updated` is untouched too.
    pub written: bool,
}
