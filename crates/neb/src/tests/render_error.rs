//! Unit tests for `render_error`.

use nebula_core::{EdgeType, Error, ErrorClass, LockHolder, Settlement, Status};
use std::time::{Duration, SystemTime};

use crate::render::error::*;
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
        Error::UnsafeId("../x".into()),
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
    assert!(hint.contains("neb --root /c config commit on"), "{hint}");
}

#[test]
fn executable_path_hints_quote_paths_and_keep_the_corpus_selected() {
    let root = PathBuf::from("/state/my corpus 'draft' $HOME; touch marker");
    let root_text = root.to_string_lossy().into_owned();
    let no_corpus = refusal(&Error::NoCorpus(root.clone())).hint.unwrap();
    let words = shlex::split(no_corpus.strip_prefix("Create one with:  ").unwrap())
        .expect("no-corpus hint parses");
    assert_eq!(
        words,
        vec!["neb".to_owned(), "init".to_owned(), root_text.clone()]
    );

    let not_git = refusal(&Error::NotGitWorkTree(root.clone())).hint.unwrap();
    let commands = not_git
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            (line.starts_with("git -C ") || line.starts_with("neb ")).then_some(line)
        })
        .map(|command| shlex::split(command).expect("not-git command parses"))
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            vec![
                "git".to_owned(),
                "-C".to_owned(),
                root_text.clone(),
                "init".to_owned()
            ],
            vec![
                "neb".to_owned(),
                "--root".to_owned(),
                root_text.clone(),
                "config".to_owned(),
                "commit".to_owned(),
                "on".to_owned()
            ],
        ]
    );
}

#[test]
fn git_and_ignored_hints_quote_shell_paths() {
    let root = PathBuf::from("/state/my corpus 'draft' $HOME; touch marker");
    let root_text = root.to_string_lossy().into_owned();
    let timed_out = refusal(&Error::GitTimedOut {
        root: root.clone(),
        context: "commit".to_owned(),
        after: std::time::Duration::from_secs(120),
    })
    .hint
    .unwrap();
    let git_commands = timed_out
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("git -C "))
        .map(|line| shlex::split(line).expect("timed-out git command parses"))
        .collect::<Vec<_>>();
    assert_eq!(git_commands[0][2], root_text);
    assert_eq!(git_commands[1][2], root_text);
    let second_git = git_commands[1]
        .iter()
        .position(|word| word == "&&")
        .unwrap()
        + 1;
    assert_eq!(git_commands[1][second_git], "git");
    assert_eq!(git_commands[1][second_git + 1], "-C");
    assert_eq!(git_commands[1][second_git + 2], root_text);

    let ignored = refusal(&Error::CorpusIgnored(root.clone())).hint.unwrap();
    let ignored_git = ignored
        .lines()
        .find(|line| line.trim_start().starts_with("git -C "))
        .unwrap()
        .trim_start();
    let ignored_neb = ignored
        .split_once("neb --root ")
        .unwrap()
        .1
        .lines()
        .next()
        .unwrap();
    let ignored_commands = [ignored_git.to_owned(), format!("neb --root {ignored_neb}")]
        .map(|line| shlex::split(&line).expect("ignored-corpus command parses"));
    assert_eq!(ignored_commands[0][2], root_text);
    assert_eq!(ignored_commands[1][2], root_text);
}

#[test]
fn config_repair_hints_quote_paths_and_keep_the_root_selected() {
    let root = PathBuf::from("/state/my corpus 'draft' $HOME; touch marker");
    let root_text = root.to_string_lossy().into_owned();
    let missing_config = refusal(&Error::MissingConfig {
        path: root.join("config.yaml"),
    })
    .hint
    .unwrap();
    let missing_migrate = missing_config
        .split_once("neb --root ")
        .unwrap()
        .1
        .lines()
        .next()
        .unwrap();
    let missing_git = missing_config
        .split_once("git -C ")
        .unwrap()
        .1
        .lines()
        .next()
        .unwrap();
    let missing_commands = [
        format!("neb --root {missing_migrate}"),
        format!("git -C {missing_git}"),
    ]
    .map(|line| shlex::split(&line).expect("missing-config command parses"));
    assert_eq!(missing_commands[0][2], root_text);
    assert_eq!(missing_commands[1][2], root_text);

    let root_and_path = refusal(&Error::RootAndPathDiffer {
        root: PathBuf::from("/another-root"),
        path: root.clone(),
    })
    .hint
    .unwrap();
    assert_eq!(
        shlex::split(
            root_and_path
                .strip_prefix("Name the corpus once, as the path:  ")
                .unwrap()
        )
        .expect("root-and-path command parses"),
        vec!["neb".to_owned(), "init".to_owned(), root_text.clone()]
    );
}

#[test]
fn pending_and_lock_recovery_hints_quote_paths() {
    let root = PathBuf::from("/state/my corpus 'draft' $HOME; touch marker");
    let pending = root.join(".pending");
    let hint = refusal(&Error::PendingWriteUnreadable {
        path: pending.clone(),
        reason: "bad record".to_owned(),
    })
    .hint
    .expect("pending-write remedy");
    for command in ["cat", "rm"] {
        let line = hint
            .split_once(&format!("{command} "))
            .expect("command in hint")
            .1
            .lines()
            .next()
            .unwrap();
        assert_eq!(
            shlex::split(&format!("{command} {line}")).expect("pending command parses"),
            vec![command.to_owned(), pending.to_string_lossy().into_owned()]
        );
    }

    let lock = root.join(nebula_core::LOCK_FILE);
    let hint = refusal(&Error::NotRegularFile {
        path: lock.clone(),
        found: nebula_core::fs::EntryKind::Symlink,
    })
    .hint
    .expect("lock remedy");
    let command = hint.split_once("rm ").unwrap().1;
    assert_eq!(
        shlex::split(&format!("rm {command}")).expect("lock command parses"),
        vec!["rm".to_owned(), lock.to_string_lossy().into_owned()]
    );
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
    let bare: serde_json::Value = serde_json::from_str(&refusal(&Error::SelfLoop).json()).unwrap();
    assert_eq!(
        bare,
        serde_json::json!({
            "error": "a node cannot link to itself",
            "code": "self_loop",
            "hint": null,
        })
    );
}
