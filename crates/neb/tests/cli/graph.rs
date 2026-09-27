//! `neb graph` exports as JSON and Mermaid, and Orbit provenance on the
//! nodes they carry.

use crate::harness::Corpus;

#[test]
fn graph_exports_the_whole_corpus_as_json() {
    let c = Corpus::new();
    let base = c.seed("base", "Base");
    let child = c.seed("child", "Child");
    c.run(&["link", &child, "derives-from", &base]).assert_ok();
    let rival = c.seed("rival", "Rival");
    c.run(&["link", &base, "contradicts", &rival]).assert_ok();

    let json = c.run(&["graph", "--json"]).assert_ok().stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    let nodes = out["nodes"].as_array().unwrap();
    let edges = out["edges"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{json}");
    assert!(
        nodes
            .iter()
            .any(|n| n["id"] == base && n["status"] == "seed")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == child && e["type"] == "derives-from" && e["to"] == base),
        "{json}"
    );
    // `contradicts` is written on both ends, so it is exported from both.
    assert_eq!(
        edges.iter().filter(|e| e["type"] == "contradicts").count(),
        2,
        "{json}"
    );

    let run = c.run(&["graph", "--from", "base", "--json"]);
    assert_eq!(run.out.status.code(), Some(2));
    run.assert_fails()
        .says("the argument '--from <ID>' cannot be used with '--json'");

    // `--json` is global, so it may come before the verb too.
    let before = c.run(&["--json", "graph"]).assert_ok().stdout();
    assert_eq!(before, json);
}

