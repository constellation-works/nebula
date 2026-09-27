//! Lifecycle and labels: status moves, tags, and the tag-drift advice read
//! after a write.

use super::{CloseTag, Retagged, StatusChange};
use crate::check_impl as check;
use crate::error::{Error, Result};
use crate::lock;
use crate::model::{self, Closed, Doc, Status};
use crate::stamp;
use crate::store::Corpus;

/// Move a node to a new status, with the transition guards applied.
///
/// `why` is required for [`Status::Refuted`], optional for
/// [`Status::Abandoned`], and meaningless on an open status. A node that
/// names a kill condition cannot move to [`Status::Seed`]; it reopens as a
/// [`Status::Hypothesis`] instead.
pub fn set_status(
    corpus: &Corpus,
    id: &str,
    status: Status,
    why: Option<&str>,
) -> Result<StatusChange> {
    let why = why.map(str::trim).filter(|w| !w.is_empty());
    // An open status is not a closing, so there is nothing for a reason to be
    // the reason for. Refused before the node is read: the arguments alone
    // decide it, so the answer does not depend on the corpus.
    if status.is_open() && why.is_some() {
        return Err(Error::ReasonOnOpenStatus(status));
    }
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    let from = doc.node.status;

    // Rule 5 at the point of action: refuting is asserting the kill
    // condition fired, and that assertion has to be written down.
    if status == Status::Refuted && why.is_none() {
        return Err(Error::RefutedNeedsWhy);
    }
    if status.needs_kill() && doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        return Err(Error::NeedsKill(status));
    }
    // Rule 6: a ruled-out idea cannot quietly come back, and a verdict is
    // part of the record: even refuted -> refuted is refused, because it
    // would silently replace `closed.why` and its date rather than leaving
    // them as the recorded verdict. Reviving one takes a new node with a
    // `reopens` edge, so the fact that it was once dead stays visible.
    if from.is_closed_by_verdict() {
        return Err(Error::RefutedCannotReopen);
    }
    // Rule 13 at the point of action: nothing is deleted, so a move to seed
    // would keep the kill condition, and a seed carrying one is what `check`
    // reads as a hand edit. A node that already names its falsifier is open
    // as a hypothesis, which is the honest way back.
    if status == Status::Seed && doc.node.kill.is_some() {
        return Err(Error::SeedWithKill);
    }
    doc.node.status = status;
    doc.node.closed = match status {
        Status::Refuted => why.map(|why| Closed {
            why: why.to_string(),
            at: stamp::today(),
        }),
        Status::Abandoned => why
            .map(|why| Closed {
                why: why.to_string(),
                at: stamp::today(),
            })
            .or_else(|| doc.node.closed.take()),
        Status::Seed | Status::Hypothesis => None,
    };
    corpus.save(&mut doc)?;
    Ok(StatusChange { doc, from })
}

/// Add tags to a node, normalised to lowercase kebab-case on the way in.
pub fn tag_add(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Doc> {
    retag(corpus, id, tags, &[]).map(|done| done.doc)
}

/// Remove tags from a node. `--remove Physics` and `--remove physics` name
/// the same label, because both are normalised first.
pub fn tag_remove(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Doc> {
    retag(corpus, id, &[], tags).map(|done| done.doc)
}

/// Remove `remove` from a node's tags and then add `add`, in one write under
/// one lock. Both are normalised first, and so are the tags the node carries.
///
/// When the result is the list the node already stores, nothing is saved
/// (STD-01 §R30): a tag that was not there to remove, or was there to add,
/// is not an edit. Each such tag is named in the result, so the caller can
/// say which part of the request did nothing.
pub fn retag(corpus: &Corpus, id: &str, add: &[String], remove: &[String]) -> Result<Retagged> {
    let (add, remove) = (model::normalize_tags(add), model::normalize_tags(remove));
    // Both edits happen under the lock. Without it the load and the save are
    // two moments, and a second process editing the same node between them
    // loses one edit entirely.
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    let before = model::normalize_tags(&doc.node.tags);
    let absent = remove
        .iter()
        .filter(|t| !before.contains(t))
        .cloned()
        .collect();
    let already = add.iter().filter(|t| before.contains(t)).cloned().collect();
    let mut tags = before;
    tags.retain(|t| !remove.contains(t));
    for t in add {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    let written = tags != doc.node.tags;
    if written {
        doc.node.tags = tags;
        corpus.save(&mut doc)?;
    }
    Ok(Retagged {
        doc,
        already,
        absent,
        written,
    })
}

/// Which of `tags` on node `id` read as a variant of a tag already in use.
///
/// A tag counts only when this write introduced it to the corpus: no node
/// but `id` carries it. It is then compared with every other tag in the
/// corpus, by the rule `check` warns on (case, or a trailing `s`), and each
/// collision is returned with the number of nodes carrying the existing
/// variant. `tags` are normalised first, so a caller passes what it was
/// given.
///
/// Run after the write, over the corpus as it now is. A read, not a guard:
/// there is no declared list to refuse against, so nothing here stops a
/// write, and an empty result says only that nothing collided. A scan of
/// every node, so run once the write lock is released (STD-03 §R1); a debug
/// build panics if the calling thread still holds it.
pub fn close_tags(corpus: &Corpus, id: &str, tags: &[String]) -> Result<Vec<CloseTag>> {
    let tags = model::normalize_tags(tags);
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    debug_assert!(
        !lock::held_by_this_thread(corpus.root()),
        "close_tags scans nodes/ for advice and must not run under the corpus lock"
    );
    let docs = corpus.load_all()?;
    let carriers = check::tag_carriers(&docs);
    let mut close = Vec::new();
    for tag in &tags {
        let introduced = carriers
            .get(tag.as_str())
            .is_none_or(|ids| ids.iter().all(|carrier| *carrier == id));
        if !introduced {
            continue;
        }
        for (existing, ids) in &carriers {
            if *existing != tag.as_str() && check::tag_drift(tag, existing).is_some() {
                close.push(CloseTag {
                    tag: tag.clone(),
                    near: (*existing).to_string(),
                    nodes: ids.len(),
                });
            }
        }
    }
    Ok(close)
}
