//! The JSON payloads: every documented verb emits machine-readable JSON, the
//! node payload's fields, and write payloads that match the read shape.

use crate::handoff::observatory_with_h012;
use crate::harness::{Corpus, run_from_home};

#[test]
fn json_preserves_stored_terminal_controls() {
    let c = Corpus::new();
    let title = "Escape\u{1b}[31m title";
    let body = "first line\nsecond \u{1b}[2J line";
    let by = "agent:\u{1b}[32m";
    let note = "note \u{1b}[0m text";
    c.run(&["new", title, "--id", "escape", "--body", body, "--by", by])
        .assert_ok();
    c.run(&[
        "cite",
        "escape",
        "--kind",
        "discussion",
        "--note",
        note,
        "--by",
        by,
    ])
    .assert_ok();

    let shown: serde_json::Value =
        serde_json::from_str(&c.run(&["show", "--json", "escape"]).assert_ok().stdout()).unwrap();
    assert_eq!(shown["node"]["title"], title);
    assert_eq!(shown["node"]["title_by"], by);
    assert_eq!(shown["body"], body);
    assert_eq!(shown["node"]["references"][0]["note"], note);
    assert_eq!(shown["node"]["references"][0]["by"], by);

    let trace: serde_json::Value =
        serde_json::from_str(&c.run(&["trace", "--json", "escape"]).assert_ok().stdout()).unwrap();
    assert_eq!(trace[0]["title"], title);
}

#[test]
#[allow(clippy::too_many_lines)] // One contract matrix is easier to audit than split verb lists.
fn every_documented_json_verb_emits_machine_readable_json() {
    let init_dir = tempfile::tempdir().unwrap();
    let init_home = init_dir.path().join("home");
    let init_root = init_dir.path().join("corpus");
    let initialized = run_from_home(&init_home, Some(&init_root), &["init", "--json"], None)
        .assert_ok()
        .stdout();
    let initialized: serde_json::Value =
        serde_json::from_str(&initialized).expect("init --json is valid JSON");
    assert_eq!(initialized["root"], init_root.display().to_string());

    let c = Corpus::new();
    let json = |args: &[&str]| {
        let out = c.run(args).assert_ok().stdout();
        serde_json::from_str::<serde_json::Value>(&out)
            .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
    };

    json(&["migrate", "--json"]);
    json(&["config", "observatory-root", "--json"]);
    json(&["config", "commit", "--json"]);

    let captured = json(&["capture", "--json", "an idea to promote"]);
    let promote_entry = captured["entry"]["id"].as_str().unwrap();
    json(&["inbox", "--json"]);
    let promoted = json(&[
        "promote",
        "--json",
        promote_entry,
        "--title",
        "Promoted idea",
    ]);
    assert_eq!(promoted["doc"]["node"]["id"], "promoted-idea");
    assert!(promoted["path"].is_string());

    let captured = json(&["capture", "--json", "an idea to drop"]);
    let drop_entry = captured["entry"]["id"].as_str().unwrap();
    let dropped = json(&["drop", "--json", drop_entry]);
    assert_eq!(dropped["id"], drop_entry);
    assert_eq!(dropped["text"], "an idea to drop");

    let created = json(&["new", "--json", "Direct idea", "--tag", "design"]);
    assert_eq!(created["doc"]["node"]["id"], "direct-idea");
    assert!(created["path"].is_string());

    json(&["new", "--json", "Idea to sharpen"]);
    let sharpened = json(&[
        "sharpen",
        "--json",
        "idea-to-sharpen",
        "--kill",
        "the evidence changes",
        "--by",
        "agent:test",
    ]);
    assert_eq!(sharpened["node"]["status"], "hypothesis");
    assert_eq!(sharpened["node"]["kill"], "the evidence changes");
    assert_eq!(sharpened["node"]["kill_by"], "agent:test");
    let confirmed = json(&["sharpen", "--json", "idea-to-sharpen", "--confirm"]);
    assert_eq!(confirmed["node"]["kill"], "the evidence changes");
    assert_eq!(confirmed["node"]["kill_by"], "human");

    json(&["new", "--json", "Idea to close"]);
    let changed = json(&[
        "status",
        "--json",
        "idea-to-close",
        "abandoned",
        "--why",
        "superseded",
    ]);
    assert_eq!(changed["from"], "seed");
    assert_eq!(changed["doc"]["node"]["status"], "abandoned");

    json(&["new", "--json", "Parent idea"]);
    json(&["new", "--json", "Child idea"]);
    let linked = json(&[
        "link",
        "--json",
        "child-idea",
        "derives-from",
        "parent-idea",
    ]);
    assert_eq!(linked.as_array().unwrap().len(), 1);
    assert_eq!(linked[0]["node"]["edges"][0]["to"], "parent-idea");

    let tagged = json(&["tag", "--json", "direct-idea", "--add", "corpus"]);
    assert_eq!(
        tagged["node"]["tags"],
        serde_json::json!(["design", "corpus"])
    );
    json(&["tag", "list", "--json"]);

    let noted = json(&["note", "--json", "direct-idea", "supporting detail"]);
    assert_eq!(noted["notes"][0]["text"], "supporting detail");
    let cited = json(&[
        "cite",
        "--json",
        "direct-idea",
        "--kind",
        "article",
        "--uri",
        "https://example.org/evidence",
        "--note",
        "supporting evidence",
    ]);
    assert_eq!(cited["reference"], "r1");
    assert_eq!(cited["doc"]["node"]["references"][0]["kind"], "article");

    let shown = json(&["show", "--json", "direct-idea"]);
    assert_eq!(shown["node"]["id"], "direct-idea");
    assert!(json(&["list", "--json"]).is_array());
    let near = json(&["near", "--json", "direct design"]);
    assert!(near["items"].is_array() && near["total"].is_u64(), "{near}");
    assert!(near["truncated"].is_boolean(), "{near}");
    assert!(json(&["trace", "--json", "child-idea"]).is_array());
    json(&["impact", "--json", "parent-idea"]);
    json(&["graph", "--json"]);
    assert!(json(&["inbox", "--json"]).is_array());
    assert!(json(&["review", "--json"]).is_array());
    assert!(json(&["review", "--short", "--json"]).is_array());

    let checked = json(&["check", "--json"]);
    assert!(checked["nodes"].as_u64().unwrap() >= 6);
}

