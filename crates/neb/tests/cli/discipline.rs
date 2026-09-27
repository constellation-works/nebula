//! Status discipline: kill conditions, refuting and abandoning, reopening a
//! refuted idea, and the edges `neb new` writes with a node.

use crate::harness::{Corpus, write};

#[test]
fn a_hypothesis_must_name_what_would_kill_it() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("kill condition");
    c.run(&[
        "sharpen",
        &id,
        "--kill",
        "if the effect vanishes under control",
    ])
    .assert_ok();
    c.run(&["show", &id])
        .assert_ok()
        .says("hypothesis")
        .says("vanishes under control");
}

#[test]
fn refuting_needs_a_reason_and_writes_the_closed_block() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["status", &id, "refuted"])
        .assert_fails()
        .says("refuted needs --why");
    c.run(&["status", &id, "refuted", "--why", "  "])
        .assert_fails()
        .says("refuted needs --why");
    c.run(&["status", &id, "refuted", "--why", "X happened"])
        .assert_ok();

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("closed:\n  why: X happened\n  at: "), "{raw}");
    c.run(&["show", &id]).assert_ok().says("X happened");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // The reason is the invariant, not the flag: strip it by hand and
    // `check` catches the gap as rule 5.
    write(
        &c.node_file(&id),
        &raw.replace("  why: X happened\n", "  why: ''\n"),
    );
    c.run(&["check"])
        .assert_fails()
        .says("[5]")
        .says("closed.why is empty");

    // A seed cannot be refuted: there is no kill condition to have fired.
    let seed = c.seed("another idea", "Another idea");
    c.run(&["status", &seed, "refuted", "--why", "no"])
        .assert_fails()
        .says("kill condition");
    // And --why means nothing on an open status.
    c.run(&["status", &seed, "hypothesis", "--why", "no"])
        .assert_fails()
        .says("a reason only applies to refuted or abandoned")
        .says("Drop --why");
}

#[test]
fn abandoning_takes_an_optional_reason_and_reviving_clears_it() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&["status", &bare, "abandoned"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&bare)).unwrap();
    assert!(!raw.contains("closed:"), "{raw}");

    let reasoned = c.seed("another idea", "Another idea");
    c.run(&["status", &reasoned, "abandoned", "--why", "lost interest"])
        .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&reasoned)).unwrap();
    assert!(raw.contains("why: lost interest"), "{raw}");

    // Abandoned is not a verdict, so it can come back, and the closed block
    // goes with it. Its kill is not part of a recorded firing either, so
    // sharpening is allowed.
    c.run(&["sharpen", &bare, "--kill", "if Y"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&bare)).unwrap();
    assert!(raw.contains("kill: if Y"), "{raw}");
    assert!(raw.contains("status: abandoned"), "{raw}");
    c.run(&["status", &reasoned, "seed"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&reasoned)).unwrap();
    assert!(!raw.contains("closed:"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_node_with_a_kill_condition_cannot_go_back_to_seed() {
    let c = Corpus::new();
    c.run(&["new", "X", "--id", "x", "--kill", "k"]).assert_ok();

    // hypothesis -> seed would keep the kill, since nothing is deleted, and
    // `check` would then blame a hand edit for what the tool did.
    let before = std::fs::read_to_string(c.node_file("x")).unwrap();
    c.run(&["status", "x", "seed"])
        .assert_fails()
        .says("`x` names a kill condition, so it cannot go back to seed")
        .says("neb status x hypothesis");
    let after = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert_eq!(before, after, "a refused move leaves the node untouched");

    // abandoned -> seed is refused the same way.
    c.run(&["status", "x", "abandoned"]).assert_ok();
    let before = std::fs::read_to_string(c.node_file("x")).unwrap();
    c.run(&["status", "x", "seed"])
        .assert_fails()
        .says("cannot go back to seed")
        .says("neb status x hypothesis");
    let after = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert_eq!(before, after, "a refused move leaves the node untouched");

    // The remedy works, keeps the kill, and leaves the corpus check-clean.
    c.run(&["status", "x", "hypothesis"])
        .assert_ok()
        .says("abandoned -> hypothesis");
    let raw = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert!(raw.contains("status: hypothesis"), "{raw}");
    assert!(raw.contains("kill: k"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

#[test]
fn a_refuted_idea_cannot_quietly_come_back() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["status", &id, "refuted", "--why", "X happened"])
        .assert_ok();

    // Reviving it takes a new node with a `reopens` edge, so the fact that it
    // was once ruled out stays visible in the graph. Not even abandoning it
    // is allowed: refuted is final.
    let refused = c
        .run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("cannot simply reopen");
    let hint = reopen_hint(&refused.stderr(), &id);
    c.run(&["status", &id, "abandoned"])
        .assert_fails()
        .says("cannot simply reopen");

    // Even refuted -> refuted is refused: a verdict is part of the record,
    // and a second `--why` would silently overwrite the first one's reason
    // and date rather than leaving the recorded verdict alone.
    let before_reverdict = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["status", &id, "refuted", "--why", "second verdict"])
        .assert_fails()
        .says("cannot simply reopen");
    let after_reverdict = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(
        before_reverdict, after_reverdict,
        "a refused refuted -> refuted leaves the node untouched"
    );
    assert!(
        after_reverdict.contains("why: X happened"),
        "{after_reverdict}"
    );

    // The kill is part of the verdict: rewriting it would leave closed.why
    // describing a falsifier the node no longer names. `--confirm` is the
    // same write path with different flags.
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["sharpen", &id, "--kill", "a different falsifier"])
        .assert_fails()
        .says(&format!("`{id}` is refuted"));
    c.run(&["sharpen", &id, "--confirm"])
        .assert_fails()
        .says(&format!("`{id}` is refuted"));
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(before, after, "a refused sharpen leaves the node untouched");
    assert!(after.contains("kill: if X"), "{after}");
    assert!(after.contains("why: X happened"), "{after}");
    c.run(&["check"]).assert_ok().says("0 errors");

    // The hint runs as printed once the placeholder title is filled in: no
    // id to copy out of one command's output into the next.
    let mut revive: Vec<&str> = hint.iter().map(String::as_str).collect();
    revive[1] = "Second attempt";
    revive.extend(["--kill", "if Y"]);
    c.run(&revive).assert_ok().says("second-attempt");
    let edges = show_edges(&c, "second-attempt");
    assert_eq!(edges, [("reopens".to_string(), id.clone())], "{edges:?}");
    c.run(&["show", "second-attempt"])
        .assert_ok()
        .says("hypothesis");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// The command the refused-reopen hint offers, as `neb` arguments, with the
/// title placeholder second. Asserts that the hint is one
/// command, not a chain, and that it names the refuted node.
fn reopen_hint(stderr: &str, id: &str) -> Vec<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("neb "))
        .unwrap_or_else(|| panic!("no command in the hint:\n{stderr}"));
    assert!(!line.contains("&&") && !line.contains('<'), "{line}");
    let words = shlex::split(line).expect("the hint parses as shell words");
    assert_eq!(words, ["neb", "new", "...", "--reopens", id], "{line}");
    words[1..].to_vec()
}

