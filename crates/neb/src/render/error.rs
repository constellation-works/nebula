//! Core errors as the refusals `neb` reports.
//!
//! The library says what is wrong; this says what to do about it, because the
//! advice names commands and only the CLI has commands. Each hint here is the
//! advice the verb printed before the error had a type, and the prose a
//! refusal renders to is byte-for-byte what it printed then, so a script
//! grepping for a line keeps working.
//!
//! Under `--json` the same refusal is one JSON object instead. Its `code` is
//! [`Error::code`], so a new core variant arrives with its code and message
//! and no hint; giving it advice is one more arm in [`hint`].

use nebula_core::{Error, Settlement};

/// A refusal as `neb` reports it: a code to match on, what is wrong, and what
/// to do about it when the CLI knows.
#[derive(Debug)]
pub struct Refusal {
    /// A core error's [`Error::code`], or the `snake_case` code of one of the
    /// CLI's own refusals. Stable: scripts match on it.
    pub code: &'static str,
    /// What is wrong.
    pub message: String,
    /// What to do about it, or `null` when there is nothing to add.
    pub hint: Option<String>,
    /// The terminal wording, for the few refusals whose prose is not the
    /// message, a blank line, and the hint.
    prose: Option<String>,
}

impl Refusal {
    /// A refusal with no hint.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            prose: None,
        }
    }

    /// The text `neb` prints after `error:` without `--json`.
    pub fn prose(&self) -> String {
        match (&self.prose, &self.hint) {
            (Some(prose), _) => prose.clone(),
            (None, Some(hint)) => format!("{}\n\n{hint}", self.message),
            (None, None) => self.message.clone(),
        }
    }

    /// The `--json` form: `{"error", "code", "hint"}` on one line, where
    /// `error` is the message and `hint` is `null` when there is none.
    pub fn json(&self) -> String {
        serde_json::json!({
            "error": self.message,
            "code": self.code,
            "hint": self.hint,
        })
        .to_string()
    }

    fn worded(self, prose: String) -> Self {
        Self {
            prose: Some(prose),
            ..self
        }
    }
}

/// The refusal for an error, with whatever hint the CLI can add.
pub fn refusal(e: &Error) -> Refusal {
    Refusal {
        hint: hint(e),
        ..Refusal::new(e.code(), e.to_string())
    }
}

/// The refusal for an error raised about one node, whose id the hint needs.
pub fn refusal_about(e: &Error, node: &str) -> Refusal {
    match e {
        Error::NeedsKill(_) => {
            let hint = format!("neb sharpen {node} --kill \"...\"");
            let refused = refusal(e);
            let prose = format!("{}:\n\n  {hint}", refused.message);
            Refusal {
                hint: Some(hint),
                ..refused
            }
            .worded(prose)
        }
        Error::RefutedNeedsWhy => {
            // The CLI names its own flag where core says "a reason".
            let message = "refuted needs --why: say how the kill condition fired";
            let hint = format!("neb status {node} refuted --why \"...\"");
            let prose = format!("{message}\n\n  {hint}");
            Refusal {
                hint: Some(hint),
                ..Refusal::new(e.code(), message)
            }
            .worded(prose)
        }
        Error::SeedWithKill => {
            let message = format!(
                "`{node}` names a kill condition, so it cannot go back to seed; \
                 the kill stays, because nothing is deleted"
            );
            let hint = format!(
                "A node with a falsifier is open as a hypothesis:\n  neb status {node} hypothesis"
            );
            let prose = format!("{message}.\n\n{hint}");
            Refusal {
                hint: Some(hint),
                ..Refusal::new(e.code(), message)
            }
            .worded(prose)
        }
        Error::RefutedCannotReopen => {
            // One command, so it runs as printed once the title is filled in:
            // the new node's id never has to be copied out of `new`'s output.
            let message = format!("`{node}` is refuted and cannot simply reopen");
            let hint = format!("neb new \"...\" --reopens {node}");
            let prose = format!("{message}.\n\nRevive it as a new node that reopens it:\n  {hint}");
            Refusal {
                hint: Some(hint),
                ..Refusal::new(e.code(), message)
            }
            .worded(prose)
        }
        Error::AbsoluteUri(_) => Refusal {
            hint: Some(format!(
                "Cite it by a path relative to nodes/, such as ../../studies/x.md, or \
                 an Observatory record by its id:\n  \
                 neb cite {node} --kind observatory --uri Q002"
            )),
            ..refusal(e)
        },
        other => refusal(other),
    }
}

