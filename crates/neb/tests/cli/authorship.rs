//! Authorship: who wrote each field, agent kills that wait for a human, and
//! corpora written before authorship existed.

use crate::harness::{
    Corpus, date_days_ago, set_created, set_updated, snapshot_corpus_files, write,
};

/// Who wrote what is stored per field, and the human is stored by omission:
/// a corpus written by hand looks exactly as it did before this existed.
#[test]
fn an_unattributed_write_is_the_humans_and_leaves_the_file_alone() {
    let c = Corpus::new();
    let parent = c.seed("a parent idea", "A parent idea");
    let id = c.seed("an idea worth keeping", "An idea worth keeping");
    c.run(&["sharpen", &id, "--kill", "if X never happens"])
        .assert_ok();
    c.run(&["link", &id, "derives-from", &parent]).assert_ok();
    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "why"])
        .assert_ok();
    c.run(&["note", &id, "my own reasoning"]).assert_ok();

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        !raw.contains("title_by") && !raw.contains("kill_by") && !raw.contains("by:"),
        "the human is stored by omission:\n{raw}"
    );
    let today = date_days_ago(0);
    assert!(
        raw.contains(&format!("- {today}: my own reasoning")),
        "the human's note line names no author:\n{raw}"
    );

    // `show --json` states the default rather than making a reader know it.
    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["title_by"], "human");
    assert_eq!(v["node"]["kill_by"], "human");
    assert_eq!(v["node"]["edges"][0]["by"], "human");
    assert_eq!(v["node"]["references"][0]["by"], "human");
    assert_eq!(v["notes"][0]["by"], "human");

    let listed = c.run(&["--json", "list"]).assert_ok().stdout();
    let nodes: Vec<serde_json::Value> = serde_json::from_str(&listed).expect("list --json");
    assert!(nodes.iter().all(|n| n["title_by"] == "human"));

    // Nothing here is the agent's, so review has nothing to confirm.
    let report = c.run(&["review"]).assert_ok().stdout();
    let section = report
        .split("## Agent-authored kills not yet confirmed by a human")
        .nth(1)
        .expect("the section is always printed");
    assert!(section.trim_start().starts_with("_none_"), "{report}");
}

/// `--by` is free text — a session id, a crew name, anything the writer
/// answers to — and lands on the field that write authored, not on the node.
/// `show` opens with a header that labels what it prints: the status and id,
/// the title with its author when that is not the human, the tags, and the
/// dates. Edges and references name an agent author the same way.
#[test]
fn show_header_labels_tags_and_dates_and_names_an_agent_author() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    c.run(&["new", "Human idea"]).assert_ok();
    c.run(&[
        "new",
        "Agent idea",
        "--tag",
        "physics",
        "--tag",
        "wake",
        "--parent",
        "human-idea",
        "--kill",
        "if Y",
        "--by",
        by,
    ])
    .assert_ok();
    c.run(&[
        "cite",
        "agent-idea",
        "--uri",
        "http://example.com",
        "--note",
        "n",
        "--by",
        by,
    ])
    .assert_ok();
    set_created(&c.node_file("agent-idea"), "2026-01-02");
    set_updated(&c.node_file("agent-idea"), "2026-02-03");

    let shown = c.run(&["show", "agent-idea"]).assert_ok().stdout();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "hypothesis agent-idea", "{shown}");
    assert_eq!(lines[1], format!("Agent idea ({by})"), "{shown}");
    assert_eq!(lines[2], "tags: physics, wake", "{shown}");
    assert_eq!(
        lines[3], "created: 2026-01-02  updated: 2026-02-03",
        "{shown}"
    );
    assert_eq!(lines[4], "", "{shown}");
    assert!(shown.contains(&format!("kill: if Y ({by})")), "{shown}");
    assert!(
        shown.contains(&format!("  derives-from   human-idea ({by})")),
        "{shown}"
    );
    assert!(
        shown.contains(&format!("http://example.com ({by})")),
        "{shown}"
    );

    // The human's own node names nobody, and has no tags to label.
    let shown = c.run(&["show", "human-idea"]).assert_ok().stdout();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "seed       human-idea", "{shown}");
    assert_eq!(lines[1], "Human idea", "{shown}");
    assert!(lines[2].starts_with("created: "), "{shown}");
    assert!(
        !shown.contains("(human)") && !shown.contains("tags:"),
        "{shown}"
    );
}