/// A JSON object's keys, sorted as `serde_json` keeps them.
pub(super) fn keys(v: &serde_json::Value) -> Vec<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("an object: {v}"))
        .keys()
        .cloned()
        .collect()
}

/// Every key of a node, whichever verb serialised it (STD-01 §R10, §R11).
pub(super) const NODE_KEYS: [&str; 13] = [
    "closed",
    "created",
    "edges",
    "id",
    "kill",
    "kill_by",
    "origin",
    "references",
    "status",
    "tags",
    "title",
    "title_by",
    "updated",
];

/// An absent value is `null` and an empty collection `[]` on every node a
/// `--json` verb returns, and a write, a `show` and a `list` agree on the
/// key set.
#[test]
fn json_node_payload_has_every_field_with_null_for_absent() {
    let c = Corpus::new();
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap()
    };
    let created = json(&["new", "--json", "X"])["doc"]["node"].clone();
    let shown = json(&["show", "--json", "x"])["node"].clone();
    let listed = json(&["list", "--json"])[0].clone();
    for node in [&created, &shown, &listed] {
        assert_eq!(keys(node), NODE_KEYS, "{node}");
        for list in ["tags", "edges", "references"] {
            assert_eq!(node[list], serde_json::json!([]), "`{list}`: {node}");
        }
        for absent in ["kill", "kill_by", "closed", "origin"] {
            assert!(node[absent].is_null(), "`{absent}`: {node}");
        }
        assert_eq!(node["title_by"], "human", "{node}");
    }
}

