//! Phrases the [`Error`](super::Error) messages share.

use crate::lock::LockHolder;

/// Who holds a lock, as [`Error::Locked`](super::Error::Locked) names it.
pub(super) fn holder_phrase(holder: Option<&LockHolder>) -> String {
    holder.map_or_else(
        || "an unidentified writer".to_owned(),
        |h| format!("`{}`, pid {}, since {}", h.label, h.pid, h.since_rfc3339()),
    )
}

/// How many candidate parents an entry has, as [`Error::NoSuchCandidate`](super::Error::NoSuchCandidate) says it.
pub(super) fn shown_candidates(shown: usize) -> String {
    match shown {
        0 => "this entry has no candidates".to_string(),
        1 => "this entry has only candidate 1".to_string(),
        n => format!("this entry has candidates 1 to {n}"),
    }
}