#[test]
fn by_records_the_author_of_each_field_it_wrote() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    let parent = c.seed("a parent idea", "A parent idea");

    let entry = c.run(&["capture", "the human's own words"]).stdout_trim();
    c.run(&["promote", &entry, "--by", by, "--parent", &parent])
        .assert_ok();
    let promoted = std::fs::read_to_string(c.node_file("the-human-s-own-words")).unwrap();
    assert!(
        !promoted.contains("title_by"),
        "a capture promoted as captured is titled in the human's words:\n{promoted}"
    );
    assert!(
        promoted.contains(&format!("to: {parent}\n  by: {by}")),
        "the parent edge was the agent's call:\n{promoted}"
    );

    let entry = c.run(&["capture", "another thought"]).stdout_trim();
    c.run(&[
        "promote",
        &entry,
        "--title",
        "A title the agent wrote",
        "--by",
        by,
    ])
    .assert_ok();
    let retitled = std::fs::read_to_string(c.node_file("a-title-the-agent-wrote")).unwrap();
    assert!(
        retitled.contains(&format!("title_by: {by}")),
        "a title the agent wrote is the agent's:\n{retitled}"
    );

    c.run(&["new", "A node the agent made", "--kill", "if Y", "--by", by])
        .assert_ok();
    let made = std::fs::read_to_string(c.node_file("a-node-the-agent-made")).unwrap();
    assert!(
        made.contains(&format!("title_by: {by}")) && made.contains(&format!("kill_by: {by}")),
        "{made}"
    );

    c.run(&[
        "link",
        "a-node-the-agent-made",
        "contradicts",
        &parent,
        "--by",
        by,
    ])
    .assert_ok();
    let both = std::fs::read_to_string(c.node_file(&parent)).unwrap();
    assert!(
        both.contains(&format!("by: {by}")),
        "a contradiction is recorded on both ends, by whoever claimed it:\n{both}"
    );

    c.run(&[
        "cite",
        "a-node-the-agent-made",
        "--uri",
        "https://example.org",
        "--note",
        "the agent found this",
        "--by",
        by,
    ])
    .assert_ok();
    c.run(&["note", "--by", by, "a-node-the-agent-made", "its reasoning"])
        .assert_ok();

    let raw = std::fs::read_to_string(c.node_file("a-node-the-agent-made")).unwrap();
    let today = date_days_ago(0);
    assert!(
        raw.contains(&format!("- {today} ({by}): its reasoning")),
        "a note carries its author in the line:\n{raw}"
    );

    let out = c
        .run(&["--json", "show", "a-node-the-agent-made"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["title_by"], by);
    assert_eq!(v["node"]["kill_by"], by);
    assert_eq!(v["node"]["edges"][0]["by"], by);
    assert_eq!(v["node"]["references"][0]["by"], by);
    assert_eq!(v["notes"][0]["by"], by);

    // Orbit's provenance answers a different question and is untouched by it.
    c.run(&["new", "Run provenance", "--task", "ORB-1", "--by", by])
        .assert_ok();
    let orbit = std::fs::read_to_string(c.node_file("run-provenance")).unwrap();
    assert!(
        orbit.contains(&format!("title_by: {by}")) && orbit.contains("origin:\n  task: ORB-1"),
        "{orbit}"
    );

    // A label that would make a note line ambiguous is refused outright.
    c.run(&["note", "--by", "crew (alpha)", "run-provenance", "x"])
        .assert_fails()
        .says("cannot be an author label");
}

/// A label that cannot be an author has its own code, not the catch-all.
#[test]
fn invalid_author_label_has_its_own_code() {
    let c = Corpus::new();
    let refused = c
        .run(&["--json", "new", "t", "--by", "a (b)"])
        .usage_refusal();
    assert_eq!(refused["code"], "invalid_author_label", "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains("`a (b)`"),
        "{refused}"
    );
    assert_eq!(std::fs::read_dir(c.root.join("nodes")).unwrap().count(), 0);
}

