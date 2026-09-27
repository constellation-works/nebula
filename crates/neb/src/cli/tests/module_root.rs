//! Unit tests for `cli/mod.rs`: [`parse_from`]'s check of a global flag
//! written before the verb.

use super::parse_cli;
use crate::cli::args::{Cli, Command};
use crate::cli::parse_from;
use clap::CommandFactory;

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
