//! Core errors as the refusals `neb` reports.
//!
//! The library says what is wrong; this says what to do about it, because the
//! advice names commands and flags and only the CLI has them. A core message
//! never names a flag, since the desktop has none (STD-02 §R26); the flag
//! that fixes a refusal is named here, in its hint.
//!
//! Under `--json` the same refusal is one JSON object instead. Its `code` is
//! [`Error::code`], so a new core variant arrives with its code and message
//! and no hint; giving it advice is one more arm in [`hint`].
//!
//! How a refusal exits is decided here too, once: [`exit`] places every core
//! variant, and the CLI's own refusals say which they are when they are made.

use crate::cli::shell_word;
use nebula_core::{EdgeType, Error, ErrorClass, LockHolder, Settlement, Status};
use std::time::{Duration, SystemTime};

fn shell_path(path: &std::path::Path) -> String {
    shell_word(&path.to_string_lossy())
}

/// How a refusal ends `neb` (STD-01 §R20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Exit 1: the command ran and could not do what was asked.
    Failure,
    /// Exit 2: the command line asks for something no corpus could give, such
    /// as a value of the wrong shape or two flags that contradict each other.
    Usage,
}

impl From<Exit> for std::process::ExitCode {
    fn from(exit: Exit) -> Self {
        match exit {
            Exit::Failure => Self::FAILURE,
            Exit::Usage => Self::from(2),
        }
    }
}

/// A refusal as `neb` reports it: a code to match on, what is wrong, and what
/// to do about it when the CLI knows.
#[derive(Debug)]
pub(crate) struct Refusal {
    /// A core error's [`Error::code`], or the `snake_case` code of one of the
    /// CLI's own refusals. Stable: scripts match on it.
    pub code: &'static str,
    /// What is wrong.
    pub message: String,
    /// What to do about it, or `null` when there is nothing to add.
    pub hint: Option<String>,
    /// How `neb` exits on it.
    pub exit: Exit,
    /// The terminal wording, for the few refusals whose prose is not the
    /// message, a blank line, and the hint.
    prose: Option<String>,
}

impl Refusal {
    /// A command failure with no hint.
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            exit: Exit::Failure,
            prose: None,
        }
    }

    /// A usage error with no hint.
    pub(crate) fn usage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit: Exit::Usage,
            ..Self::new(code, message)
        }
    }

    /// The same refusal, with `hint` as what to do about it.
    #[must_use]
    pub(crate) fn hinted(self, hint: Option<String>) -> Self {
        Self { hint, ..self }
    }

    /// The same refusal as a command failure, however [`exit`] would class
    /// its error: for a refusal of input read from somewhere other than the
    /// command line, which no usage error is about.
    #[must_use]
    pub(crate) fn failed(self) -> Self {
        Self {
            exit: Exit::Failure,
            ..self
        }
    }

    /// The text `neb` prints after `error:` without `--json`.
    pub(crate) fn prose(&self) -> String {
        match (&self.prose, &self.hint) {
            (Some(prose), _) => prose.clone(),
            (None, Some(hint)) => format!("{}\n\n{hint}", self.message),
            (None, None) => self.message.clone(),
        }
    }

    /// The `--json` form: `{"error", "code", "hint"}` on one line, where
    /// `error` is the message and `hint` is `null` when there is none.
    pub(crate) fn json(&self) -> String {
        serde_json::json!({
            "error": self.message,
            "code": self.code,
            "hint": self.hint,
        })
        .to_string()
    }

    /// This refusal, naming where the text a person typed was kept: an edit
    /// refused after the editor exited (STD-03 §R30). The path goes into the
    /// message, so it is in the prose and in the `--json` `error` alike.
    pub(crate) fn kept_at(self, path: &std::path::Path) -> Self {
        self.also(&format!("your edited text is kept at {}", path.display()))
    }

    /// This refusal, saying the typed text could not be kept, and why.
    pub(crate) fn not_kept(self, why: &str) -> Self {
        self.also(&format!("your edited text could not be kept: {why}"))
    }

    fn also(self, clause: &str) -> Self {
        Self {
            message: format!("{}; {clause}", self.message),
            prose: self.prose.map(|prose| format!("{prose}\n\n{clause}")),
            ..self
        }
    }

    fn worded(self, prose: String) -> Self {
        Self {
            prose: Some(prose),
            ..self
        }
    }
}

/// The refusal for an error, with whatever hint the CLI can add.
pub(crate) fn refusal(e: &Error) -> Refusal {
    Refusal {
        hint: hint(e),
        exit: exit(e),
        ..Refusal::new(e.code(), message(e))
    }
}