/// The advice for an error, where the CLI has any.
fn hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::NoSuchNode(_) => "List what exists with:  neb list".to_owned(),
        Error::NoSuchInboxEntry(_) => "See them with:  neb inbox".to_owned(),
        Error::InboxEntrySettled {
            settlement: Settlement::Promoted(node),
            ..
        } => format!("See the node with:  neb show {node}"),
        Error::InboxEntrySettled { .. } => "See what is still waiting with:  neb inbox".to_owned(),
        Error::NoCorpus(root) => format!("Create one with:  neb init {}", root.display()),
        Error::SchemaMismatch {
            found, expected, ..
        } if found > expected => {
            "This corpus was written by a newer nebula. Upgrade this build.".to_owned()
        }
        Error::SchemaMismatch { .. } => "Bring the corpus forward with:  neb migrate".to_owned(),
        Error::Locked { .. } => {
            "Another `neb`, an agent session, or the desktop app is mid-write. \
             Nothing changed, so run it again in a moment.\n\n\
             The lock goes with that writer's process, so there is no stale \
             .lock to remove."
                .to_owned()
        }
        Error::StagedElsewhere { root, .. } => format!(
            "Commit or unstage them, then catch the corpus up:\n  \
             git -C {0} add nodes inbox config.yaml .gitignore && git -C {0} commit -m \"neb\"\n\n\
             Or skip the commit next time with --no-commit.",
            root.display()
        ),
        Error::CorpusIgnored(root) => format!(
            "Make the corpus its own repository, so the containing one's ignore \
             rules stop applying:\n  git -C {} init\n\nOr turn the setting off:  \
             neb config commit off",
            root.display()
        ),
        Error::ParentAndReopens(id) => {
            format!("`--reopens {id}` already records the descent, so drop `--parent {id}`.")
        }
        Error::RelativeObservatoryRoot {
            setting: Some(path),
            ..
        } => format!(
            "Set it again with an absolute path:  neb config observatory-root <DIR>\n\
             Or remove {} to fall back to the other settings.",
            path.display()
        ),
        Error::RelativeObservatoryRoot { setting: None, .. } => {
            "The setting is read from whatever directory a command runs in, so \
             pass the checkout's absolute path."
                .to_owned()
        }
        Error::Interactive(_) => "Script the same steps with:\n  \
             neb inbox --json\n  \
             neb near <text> --json\n  \
             neb promote <entry> [--title <title>] [--parent <id>]\n  \
             neb drop <entry>"
            .to_owned(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn prose_is_the_message_then_the_hint() {
        let refused = refusal(&Error::NoSuchNode("nope".into()));
        assert_eq!(
            refused.prose(),
            "no node `nope`\n\nList what exists with:  neb list"
        );
        assert_eq!(refused.code, "no_such_node");
    }

    #[test]
    fn an_error_without_advice_is_its_message_alone() {
        let refused = refusal(&Error::SelfLoop);
        assert_eq!(refused.prose(), "a node cannot link to itself");
        assert_eq!(refused.hint, None);
    }

    #[test]
    fn a_reworded_refusal_keeps_its_prose_and_a_clean_hint() {
        let refused = refusal_about(&Error::NeedsKill(nebula_core::Status::Hypothesis), "n");
        assert_eq!(
            refused.prose(),
            "`hypothesis` needs a kill condition first:\n\n  neb sharpen n --kill \"...\""
        );
        assert_eq!(refused.message, "`hypothesis` needs a kill condition first");
        assert_eq!(
            refused.hint.as_deref(),
            Some("neb sharpen n --kill \"...\"")
        );
    }

    #[test]
    fn the_envelope_is_one_line_with_every_field() {
        let refused = refusal(&Error::NoCorpus(PathBuf::from("/x")));
        let line = refused.json();
        assert!(!line.contains('\n'), "{line}");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "error": "no corpus at /x",
                "code": "no_corpus",
                "hint": "Create one with:  neb init /x",
            })
        );
        let bare: serde_json::Value =
            serde_json::from_str(&refusal(&Error::SelfLoop).json()).unwrap();
        assert_eq!(
            bare,
            serde_json::json!({
                "error": "a node cannot link to itself",
                "code": "self_loop",
                "hint": null,
            })
        );
    }
}
