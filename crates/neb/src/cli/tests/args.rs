//! Unit tests for `cli/args.rs`.

use super::parse_cli;
use crate::cli::args::{Cli, Command, EdgeKindArg, MIN_COUNT, MIN_DAYS, StatusArg};
use clap::{CommandFactory, ValueEnum};
use nebula_core::{EdgeType, Status};

#[test]
fn no_subcommand_argument_shadows_a_global_id() {
    use std::collections::BTreeSet;

    fn globals(command: &clap::Command, ids: &mut BTreeSet<String>) {
        for arg in command.get_arguments().filter(|arg| arg.is_global_set()) {
            ids.insert(arg.get_id().to_string());
        }
        for sub in command.get_subcommands() {
            globals(sub, ids);
        }
    }
    fn check(command: &clap::Command, path: &str, ids: &BTreeSet<String>) {
        for sub in command.get_subcommands() {
            let subpath = if path.is_empty() {
                sub.get_name().to_owned()
            } else {
                format!("{path} {}", sub.get_name())
            };
            for arg in sub.get_arguments().filter(|arg| !arg.is_global_set()) {
                assert!(
                    !ids.contains(&arg.get_id().to_string()),
                    "{subpath} shadows global argument id {}",
                    arg.get_id()
                );
            }
            check(sub, &subpath, ids);
        }
    }
    let mut command = Cli::command();
    command.build();
    let mut ids = BTreeSet::new();
    globals(&command, &mut ids);
    check(&command, "", &ids);
}

/// Whether `neb <path...> --help` lists `--no-commit` as an option, as
/// opposed to prose that merely mentions it.
fn offers_no_commit(path: &[&str]) -> bool {
    let mut cmd = Cli::command();
    cmd.build();
    let mut at = &mut cmd;
    for name in path {
        at = at
            .find_subcommand_mut(name)
            .unwrap_or_else(|| panic!("no subcommand {name}"));
    }
    at.render_long_help()
        .to_string()
        .lines()
        .any(|l| l.trim_start().starts_with("--no-commit"))
}

/// `--no-commit` is offered by exactly the verbs that can write, which
/// are exactly the ones [`Command::no_commit`] reads it from.
#[test]
fn no_commit_is_offered_only_by_verbs_that_write() {
    let writes: [&[&str]; 16] = [
        &["migrate"],
        &["config", "observatory-root"],
        &["config", "commit"],
        &["capture"],
        &["promote"],
        &["drop"],
        &["triage"],
        &["new"],
        &["edit"],
        &["sharpen"],
        &["status"],
        &["link"],
        &["tag"],
        &["note"],
        &["cite"],
        &["handoff"],
    ];
    for path in writes {
        assert!(
            offers_no_commit(path),
            "{path:?} writes, so its help offers --no-commit"
        );
    }
    let reads: [&[&str]; 13] = [
        &["init"],
        &["check"],
        &["config"],
        &["completions"],
        &["inbox"],
        &["show"],
        &["log"],
        &["list"],
        &["near"],
        &["trace"],
        &["impact"],
        &["graph"],
        &["review"],
    ];
    for path in reads {
        assert!(
            !offers_no_commit(path),
            "{path:?} only reads, so its help does not offer --no-commit"
        );
    }
    let visible = Cli::command()
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .count();
    // Every verb is in one list; `config` is in `reads` as itself and in
    // `writes` as its two settings.
    assert_eq!(writes.len() - 2 + reads.len(), visible);
}

/// Whether the verb's own `--no-commit` was given.
fn skips(command: &Command) -> bool {
    command.commit_arg().is_some_and(|c| c.no_commit)
}

/// The lock's holder record names the verb as typed and the one node or
/// entry it acts on, and never the text a verb was given.
#[test]
fn the_lock_label_is_the_verb_and_its_target_only() {
    let label = |argv: &[&str]| parse_cli(argv).expect("parses").lock_label();
    assert_eq!(label(&["edit", "a-node"]), "neb edit a-node");
    assert_eq!(
        label(&["--root", "/c", "tag", "n", "--add", "x"]),
        "neb tag n"
    );
    assert_eq!(label(&["link", "a", "refines", "b"]), "neb link a");
    assert_eq!(label(&["drop", "i1", "--no-commit"]), "neb drop i1");
    assert_eq!(
        label(&["capture", "a", "private", "thought"]),
        "neb capture"
    );
    assert_eq!(label(&["new", "A private title", "--body", "b"]), "neb new");
    assert_eq!(
        label(&["config", "observatory-root", "/obs"]),
        "neb config observatory-root"
    );
    assert_eq!(label(&["migrate"]), "neb migrate");
}

