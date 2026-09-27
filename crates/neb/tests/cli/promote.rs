//! `neb promote` and the capture payloads around it: suggested parents,
//! minted ids and the JSON shape of the created node.

use crate::harness::Corpus;
use crate::near::lexical_fixture;

#[test]
fn capture_json_carries_the_entry_and_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let json = c
        .run(&["capture", "--json", "a taxonomy of tags"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["entry"]["text"], "a taxonomy of tags", "{json}");
    assert_eq!(out["entry"]["id"].as_str().unwrap().len(), 4, "{json}");
    assert!(out["entry"]["at"].is_string(), "{json}");
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(near[0]["score"].is_number(), "{json}");
    assert!(near[0]["band"].is_string(), "{json}");

    let json = c
        .run(&[
            "capture",
            "--json",
            "--quiet",
            "a taxonomy of tags, quietly",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        out["near"],
        serde_json::json!([]),
        "quiet empties it: {json}"
    );
    assert!(out["entry"]["id"].is_string(), "{json}");
}

#[test]
fn promote_without_a_parent_suggests_and_proceeds_as_a_root() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "search ranking decays with age"])
        .assert_ok()
        .stdout_trim();

    let out = c.run(&["promote", &entry]).assert_ok().stdout();
    let mut lines = out.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("search-ranking-decays-with-age "),
        "id and path first: {out}"
    );
    assert_eq!(lines.next(), Some("near:"), "{out}");
    let rest: Vec<&str> = lines.collect();
    assert_eq!(rest, [rest[0]], "one node shares a word: {out}");
    assert!(rest[0].contains("ranking-decay-half-life"), "{out}");

    // Promoted as a root: no parent, no edge, whatever was suggested.
    let raw = std::fs::read_to_string(c.node_file("search-ranking-decays-with-age")).unwrap();
    assert!(!raw.contains("edges:"), "suggesting never links:\n{raw}");
    let trace = c
        .run(&["trace", "--json", "search-ranking-decays-with-age"])
        .assert_ok()
        .stdout();
    let walk: serde_json::Value = serde_json::from_str(&trace).unwrap();
    assert_eq!(walk.as_array().unwrap().len(), 1, "{trace}");
    assert!(walk[0]["parents"].as_array().unwrap().is_empty(), "{trace}");

    // --quiet: the id and path alone.
    let entry = c
        .run(&["capture", "-q", "ranking decay, quietly"])
        .assert_ok()
        .stdout_trim();
    let out = c.run(&["promote", "--quiet", &entry]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");

    // A parent named is a decision made: nothing to suggest.
    let entry = c
        .run(&["capture", "-q", "ranking decay, parented"])
        .assert_ok()
        .stdout_trim();
    let out = c
        .run(&["promote", &entry, "--parent", "ranking-decay-half-life"])
        .assert_ok()
        .stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
}

/// A capture promoted without `--title` or `--id` is titled with the whole
/// sentence but takes a short id. A different thought colliding with it
/// falls back to a longer id from its own text; the same thought again is
/// refused, as it always was.
#[test]
fn promote_without_title_or_id_mints_a_short_id_from_a_long_capture() {
    let c = Corpus::new();
    let text = "gravity might be a scarcity gradient in some shared resource";
    let short = "gravity-scarcity-gradient-shared-resource";
    let full = "gravity-might-be-a-scarcity-gradient-in-some-shared-resource";

    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, short);
    let raw = std::fs::read_to_string(c.node_file(short)).unwrap();
    assert!(raw.contains(&format!("id: {short}\n")), "{raw}");
    assert!(raw.contains(&format!("title: {text}\n")), "{raw}");

    // The same sentence again: already a node, so refused by that node's
    // id rather than duplicated under a fallback, and the capture waits.
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    c.run(&["promote", "-q", &entry])
        .assert_fails()
        .says(&format!("node `{short}` already exists"));
    assert!(c.run(&["inbox"]).assert_ok().stdout().contains(&entry));
    assert!(!c.node_file(full).exists(), "no duplicate node");

    // A different thought sharing the first five significant words falls
    // back to one more word, and the node holding the short id is untouched.
    let entry = c
        .run(&["capture", "-q", &format!("{text} pool")])
        .assert_ok()
        .stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, format!("{short}-pool"));
    assert_eq!(
        std::fs::read_to_string(c.node_file(short)).unwrap(),
        raw,
        "the node holding the short id is untouched"
    );

    // `--title` and `--id` decide the id exactly as they always have.
    let entry = c
        .run(&["capture", "-q", "a thought"])
        .assert_ok()
        .stdout_trim();
    let id = c
        .run(&[
            "promote",
            "-q",
            &entry,
            "--title",
            "Gravity might be a scarcity gradient in a shared pool",
        ])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "gravity-might-be-a-scarcity-gradient-in-a-shared-pool");
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c
        .run(&["promote", "-q", &entry, "--id", "scarcity-gravity"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "scarcity-gravity");
}

#[test]
fn promote_json_is_the_created_node_with_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "decay of a taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&["promote", "--json", &entry, "--title", "Taxonomies decay"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["doc"]["node"]["id"], "taxonomies-decay", "{json}");
    assert_eq!(out["doc"]["body"], "decay of a taxonomy", "{json}");
    assert!(
        out["path"]
            .as_str()
            .unwrap()
            .ends_with("taxonomies-decay.md"),
        "{json}"
    );
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(
        near.iter().all(|n| n["id"] != "taxonomies-decay"),
        "a promotion is not its own neighbour: {json}"
    );
    assert_eq!(out["doc"]["node"]["edges"], serde_json::json!([]), "{json}");

    // With a parent, `near` is empty: present, never omitted.
    let entry = c
        .run(&["capture", "-q", "another taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&[
            "promote",
            "--json",
            &entry,
            "--parent",
            "a-single-global-taxonomy",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["near"], serde_json::json!([]), "{json}");
}
