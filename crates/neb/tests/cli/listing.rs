//! Listings: piped output, `--limit` and its notices, JSON envelopes, and
//! empty results.

use crate::harness::{
    Corpus, date_days_ago, git_commit_at, git_init, set_created, set_inbox_stamp_for, set_updated,
    snapshot_corpus_files, stamp_days_ago,
};
use crate::json::{NODE_KEYS, keys};

/// Piped, a listing is one tab-separated line per node with no header, and
/// every line has every field: an untagged node's tags are `-`, so `cut -f`
/// never shifts (STD-01 §R9).
#[test]
fn piped_list_is_tab_separated() {
    let c = Corpus::new();
    c.run(&["new", "A seed", "--tag", "physics", "--tag", "design"])
        .assert_ok();
    c.run(&["new", "A hypothesis", "--kill", "if X"])
        .assert_ok();
    c.run(&["new", "Untagged idea"]).assert_ok();
    let out = c.run(&["list"]).assert_ok().stdout();
    let rows: Vec<Vec<&str>> = out.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 3, "one line per node, no header: {out}");
    assert!(rows.iter().all(|r| r.len() == 4), "{out}");
    assert!(!out.contains("STATUS") && !out.contains('\x1b'), "{out}");
    let row = |id: &str| {
        rows.iter()
            .find(|r| r[1] == id)
            .unwrap_or_else(|| panic!("{id} in: {out}"))
    };
    assert_eq!(
        row("a-seed")[..],
        ["seed", "a-seed", "physics,design", "A seed"]
    );
    assert_eq!(
        row("a-hypothesis")[..3],
        ["hypothesis", "a-hypothesis", "-"]
    );
    assert_eq!(
        row("untagged-idea")[..],
        ["seed", "untagged-idea", "-", "Untagged idea"]
    );
}

/// Every other list-shaped verb pipes as `list` does: one tab-separated line
/// per record, the same number of fields on every line, and no header.
#[test]
fn piped_list_shaped_outputs_are_tab_separated() {
    let c = Corpus::new();
    // A hypothesis fourteen days old with no references, so `review --short`
    // has a finding, beside nodes with and without tags.
    let hyp = c.seed("gravity carries information", "Gravity carries information");
    c.run(&["sharpen", &hyp, "--kill", "if X"]).assert_ok();
    c.run(&["tag", &hyp, "--add", "physics"]).assert_ok();
    set_created(&c.node_file(&hyp), &date_days_ago(14));
    c.run(&[
        "new",
        "Gravity is information",
        "--tag",
        "physics",
        "--tag",
        "zz",
    ])
    .assert_ok();
    c.run(&["new", "Untagged gravity idea", "--parent", &hyp])
        .assert_ok();
    c.run(&["capture", "--quiet", "a thought about gravity"])
        .assert_ok();
    c.run(&["capture", "--quiet", "another thought"])
        .assert_ok();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "first");
    c.run(&["note", &hyp, "a second thought"]).assert_ok();
    git_commit_at(&c.root, "2020-01-02", "second");

    for (args, header, fields, records) in [
        (vec!["inbox"], "ID", 3, 2),
        (vec!["near", "gravity information"], "BAND", 5, 3),
        (vec!["near", hyp.as_str()], "BAND", 5, 2),
        (vec!["tag", "list"], "TAG", 2, 2),
        (vec!["review", "--short"], "ID", 2, 1),
        (vec!["log", hyp.as_str()], "HASH", 3, 2),
    ] {
        let out = c.run(&args).assert_ok().stdout();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), records, "{args:?}: {out}");
        for line in &lines {
            let cut: Vec<&str> = line.split('\t').collect();
            assert_eq!(cut.len(), fields, "{args:?}: {out}");
            assert!(cut.iter().all(|f| !f.is_empty()), "{args:?}: {out}");
            assert_ne!(cut[0], header, "no header: {out}");
        }
        assert!(!out.contains("\x1b["), "{args:?}: {out}");
    }
}

