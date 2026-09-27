//! Unit tests for `cli/help.rs`: the template and the clap tree agree.

use super::parse_cli;
use crate::cli::args::{Cli, Command};
use clap::CommandFactory;
use std::path::Path;

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

    let manifest: BTreeSet<String> = serde_json::from_str::<Vec<String>>(include_str!(
        "../../../tests/goldens/help/commands.json"
    ))
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