/// A node's edges as `(type, to)` pairs, read through `show --json`.
fn show_edges(c: &Corpus, id: &str) -> Vec<(String, String)> {
    let shown: serde_json::Value =
        serde_json::from_str(&c.run(&["show", id, "--json"]).assert_ok().stdout()).unwrap();
    shown["node"]["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("no edges in {shown}"))
        .iter()
        .map(|e| {
            (
                e["type"].as_str().unwrap().to_string(),
                e["to"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn new_writes_reopens_and_contradicts_edges_with_the_node() {
    let c = Corpus::new();
    let dead = c.seed("an idea", "An idea");
    c.run(&["sharpen", &dead, "--kill", "if X"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "X happened"])
        .assert_ok();
    let rival = c.seed("a rival", "A rival");
    let files = || std::fs::read_dir(c.root.join("nodes")).unwrap().count();

    // Every refusal comes before anything is written.
    for (args, says) in [
        (vec!["--reopens", "absent"], "no node `absent`"),
        (vec!["--contradicts", "absent"], "no node `absent`"),
        (
            vec!["--parent", &dead, "--reopens", &dead],
            "named as both a parent and the node this reopens",
        ),
        (
            vec!["--contradicts", &rival, "--contradicts", &rival],
            &format!("the edge `take-two` contradicts `{rival}` already exists"),
        ),
    ] {
        let mut argv = vec!["new", "Take two"];
        argv.extend(args);
        c.run(&argv).assert_fails().says(says);
        assert_eq!(files(), 2, "`neb {}` wrote nothing", argv.join(" "));
    }
    c.run(&["new", "Take two", "--parent", &dead, "--reopens", &dead])
        .assert_fails()
        .says(&format!("drop `--parent {dead}`"));

    c.run(&[
        "new",
        "Take two",
        "--reopens",
        &dead,
        "--contradicts",
        &rival,
    ])
    .assert_ok();
    assert_eq!(
        show_edges(&c, "take-two"),
        [
            ("reopens".to_string(), dead.clone()),
            ("contradicts".to_string(), rival.clone())
        ]
    );
    // `contradicts` is a claim about both nodes, as `link` records it.
    assert_eq!(
        show_edges(&c, &rival),
        [("contradicts".to_string(), "take-two".to_string())]
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn new_refuses_a_reopens_edge_that_closes_a_loop() {
    let c = Corpus::new();
    let dead = c.seed("an idea", "An idea");
    // A hand edit left a dangling edge to a node that does not exist yet.
    let path = c.node_file(&dead);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen(
            "status: seed\n",
            "status: seed\nedges:\n- type: derives-from\n  to: take-two\n",
            1,
        ),
    );
    c.run(&["new", "Take two", "--reopens", &dead])
        .assert_fails()
        .says("its own ancestor");
    assert!(!c.node_file("take-two").exists());
}
