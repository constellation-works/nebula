//! Core errors as the messages `neb` prints.
//!
//! The library says what is wrong; this says what to do about it, because the
//! advice names commands and only the CLI has commands. Each arm here is the
//! message the verb printed before the error had a type, so a script grepping
//! for a line keeps working.

use nebula_core::Error;

/// The message for an error, with whatever hint the CLI can add.
pub fn message(e: &Error) -> String {
    match e {
        Error::NoSuchNode(id) => format!("no node `{id}`\n\nList what exists with:  neb list"),
        Error::NoSuchInboxEntry(id) => {
            format!("no open inbox entry `{id}`\n\nSee them with:  neb inbox")
        }
        Error::NoCorpus(root) => format!(
            "no corpus at {0}\n\nCreate one with:  neb init {0}",
            root.display()
        ),
        Error::SchemaMismatch { .. } => {
            format!("{e}\n\nBring the corpus forward with:  neb migrate")
        }
        other => other.to_string(),
    }
}

/// The message for an error raised about one node, whose id the hint needs.
pub fn message_about(e: &Error, node: &str) -> String {
    match e {
        Error::NeedsKill(status) => format!(
            "`{status}` needs a kill condition first:\n\n  neb sharpen {node} --kill \"...\""
        ),
        Error::RefutedNeedsWhy => format!(
            "refuted needs --why: say how the kill condition fired\n\n  \
             neb status {node} refuted --why \"...\""
        ),
        Error::RefutedCannotReopen => format!(
            "`{node}` is refuted and cannot simply reopen.\n\n\
             Create the new idea and link it:\n  \
             neb new \"...\" && neb link <new> reopens {node}"
        ),
        other => message(other),
    }
}