/// The point of recording authorship: a kill condition the agent proposed is
/// not yet the human's claim, and `review` says so until one is confirmed.
#[test]
fn an_agent_kill_stays_on_review_until_a_human_confirms_it() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    let id = c.seed("a sharpenable idea", "A sharpenable idea");
    c.run(&[
        "sharpen",
        &id,
        "--kill",
        "if the corpus stays small",
        "--by",
        by,
    ])
    .assert_ok();

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).expect("review --json");
    let found = items
        .iter()
        .find(|i| i["rule"] == "unconfirmed-kill")
        .unwrap_or_else(|| panic!("no unconfirmed-kill finding in {json}"));
    assert_eq!(found["id"], id.as_str());
    assert!(found["reason"].as_str().unwrap().contains(by), "{json}");
    c.run(&["review"])
        .assert_ok()
        .says("## Agent-authored kills not yet confirmed by a human")
        .says(&id);
    c.run(&["show", &id]).assert_ok().says(&format!("({by})"));

    // Confirming changes the author and nothing else.
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["sharpen", &id, "--confirm"]).assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        after.contains("kill: if the corpus stays small"),
        "the kill text is untouched:\n{after}"
    );
    assert!(!after.contains("kill_by"), "{after}");
    assert_eq!(
        before.replace(&format!("kill_by: {by}\n"), ""),
        after,
        "confirming appends nothing and rewrites nothing else"
    );

    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["kill_by"], "human");
    assert_eq!(v["node"]["status"], "hypothesis");
    assert!(v["notes"].as_array().is_none_or(Vec::is_empty), "{out}");

    // A confirmed falsifier is human-owned content, not a field an agent can
    // quietly replace by sharpening the already-open hypothesis again.
    let confirmed = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let refused = c
        .run(&[
            "sharpen",
            &id,
            "--kill",
            "if an agent prefers a different test",
            "--by",
            by,
        ])
        .assert_fails();
    assert!(refused.stdout().is_empty(), "{}", refused.stdout());
    assert!(
        refused
            .stderr()
            .contains("kill condition is already `if the corpus stays small`; it was not replaced"),
        "{}",
        refused.stderr()
    );
    assert!(
        !refused.stderr().contains("is now hypothesis"),
        "a refused re-sharpen must not report a status transition: {}",
        refused.stderr()
    );
    assert_eq!(
        confirmed,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "a refused re-sharpen leaves the confirmed falsifier untouched"
    );

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).expect("review --json");
    assert!(
        !items.iter().any(|i| i["rule"] == "unconfirmed-kill"),
        "a confirmed kill is off the list: {json}"
    );

    // Confirming what nobody wrote is a refusal, not a silent no-op.
    let seed = c.seed("an unsharpened idea", "An unsharpened idea");
    c.run(&["sharpen", &seed, "--confirm"])
        .assert_fails()
        .says("no kill condition to confirm");
    // `--confirm` is not a way to rewrite the text.
    c.run(&["sharpen", &id, "--kill", "something else", "--confirm"])
        .assert_fails()
        .says("cannot be used with");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// A v2 node as it was written before authorship existed: no `title_by`, no
/// `kill_by`, no `by` on the edge, the reference or the note.
const PRE_AUTHORSHIP_NODE: &str = r"---
id: written-by-hand
title: Written by hand
status: hypothesis
created: 2026-09-01
updated: 2026-09-01
kill: if nobody ever writes one
edges:
- type: derives-from
  to: also-by-hand
references:
- id: r1
  kind: article
  uri: https://example.org
  note: why it is here
  added: 2026-09-01
---

the argument

## Notes

- 2026-09-02: an older note
";

const PRE_AUTHORSHIP_PARENT: &str = r"---
id: also-by-hand
title: Also by hand
status: seed
created: 2026-09-01
updated: 2026-09-01
---

the older argument
";

/// A node written before authorship existed loads as the human's own, and
/// `neb migrate` has nothing to do about it either way.
#[test]
fn a_corpus_written_before_authorship_loads_unchanged() {
    let c = Corpus::new();
    write(&c.node_file("written-by-hand"), PRE_AUTHORSHIP_NODE);
    write(&c.node_file("also-by-hand"), PRE_AUTHORSHIP_PARENT);
    // One authored node beside them, so migration has both shapes to keep.
    c.run(&["new", "An authored node", "--by", "agent:crew-alpha"])
        .assert_ok();

    c.run(&["check"]).assert_ok().says("0 errors");
    let out = c
        .run(&["--json", "show", "written-by-hand"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    for field in ["title_by", "kill_by"] {
        assert_eq!(v["node"][field], "human", "{field} in {out}");
    }
    assert_eq!(v["node"]["edges"][0]["by"], "human");
    assert_eq!(v["node"]["references"][0]["by"], "human");
    assert_eq!(v["notes"][0]["by"], "human");
    let report = c.run(&["review", "--json"]).assert_ok().stdout();
    assert!(
        !report.contains("unconfirmed-kill"),
        "a kill nobody attributed is the human's own: {report}"
    );

    let before = snapshot_corpus_files(&c.root);
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(
        before,
        snapshot_corpus_files(&c.root),
        "migration is a no-op: absent authorship reads as human, and an \
         authored node keeps its labels"
    );
}
