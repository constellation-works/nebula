//! Triage: the inbox one entry at a time, oldest first, each decision made
//! through the op that would make it as a single verb.
//!
//! [`Triage`] is a small state machine. It takes a snapshot of the inbox when
//! it starts, presents the oldest waiting entry as a [`Waiting`] with its age
//! and the nodes it reads closest to, and moves on when an [`Action`] settles
//! or skips it. Promotion and dropping go through [`ops::promote`] and
//! [`ops::drop`] unchanged, so every invariant, every refusal and the `by`
//! attribution are the ones the single verbs already have. What to commit,
//! and how to ask for the next action, is the caller's: this module never
//! reads a key and never commits.
//!
//! Nothing links automatically. The candidates are a numbered suggestion, and
//! a parent is written only when the caller names one of them by number.

use crate::error::{Error, Result};
use crate::graph::{NEAR_DEFAULT, Neighbour};
use crate::ops::{self, Created, Promotion};
use crate::store::{self, Corpus, InboxEntry};
use serde::Serialize;
use std::collections::VecDeque;

/// The entry triage is on, and what a decision about it needs.
#[derive(Debug, Clone, Serialize)]
pub struct Waiting {
    /// The capture.
    pub entry: InboxEntry,
    /// Whole days since it was captured; `None` when its stamp does not parse.
    pub days: Option<i64>,
    /// The nodes it reads closest to, best first. Numbered from 1 in this
    /// order by [`Action::PromoteUnder`]. A suggestion, never an edge.
    pub near: Vec<Neighbour>,
    /// The title a promotion will use, once [`Action::Title`] set one; `None`
    /// is the captured text, which is what `promote` defaults to as well.
    pub title: Option<String>,
    /// Where this entry is in the session, counting from 1.
    pub position: usize,
    /// How many entries the session started with.
    pub total: usize,
}

/// One decision about the current entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Promote it as a root: `promote <entry>`.
    Promote,
    /// Promote it under the candidate with this number, counting from 1 in
    /// [`Waiting::near`] order: `promote <entry> --parent <id>`.
    PromoteUnder(usize),
    /// Title the promotion that follows. Blank goes back to the captured text.
    Title(String),
    /// Drop it: `drop <entry>`.
    Drop,
    /// Leave it waiting and move on.
    Skip,
    /// Stop. Everything not yet decided stays in the inbox.
    Quit,
}

/// What an [`Action`] did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// The entry became a node, exactly as `promote` would have written it.
    Promoted {
        /// The entry that was settled.
        entry: InboxEntry,
        /// The node it became. Boxed because it dwarfs the other steps.
        created: Box<Created>,
    },
    /// The entry was struck through, exactly as `drop` would have.
    Dropped {
        /// The entry that was settled.
        entry: InboxEntry,
    },
    /// The current entry has a new title for its promotion; nothing written.
    Titled {
        /// The title now set, or `None` for the captured text.
        title: Option<String>,
    },
    /// The entry was left in the inbox; nothing written.
    Skipped {
        /// The entry passed over.
        entry: InboxEntry,
    },
    /// The session is over; nothing written.
    Quit,
}

/// What a session has done so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// Entries promoted to nodes.
    pub promoted: usize,
    /// Entries dropped.
    pub dropped: usize,
    /// Entries passed over, still in the inbox.
    pub skipped: usize,
    /// Entries the session never reached, still in the inbox.
    pub untouched: usize,
}

/// A triage session over one inbox snapshot.
#[derive(Debug)]
pub struct Triage {
    /// Entries not yet decided, oldest first. The front one is current.
    queue: VecDeque<InboxEntry>,
    /// The front entry with its age and candidates, once it has been looked at.
    current: Option<Waiting>,
    /// Who authors the titles and chooses the parents; `None` is the human.
    by: Option<String>,
    /// How many entries the snapshot held.
    total: usize,
    tally: Tally,
    quit: bool,
}

impl Triage {
    /// Start over the inbox as it is now, oldest capture first.
    ///
    /// The snapshot is fixed: a capture that lands during the session waits
    /// for the next one, so the session's length never grows under the
    /// person working through it.
    pub fn start(corpus: &Corpus, by: Option<String>) -> Result<Self> {
        let mut entries = corpus.inbox()?.0;
        // Month files are read in order and captures append, so this is
        // usually already sorted; a hand-edited file is why it is not assumed.
        // By instant rather than text, since stamps carry different offsets
        // and a legacy one carries none. Stable, so two captures in one
        // second keep their file order; a stamp that does not parse sorts
        // last.
        entries.sort_by_cached_key(|entry| {
            let at = store::parse_stamp(&entry.at);
            (at.is_none(), at)
        });
        Ok(Self {
            total: entries.len(),
            queue: entries.into(),
            current: None,
            by,
            tally: Tally::default(),
            quit: false,
        })
    }

    /// The entry to decide next, or `None` once every entry has been decided
    /// or the session quit.
    ///
    /// Its candidates are read from the corpus as it is now, so a node
    /// promoted earlier in the session can be the parent of a later entry.
    pub fn current(&mut self, corpus: &Corpus) -> Result<Option<&Waiting>> {
        self.look(corpus)?;
        Ok(self.current.as_ref())
    }

