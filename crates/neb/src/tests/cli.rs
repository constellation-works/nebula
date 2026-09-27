//! Unit tests for `cli`.

use crate::output::{self, errln, out, outln};
use crate::render::{self, json};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use nebula_core::triage::{Action, Step};
use nebula_core::verb::{self, CiteReport, CommitPolicy, RootWarning, WriteOptions};
use nebula_core::{
    Citation, CloseTag, CommitOutcome, Corpus, CorpusLock, Direction, EdgeType, Error, Handoff,
    Locations, NEAR_DEFAULT, NewNode, OBSERVATORY_ROOT_ENV, ObservatoryLink, ObservatoryRoot,
    ObservatorySource, Promotion, Severity, Status, Triage, graph, ops,
};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

use crate::cli::*;

/// The fixture names are the exact assembled clap tree, including hidden
/// commands and the two settings below `config`.
#[test]
fn help_goldens_cover_the_command_tree() {
    use std::collections::BTreeSet;

    fn walk(command: &clap::Command, prefix: &str, found: &mut BTreeSet<String>) {
        found.insert(prefix.to_owned());
        for sub in command.get_subcommands() {
            let path = if prefix.is_empty() {
                sub.get_name().to_owned()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            walk(sub, &path, found);
        }
    }

    fn fixture_names(dir: &Path, prefix: &str, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("help fixture directory") {
            let path = entry.expect("help fixture entry").path();
            if path.is_dir() {
                let child = if prefix.is_empty() {
                    path.file_name().unwrap().to_string_lossy().into_owned()
                } else {
                    format!("{prefix} {}", path.file_name().unwrap().to_string_lossy())
                };
                fixture_names(&path, &child, found);
            } else if path.extension().is_some_and(|extension| extension == "txt") {
                let stem = path.file_stem().unwrap().to_string_lossy();
                found.insert(if prefix.is_empty() && stem == "neb" {
                    String::new()
                } else if prefix.is_empty() {
                    stem.into_owned()
                } else {
                    format!("{prefix} {stem}")
                });
            }
        }
    }

    let manifest: BTreeSet<String> =
        serde_json::from_str::<Vec<String>>(include_str!("../../tests/goldens/help/commands.json"))
            .expect("help fixture manifest")
            .into_iter()
            .collect();
    let mut tree = BTreeSet::new();
    walk(&Cli::command(), "", &mut tree);
    assert_eq!(manifest, tree, "add a fixture for every command");

    let mut files = BTreeSet::new();
    fixture_names(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/help"),
        "",
        &mut files,
    );
    assert_eq!(files, tree, "help fixture filenames differ from clap tree");
}

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

fn help() -> String {
    Cli::command().render_long_help().to_string()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every visible subcommand clap knows about has a row in the template,
/// and the row's text is the variant's own one-liner, so the two cannot
/// drift. Hidden aliases are the exception, and have no row.
#[test]
fn help_rows_match_the_variants() {
    let flat = squash(&help());
    let mut seen = 0;
    for sub in Cli::command()
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
    {
        let about = sub.get_about().map(ToString::to_string).unwrap_or_default();
        let row = squash(&format!("{} {about}", sub.get_name()));
        assert!(flat.contains(&row), "help is missing the row {row:?}");
        seen += 1;
    }
    assert_eq!(seen, 27, "template rows need updating for a new subcommand");
    assert!(
        Cli::command().find_subcommand("help").is_none(),
        "clap's `help` subcommand should be disabled"
    );
}

/// The sections appear in lifecycle order, and clap's own usage line and
/// global options are still rendered around them.
#[test]
fn help_sections_are_in_lifecycle_order() {
    let text = help();
    let headings = [
        "Corpus:",
        "Inbox:",
        "Nodes:",
        "References:",
        "Query:",
        "Maintenance:",
        "Options:",
    ];
    let mut last = 0;
    for h in headings {
        let at = text
            .find(&format!("\n{h}\n"))
            .unwrap_or_else(|| panic!("no {h} section"));
        assert!(at > last, "{h} is out of order");
        last = at;
    }
    assert!(text.contains("Usage: neb [OPTIONS] <COMMAND>"));
    assert!(text.contains("--root <DIR>"));
    assert!(
        !text.contains("--no-commit"),
        "--no-commit belongs to the verbs that write"
    );
    assert!(text.contains("The corpus lives outside this repository"));
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

/// `open` is a deprecated alias for `review --short`: it still parses,
/// but `--help` no longer offers it.
#[test]
fn the_deprecated_open_alias_is_hidden() {
    let open = Cli::command()
        .find_subcommand("open")
        .cloned()
        .expect("`open` still parses");
    assert!(open.is_hide_set(), "`open` must be hidden from --help");
    assert!(!help().contains("\n  open "), "`open` has no row in --help");
    assert!(matches!(
        parse_cli(&["open", "--tag", "physics"]).map(|c| c.command),
        Ok(Command::Open { tags }) if tags == ["physics"]
    ));
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

/// `graph`'s formats conflict with the global `--json` on either side of
/// the verb, with the error clap gives when `--json` follows it.
#[test]
fn graph_formats_refuse_json_before_or_after_the_verb() {
    for (before, after) in [
        (
            ["--json", "graph", "--mermaid"].as_slice(),
            ["graph", "--mermaid", "--json"].as_slice(),
        ),
        (
            &["--json", "graph", "--mermaid", "--from", "x"],
            &["graph", "--mermaid", "--from", "x", "--json"],
        ),
        (
            &["--json", "graph", "--from", "x", "--mermaid"],
            &["graph", "--from", "x", "--mermaid", "--json"],
        ),
    ] {
        let Err(refused) = parse_from(std::iter::once("neb").chain(before.iter().copied())) else {
            panic!("{before:?} parsed");
        };
        assert_eq!(
            refused.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "{before:?}"
        );
        let err = refused.render().to_string();
        assert!(err.contains("cannot be used with '--json'"), "{err}");
        assert_eq!(Err(err), parse_cli(after).map(|_| ()), "{after:?}");
    }
    assert!(parse_cli(&["--json", "graph"]).is_ok_and(|cli| cli.json));
    assert!(matches!(
        parse_cli(&["--no-commit", "graph", "--mermaid", "--from", "x"]).map(|c| c.command),
        Ok(Command::Graph { mermaid: true, from: Some(from) }) if from == "x"
    ));
}

/// `completions` refuses `--root` and `--json`, which it would ignore, on
/// either side of the verb, with the error clap gives after it.
#[test]
fn completions_refuse_root_and_json_before_or_after_the_verb() {
    for (before, after, global) in [
        (
            ["--json", "completions", "bash"].as_slice(),
            ["completions", "bash", "--json"].as_slice(),
            "--json",
        ),
        (
            &["--root", "x", "completions", "bash"],
            &["completions", "bash", "--root", "x"],
            "--root <DIR>",
        ),
    ] {
        let Err(refused) = parse_from(std::iter::once("neb").chain(before.iter().copied())) else {
            panic!("{before:?} parsed");
        };
        assert_eq!(
            refused.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "{before:?}"
        );
        let err = refused.render().to_string();
        assert!(
            err.contains(&format!("cannot be used with '{global}'")),
            "{err}"
        );
        assert_eq!(Err(err), parse_cli(after).map(|_| ()), "{after:?}");
    }
    assert!(parse_cli(&["completions", "bash"]).is_ok());
}

/// The declared conflicts between a verb's argument and a global flag.
/// Clap checks these only when the flag follows the verb and
/// [`parse_from`] covers the other side; each one here needs a case in
/// the test above and in the CLI tests.
#[test]
fn global_conflicts_are_the_ones_tested() {
    fn walk(cmd: &clap::Command, found: &mut Vec<String>) {
        for sub in cmd.get_subcommands() {
            for arg in sub.get_arguments().filter(|a| !a.is_global_set()) {
                for other in sub.get_arg_conflicts_with(arg) {
                    if other.is_global_set() {
                        found.push(format!("{} {} {}", sub.get_name(), arg, other));
                    }
                }
            }
            walk(sub, found);
        }
    }
    let mut cmd = Cli::command();
    cmd.build();
    let mut found = Vec::new();
    walk(&cmd, &mut found);
    assert_eq!(
        found,
        [
            "completions <SHELL> --root <DIR>",
            "completions <SHELL> --json",
            "graph --mermaid --json",
            "graph --from <ID> --json"
        ]
    );
}

/// The variant order mirrors the template's section order, so a command
/// left out of the template would still surface next to its group.
#[test]
fn variant_order_matches_the_template() {
    let names: Vec<_> = Cli::command()
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect();
    let expected = [
        ["init", "check", "migrate", "config", "completions"].as_slice(),
        &["capture", "inbox", "promote", "drop", "triage"],
        &["new", "edit", "sharpen", "status", "link", "tag", "note"],
        &["cite", "handoff"],
        &["show", "log", "list", "near", "trace", "impact", "graph"],
        &["review"],
    ]
    .concat();
    assert_eq!(names, expected);
}

/// The wrappers are the only place a status or an edge kind is spelled
/// for the command line, so they have to agree with the core types they
/// stand in for.
fn parse_cli(args: &[&str]) -> Result<Cli, String> {
    parse_from(std::iter::once("neb").chain(args.iter().copied()))
        .map_err(|e| e.render().to_string())
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

/// Each key is one action, `t` is the only one that takes text, and
/// anything else is refused by name rather than guessed at.
#[test]
fn triage_keys_parse_to_their_actions() {
    for (line, key) in [
        ("p\n", Key::Act(Action::Promote)),
        ("  2 ", Key::Act(Action::PromoteUnder(2))),
        ("d", Key::Act(Action::Drop)),
        ("s", Key::Act(Action::Skip)),
        ("q", Key::Act(Action::Quit)),
        ("t", Key::AskTitle),
        (
            "t  A title  here ",
            Key::Act(Action::Title("A title  here".into())),
        ),
        ("?", Key::Help),
        ("h", Key::Help),
        ("   \n", Key::Nothing),
    ] {
        assert_eq!(parse_key(line).unwrap(), key, "{line:?}");
    }
    for line in [
        "x",
        "pp",
        "p now",
        "d 2",
        "+1",
        "-1",
        "1.5",
        "99999999999999999999999",
    ] {
        let Err(KeyError::Unknown(said)) = parse_key(line) else {
            panic!("{line:?} should be refused")
        };
        assert_eq!(said, line.trim());
    }
}

/// On a terminal the keys are listed and prompted for, and a refusal is
/// reported and the same entry asked about again rather than ending the
/// session, which is what lets a person correct a slip.
#[test]
fn interactive_triage_prompts_and_asks_again_after_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(
        &nebula_core::Locations::default(),
        &dir.path().join("corpus"),
    )
    .unwrap();
    let entry = ops::capture(&corpus, "a thought worth keeping").unwrap();
    let mut input = std::io::Cursor::new("x\n7\nt\n!!!\np\nt Worth keeping\np\n");
    let mut out = Vec::new();
    let commits = CommitOpts {
        skip: false,
        json: false,
    };
    let done = triage(&corpus, None, &mut input, &mut out, true, commits);
    assert!(done.is_ok(), "refusals do not end a terminal session");
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("p promote as a root"), "{out}");
    assert!(out.contains("title> "), "{out}");
    assert_eq!(
        out.matches("\n[1/1]").count(),
        1,
        "the entry is shown once, then asked about again:\n{out}"
    );
    assert!(
        out.contains(&format!("promoted {} -> worth-keeping", entry.id)),
        "{out}"
    );
    assert!(corpus.inbox().unwrap().0.is_empty());
}

/// A screen that goes away once the first decision is reported.
struct GoesAfterFirstStep(Vec<u8>);

impl Write for GoesAfterFirstStep {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl output::Closable for GoesAfterFirstStep {
    fn is_closed(&self) -> bool {
        String::from_utf8_lossy(&self.0).contains("dropped ")
    }
}

/// When the screen closes mid-session, the session ends as `q` ends it:
/// the decision already made stays, and the keys typed after it are
/// never applied to entries nobody saw.
#[test]
fn triage_ends_as_quit_does_when_its_screen_closes() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(
        &nebula_core::Locations::default(),
        &dir.path().join("corpus"),
    )
    .unwrap();
    let first = ops::capture(&corpus, "the first thought").unwrap();
    let second = ops::capture(&corpus, "the second thought").unwrap();
    let mut input = std::io::Cursor::new("d\nd\n");
    let mut out = GoesAfterFirstStep(Vec::new());
    let commits = CommitOpts {
        skip: true,
        json: false,
    };
    let done = triage(&corpus, None, &mut input, &mut out, false, commits);
    assert!(done.is_ok(), "a closed screen is no refusal");
    let waiting: Vec<_> = corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(waiting, [second.id], "only {} was dropped", first.id);
}

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
