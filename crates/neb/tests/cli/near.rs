//! `neb near` and the neighbours capture prints: ranking, links already
//! made, limits and their refusals.

use crate::harness::{Corpus, corpus_repo, log, snapshot_corpus_files, write};

/// Nodes whose vocabulary overlaps in known ways, so a query has one right
/// answer and one wrong one.
pub(super) fn lexical_fixture(c: &Corpus) {
    c.run(&[
        "new",
        "Tags beat domains",
        "--tag",
        "design",
        "--tag",
        "corpus",
        "--kill",
        "a corpus of 50+ nodes needs a query that tags cannot answer",
    ])
    .assert_ok();
    c.run(&["new", "A single global taxonomy", "--tag", "design"])
        .assert_ok();
    c.run(&["new", "Ranking decay half-life", "--tag", "search"])
        .assert_ok();
    c.run(&["new", "Proper time is a count", "--tag", "physics"])
        .assert_ok();
}

/// The ids in `near --json`'s envelope, in order.
fn ids(json: &str) -> Vec<String> {
    let out: serde_json::Value = serde_json::from_str(json).unwrap();
    out["items"]
        .as_array()
        .unwrap_or_else(|| panic!("an envelope's items: {json}"))
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn near_ranks_existing_nodes_against_free_text() {
    let c = Corpus::new();
    lexical_fixture(&c);

    let json = c
        .run(&[
            "near", "--json", "tags", "and", "domains", "beat", "a", "taxonomy",
        ])
        .assert_ok()
        .stdout();
    assert_eq!(
        ids(&json),
        ["tags-beat-domains", "a-single-global-taxonomy"],
        "{json}"
    );
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["total"], 2, "{json}");
    assert_eq!(out["truncated"], false, "{json}");
    let out = &out["items"];
    let first = &out[0];
    assert_eq!(first["title"], "Tags beat domains");
    assert_eq!(first["status"], "hypothesis");
    assert_eq!(first["tags"], serde_json::json!(["design", "corpus"]));
    let score = first["score"].as_f64().unwrap();
    assert!(score > 0.0 && score <= 1.0, "{json}");
    assert!(
        score >= out[1]["score"].as_f64().unwrap(),
        "best first: {json}"
    );
    // The raw score stays in JSON, beside the band a human is shown; free
    // text is no node, so nothing is linked to it.
    for n in out.as_array().unwrap() {
        assert!(
            ["strong", "some", "weak"].contains(&n["band"].as_str().unwrap()),
            "{json}"
        );
        assert!(n["linked"].is_null(), "present, and null: {json}");
    }
    assert_eq!(first["band"], "strong", "{json}");

    // Text mode, piped: one line per neighbour, band first, the best on
    // top, and no bare number to misread.
    let text = c
        .run(&["near", "tags and domains beat a taxonomy"])
        .assert_ok()
        .stdout();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("\ttags-beat-domains\t"), "{text}");
    assert!(lines[1].contains("\ta-single-global-taxonomy\t"), "{text}");
    assert!(
        lines[0].starts_with("strong\t"),
        "band leads the line: {text}"
    );
    let band = out[1]["band"].as_str().unwrap();
    assert!(lines[1].starts_with(band), "the JSON's band: {text}");
    assert!(!text.contains("0."), "no raw score in text: {text}");

    // A limit caps the answer.
    let json = c
        .run(&["near", "--json", "--limit", "1", "tags taxonomy ranking"])
        .assert_ok()
        .stdout();
    assert_eq!(ids(&json).len(), 1, "{json}");
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["truncated"], true, "{json}");
}

#[test]
fn near_takes_a_node_id_and_never_returns_that_node() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let json = c
        .run(&["near", "--json", "tags-beat-domains"])
        .assert_ok()
        .stdout();
    let found = ids(&json);
    assert!(!found.contains(&"tags-beat-domains".to_string()), "{json}");
    assert_eq!(
        found.first().map(String::as_str),
        Some("a-single-global-taxonomy"),
        "{json}"
    );
}