/// `--limit` cuts `list` to its first N matches and says on stderr how many
/// it left out, in every mode; without it every match is listed, as before,
/// and the count is stderr's too. `--json` gets the cut list in the envelope
/// that says so.
#[test]
fn list_limit_bounds_the_listing_and_defaults_to_all() {
    let c = Corpus::new();
    for title in ["One", "Two", "Three"] {
        c.run(&["new", title, "--tag", "t"]).assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();

    let all = c.run(&["list"]).assert_ok();
    assert_eq!(all.stdout().lines().count(), 4, "{}", all.stdout());
    assert_eq!(all.stderr(), "4 of 4 nodes\n");
    let tagged = c.run(&["list", "--tag", "t"]).assert_ok();
    assert_eq!(tagged.stdout().lines().count(), 3, "{}", tagged.stdout());
    assert_eq!(tagged.stderr(), "3 of 4 nodes\n");

    let cut = c.run(&["list", "--tag", "t", "--limit", "2"]).assert_ok();
    assert_eq!(cut.stdout().lines().count(), 2, "{}", cut.stdout());
    assert!(
        cut.stdout().lines().all(|l| l.starts_with("seed")),
        "{}",
        cut.stdout()
    );
    assert_eq!(
        cut.stderr(),
        "2 of 3 matching nodes shown, of 4 in all; raise --limit for more\n"
    );
    let roomy = c.run(&["list", "--tag", "t", "--limit", "3"]).assert_ok();
    assert_eq!(
        (roomy.stdout(), roomy.stderr()),
        (tagged.stdout(), tagged.stderr()),
        "a limit nobody reaches changes nothing"
    );
    let unfiltered = c.run(&["list", "--limit", "1"]).assert_ok();
    assert_eq!(unfiltered.stdout().lines().count(), 1);
    assert_eq!(
        unfiltered.stderr(),
        "1 of 4 nodes shown; raise --limit for more\n"
    );

    let json = c.run(&["list", "--json", "--limit", "1"]).assert_ok();
    let listed: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["total"], 4, "{listed}");
    assert_eq!(
        json.stderr(),
        "1 of 4 nodes shown; raise --limit for more\n"
    );

    let single = Corpus::new();
    single.run(&["new", "Alone"]).assert_ok();
    let one = single.run(&["list"]).assert_ok();
    assert_eq!(one.stdout().lines().count(), 1, "{}", one.stdout());
    assert_eq!(one.stderr(), "1 of 1 node\n");
}

/// Piped, `list` is one line per node and nothing else, so `wc -l` counts
/// nodes; the count of the corpus is on stderr.
#[test]
fn piped_list_has_one_line_per_record() {
    let c = Corpus::new();
    for n in 0..150 {
        c.run(&["new", &format!("Filler {n}"), "--tag", "t", "--no-commit"])
            .assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();
    let listed = c.run(&["list", "--tag", "t"]).assert_ok();
    let out = listed.stdout();
    assert_eq!(out.lines().count(), 150, "{out}");
    assert!(out.lines().all(|l| l.starts_with("seed\t")), "{out}");
    assert_eq!(listed.stderr(), "150 of 151 nodes\n");
}

/// `--limit 0` would print an empty listing, or "nothing needs attention",
/// over a corpus with plenty in it. On every verb that takes it, it is a usage
/// error with nothing on stdout, and the least limit still answers.
#[test]
fn limit_zero_is_a_usage_error() {
    let c = Corpus::new();
    let seed = c.seed("an old seed", "An old seed");
    set_updated(&c.node_file(&seed), &date_days_ago(100));
    c.run(&["capture", "-q", "waiting"]).assert_ok();
    for verb in [
        ["list"].as_slice(),
        &["inbox"],
        &["review"],
        &["review", "--short"],
    ] {
        for json in [false, true] {
            let json = if json { ["--json"].as_slice() } else { &[] };
            let zero = c.run(&[json, verb, &["--limit", "0"]].concat());
            assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
            assert_eq!(zero.stdout(), "", "`neb {}`", zero.args);
            assert!(
                zero.stderr().contains("--limit") && zero.stderr().contains("at least 1"),
                "`neb {}`: {}",
                zero.args,
                zero.stderr()
            );
        }
        let one = c.run(&[verb, &["--limit", "1"]].concat()).assert_ok();
        assert_ne!(one.stdout(), "", "`neb {}`", one.args);
    }
}

/// `--limit` cuts the inbox to its oldest entries. How many wait, and how
/// many the cut left out, are on stderr.
#[test]
fn inbox_limit_shows_the_oldest_and_counts_the_rest() {
    let c = Corpus::new();
    for text in ["first thought", "second thought", "third thought"] {
        c.run(&["capture", "--quiet", text]).assert_ok();
    }
    let all = c.run(&["inbox"]).assert_ok();
    assert_eq!(all.stdout().lines().count(), 3, "{}", all.stdout());
    assert_eq!(all.stderr(), "3 waiting. Promote or drop each one.\n");

    let cut = c.run(&["inbox", "--limit", "2"]).assert_ok();
    let listed = cut.stdout();
    assert_eq!(listed.lines().count(), 2, "{listed}");
    assert!(
        listed.contains("first thought") && listed.contains("second thought"),
        "{listed}"
    );
    assert!(!listed.contains("third thought"), "{listed}");
    assert_eq!(
        cut.stderr(),
        "2 of 3 waiting shown; raise --limit for more. Promote or drop each one.\n"
    );

    let json = c.run(&["inbox", "--json", "--limit", "1"]).assert_ok();
    let listed: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["items"][0]["text"], "first thought", "{listed}");
    assert_eq!(listed["total"], 3, "{listed}");
    assert!(
        json.stderr()
            .starts_with("1 of 3 waiting shown; raise --limit"),
        "{}",
        json.stderr()
    );
    let uncut = c.run(&["inbox", "--json"]).assert_ok();
    assert_eq!(uncut.stderr(), "", "a plain count is for a person");
}

