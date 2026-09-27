//! `neb note`: notes appended in order and shown with the node.

use crate::harness::{Corpus, corpus_repo, date_days_ago, log, set_updated};

#[test]
fn note_appends_in_order_and_show_exposes_them() {
    let c = Corpus::new();
    let id = c.seed("the original thought", "The original thought");
    c.run(&["tag", &id, "--add", "physics"]).assert_ok();
    let parent = c.seed("a parent idea", "A parent idea");
    c.run(&["link", &id, "derives-from", &parent]).assert_ok();
    set_updated(&c.node_file(&id), &date_days_ago(7));

    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["note", &id, "first thought"]).assert_ok();
    c.run(&["note", &id, "second thought"]).assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();

    let today = date_days_ago(0);
    assert!(
        after.contains("the original thought"),
        "the capture is still there:\n{after}"
    );
    let first = format!("- {today}: first thought");
    let second = format!("- {today}: second thought");
    let first_at = after.find(&first).expect("first note");
    let second_at = after.find(&second).expect("second note");
    assert!(first_at < second_at, "notes accumulate in order:\n{after}");
    assert!(after.contains("## Notes"), "{after}");
    assert!(
        after.contains("status: seed") && after.contains("tags:\n- physics"),
        "status and tags stay put:\n{after}"
    );
    assert!(
        after.contains(&format!("to: {parent}")),
        "edges stay put:\n{after}"
    );
    assert!(
        after.contains(&format!("updated: {today}")),
        "updated is bumped:\n{after}"
    );
    assert!(
        before.contains(&format!("updated: {}", date_days_ago(7))),
        "the stamp really moved from a past date"
    );

    c.run(&["show", &id])
        .assert_ok()
        .says("the original thought")
        .says("## Notes")
        .says(&first)
        .says(&second);

    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["notes"][0]["text"], "first thought");
    assert_eq!(v["notes"][1]["text"], "second thought");
    assert_eq!(v["notes"][0]["at"], today);
    assert_eq!(v["node"]["status"], "seed");
    assert_eq!(v["node"]["tags"][0], "physics");

    let noted = c
        .run(&["--json", "note", &id, "third thought"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&noted).expect("note --json is valid JSON");
    assert_eq!(v["notes"].as_array().map(Vec::len), Some(3));
    assert_eq!(v["notes"][2]["text"], "third thought");

    c.run(&["note", "nope", "lost"])
        .assert_fails()
        .says("no node `nope`");
}

/// `--no-commit` after the reasoning is a flag, not more text; `--quiet` is
/// not a note flag, so it is refused rather than written into the body.
#[test]
fn note_trailing_no_commit_is_a_flag_and_quiet_is_refused() {
    let (c, _remote) = corpus_repo();
    let id = c.seed("the original thought", "The original thought");
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = log(&c.root).len();
    c.run(&["note", &id, "a thought", "--no-commit"])
        .assert_ok();
    assert_eq!(log(&c.root).len(), before, "trailing --no-commit skips git");
    let body = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let today = date_days_ago(0);
    assert!(
        body.contains(&format!("- {today}: a thought")),
        "the note should land without the flag:\n{body}"
    );
    assert!(
        !body.contains("--no-commit"),
        "the flag must not land in the body:\n{body}"
    );
    c.run(&["note", &id, "a thought", "--quiet"])
        .assert_fails()
        .says("--quiet");
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(body, after, "a refused --quiet must not write");
}