/// Asked about a node, `near` says which neighbours are already linked to
/// it and by what, in text and in JSON, and still writes nothing.
#[test]
fn near_marks_candidates_already_linked_to_the_node() {
    let c = Corpus::new();
    lexical_fixture(&c);
    c.run(&[
        "new",
        "Tags beat domains in corpus design",
        "--parent",
        "tags-beat-domains",
    ])
    .assert_ok();
    c.run(&[
        "link",
        "tags-beat-domains",
        "contradicts",
        "a-single-global-taxonomy",
    ])
    .assert_ok();
    let before = snapshot_corpus_files(&c.root);

    let text = c.run(&["near", "tags-beat-domains"]).assert_ok().stdout();
    let line = |id: &str| {
        text.lines()
            .find(|l| l.contains(id))
            .unwrap_or_else(|| panic!("{id} in: {text}"))
            .to_string()
    };
    assert!(
        line("tags-beat-domains-in-corpus-design").ends_with("\tchild (derives-from)"),
        "{text}"
    );
    assert!(
        line("a-single-global-taxonomy").ends_with("\tcontradicts"),
        "one mark for a contradiction stored on both ends: {text}"
    );

    let text = c
        .run(&["near", "tags-beat-domains-in-corpus-design"])
        .assert_ok()
        .stdout();
    let parent = text
        .lines()
        .find(|l| l.contains("\ttags-beat-domains\t"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(parent.ends_with("\tparent (derives-from)"), "{text}");
    let unlinked = text
        .lines()
        .find(|l| l.contains("a-single-global-taxonomy"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(unlinked.ends_with("\t-"), "nothing linked is `-`: {text}");

    let json = c
        .run(&["near", "--json", "tags-beat-domains-in-corpus-design"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    let by_id = |id: &str| {
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("{id} in: {json}"))
            .clone()
    };
    assert_eq!(
        by_id("tags-beat-domains")["linked"],
        serde_json::json!([{
            "from": "tags-beat-domains-in-corpus-design",
            "type": "derives-from",
            "to": "tags-beat-domains",
        }]),
        "{json}"
    );
    assert!(
        by_id("a-single-global-taxonomy")["linked"].is_null(),
        "{json}"
    );

    // A suggestion only: marking a link is a read, and nothing changed.
    assert_eq!(before, snapshot_corpus_files(&c.root));
}

#[test]
fn near_says_so_when_nothing_matches() {
    let c = Corpus::new();
    lexical_fixture(&c);
    c.run(&["near", "quantum gravity"])
        .assert_ok()
        .says("nothing near");
    let json = c
        .run(&["near", "--json", "quantum", "gravity"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        out,
        serde_json::json!({"items": [], "total": 0, "truncated": false})
    );
    c.run(&["near", "   "])
        .assert_fails()
        .says("nothing to look near");
}

/// `near` always has a limit, so `--json` is always the envelope: the
/// neighbours kept, how many shared a word with the query, and whether `-k`
/// cut any. A cut is named on stderr in every mode.
#[test]
fn near_json_reports_total_and_truncated() {
    let c = Corpus::new();
    for i in 0..150 {
        let id = format!("lantern-{i:03}");
        write(
            &c.node_file(&id),
            &format!(
                "---\nid: {id}\ntitle: Lantern number {i}\nstatus: seed\n\
                 created: 2026-09-01\nupdated: 2026-09-01\n---\n"
            ),
        );
    }
    c.run(&["new", "Unrelated idea"]).assert_ok();

    let cut = c.run(&["near", "--json", "lantern"]).assert_ok();
    let out: serde_json::Value = serde_json::from_str(&cut.stdout()).unwrap();
    assert_eq!(out["items"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(out["total"], 150, "{out}");
    assert_eq!(out["truncated"], true, "{out}");
    assert!(
        cut.stderr().contains("3 of 150 shown; raise -k for more"),
        "{}",
        cut.stderr()
    );
    let text = c.run(&["near", "lantern"]).assert_ok();
    assert!(text.stderr().contains("3 of 150"), "{}", text.stderr());
    assert!(!text.stdout().contains("3 of 150"), "{}", text.stdout());

    let whole = c
        .run(&["near", "--json", "lantern", "-k", "500"])
        .assert_ok();
    let out: serde_json::Value = serde_json::from_str(&whole.stdout()).unwrap();
    assert_eq!(out["truncated"], false, "{out}");
    assert_eq!(out["total"], 150, "{out}");
    assert_eq!(out["items"].as_array().unwrap().len(), 150, "{out}");
    assert_eq!(whole.stderr(), "", "no notice for a whole answer");
}

/// `--limit` after the query is a flag; `--quiet` is not a near flag, and
/// `near` writes nothing so `--no-commit` is not one either: both are refused
/// rather than folded into the text.
#[test]
fn near_trailing_limit_is_a_flag_and_quiet_and_no_commit_are_refused() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let trailing = c
        .run(&["near", "--json", "one global taxonomy", "--limit", "1"])
        .assert_ok()
        .stdout();
    let leading = c
        .run(&["near", "--json", "--limit", "1", "one global taxonomy"])
        .assert_ok()
        .stdout();
    assert_eq!(
        trailing, leading,
        "trailing --limit must not join the query"
    );
    for flag in ["--quiet", "--no-commit"] {
        c.run(&["near", "one global taxonomy", flag])
            .assert_fails()
            .says(flag);
    }
}

/// `-k 0` would answer "nothing near" for a query with a strong match: a
/// false empty answer. It is a usage error under either spelling.
#[test]
fn near_k_zero_is_a_usage_error() {
    let c = Corpus::new();
    lexical_fixture(&c);
    for flag in ["-k", "--limit"] {
        for json in [false, true] {
            let json = if json { ["--json"].as_slice() } else { &[] };
            let zero = c.run(&[json, &["near", flag, "0", "one global taxonomy"]].concat());
            assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
            assert_eq!(zero.stdout(), "", "`neb {}`", zero.args);
            // clap names the flag by its long form, whichever was typed.
            assert!(
                zero.stderr().contains("'--limit <K>'") && zero.stderr().contains("at least 1"),
                "`neb {}`: {}",
                zero.args,
                zero.stderr()
            );
        }
        c.run(&["near", flag, "1", "one global taxonomy"])
            .assert_ok();
    }
}

#[test]
fn capture_prints_the_nearest_nodes_after_the_id_unless_quiet() {
    let c = Corpus::new();
    lexical_fixture(&c);

    let out = c
        .run(&["capture", "a global taxonomy for tags"])
        .assert_ok()
        .stdout();
    let mut lines = out.lines();
    let id = lines.next().unwrap();
    assert_eq!(id.len(), 4, "the entry id alone on the first line: {out}");
    assert_eq!(lines.next(), Some("near:"), "{out}");
    let rest: Vec<&str> = lines.collect();
    assert_eq!(rest.len(), 2, "two nodes share a word, so two lines: {out}");
    assert!(rest[0].contains("a-single-global-taxonomy"), "{out}");
    assert!(rest[1].contains("tags-beat-domains"), "{out}");

    // The capture landed, and nothing else changed: no node, no edge.
    c.run(&["inbox"])
        .assert_ok()
        .says("a global taxonomy for tags");
    let graph = c.run(&["graph", "--json"]).assert_ok().stdout();
    let out: serde_json::Value = serde_json::from_str(&graph).unwrap();
    assert_eq!(out["nodes"].as_array().unwrap().len(), 4, "{graph}");
    assert!(out["edges"].as_array().unwrap().is_empty(), "{graph}");

    // --quiet: the id and nothing else.
    let out = c
        .run(&["capture", "--quiet", "a taxonomy again"])
        .assert_ok()
        .stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
    let out = c.run(&["capture", "-q", "tags again"]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");

    // Nothing near: the id alone, with no empty heading under it.
    let out = c.run(&["capture", "quantum gravity"]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
}

/// `--quiet` and `--no-commit` after the thought are flags, not more text.
#[test]
fn capture_trailing_quiet_and_no_commit_are_flags() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let out = c
        .run(&["capture", "an idea", "--quiet"])
        .assert_ok()
        .stdout();
    assert_eq!(
        out.lines().count(),
        1,
        "trailing --quiet still quiets: {out}"
    );
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    let inbox: serde_json::Value = serde_json::from_str(&json).unwrap();
    let texts: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    assert!(texts.contains(&"an idea"), "{json}");
    assert!(
        texts.iter().all(|t| !t.contains("--quiet")),
        "the flag must not land in the inbox: {json}"
    );

    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = log(&c.root).len();
    c.run(&["capture", "kept out of git", "--no-commit"])
        .assert_ok();
    assert_eq!(log(&c.root).len(), before, "trailing --no-commit skips git");
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    assert!(json.contains("kept out of git"), "{json}");
    assert!(!json.contains("--no-commit"), "{json}");
}

#[test]
fn capture_prints_its_id_before_a_node_that_will_not_parse_can_get_in_the_way() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\nverdict: supports"),
    );

    // The write lands and the id is printed; the suggestions are what fail,
    // after it, and they are a side channel: said on stderr, never the
    // capture's exit status. Nothing about the capture depended on `nodes/`
    // parsing, and it must not read as a lost thought.
    let run = c.run(&["capture", "another idea"]);
    let out = run.stdout();
    assert_eq!(out.lines().count(), 1, "the id, alone: {out}");
    assert_eq!(out.trim().len(), 4, "{out}");
    run.assert_ok()
        .says("suggestions unavailable")
        .says("unknown field `verdict`");
    c.run(&["inbox"]).assert_ok().says("another idea");

    // --quiet never reads `nodes/`, so it does not even see the problem.
    c.run(&["capture", "-q", "quietly"]).assert_ok();
}
