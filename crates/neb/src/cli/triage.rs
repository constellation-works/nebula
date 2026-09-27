//! `neb triage`: the session that works through the inbox one key per entry.

use super::emit::{CommitOpts, report_commit, say};
use super::failure::{Failure, KeyError};
use crate::output::{self, errln};
use crate::render;
use nebula_core::triage::{Action, Step};
use nebula_core::{Corpus, Error, Triage};
use std::io::BufRead;

/// One line of `neb triage` input, read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Key {
    /// A decision about the current entry.
    Act(Action),
    /// `t` alone: the title follows on the next line.
    AskTitle,
    /// `?` or `h`: list the keys again.
    Help,
    /// A blank line, which decides nothing.
    Nothing,
}

/// Read one line as a triage key. `t` is the one key that takes text.
pub(crate) fn parse_key(line: &str) -> std::result::Result<Key, KeyError> {
    let line = line.trim();
    let (key, rest) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(key, rest)| (key, rest.trim()));
    Ok(match (key, rest) {
        ("", _) => Key::Nothing,
        ("t", "") => Key::AskTitle,
        ("t", title) => Key::Act(Action::Title(title.to_string())),
        ("p", "") => Key::Act(Action::Promote),
        ("d", "") => Key::Act(Action::Drop),
        ("s", "") => Key::Act(Action::Skip),
        ("q", "") => Key::Act(Action::Quit),
        ("?" | "h", "") => Key::Help,
        (number, "") if number.bytes().all(|b| b.is_ascii_digit()) => {
            let number = number
                .parse()
                .map_err(|_| KeyError::Unknown(line.to_string()))?;
            Key::Act(Action::PromoteUnder(number))
        }
        _ => return Err(KeyError::Unknown(line.to_string())),
    })
}

/// `neb triage`: show the current entry, read a line, carry it out, repeat.
///
/// `interactive` is whether a person is at the keys. It decides three
/// things: whether the key legend and a prompt are shown, and whether a
/// refusal is reported and asked about again or ends the session, since
/// scripted lines after a refusal were written for an entry that did not
/// move. A commit that git refuses ends it either way, as it ends the single
/// verb: every later write would be left uncommitted the same way.
///
/// When `out` closes, nobody can see the next entry, so the session ends as
/// `q` ends it: every decision already made stays applied and committed.
/// Each decision is one [`Triage::decide`]: the write and its commit under
/// one lock, never held across the wait for the next line.
pub(crate) fn triage(
    corpus: &Corpus,
    by: Option<String>,
    input: &mut impl BufRead,
    out: &mut impl output::Closable,
    interactive: bool,
    commits: CommitOpts,
) -> std::result::Result<(), Failure> {
    let refuse = |failure: Failure| {
        if interactive {
            errln!("{} {}", render::error_label(), failure.0.prose());
            Ok(())
        } else {
            // A line of input was refused, not the command line, so it ends
            // the session as a failure whatever the verb would call it.
            Err(Failure(failure.0.failed()))
        }
    };
    let mut session = Triage::start(corpus, by)?;
    let mut shown = false;
    let mut titling = false;
    while let Some(waiting) = session.current(corpus)? {
        if !shown {
            say(out, &render::waiting(waiting))?;
            if interactive {
                say(out, &render::triage_keys(waiting))?;
            }
            shown = true;
        }
        if interactive {
            say(out, if titling { "title> " } else { "> " })?;
            out.flush().map_err(output::StdoutFailed::from)?;
        }
        if out.is_closed() {
            break;
        }
        let mut line = String::new();
        if input
            .read_line(&mut line)
            .map_err(|source| Error::IoStdin {
                what: "a triage key",
                source,
            })?
            == 0
        {
            if interactive {
                say(out, "\n")?;
            }
            // End of input stops as `q` does, unless a title is waiting to
            // be used: stopping then would drop it without a word (STD-01
            // §R27, recorded in docs/design/lineage-graph/4_decisions.md).
            if titling || waiting.title.is_some() {
                return Err(KeyError::TitleLost {
                    entry: waiting.entry.id.clone(),
                    title: waiting.title.clone().filter(|_| !titling),
                }
                .into());
            }
            break;
        }
        let action = if std::mem::take(&mut titling) {
            Action::Title(line)
        } else {
            match parse_key(&line) {
                Ok(Key::Act(action)) => action,
                Ok(Key::AskTitle) => {
                    titling = true;
                    continue;
                }
                Ok(Key::Help) => {
                    say(out, &render::triage_keys(waiting))?;
                    continue;
                }
                Ok(Key::Nothing) => continue,
                Err(e) => {
                    refuse(e.into())?;
                    continue;
                }
            }
        };
        let decided = session.decide(corpus, action, &commits.options())?;
        match decided.value {
            Ok(step) => {
                say(out, &render::step(&step))?;
                report_commit(corpus.root(), commits, decided.commit)?;
                if let Step::Titled { .. } = step {
                    continue;
                }
                shown = false;
            }
            Err(e) => {
                // Settled by another writer since the session began: the
                // session has moved past it, so the next entry is new.
                if nebula_core::triage::settled_elsewhere(&e) {
                    shown = false;
                }
                refuse(e.into())?;
            }
        }
    }
    say(out, &render::tally(&session.tally(), session.total()))
}
