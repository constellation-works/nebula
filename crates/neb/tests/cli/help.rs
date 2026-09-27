//! Help and completions: the pages every command documents, their examples,
//! no real record ids, and output that does not depend on the terminal.

use crate::harness::{Corpus, run_from_home};

/// Help aliases reach the same long-help pages through the real entry point.
#[test]
fn help_subcommand_matches_long_help() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let paths: Vec<String> =
        serde_json::from_str(include_str!("../goldens/help/commands.json")).unwrap();
    for path in paths {
        let mut flags: Vec<_> = path.split_whitespace().collect();
        flags.push("--help");
        let mut alias = vec!["help"];
        alias.extend(path.split_whitespace());
        let expected = run_from_home(&home, None, &flags, None).assert_ok();
        let actual = run_from_home(&home, None, &alias, None).assert_ok();
        assert_eq!(actual.stdout(), expected.stdout(), "{alias:?}");
        assert_eq!(actual.stderr(), "", "{alias:?}");
    }
}

#[test]
fn bare_invocation_and_help_share_a_tagline() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let bare = run_from_home(&home, None, &[], None).assert_fails();
    assert_eq!(bare.out.status.code(), Some(2));
    assert_eq!(bare.stdout(), "");
    let help = run_from_home(&home, None, &["--help"], None).assert_ok();
    assert_eq!(bare.stderr().lines().next(), help.stdout().lines().next());
}

#[test]
fn zsh_completions_include_the_cli_commands() {
    let dir = tempfile::tempdir().unwrap();
    run_from_home(
        &dir.path().join("home"),
        None,
        &["completions", "zsh"],
        None,
    )
    .assert_ok()
    .says("#compdef neb")
    .says("capture");
}

/// The script is the same for every corpus and never JSON, so `--root` and
/// `--json` would be ignored: each is refused as a usage error, on either
/// side of the verb, with nothing on stdout (STD-01 §R28, §R36). The ambient
/// `$NEBULA_ROOT` is not a flag the caller typed, and is not read.
#[test]
fn completions_refuses_json_and_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let nowhere = dir.path().join("nowhere");
    let after = ["completions", "bash", "--root", nowhere.to_str().unwrap()];
    for (root, args) in [
        (None, ["--json", "completions", "bash"].as_slice()),
        (None, &["completions", "bash", "--json"]),
        (Some(nowhere.as_path()), &["completions", "bash"]),
        (None, &after),
    ] {
        let run = run_from_home(&home, root, args, None);
        assert_eq!(
            run.out.status.code(),
            Some(2),
            "{}: {}",
            run.args,
            run.stderr()
        );
        assert_eq!(run.stdout(), "", "{}", run.args);
        assert!(
            run.stderr().contains("cannot be used with"),
            "{}",
            run.stderr()
        );
    }
    assert!(!nowhere.exists());

    run_from_home(&home, None, &["completions", "bash"], Some(&nowhere))
        .assert_ok()
        .says("neb");
    assert!(!nowhere.exists());
}

/// The command names listed under `heading:` in a rendered help page, up to
/// the next blank-line-separated heading.
fn listed_commands(help: &str, headings: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    let mut listing = false;
    for line in help.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            listing = headings.contains(&line.trim_end_matches(':'));
            continue;
        }
        if listing
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && let Some(name) = line.split_whitespace().next()
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every help page the built binary renders: the top level, each command
/// it lists, each `config` setting, and the hidden `open`.
fn every_help_page(c: &Corpus) -> Vec<(String, String)> {
    let top = c.run(&["--help"]).assert_ok().stdout();
    let sections = [
        "Corpus",
        "Inbox",
        "Nodes",
        "References",
        "Query",
        "Maintenance",
    ];
    let mut paths: Vec<Vec<String>> = listed_commands(&top, &sections)
        .into_iter()
        .map(|name| vec![name])
        .collect();
    let config = c.run(&["config", "--help"]).assert_ok().stdout();
    paths.extend(
        listed_commands(&config, &["Commands"])
            .into_iter()
            // Clap's generated help dispatcher takes command names, not --help.
            .filter(|name| name != "help")
            .map(|name| vec!["config".to_owned(), name]),
    );
    paths.push(vec!["open".to_owned()]);
    assert!(paths.len() >= 30, "{paths:?}");
    let mut pages = vec![("neb".to_owned(), top)];
    for path in paths {
        let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
        args.push("--help");
        pages.push((
            format!("neb {}", path.join(" ")),
            c.run(&args).assert_ok().stdout(),
        ));
    }
    pages
}

/// Every word shaped like an Observatory record id: Q, H, T or R and three
/// digits, standing alone.
fn record_ids(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| {
            w.len() == 4
                && w.starts_with(['Q', 'H', 'T', 'R'])
                && w[1..].bytes().all(|b| b.is_ascii_digit())
        })
        .map(str::to_owned)
        .collect()
}

/// A command whose use is not obvious ends its help with examples (STD-01
/// §R22), written with placeholders.
#[test]
fn help_has_examples_for_non_obvious_commands() {
    let c = Corpus::new();
    for verb in [
        "promote", "triage", "link", "cite", "handoff", "near", "trace",
    ] {
        let help = c.run(&[verb, "--help"]).assert_ok().stdout();
        let Some((_, examples)) = help.split_once("\nExamples:\n") else {
            panic!("`neb {verb} --help` has no Examples:\n{help}");
        };
        assert!(
            examples
                .lines()
                .filter(|l| !l.trim().is_empty())
                .all(|l| l.contains(&format!("neb {verb}"))),
            "each example runs `neb {verb}`:\n{examples}"
        );
    }
}

/// No help page and no refusal names a real record: the examples use
/// placeholders (STD-01 §R23).
#[test]
fn no_real_record_ids_in_help_or_refusals() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for (page, help) in every_help_page(&c) {
        assert_eq!(record_ids(&help), Vec::<String>::new(), "`{page} --help`");
    }
    for args in [
        vec!["cite", &id, "--uri", "/abs/x", "--note", "n"],
        vec!["handoff", &id, "nonsense"],
    ] {
        let run = c.run(&args).assert_fails();
        assert_eq!(
            record_ids(&run.stderr()),
            Vec::<String>::new(),
            "{}",
            run.stderr()
        );
        let mut with_json = vec!["--json"];
        with_json.extend_from_slice(&args);
        let refused = c.run(&with_json).usage_refusal().to_string();
        assert_eq!(record_ids(&refused), Vec::<String>::new(), "{refused}");
    }
    assert_eq!(
        record_ids("Q002, H012 and `T123`"),
        ["Q002", "H012", "T123"]
    );
    assert!(record_ids("Q<nnn> Q0021 XQ002 q002").is_empty());
}

/// Help reads the same on every machine: clap is built without `wrap_help`
/// and `color`, so neither the width nor the terminal changes a byte
/// (STD-01 §R17).
#[test]
fn help_does_not_depend_on_columns() {
    let c = Corpus::new();
    for args in [["list", "--help"].as_slice(), &["--help"]] {
        let narrow = c.run_with_env(args, &[("COLUMNS", "50")]).assert_ok();
        let wide = c.run_with_env(args, &[("COLUMNS", "200")]).assert_ok();
        let plain = c.run_with_env(args, &[("NO_COLOR", "")]).assert_ok();
        assert_eq!(narrow.stdout(), wide.stdout(), "{args:?}");
        assert_eq!(plain.stdout(), wide.stdout(), "{args:?}");
        assert!(!wide.stdout().contains('\x1b'), "{args:?}");
    }
}