/// Every write verb's node is the shape `show` gives, authors stated: a
/// field written without `--by` says `human` rather than leaving it out.
#[test]
fn json_write_payloads_match_the_read_shape() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap().to_owned();
    let json = |args: &[&str]| -> serde_json::Value {
        let out = c
            .run_with_env(args, &[("OBSERVATORY_ROOT", &obs)])
            .assert_ok()
            .stdout();
        serde_json::from_str(&out)
            .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
    };
    // The node a write returned, against what `show` says of it now.
    let same_as_show = |verb: &str, node: &serde_json::Value| {
        let id = node["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{verb}: {node}"));
        let shown = json(&["show", "--json", id])["node"].clone();
        assert_eq!(keys(node), keys(&shown), "`{verb}` vs `show {id}`: {node}");
        assert_eq!(keys(node), NODE_KEYS, "`{verb}`: {node}");
        assert_eq!(node["title_by"], "human", "`{verb}`: {node}");
        for list in ["edges", "references"] {
            for item in node[list].as_array().unwrap() {
                assert_eq!(item["by"], "human", "`{verb}` {list}: {node}");
            }
        }
    };

    json(&["new", "--json", "Parent"]);
    let created = json(&["new", "--json", "Child", "--parent", "parent"]);
    assert_eq!(created["doc"]["node"]["edges"][0]["by"], "human");
    same_as_show("new", &created["doc"]["node"]);

    let entry = json(&["capture", "--json", "-q", "a promoted thought"])["entry"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let promoted = json(&["promote", "--json", &entry, "--parent", "parent"]);
    assert_eq!(promoted["doc"]["node"]["edges"][0]["by"], "human");
    same_as_show("promote", &promoted["doc"]["node"]);

    let changed = json(&["status", "--json", "child", "abandoned", "--why", "moot"]);
    same_as_show("status", &changed["doc"]["node"]);

    json(&["new", "--json", "Linked"]);
    let linked = json(&["link", "--json", "linked", "derives-from", "parent"]);
    assert_eq!(linked[0]["node"]["edges"][0]["by"], "human");
    same_as_show("link", &linked[0]["node"]);

    let tagged = json(&["tag", "--json", "linked", "--add", "t"]);
    same_as_show("tag", &tagged["node"]);

    let sharpened = json(&["sharpen", "--json", "parent", "--kill", "if not"]);
    assert_eq!(sharpened["node"]["kill_by"], "human");
    same_as_show("sharpen", &sharpened["node"]);

    let cited = json(&[
        "cite",
        "--json",
        "parent",
        "--uri",
        "https://example.org",
        "--note",
        "why",
    ]);
    assert_eq!(cited["doc"]["node"]["references"][0]["by"], "human");
    same_as_show("cite", &cited["doc"]["node"]);

    let handed = json(&["handoff", "--json", "linked", "H012", "--note", "n"]);
    assert_eq!(handed["doc"]["node"]["references"][0]["by"], "human");
    same_as_show("handoff", &handed["doc"]["node"]);
}

/// Optionals nested inside a node, and beside it in a payload, are `null`
/// or `[]` rather than left out.
#[test]
fn json_nested_optionals_are_null_not_omitted() {
    let c = Corpus::new();
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap()
    };

    json(&["new", "--json", "Plain"]);
    let cited = json(&["cite", "--json", "plain", "--uri", "https://example.org"]);
    let reference = &cited["doc"]["node"]["references"][0];
    assert_eq!(
        keys(reference),
        [
            "added", "by", "id", "kind", "note", "origin", "title", "uri"
        ],
        "{reference}"
    );
    for absent in ["title", "note", "origin"] {
        assert!(reference[absent].is_null(), "`{absent}`: {reference}");
    }

    let tasked = json(&["new", "--json", "Tasked", "--task", "T"]);
    assert_eq!(
        tasked["doc"]["node"]["origin"],
        serde_json::json!({
            "task": "T",
            "workspace": null,
            "run": null,
            "artifact": null,
            "agent": null,
            "at": null,
        })
    );

    let shown = json(&["show", "--json", "tasked"]);
    assert_eq!(
        keys(&shown),
        ["body", "handed_off_to", "node", "notes", "observatory"],
        "{shown}"
    );
    assert_eq!(shown["notes"], serde_json::json!([]), "{shown}");
    assert_eq!(shown["observatory"], serde_json::json!([]), "{shown}");
    assert!(shown["handed_off_to"].is_null(), "{shown}");

    // No observatory root on this machine, so the record cannot resolve.
    c.run(&[
        "cite",
        "plain",
        "--kind",
        "observatory",
        "--uri",
        "H012",
        "--note",
        "n",
    ])
    .assert_ok();
    let shown = json(&["show", "--json", "plain"]);
    assert_eq!(
        shown["observatory"],
        serde_json::json!([{"reference": "r2", "record": "H012", "path": null}]),
        "{shown}"
    );

    let walk = json(&["trace", "--json", "tasked"]);
    assert!(walk[0]["handed_off_to"].is_null(), "{walk}");
    assert!(walk[0].get("handed_off_to").is_some(), "{walk}");

    let captured = json(&["capture", "--json", "-q", "plain again"]);
    assert_eq!(captured["near"], serde_json::json!([]), "{captured}");
    let entry = captured["entry"]["id"].as_str().unwrap();
    let promoted = json(&["promote", "--json", entry, "-q"]);
    assert_eq!(promoted["near"], serde_json::json!([]), "{promoted}");
}

#[test]
fn an_empty_corpus_is_valid() {
    let c = Corpus::new();
    c.run(&["check"]).assert_ok().says("0 nodes, 0 errors");
    c.run(&["list"]).assert_ok().says("no nodes match");
}

#[test]
fn round_tripping_a_node_preserves_prose_and_fields() {
    let c = Corpus::new();
    let id = c.seed("the original thought", "The original thought");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["cite", &id, "--uri", "http://example.com", "--note", "why"])
        .assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        after.contains("the original thought"),
        "prose survives a write:\n{after}"
    );
    assert!(
        before.contains("id: the-original-thought") && after.contains("id: the-original-thought")
    );
}