/// What is wrong. Core's message, except where the CLI can say it better
/// for a person at a prompt: a held lock names how long its holder has had
/// it rather than the time it took it.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and other messages pass through"
)]
fn message(e: &Error) -> String {
    match e {
        Error::Locked {
            root,
            holder: Some(holder),
        } => {
            let held = SystemTime::now()
                .duration_since(holder.since)
                .unwrap_or_default();
            format!(
                "another nebula writer is holding {} (`{}`, pid {}, for {}); nothing was written",
                root.display(),
                holder.label,
                holder.pid,
                age(held)
            )
        }
        // Core says the rule; the CLI names the verb that appends.
        Error::NotesChanged => "an existing ## Notes section was removed, reordered, or changed; \
                                use `neb note` to append notes"
            .to_owned(),
        other => other.to_string(),
    }
}

/// The refusal for an error from `new`, whose edges all come from its own
/// flags: a repeated edge there is a flag given twice, not an edge the corpus
/// already has, so there is no node to show yet.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and other refusals use the generic path"
)]
pub(crate) fn refusal_for_new(e: &Error) -> Refusal {
    match e {
        Error::DuplicateEdge { kind, to, .. } => {
            let flag = match kind {
                EdgeType::Reopens => "--reopens",
                EdgeType::Contradicts => "--contradicts",
                EdgeType::DerivesFrom | EdgeType::Refines | EdgeType::Generalizes => "--parent",
            };
            Refusal {
                hint: Some(format!("`{flag} {to}` is given twice; give it once.")),
                ..refusal(e)
            }
        }
        other => refusal(other),
    }
}

/// The refusal for an error raised about one node, whose id the hint needs.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and other refusals use the generic path"
)]
pub(crate) fn refusal_about(e: &Error, node: &str) -> Refusal {
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
                message: message.to_owned(),
                hint: Some(hint),
                ..refusal(e)
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
                message,
                hint: Some(hint),
                ..refusal(e)
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
                message,
                hint: Some(hint),
                ..refusal(e)
            }
            .worded(prose)
        }
        Error::AbsoluteUri(_) => Refusal {
            hint: Some(format!(
                "Cite it by a path relative to nodes/, such as ../../studies/x.md, or \
                 an Observatory record by its id:\n  \
                 neb cite {node} --kind observatory --uri <record-id>"
            )),
            ..refusal(e)
        },
        other => refusal(other),
    }
}

/// How a core refusal exits: the one place that is decided, from the class
/// core gives every variant (STD-01 §R20). An argument no corpus could
/// accept is a usage error; anything that turns on what the corpus, the
/// machine or git holds is a failure. The class lives in core because
/// [`Error`] is `#[non_exhaustive]`, so only there can the match be
/// exhaustive (STD-02 §R27).
fn exit(e: &Error) -> Exit {
    match e.class() {
        ErrorClass::Argument => Exit::Usage,
        ErrorClass::State => Exit::Failure,
    }
}