/// After a writing verb the flag is that verb's; before any verb it is
/// the old global spelling, which `run` warns about or refuses; after a
/// read-only verb it is an unknown argument rather than a silent no-op.
#[test]
fn no_commit_parses_where_it_means_something() {
    let after = parse_cli(&["drop", "i1", "--no-commit"]).expect("drop --no-commit");
    assert!(!after.no_commit && skips(&after.command));
    let config = parse_cli(&["config", "commit", "on", "--no-commit"]).expect("config");
    assert!(skips(&config.command));
    let before = parse_cli(&["--no-commit", "drop", "i1"]).expect("--no-commit drop");
    assert!(before.no_commit && !skips(&before.command));
    let plain = parse_cli(&["drop", "i1"]).expect("drop");
    assert!(!plain.no_commit && !skips(&plain.command));
    // What `run` names in its warning or its refusal.
    assert_eq!(before.verb, "drop");
    assert_eq!(config.verb, "config commit");
    let read = parse_cli(&["--no-commit", "list"]).expect("--no-commit list");
    assert!(read.no_commit && read.command.commit_arg().is_none());
    assert_eq!(read.verb, "list");
    // Neither spelling is global, so `parse_from`'s check for a verb
    // argument conflicting with a global before the verb never sees one,
    // alone or beside `--json`.
    let root = Cli::command();
    let hidden = root
        .get_arguments()
        .find(|a| a.get_long() == Some("no-commit"))
        .expect("the pre-verb spelling");
    assert!(hidden.is_hide_set() && !hidden.is_global_set());
    for args in [
        ["--json", "--no-commit", "drop", "i1"].as_slice(),
        &["--no-commit", "--json", "triage"],
        &["--json", "drop", "i1", "--no-commit"],
        &["drop", "--json", "i1", "--no-commit"],
    ] {
        let cli = parse_cli(args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        assert!(
            cli.json && (cli.no_commit || skips(&cli.command)),
            "{args:?}"
        );
    }
    for args in [
        ["show", "x", "--no-commit"].as_slice(),
        &["trace", "x", "--no-commit"],
        &["review", "--no-commit"],
    ] {
        let err = parse_cli(args).err().unwrap_or_default();
        assert!(
            err.contains("unexpected argument '--no-commit'"),
            "{args:?}: {err}"
        );
    }
}

/// `--limit` and `--depth` are optional: without them nothing is cut.
#[test]
fn output_bounds_default_to_everything() {
    assert!(matches!(
        parse_cli(&["list"]).map(|c| c.command),
        Ok(Command::List { limit: None, .. })
    ));
    assert!(matches!(
        parse_cli(&["inbox"]).map(|c| c.command),
        Ok(Command::Inbox { limit: None })
    ));
    assert!(matches!(
        parse_cli(&["review"]).map(|c| c.command),
        Ok(Command::Review { limit: None, .. })
    ));
    assert!(matches!(
        parse_cli(&["trace", "x"]).map(|c| c.command),
        Ok(Command::Trace { depth: None, .. })
    ));
    assert!(matches!(
        parse_cli(&["review", "--short", "--limit", "3"]).map(|c| c.command),
        Ok(Command::Review {
            short: true,
            limit: Some(3),
            ..
        })
    ));
    let err = parse_cli(&["list", "--limit", "all"])
        .err()
        .unwrap_or_default();
    assert!(err.contains("--limit"), "{err}");
}

/// Every count flag goes through the one bounded parser, found by walking
/// the whole command tree so a new `--limit` cannot opt out: each refuses
/// the value under its minimum, takes the minimum, and says it in `--help`.
#[test]
fn count_flags_share_one_lower_bound() {
    fn walk(cmd: &clap::Command, found: &mut Vec<(String, clap::Arg)>) {
        for arg in cmd.get_arguments() {
            let bounded = matches!(arg.get_long(), Some("limit" | "depth" | "since"))
                || arg.get_short() == Some('k');
            if bounded {
                found.push((cmd.get_name().to_owned(), arg.clone()));
            }
        }
        for sub in cmd.get_subcommands() {
            walk(sub, found);
        }
    }
    let mut found = Vec::new();
    walk(&Cli::command(), &mut found);

    let mut seen = Vec::new();
    for (verb, arg) in &found {
        let flag = arg.get_long().unwrap_or_default();
        let (below, least) = if flag == "since" {
            ("-1", MIN_DAYS.to_string())
        } else {
            ("0", MIN_COUNT.to_string())
        };
        // The argument alone, as its verb declares it, so the verb's
        // other required arguments and conflicts are not in the way.
        let parse = |value: &str| {
            clap::Command::new("probe")
                .arg(arg.clone())
                .try_get_matches_from(["probe".to_owned(), format!("--{flag}={value}")])
        };
        let refused = parse(below).expect_err(&format!("{verb} --{flag} {below}"));
        assert!(
            refused.to_string().contains(&format!("at least {least}")),
            "{verb} --{flag}: {refused}"
        );
        assert!(parse(&least).is_ok(), "{verb} --{flag} {least}");
        let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
        assert!(
            help.contains(&format!("at least {least}")),
            "{verb} --{flag} help does not state its minimum: {help}"
        );
        seen.push(format!("{verb} --{flag}"));
    }
    seen.sort();
    assert_eq!(
        seen,
        [
            "inbox --limit",
            "list --limit",
            "near --limit",
            "review --limit",
            "review --since",
            "trace --depth",
        ],
        "the bounded flags this walk found"
    );
}

/// `--short` is its own view with its own thresholds, so it refuses the
/// full report's `--since` and `--out`; `--tag` belongs to it alone.
#[test]
fn review_short_owns_tag_and_refuses_since_and_out() {
    assert!(matches!(
        parse_cli(&["review", "--short", "--tag", "a", "--tag", "b"]).map(|c| c.command),
        Ok(Command::Review { short: true, tags, .. }) if tags == ["a", "b"]
    ));
    for args in [
        ["review", "--short", "--since", "7"].as_slice(),
        &["review", "--short", "--out", "r.md"],
    ] {
        let err = parse_cli(args).err().unwrap_or_default();
        assert!(err.contains("cannot be used with"), "{args:?}: {err}");
    }
    let err = parse_cli(&["review", "--tag", "a"])
        .err()
        .unwrap_or_default();
    assert!(err.contains("--short"), "--tag without --short: {err}");
}

/// `capture`, `note` and `near` take remaining words as the thought, but
/// a flag after those words is still a flag. Swallowing `--quiet` or
/// `--no-commit` into the text is how the corpus got silently polluted.
#[test]
fn trailing_flags_after_free_text_are_flags() {
    let cli = parse_cli(&["capture", "an idea", "--quiet"]).expect("capture --quiet");
    assert!(!skips(&cli.command));
    let Command::Capture { quiet, text, .. } = cli.command else {
        panic!("expected capture")
    };
    assert!(quiet);
    assert_eq!(text, ["an idea"]);

    let cli = parse_cli(&["capture", "an", "idea", "--no-commit"]).expect("capture --no-commit");
    assert!(skips(&cli.command));
    let Command::Capture { quiet, text, .. } = cli.command else {
        panic!("expected capture")
    };
    assert!(!quiet);
    assert_eq!(text, ["an", "idea"]);

    let cli =
        parse_cli(&["note", "alpha-beta", "a thought", "--no-commit"]).expect("note --no-commit");
    assert!(skips(&cli.command));
    let Command::Note { node, by, text, .. } = cli.command else {
        panic!("expected note")
    };
    assert_eq!(node, "alpha-beta");
    assert_eq!(by, None);
    assert_eq!(text, ["a thought"]);

    let cli = parse_cli(&["near", "some query", "--limit", "2"]).expect("near --limit");
    let Command::Near { limit, query } = cli.command else {
        panic!("expected near")
    };
    assert_eq!(limit, 2);
    assert_eq!(query, ["some query"]);

    let Err(err) = parse_cli(&["near", "some query", "--quiet"]) else {
        panic!("near has no --quiet")
    };
    assert!(
        err.contains("--quiet"),
        "unknown trailing flag should be named: {err}"
    );

    let cli = parse_cli(&["capture", "--", "an idea", "--quiet"]).expect("capture -- escape");
    assert!(!skips(&cli.command));
    let Command::Capture { quiet, text, .. } = cli.command else {
        panic!("expected capture")
    };
    assert!(!quiet);
    assert_eq!(text, ["an idea", "--quiet"]);
}

/// The wrappers are the only place a status or an edge kind is spelled
/// for the command line, so they have to agree with the core types they
/// stand in for.
#[test]
fn value_enums_match_the_core_types() {
    for arg in [
        StatusArg::Seed,
        StatusArg::Hypothesis,
        StatusArg::Refuted,
        StatusArg::Abandoned,
    ] {
        let spelled = arg.to_possible_value().unwrap();
        assert_eq!(spelled.get_name(), Status::from(arg).to_string());
    }
    for arg in [
        EdgeKindArg::DerivesFrom,
        EdgeKindArg::Refines,
        EdgeKindArg::Generalizes,
        EdgeKindArg::Reopens,
        EdgeKindArg::Contradicts,
    ] {
        let spelled = arg.to_possible_value().unwrap();
        assert_eq!(spelled.get_name(), EdgeType::from(arg).to_string());
    }
}
