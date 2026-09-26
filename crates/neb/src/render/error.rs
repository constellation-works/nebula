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

use nebula_core::{EdgeType, Error, ErrorClass, LockHolder, Settlement, Status};
use std::time::{Duration, SystemTime};

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
pub struct Refusal {
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
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            exit: Exit::Failure,
            prose: None,
        }
    }

    /// A usage error with no hint.
    pub fn usage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit: Exit::Usage,
            ..Self::new(code, message)
        }
    }

    /// The same refusal, with `hint` as what to do about it.
    #[must_use]
    pub fn hinted(self, hint: Option<String>) -> Self {
        Self { hint, ..self }
    }

    /// The same refusal as a command failure, however [`exit`] would class
    /// its error: for a refusal of input read from somewhere other than the
    /// command line, which no usage error is about.
    #[must_use]
    pub fn failed(self) -> Self {
        Self {
            exit: Exit::Failure,
            ..self
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

    /// This refusal, naming where the text a person typed was kept: an edit
    /// refused after the editor exited (STD-03 §R30). The path goes into the
    /// message, so it is in the prose and in the `--json` `error` alike.
    pub fn kept_at(self, path: &std::path::Path) -> Self {
        self.also(&format!("your edited text is kept at {}", path.display()))
    }

    /// This refusal, saying the typed text could not be kept, and why.
    pub fn not_kept(self, why: &str) -> Self {
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
pub fn refusal(e: &Error) -> Refusal {
    Refusal {
        hint: hint(e),
        exit: exit(e),
        ..Refusal::new(e.code(), message(e))
    }
}

/// What is wrong. Core's message, except where the CLI can say it better
/// for a person at a prompt: a held lock names how long its holder has had
/// it rather than the time it took it.
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
        other => other.to_string(),
    }
}

/// The refusal for an error from `new`, whose edges all come from its own
/// flags: a repeated edge there is a flag given twice, not an edge the corpus
/// already has, so there is no node to show yet.
pub fn refusal_for_new(e: &Error) -> Refusal {
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
    clippy::too_many_lines,
    reason = "a lookup table: one arm per refusal with advice"
)]
fn hint(e: &Error) -> Option<String> {
    if let Some(hint) = repair_hint(e) {
        return Some(hint);
    }
    Some(match e {
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
        Error::NoCorpus(root) => format!("Create one with:  neb init {}", root.display()),
        Error::NotGitWorkTree(root) => format!(
            "History comes from git. Make the corpus a repository, then have each \
             write committed:\n  git -C {} init\n  neb config commit on",
            root.display()
        ),
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
        Error::GitTimedOut { root, context, .. } if context == "commit" => format!(
            "A git hook is the likely cause: `git commit` runs the repository's \
             pre-commit and commit-msg hooks, and one did not finish. Run it by \
             hand to see where it stops:\n  git -C {0} hook run pre-commit\n\n\
             The write is in place. Skip the commit next time with --no-commit, \
             or record it once the hook is fixed:\n  \
             git -C {0} add nodes inbox config.yaml .gitignore && git -C {0} commit -m \"neb\"",
            root.display()
        ),
        Error::GitTimedOut { root, context, .. } => format!(
            "Run it by hand to see what it is waiting on:\n  git -C {} {context}",
            root.display()
        ),
        Error::CorpusIgnored(root) => format!(
            "Make the corpus its own repository, so the containing one's ignore \
             rules stop applying:\n  git -C {} init\n\nOr turn the setting off:  \
             neb config commit off",
            root.display()
        ),
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
            path.display()
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
fn schema_hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::DirtyTree(_) => "Commit or stash them, then run it again.".to_owned(),
        Error::SchemaMismatch {
            found, expected, ..
        } if found > expected => {
            "This corpus was written by a newer nebula. Upgrade this build.".to_owned()
        }
        Error::SchemaMismatch { .. } => "Bring the corpus forward with:  neb migrate".to_owned(),
        Error::MissingConfig { path } => format!(
            "If the corpus predates config.yaml, bring it forward with:  neb migrate\n\
             If the file was deleted, restore it instead, which keeps the corpus's \
             id and settings:  git -C {} checkout -- config.yaml",
            path.parent().unwrap_or(path).display()
        ),
        Error::CurrentSchemaUnreadable { .. } => {
            "Fix the node by hand (`neb check` names the same problem), then run \
             `neb migrate` again."
                .to_owned()
        }
        Error::V1NodeUnderCurrentSchema { path, config, .. } => format!(
            "If this corpus was never migrated, an older neb stamped its config. Repair it:\n  \
             set `schema_version: 1` in {} (or delete that file), then run:  neb migrate\n\
             If the key was added by hand, remove it from {} instead.",
            config.display(),
            path.display()
        ),
        _ => return None,
    })
}