/// `review --limit` keeps the first N findings under each heading, so a
/// crowded section cannot push a short one out, and says what it cut: in the
/// markdown, which is the file `--out` writes, and on stderr. With `--short`
/// it keeps the first N lines, and says so on stderr alone.
#[test]
fn review_limit_cuts_each_section_and_the_short_form() {
    let c = Corpus::new();
    for title in ["Cold one", "Cold two", "Cold three"] {
        c.run(&["new", title]).assert_ok();
    }
    for id in ["cold-one", "cold-two", "cold-three"] {
        set_updated(&c.node_file(id), &date_days_ago(100));
    }
    c.run(&["capture", "--quiet", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    let whole = c.run(&["review"]).assert_ok();
    assert_eq!(whole.stdout().matches("`cold-").count(), 3);
    assert!(!whole.stdout().contains("--limit"), "{}", whole.stdout());
    assert_eq!(whole.stderr(), "");

    let cut = c.run(&["review", "--limit", "1"]).assert_ok();
    let report = cut.stdout();
    assert_eq!(report.matches("`cold-").count(), 1, "{report}");
    assert!(
        report.contains("- _… and 2 more; raise --limit for more_"),
        "{report}"
    );
    assert!(
        report.contains("1 capture waiting over fourteen days"),
        "the inbox section keeps its one finding:\n{report}"
    );
    assert_eq!(
        cut.stderr(),
        "2 findings not shown; raise --limit for more\n"
    );
    let json = c.run(&["review", "--json", "--limit", "1"]).assert_ok();
    let items: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(items["items"].as_array().unwrap().len(), 2, "{items}");
    assert_eq!(items["total"], 4, "every rule's findings: {items}");
    assert_eq!(json.stderr(), cut.stderr(), "said in every mode");

    let short = c.run(&["review", "--short"]).assert_ok();
    assert_eq!(short.stdout().lines().count(), 4, "{}", short.stdout());
    assert_eq!(short.stderr(), "");
    let short_cut = c.run(&["review", "--short", "--limit", "2"]).assert_ok();
    assert_eq!(
        short_cut.stdout().lines().count(),
        2,
        "{}",
        short_cut.stdout()
    );
    assert_eq!(short_cut.stderr(), "2 of 4 shown; raise --limit for more\n");
    let json = c
        .run(&["review", "--short", "--json", "--limit", "2"])
        .assert_ok();
    let items: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(items["items"].as_array().unwrap().len(), 2, "{items}");
    assert_eq!(items["total"], 4, "{items}");
    assert_eq!(json.stderr(), short_cut.stderr(), "said in every mode");
}

/// Run a `--json` command and parse what it printed.
pub(super) fn json_of(c: &Corpus, args: &[&str]) -> serde_json::Value {
    let out = c.run(args).assert_ok().stdout();
    serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
}

/// The STD-01 §R34 envelope: `items` of length `kept`, `total` and
/// `truncated` as given, and no other key.
pub(super) fn assert_envelope(v: &serde_json::Value, kept: usize, total: usize, truncated: bool) {
    assert_eq!(keys(v), ["items", "total", "truncated"], "{v}");
    assert_eq!(v["items"].as_array().unwrap().len(), kept, "{v}");
    assert_eq!(v["total"], total, "{v}");
    assert_eq!(v["truncated"], truncated, "{v}");
}

/// `list --limit` answers in the envelope, cut or not, with the number the
/// filter matched as `total`; without the flag it is the bare array.
#[test]
fn list_limit_json_envelope() {
    let c = Corpus::new();
    for title in ["One", "Two", "Three"] {
        c.run(&["new", title, "--tag", "t"]).assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();

    assert_envelope(
        &json_of(&c, &["list", "--json", "--limit", "2"]),
        2,
        4,
        true,
    );
    let tagged = json_of(&c, &["list", "--json", "--tag", "t", "--limit", "2"]);
    assert_envelope(&tagged, 2, 3, true);
    assert_eq!(keys(&tagged["items"][0]), NODE_KEYS, "{tagged}");
    assert_envelope(
        &json_of(&c, &["list", "--json", "--limit", "500"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["list", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

/// `inbox --limit` answers in the envelope, cut or not; without the flag it
/// is the bare array.
#[test]
fn inbox_limit_json_envelope() {
    let c = Corpus::new();
    for text in ["first thought", "second thought", "third thought"] {
        c.run(&["capture", "--quiet", text]).assert_ok();
    }
    let cut = json_of(&c, &["inbox", "--json", "--limit", "2"]);
    assert_envelope(&cut, 2, 3, true);
    assert_eq!(cut["items"][0]["text"], "first thought", "{cut}");
    assert_envelope(
        &json_of(&c, &["inbox", "--json", "--limit", "5"]),
        3,
        3,
        false,
    );
    let bare = json_of(&c, &["inbox", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(3), "{bare}");
}

/// `review --limit`'s `total` counts every rule's findings before the cut
/// kept each rule's first N; `review --short --limit` counts its lines. Both
/// are envelopes with the flag, cut or not, and bare arrays without it.
#[test]
fn review_limit_json_envelope() {
    let c = Corpus::new();
    for title in ["Cold one", "Cold two", "Cold three"] {
        c.run(&["new", title]).assert_ok();
    }
    for id in ["cold-one", "cold-two", "cold-three"] {
        set_updated(&c.node_file(id), &date_days_ago(100));
    }
    c.run(&["capture", "--quiet", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    // Three cold seeds under one rule and the stale capture under another.
    let cut = json_of(&c, &["review", "--json", "--limit", "1"]);
    assert_envelope(&cut, 2, 4, true);
    assert_envelope(
        &json_of(&c, &["review", "--json", "--limit", "10"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["review", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");

    assert_envelope(
        &json_of(&c, &["review", "--short", "--json", "--limit", "1"]),
        1,
        4,
        true,
    );
    assert_envelope(
        &json_of(&c, &["review", "--short", "--json", "--limit", "10"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["review", "--short", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

/// Counts agree with their nouns wherever the reports print one.
#[test]
fn reports_count_in_the_singular_for_one() {
    let c = Corpus::new();
    c.run(&["new", "Alone"]).assert_ok();
    let checked = c.run(&["check"]).assert_ok();
    assert_eq!(checked.stdout(), format!("corpus: {}\n", c.root.display()));
    assert_eq!(
        checked.stderr(),
        "1 node, 0 errors, 0 warnings, 0 unreadable files\n"
    );
    let review = c.run(&["review", "--since", "1"]).assert_ok().stdout();
    assert!(
        review.contains("## Hypotheses untouched for 1 day\n"),
        "{review}"
    );
    assert!(
        review.contains("## Seeds untouched for 1 day\n"),
        "{review}"
    );
}

/// A query that finds nothing exits 0 with nothing on stdout and one line
/// on stderr saying so, in every mode; `--json` still prints its empty
/// document (STD-01 §R16).
#[test]
fn empty_results_leave_stdout_empty_and_explain_on_stderr() {
    let empty = Corpus::new();
    let c = Corpus::new();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "initial repository");
    let root = c.seed("a root thought", "Root thought");
    c.run(&["new", "Leaf", "--parent", &root]).assert_ok();

    for (corpus, args, said) in [
        (&c, vec!["list", "--status", "refuted"], "no nodes match"),
        (&c, vec!["near", "xyzzy"], "nothing near"),
        (&c, vec!["impact", "leaf"], "nothing descends from"),
        (
            &c,
            vec!["review", "--short", "--tag", "none"],
            "nothing needs attention",
        ),
        (&empty, vec!["tag", "list"], "no tags"),
        (&empty, vec!["inbox"], "inbox is empty"),
        (&c, vec!["log", "leaf"], "no commits touched this node"),
    ] {
        let human = corpus.run(&args).assert_ok();
        assert_eq!(human.stdout(), "", "`neb {}`", human.args);
        assert_eq!(human.stderr().lines().count(), 1, "`neb {}`", human.args);
        assert!(human.stderr().contains(said), "{}", human.stderr());

        let json = corpus
            .run(&[["--json"].as_slice(), &args].concat())
            .assert_ok();
        let doc: serde_json::Value = serde_json::from_str(&json.stdout())
            .unwrap_or_else(|e| panic!("`neb {}`: {e}\n{}", json.args, json.stdout()));
        // `near` is always the capped-list envelope (STD-01 §R34).
        let records = if doc.is_object() { &doc["items"] } else { &doc };
        assert_eq!(records, &serde_json::json!([]), "`neb {}`", json.args);
        assert_eq!(json.stderr(), human.stderr(), "`neb {}`", json.args);
    }
}

/// A notice that `--limit` or `--depth` cut the result is stderr's in every
/// mode, and stdout never mentions the bound (STD-01 §R12, §R34).
#[test]
fn limit_notices_go_to_stderr_in_every_mode() {
    let c = Corpus::new();
    let root = c.seed("a root thought", "Root");
    for title in ["One", "Two"] {
        c.run(&["new", title, "--parent", &root]).assert_ok();
        set_updated(&c.node_file(&title.to_lowercase()), &date_days_ago(100));
    }
    c.run(&["new", "Three", "--parent", "one"]).assert_ok();
    for text in ["first", "second", "third"] {
        c.run(&["capture", "-q", text]).assert_ok();
    }
    for args in [
        vec!["list", "--limit", "1"],
        vec!["inbox", "--limit", "1"],
        vec!["review", "--short", "--limit", "1"],
        vec!["trace", &root, "--down", "--depth", "1"],
    ] {
        for json in [false, true] {
            let args = if json {
                [["--json"].as_slice(), &args].concat()
            } else {
                args.clone()
            };
            let run = c.run(&args).assert_ok();
            let out = run.stdout();
            assert!(
                !out.contains("--limit") && !out.contains("--depth"),
                "`neb {}`:\n{out}",
                run.args
            );
            let err = run.stderr();
            assert_eq!(err.lines().count(), 1, "`neb {}`:\n{err}", run.args);
            assert!(
                err.contains("raise --limit for more") || err.contains("raise --depth for more"),
                "`neb {}`:\n{err}",
                run.args
            );
        }
    }
}

#[test]
fn an_empty_corpus_produces_an_empty_review() {
    let c = Corpus::new();
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(out.contains("## Hypotheses untouched for 30 days"));
    assert!(out.contains("## Seeds untouched for 90 days"));
    assert!(out.contains("## Nodes with no references"));
    assert!(out.contains("## Agent-authored kills not yet confirmed by a human"));
    assert!(out.contains("## Inbox entries waiting 14 days or more"));
    assert_eq!(
        out.matches("_none_").count(),
        5,
        "every section is empty:\n{out}"
    );

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(items.is_empty(), "{json}");
}

#[test]
fn review_never_touches_nodes_or_inbox_on_disk() {
    let c = Corpus::new();
    let hyp = c.seed("an idea", "An idea");
    c.run(&["sharpen", &hyp, "--kill", "if X"]).assert_ok();
    c.run(&["capture", "a thought"]).assert_ok();

    let before = snapshot_corpus_files(&c.root);
    c.run(&["review"]).assert_ok();
    c.run(&["review", "--json"]).assert_ok();
    let out_path = c.workdir().join("review.md");
    c.run(&["review", "--out", out_path.to_str().unwrap()])
        .assert_ok();
    let after = snapshot_corpus_files(&c.root);

    assert_eq!(before, after, "review must never mutate nodes/ or inbox/");
}