/// The advice for an error, where the CLI has any.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive; unmatched variants continue to flag hints"
)]
#[allow(
    clippy::too_many_lines,
    reason = "a lookup table: one arm per refusal with advice"
)]
fn hint(e: &Error) -> Option<String> {
    if let Some(hint) = repair_hint(e) {
        return Some(hint);
    }
    Some(match e {
        Error::InvalidReadOnlyEnvironment { .. } => "Set NEBULA_READ_ONLY=1 to refuse writes, or unset it.".to_owned(),
        Error::InvalidOriginEnvironment { .. } => "Unset the invalid Orbit variable or provide an explicit --task or --run value.".to_owned(),
        Error::ReadOnly => "Unset NEBULA_READ_ONLY to permit writes.".to_owned(),
        Error::ByRequired => "Pass --by human or --by <agent-label> to state who wrote the words.".to_owned(),
        Error::HumanOnly => "Ask the human to run `neb sharpen <node> --confirm` outside the Orbit run.".to_owned(),
        Error::UnreadableNodes { .. } => "Run `neb check` for every unreadable file and the findings from the rest of the corpus.".to_owned(),
        Error::NoSuchNode(_) => "List what exists with:  neb list".to_owned(),
        Error::NoSuchInboxEntry(_) => "See them with:  neb inbox".to_owned(),
        Error::InboxEntrySettled {
            settlement: Settlement::Promoted(node),
            ..
        } => format!("See the node with:  neb show {node}"),
        Error::InboxEntrySettled { .. } => "See what is still waiting with:  neb inbox".to_owned(),
        Error::UnknownRevision { .. } => "Pass a hash from the node's history, which \
             `neb log <id>` lists, or a date as YYYY-MM-DD."
            .to_owned(),
        Error::NoCorpus(root) => format!("Create one with:  neb init {}", shell_path(root)),
        Error::NotGitWorkTree(root) => {
            let root = shell_path(root);
            format!(
                "History comes from git. Make the corpus a repository, then have each \
                 write committed:\n  git -C {root} init\n  neb --root {root} config commit on"
            )
        }
        Error::DuplicateEdge { from, .. } => format!("See its edges with:  neb show {from}"),
        Error::Locked { holder, .. } => locked_hint(holder.as_ref()),
        Error::EditConflict(id) => format!(
            "Another writer changed the body while it was open in the editor. \
             See what it holds now with:  neb show {id}\n\
             then run  neb edit {id}  again and carry your text over."
        ),
        Error::InputTooLarge { .. } => format!(
            "Standard input takes up to {} for a capture and {} for a body. \
             Keep longer text in a file and cite it with `neb cite`.",
            size(nebula_core::ops::CAPTURE_INPUT_LIMIT),
            size(nebula_core::ops::BODY_INPUT_LIMIT)
        ),
        Error::GitTimedOut { root, context, .. } if context == "commit" => {
            let root = shell_path(root);
            format!(
                "A git hook is the likely cause: `git commit` runs the repository's \
                 pre-commit and commit-msg hooks, and one did not finish. Run it by \
                 hand to see where it stops:\n  git -C {root} hook run pre-commit\n\n\
                 The write is in place. Skip the commit next time with --no-commit, \
                 or record it once the hook is fixed:\n  \
                 git -C {root} add nodes inbox config.yaml .gitignore && git -C {root} commit -m \"neb\""
            )
        }
        Error::GitTimedOut { root, context, .. } => format!(
            "Run it by hand to see what it is waiting on:\n  git -C {} {context}",
            shell_path(root)
        ),
        Error::CorpusIgnored(root) => {
            let root = shell_path(root);
            format!(
                "Make the corpus its own repository, so the containing one's ignore \
                 rules stop applying:\n  git -C {root} init\n\nOr turn the setting off:  \
                 neb --root {root} config commit off"
            )
        }
        Error::AlreadyClosed {
            id,
            status: Status::Refuted,
        } => format!(
            "A refuted idea stays refuted. Revive it as a new node that reopens it, \
             and hand that one off:\n  neb new \"...\" --reopens {id}"
        ),
        Error::AlreadyClosed { id, .. } => format!("See how it closed with:  neb show {id}"),
        Error::UnresolvedObservatoryRecord { .. } => {
            "Check the record id, or update the checkout until it carries the record. \
             `neb config observatory-root` shows which root is in effect."
                .to_owned()
        }
        Error::RootAndPathDiffer { path, .. } => format!(
            "Name the corpus once, as the path:  neb init {}",
            shell_path(path)
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
        // No `--force`: a setting that fails the rule is replaced without
        // one, and a valid one already there should still be refused.
        Error::EmptyRootSetting(_) | Error::RelativeRootSetting { .. } => {
            "The setting is read from whatever directory a command runs in, so \
             name the corpus by its absolute path:\n  neb init <DIR> --set-root"
                .to_owned()
        }
        Error::Interactive(_) => "Script the same steps with:\n  \
             neb inbox --json\n  \
             neb near <text> --json\n  \
             neb promote <entry> [--title <title>] [--parent <id>]\n  \
             neb drop <entry>"
            .to_owned(),
        _ => return flag_hint(e),
    })
}

/// The advice for a refusal whose fix is a flag. Core's message is the
/// desktop's too, so it states the fact and the flag is named here.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and only listed variants have flag hints"
)]
fn flag_hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::EmptyRoot => {
            "Pass --root a corpus directory, or leave it out to resolve the corpus as usual."
                .to_owned()
        }
        Error::RootConfigConflict { .. } => "Pass --force to replace it.".to_owned(),
        Error::NoKillToConfirm(id) => {
            format!("Name one first with:  neb sharpen {id} --kill \"...\"")
        }
        Error::UriRequired { .. } => {
            "Pass where it lives with --uri, or cite it with --kind discussion \
             when it has no location."
                .to_owned()
        }
        Error::ReasonOnOpenStatus(_) => "Drop --why when moving to an open status.".to_owned(),
        Error::InvalidAt(_) => {
            "Pass --at a commit hash or a YYYY-MM-DD date; `neb log <NODE>` lists the commits."
                .to_owned()
        }
        _ => return None,
    })
}

/// The advice that names a repair to something on disk, where a helper
/// below has it. Kept out of [`hint`] so that one stays a single table.
fn repair_hint(e: &Error) -> Option<String> {
    schema_hint(e)
        .or_else(|| interrupted_write_hint(e))
        .or_else(|| entry_hint(e))
}