/// The advice for a write an earlier process left unfinished, which every
/// writer refuses to finish by guessing: a person reads the record and
/// removes it.
fn interrupted_write_hint(e: &Error) -> Option<String> {
    Some(match e {
        Error::PendingWriteUnreadable { path, .. } => format!(
            "It is what an interrupted `neb promote` leaves behind. Read it with  \
             cat {0}\n\
             check that the node it names and its inbox line say what you want, \
             then remove it:  rm {0}",
            path.display()
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
fn age(held: Duration) -> String {
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
                path.display()
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
fn size(bytes: usize) -> String {
    match bytes {
        b if b >= 1 << 20 && b % (1 << 20) == 0 => format!("{} MiB", b >> 20),
        b if b >= 1 << 10 && b % (1 << 10) == 0 => format!("{} KiB", b >> 10),
        b => format!("{b} bytes"),
    }
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

    /// The argument-shape refusals exit 2 and a refusal about what the
    /// corpus holds exits 1; a refusal the CLI makes itself says which.
    #[test]
    fn the_exit_is_usage_only_for_what_no_corpus_could_accept() {
        for usage in [
            Error::EmptyRoot,
            Error::RootAndPathDiffer {
                root: PathBuf::from("a"),
                path: PathBuf::from("b"),
            },
            Error::RelativeObservatoryRoot {
                root: PathBuf::from("rel"),
                setting: None,
            },
            Error::SelfLoop,
            Error::ParentAndReopens("a".into()),
            Error::EmptyKill,
            Error::RefutedNeedsWhy,
            Error::Interactive("triage".into()),
            Error::AbsoluteUri("/x".into()),
            Error::UnusableTitle("!!!".into()),
            Error::UnknownReferenceKind("bogus".into()),
            Error::InvalidObservatoryId("NONSENSE".into()),
            Error::InvalidId("Bad Id".into()),
        ] {
            assert_eq!(refusal(&usage).exit, Exit::Usage, "{usage:?}");
            assert_eq!(refusal_about(&usage, "n").exit, Exit::Usage, "{usage:?}");
        }
        for failure in [
            Error::NoSuchNode("x".into()),
            Error::RelativeObservatoryRoot {
                root: PathBuf::from("rel"),
                setting: Some(PathBuf::from("/h/observatory-root")),
            },
            Error::UnsafeId("../x".into()),
            Error::SeedWithKill,
            Error::RefutedCannotReopen,
        ] {
            assert_eq!(refusal(&failure).exit, Exit::Failure, "{failure:?}");
            assert_eq!(
                refusal_about(&failure, "n").exit,
                Exit::Failure,
                "{failure:?}"
            );
        }
        assert_eq!(Refusal::new("json", "x").exit, Exit::Failure);
        assert_eq!(Refusal::usage("usage", "x").exit, Exit::Usage);
        assert_eq!(refusal(&Error::EmptyKill).failed().exit, Exit::Failure);
    }

    /// A repeated edge names both ends and the kind. From `link` the hint
    /// shows the node that has it; from `new` there is no node yet, and the
    /// flag given twice is the fix.
    #[test]
    fn a_duplicate_edge_names_the_edge_and_where_to_look() {
        let e = Error::DuplicateEdge {
            from: "b".into(),
            kind: EdgeType::DerivesFrom,
            to: "a".into(),
        };
        let refused = refusal(&e);
        assert_eq!(refused.code, "duplicate_edge");
        assert_eq!(
            refused.message,
            "the edge `b` derives-from `a` already exists"
        );
        assert_eq!(
            refused.hint.as_deref(),
            Some("See its edges with:  neb show b")
        );
        assert_eq!(refused.exit, Exit::Failure);
        let new = refusal_for_new(&Error::DuplicateEdge {
            from: "take-two".into(),
            kind: EdgeType::Contradicts,
            to: "rival".into(),
        });
        assert_eq!(
            new.hint.as_deref(),
            Some("`--contradicts rival` is given twice; give it once.")
        );
    }

    #[test]
    fn a_corpus_outside_git_is_told_how_to_start_its_history() {
        let hint = refusal(&Error::NotGitWorkTree(PathBuf::from("/c")))
            .hint
            .expect("a hint");
        assert!(hint.contains("git -C /c init"), "{hint}");
        assert!(hint.contains("neb config commit on"), "{hint}");
    }

    #[test]
    fn a_timed_out_git_blames_a_hook_only_when_it_was_committing() {
        let timed_out = |context: &str| {
            refusal(&Error::GitTimedOut {
                root: PathBuf::from("/c"),
                context: context.into(),
                after: std::time::Duration::from_secs(120),
            })
        };
        let commit = timed_out("commit");
        assert_eq!(commit.code, "git_timed_out");
        assert_eq!(
            commit.message,
            "git commit did not finish within 120s in /c and was stopped"
        );
        let hint = commit.hint.expect("a hint");
        for named in ["hook", "git -C /c hook run pre-commit", "--no-commit"] {
            assert!(hint.contains(named), "{named} in {hint}");
        }
        assert_eq!(
            timed_out("log").hint.as_deref(),
            Some("Run it by hand to see what it is waiting on:\n  git -C /c log")
        );
    }

    #[test]
    fn a_locked_error_names_the_holder_and_its_age() {
        let holder = nebula_core::LockHolder {
            pid: 4242,
            // Well inside the third hour, so the age cannot tick over while
            // the test runs.
            since: SystemTime::now() - Duration::from_secs(3 * 3600 + 1800),
            label: "neb edit a-node".to_owned(),
        };
        let refused = refusal(&Error::Locked {
            root: PathBuf::from("/c"),
            holder: Some(holder),
        });
        assert_eq!(refused.code, "locked");
        assert_eq!(
            refused.message,
            "another nebula writer is holding /c (`neb edit a-node`, pid 4242, for 3h); \
             nothing was written"
        );
        let hint = refused.hint.as_deref().expect("a hint");
        for named in [
            "Nothing was written",
            "`neb edit a-node` (pid 4242)",
            "ps -p 4242",
        ] {
            assert!(hint.contains(named), "{named} in {hint}");
        }
        assert!(!hint.contains("in a moment"), "{hint}");
        let value: serde_json::Value = serde_json::from_str(&refused.json()).unwrap();
        assert_eq!(value["error"], refused.message.as_str());
        assert_eq!(value["hint"], hint);
    }

    #[test]
    fn a_locked_error_without_a_record_says_unidentified_writer() {
        let refused = refusal(&Error::Locked {
            root: PathBuf::from("/c"),
            holder: None,
        });
        assert_eq!(
            refused.message,
            "another nebula writer is holding /c (an unidentified writer); nothing was written"
        );
        let hint = refused.hint.expect("a hint");
        assert!(hint.starts_with("Nothing was written."), "{hint}");
        assert!(hint.contains("no stale .lock"), "{hint}");
        assert!(
            !hint.contains("in a moment") && !hint.contains("pid"),
            "{hint}"
        );
    }

    #[test]
    fn an_age_reads_in_whole_units() {
        let age = |secs| age(Duration::from_secs(secs));
        assert_eq!(age(0), "0s");
        assert_eq!(age(90), "90s");
        assert_eq!(age(120), "2m");
        assert_eq!(age(119 * 60 + 59), "119m");
        assert_eq!(age(2 * 3600), "2h");
        assert_eq!(age(47 * 3600), "47h");
        assert_eq!(age(5 * 86400), "5d");
    }

    #[test]
    fn a_holder_whose_clock_runs_ahead_has_held_it_for_no_time() {
        let refused = refusal(&Error::Locked {
            root: PathBuf::from("/c"),
            holder: Some(nebula_core::LockHolder {
                pid: 1,
                since: SystemTime::now() + Duration::from_secs(3600),
                label: "nebula".to_owned(),
            }),
        });
        assert!(refused.message.contains("for 0s"), "{}", refused.message);
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
    fn a_kept_edit_is_named_in_the_message_the_prose_and_the_envelope() {
        let kept = std::path::Path::new("/state/nebula/edits/n-20260926T000000Z.md");
        let refused = refusal_about(&Error::EditConflict("n".into()), "n").kept_at(kept);
        assert_eq!(refused.code, "edit_conflict");
        assert_eq!(
            refused.message,
            "`n`'s body changed while it was being edited; nothing was written; \
             your edited text is kept at /state/nebula/edits/n-20260926T000000Z.md"
        );
        assert!(
            refused.prose().starts_with(&refused.message),
            "{}",
            refused.prose()
        );
        assert!(refused.hint.as_deref().unwrap().contains("neb show n"));
        let value: serde_json::Value = serde_json::from_str(&refused.json()).unwrap();
        assert_eq!(value["error"], refused.message.as_str());

        // A reworded refusal carries it in its own prose too.
        let reworded = refusal_about(&Error::RefutedNeedsWhy, "n").kept_at(kept);
        assert!(
            reworded
                .prose()
                .ends_with("kept at /state/nebula/edits/n-20260926T000000Z.md")
        );

        let lost = refusal(&Error::SelfLoop).not_kept("disk full");
        assert!(
            lost.message
                .ends_with("your edited text could not be kept: disk full")
        );
    }

    #[test]
    fn an_input_limit_hint_names_both_ceilings_readably() {
        let refused = refusal(&Error::InputTooLarge {
            what: "a capture",
            limit: nebula_core::ops::CAPTURE_INPUT_LIMIT,
        });
        assert_eq!(
            refused.message,
            "a capture on standard input is larger than 65536 bytes; nothing was written"
        );
        let hint = refused.hint.expect("a hint");
        assert!(hint.contains("64 KiB") && hint.contains("1 MiB"), "{hint}");
        assert_eq!(size(1000), "1000 bytes");
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