    /// Carry out one decision about the current entry.
    ///
    /// On success the session moves to the next entry, except after
    /// [`Action::Title`], which stays. On a refusal nothing moves, so the
    /// same entry can be decided another way (a promotion whose title will
    /// not make an id can be retitled and tried again), with one exception:
    /// an entry that something else settled after the session started is
    /// gone, so the session moves past it and returns the refusal.
    ///
    /// Once the session is over there is nothing left to act on, and every
    /// action is [`Step::Quit`].
    pub fn apply(&mut self, corpus: &Corpus, action: Action) -> Result<Step> {
        self.look(corpus)?;
        let Some(waiting) = self.current.as_mut() else {
            return Ok(Step::Quit);
        };
        let step = match action {
            Action::Promote => promote(corpus, waiting, self.by.clone(), Vec::new()),
            Action::PromoteUnder(number) => {
                let parent = number
                    .checked_sub(1)
                    .and_then(|i| waiting.near.get(i))
                    .ok_or(Error::NoSuchCandidate {
                        number,
                        shown: waiting.near.len(),
                    })?;
                let parents = vec![parent.id.clone()];
                promote(corpus, waiting, self.by.clone(), parents)
            }
            Action::Title(title) => {
                let title = title.trim();
                waiting.title = (!title.is_empty()).then(|| title.to_string());
                return Ok(Step::Titled {
                    title: waiting.title.clone(),
                });
            }
            Action::Drop => {
                let entry = waiting.entry.clone();
                ops::drop(corpus, &entry.id).map(|_| Step::Dropped { entry })
            }
            Action::Skip => Ok(Step::Skipped {
                entry: waiting.entry.clone(),
            }),
            Action::Quit => {
                self.quit = true;
                self.current = None;
                return Ok(Step::Quit);
            }
        };
        match &step {
            Ok(Step::Promoted { .. }) => self.tally.promoted += 1,
            Ok(Step::Dropped { .. }) => self.tally.dropped += 1,
            Ok(Step::Skipped { .. }) => self.tally.skipped += 1,
            Err(e) if settled_elsewhere(e) => {}
            Ok(Step::Titled { .. } | Step::Quit) | Err(_) => return step,
        }
        self.queue.pop_front();
        self.current = None;
        step
    }

    /// What the session has done so far, and how much of it is left.
    pub fn tally(&self) -> Tally {
        // The current entry is still at the front of the queue until it is
        // decided, so an undecided one at quit counts as never reached.
        Tally {
            untouched: self.queue.len(),
            ..self.tally
        }
    }

    /// How many entries the session started with.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Fill in [`Self::current`] for the front entry, if it is not already.
    fn look(&mut self, corpus: &Corpus) -> Result<()> {
        if self.quit || self.current.is_some() {
            return Ok(());
        }
        let Some(entry) = self.queue.front() else {
            return Ok(());
        };
        let near = ops::suggest(corpus, &entry.text, NEAR_DEFAULT)?;
        self.current = Some(Waiting {
            days: store::days_since_stamp(&entry.at),
            entry: entry.clone(),
            near,
            title: None,
            position: self.total - self.queue.len() + 1,
            total: self.total,
        });
        Ok(())
    }
}

/// `promote` the entry being decided under `parents`, with its title and the
/// session's author. Its candidates were shown already, so none are asked for.
fn promote(
    corpus: &Corpus,
    waiting: &Waiting,
    by: Option<String>,
    parents: Vec<String>,
) -> Result<Step> {
    let created = ops::promote(
        corpus,
        &waiting.entry.id,
        &Promotion {
            title: waiting.title.clone(),
            parents,
            by,
            ..Promotion::default()
        },
        0,
    )?;
    Ok(Step::Promoted {
        entry: waiting.entry.clone(),
        created: Box::new(created),
    })
}

/// Whether a refusal from [`Triage::apply`] means another writer settled the
/// entry since the session began: promoted or dropped it (`InboxEntrySettled`)
/// or removed its line (`NoSuchInboxEntry`). The session has then moved past
/// it, so the next entry shown is a new one.
pub fn settled_elsewhere(e: &Error) -> bool {
    matches!(
        e,
        Error::NoSuchInboxEntry(_) | Error::InboxEntrySettled { .. }
    )
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures hand-write inbox month files, not nebula state"
)]
mod tests {
    use super::*;

    #[test]
    fn triage_orders_mixed_stamp_forms_by_instant() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = Corpus::init(&dir.path().join("corpus")).unwrap();
        let inbox = dir.path().join("corpus/inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        // In file order, and so that text order differs from instant order:
        // `b` sorts before `a` as text but is an hour later. The legacy
        // stamp is local time, which is within fourteen hours of UTC
        // wherever this runs, so it falls between `b` and `d` on any host.
        std::fs::write(
            inbox.join("2026-09.md"),
            "- [000d] 2026-09-05T01:00:00+09:00 d\n\
             - [000c] 2026-09-03T08:00 c\n\
             - [000b] 2026-09-01T06:00:00-05:00 b\n\
             - [000e] someday e\n\
             - [000a] 2026-09-01T12:00:00+02:00 a\n",
        )
        .unwrap();

        let session = Triage::start(&corpus, None).unwrap();

        let order: Vec<&str> = session.queue.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(order, ["a", "b", "c", "d", "e"]);

        // An entry's age counts from the date its stamp was taken on, as its
        // own offset has it, whichever form the stamp is in.
        let legacy = store::days_since_stamp("2026-09-01T08:00");
        assert!(legacy.is_some());
        for stamp in [
            "2026-09-01T08:00:00+02:00",
            "2026-09-01T08:00:00Z",
            "2026-09-01T23:30:00-05:00",
        ] {
            assert_eq!(store::days_since_stamp(stamp), legacy, "{stamp}");
        }
        assert_eq!(store::days_since_stamp("someday"), None);
    }
}