/// There is no default text form, so `graph` without a format is refused as
/// a usage error, on stderr like every other (STD-01 §R19, §R20).
#[test]
fn graph_without_a_format_is_a_usage_error_on_stderr() {
    let c = Corpus::new();
    c.seed("base", "Base");
    let run = c.run(&["graph"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    let stderr = run.stderr();
    assert_eq!(
        stderr,
        "error: neb graph needs an output format; pass --json or --mermaid\n"
    );
    for format in ["--json", "--mermaid"] {
        assert!(stderr.contains(format), "{stderr}");
    }
}

/// `--json` is a global flag, so `graph`'s formats conflict with it on either
/// side of the verb: the same usage error, exit 2, and nothing on stdout.
#[test]
fn graph_formats_refuse_json_before_or_after_the_verb() {
    let c = Corpus::new();
    let base = c.seed("base", "Base");

    for (before, after, says) in [
        (
            vec!["--json", "graph", "--mermaid"],
            vec!["graph", "--mermaid", "--json"],
            "the argument '--mermaid' cannot be used with '--json'",
        ),
        (
            vec!["--json", "graph", "--mermaid", "--from", &base],
            vec!["graph", "--mermaid", "--from", &base, "--json"],
            "the argument '--mermaid' cannot be used with '--json'",
        ),
        (
            vec!["--json", "graph", "--from", &base, "--mermaid"],
            vec!["graph", "--from", &base, "--mermaid", "--json"],
            "the argument '--from <ID>' cannot be used with '--json'",
        ),
    ] {
        let first = c.run(&before);
        let second = c.run(&after);
        for run in [&first, &second] {
            assert_eq!(run.out.status.code(), Some(2), "{}", run.args);
            assert_eq!(run.stdout(), "", "{}", run.args);
            assert!(
                run.stderr().starts_with(&format!("error: {says}\n")),
                "{}: {}",
                run.args,
                run.stderr()
            );
            assert!(
                run.stderr().contains("\nUsage: neb graph "),
                "{}",
                run.stderr()
            );
        }
        assert_eq!(first.stderr(), second.stderr(), "{}", first.args);
    }

    // A refused `--from` is a usage error before the verb too, never a
    // lookup of the node under `--json`.
    let run = c.run(&["--json", "graph", "--mermaid", "--from", "nope"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert!(!run.stderr().contains("NoSuchNode"), "{}", run.stderr());

    // `--no-commit` before the verb is not global, so it is no conflict
    // either: `graph` never commits, and that is what refuses it.
    let run = c.run(&["--no-commit", "graph", "--mermaid"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    assert!(
        run.stderr().contains("`graph` never commits"),
        "{}",
        run.stderr()
    );
}

#[test]
fn graph_exports_mermaid_and_limits_it_to_lineage() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "A \"quoted\" [ancestor] & <source>",
        "--id",
        "ancestor",
    ])
    .assert_ok();
    c.run(&[
        "new", "Focus", "--id", "focus", "--parent", "ancestor", "--kill", "evidence",
    ])
    .assert_ok();
    c.run(&[
        "new",
        "Descendant",
        "--id",
        "descendant",
        "--parent",
        "focus",
        "--kill",
        "evidence",
    ])
    .assert_ok();
    c.run(&[
        "status",
        "descendant",
        "refuted",
        "--why",
        "evidence arrived",
    ])
    .assert_ok();
    c.run(&["new", "Sibling", "--id", "sibling", "--parent", "ancestor"])
        .assert_ok();
    c.run(&["status", "sibling", "abandoned"]).assert_ok();
    c.run(&["new", "Unrelated", "--id", "unrelated"])
        .assert_ok();
    c.run(&["link", "ancestor", "contradicts", "focus"])
        .assert_ok();

    let whole = c.run(&["graph", "--mermaid"]).assert_ok().stdout();
    assert!(whole.starts_with("graph BT\n"), "{whole}");
    assert!(
        whole.contains(
            "ancestor[\"A &quot;quoted&quot; &#91;ancestor&#93; &amp; &lt;source&gt;\"]:::seed"
        ),
        "{whole}"
    );
    for status in ["seed", "hypothesis", "refuted", "abandoned"] {
        assert!(whole.contains(&format!("  classDef {status} ")), "{whole}");
    }
    assert_eq!(whole.matches("|contradicts|").count(), 1, "{whole}");
    assert_eq!(
        whole.lines().filter(|line| line.contains(":::")).count(),
        5,
        "{whole}"
    );
    assert_eq!(
        whole.lines().filter(|line| line.contains("| ")).count(),
        4,
        "{whole}"
    );

    let lineage = c
        .run(&["graph", "--mermaid", "--from", "focus"])
        .assert_ok()
        .stdout();
    for id in ["ancestor", "focus", "descendant"] {
        assert!(lineage.contains(&format!("  {id}[")), "{lineage}");
    }
    assert!(!lineage.contains("  sibling["), "{lineage}");
    assert!(!lineage.contains("  unrelated["), "{lineage}");
    assert_eq!(
        lineage.lines().filter(|line| line.contains(":::")).count(),
        3,
        "{lineage}"
    );
    assert_eq!(
        lineage.lines().filter(|line| line.contains("| ")).count(),
        3,
        "{lineage}"
    );
}

/// An unknown `--from` is core's `NoSuchNode`, with its message and hint,
/// rather than a code the CLI made up.
#[test]
fn mermaid_from_an_unknown_node_is_no_such_node() {
    let c = Corpus::new();
    c.run(&["new", "Something", "--id", "something"])
        .assert_ok();
    let run = c
        .run(&["graph", "--mermaid", "--from", "nope"])
        .assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(run.stdout(), "");
    run.says("no node `nope`").says("neb list");
}

#[test]
fn orbit_provenance_is_recorded_only_when_supplied() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "Orbit-produced idea",
        "--task",
        "ORB-12345",
        "--run",
        "jrun-20260907-0001",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("orbit-produced-idea")).unwrap();
    assert!(raw.contains("origin:\n  task: ORB-12345\n  run: jrun-20260907-0001"));

    c.run(&["new", "Unattributed idea"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file("unattributed-idea")).unwrap();
    assert!(!raw.contains("origin:"));

    let cited = c.seed("context", "Context");
    c.run(&[
        "cite",
        &cited,
        "--uri",
        "https://example.com",
        "--task",
        "ORB-12345",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&cited)).unwrap();
    assert!(raw.contains("references:"));
    assert!(raw.contains("task: ORB-12345"));

    let entry = c
        .run(&["capture", "promoted with provenance"])
        .stdout_trim();
    c.run(&[
        "promote",
        &entry,
        "--title",
        "Promoted with provenance",
        "--run",
        "jrun-20260907-0002",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("promoted-with-provenance")).unwrap();
    assert!(raw.contains("run: jrun-20260907-0002"));
}