/// The advice for a corpus this build cannot read as it stands: its config
/// is missing or at another schema, or a node is not at the schema the
/// config declares, or `neb migrate` cannot run yet. Each names `neb migrate`
/// or the repair that precedes it.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and only listed variants have schema hints"
)]
fn schema_hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::DirtyTree(_) => "Commit or stash them, then run it again.".to_owned(),
        Error::SchemaMismatch {
            found, expected, ..
        } if found > expected => {
            "This corpus was written by a newer nebula. Upgrade this build.".to_owned()
        }
        Error::SchemaMismatch { .. } => "Bring the corpus forward with:  neb migrate".to_owned(),
        Error::MissingConfig { path } => {
            let root = shell_path(path.parent().unwrap_or(path));
            format!(
                "If the corpus predates config.yaml, bring it forward with:  neb --root {root} migrate\n\
                 If the file was deleted, restore it instead, which keeps the corpus's \
                 id and settings:  git -C {root} checkout -- config.yaml"
            )
        }
        Error::CurrentSchemaUnreadable { .. } => {
            "Fix the node by hand (`neb check` names the same problem), then run \
             `neb migrate` again."
                .to_owned()
        }
        Error::V1NodeUnderCurrentSchema { path, config, .. } => {
            format!(
                "If this corpus was never migrated, an older neb stamped its config. Repair it:\n  \
                 set `schema_version: 1` in {} (or delete that file), then run:  neb migrate\n\
                 If the key was added by hand, remove it from {} instead.",
                shell_path(config),
                shell_path(path)
            )
        }
        _ => return None,
    })
}

/// The advice for a write an earlier process left unfinished, which every
/// writer refuses to finish by guessing: a person reads the record and
/// removes it.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "the public Error is non-exhaustive and only the pending-write error has this hint"
)]
fn interrupted_write_hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::PendingWriteUnreadable { path, .. } => format!(
            "It is what an interrupted `neb promote` leaves behind. Read it with  \
             cat {0}\n\
             check that the node it names and its inbox line say what you want, \
             then remove it:  rm {0}",
            shell_path(path)
        ),
        _ => return None,
    })
}

/// The advice for a held lock: nothing was written, and who to wait for.
/// Never "try again in a moment": the holder may be a long commit, and the
/// record says which writer it is.
fn locked_hint(holder: Option<&LockHolder>) -> String {
    match holder {
        Some(holder) => format!(
            "Nothing was written. `{0}` (pid {1}) is mid-write: wait for it to \
             finish, then run this again. If it does not finish, see what it is \
             doing with:  ps -p {1}\n\n{LOCK_GOES_WITH_ITS_PROCESS}",
            holder.label, holder.pid
        ),
        None => format!(
            "Nothing was written. Another `neb`, an agent session, or the desktop \
             app is mid-write and did not record which: wait for it to finish, \
             then run this again. If it does not finish, look for a running `neb` \
             or the desktop app.\n\n{LOCK_GOES_WITH_ITS_PROCESS}"
        ),
    }
}

/// Why a held lock never needs clearing by hand, in both of its hints.
const LOCK_GOES_WITH_ITS_PROCESS: &str =
    "The lock goes with that writer's process, so there is no stale .lock to remove.";

/// How long something has been going, as a person reads it: `42s`, `3m`,
/// `2h`, `5d`. Whole units, rounded down, and never the next unit up until
/// two of it have passed, so `90s` stays `90s`.
pub(crate) fn age(held: Duration) -> String {
    let secs = held.as_secs();
    match secs {
        s if s < 120 => format!("{s}s"),
        s if s < 120 * 60 => format!("{}m", s / 60),
        s if s < 48 * 3600 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

/// The advice for an entry below the root that is not a regular file. The
/// lock file is nebula's own and is simply made again; anything else is the
/// owner's to put right, by replacing it with the file itself or moving it
/// out of the corpus.
fn entry_hint(e: &Error) -> Option<String> {
    let Error::NotRegularFile { path, .. } = e else {
        return None;
    };
    Some(
        if path.file_name() == Some(std::ffi::OsStr::new(nebula_core::LOCK_FILE)) {
            format!(
                "The next writer makes a fresh lock file. Remove this one:  rm {}",
                shell_path(path)
            )
        } else {
            format!(
                "nebula reads and writes only regular files inside the corpus, never \
                 through a symlink, a pipe or a device. Put the file itself at {}, \
                 or move the entry out of the corpus.",
                path.display()
            )
        },
    )
}

/// A byte count as a person reads it: `64 KiB`, `1 MiB`.
pub(crate) fn size(bytes: usize) -> String {
    match bytes {
        b if b >= 1 << 20 && b % (1 << 20) == 0 => format!("{} MiB", b >> 20),
        b if b >= 1 << 10 && b % (1 << 10) == 0 => format!("{} KiB", b >> 10),
        b => format!("{b} bytes"),
    }
}
