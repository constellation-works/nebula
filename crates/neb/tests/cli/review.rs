//! `neb review` and its short form: the four rules and their thresholds, the
//! deprecated `open`, `--out`, and the report's JSON.

use crate::harness::{
    Corpus, date_days_ago, set_created, set_inbox_stamp_for, set_updated, stamp_days_ago,
};

#[test]
fn review_short_finds_the_hypothesis_with_no_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));
    c.run(&["review", "--short"])
        .assert_ok()
        .says(&id)
        .says("hypothesis with no references");

    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "n"])
        .assert_ok();
    let after = c.run(&["review", "--short"]).assert_ok().stdout();
    assert!(
        !after.contains("no references"),
        "a reference closes the gap:\n{after}"
    );
}

#[test]
fn review_short_json_items_have_exactly_id_and_why_keys() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));

    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        !items.is_empty(),
        "expected at least one short-review item:\n{json}"
    );
    for item in &items {
        let keys: std::collections::BTreeSet<&str> = item
            .as_object()
            .expect("review --short --json item is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from(["id", "why"]),
            "review --short --json item key set drifted from {{id, why}}: {item}"
        );
    }
}

#[test]
fn review_and_its_short_form_apply_the_no_references_grace_at_fourteen_days() {
    let c = Corpus::new();
    let grace = c.seed("still in grace", "Still in grace");
    c.run(&["sharpen", &grace, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&grace), &date_days_ago(13));

    let due = c.seed("now due", "Now due");
    c.run(&["sharpen", &due, "--kill", "if Y"]).assert_ok();
    c.run(&["note", &due, "reasoning is not a reference"])
        .assert_ok();
    set_created(&c.node_file(&due), &date_days_ago(14));

    for args in [["review", "--short"].as_slice(), &["review"]] {
        let out = c.run(args).assert_ok().stdout();
        assert!(
            !out.contains(&grace),
            "{args:?} raised a 13-day-old node:\n{out}"
        );
        assert!(
            out.contains(&due),
            "{args:?} missed a 14-day-old node:\n{out}"
        );
    }
}

#[test]
fn review_short_finds_seeds_untouched_for_ninety_days_and_filters_by_tag() {
    let c = Corpus::new();
    let old = c.seed("an old seed", "An old seed");
    c.run(&["tag", &old, "--add", "physics"]).assert_ok();
    set_updated(&c.node_file(&old), &date_days_ago(90));
    let fresh = c.seed("a fresh seed", "A fresh seed");
    set_updated(&c.node_file(&fresh), &date_days_ago(89));

    let out = c.run(&["review", "--short"]).assert_ok().stdout();
    assert!(
        out.contains(&old) && out.contains("seed untouched for ninety days"),
        "{out}"
    );
    assert!(!out.contains(&fresh), "{out}");

    let out = c
        .run(&["review", "--short", "--tag", "physics"])
        .assert_ok()
        .stdout();
    assert!(out.contains(&old), "{out}");
    let out = c
        .run(&["review", "--short", "--tag", "orrery"])
        .assert_ok()
        .stdout();
    assert!(!out.contains(&old), "{out}");
}

#[test]
fn review_short_finds_inbox_captures_waiting_over_fourteen_days() {
    let c = Corpus::new();
    c.run(&["capture", "an old capture"]);

    let inbox_file = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut raw = std::fs::read_to_string(&inbox_file).unwrap();
    let stamp_start = raw.find("] ").unwrap() + 2;
    let stamp_end = stamp_start + raw[stamp_start..].find(' ').unwrap();
    raw.replace_range(stamp_start..stamp_end, "2020-01-01T00:00");
    std::fs::write(inbox_file, raw).unwrap();

    c.run(&["review", "--short"])
        .assert_ok()
        .says("1 capture waiting over fourteen days; promote or drop them");

    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items.iter().any(|item| item["id"] == "inbox"),
        "inbox finding missing: {json}"
    );
}

#[test]
fn review_short_on_an_empty_corpus_says_nothing_needs_attention() {
    let c = Corpus::new();
    c.run(&["review", "--short"])
        .assert_ok()
        .says("nothing needs attention");
    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(items.is_empty(), "{json}");
}

#[test]
fn review_short_refuses_the_full_reports_since_and_out_and_tag_needs_it() {
    let c = Corpus::new();
    let out_path = c.workdir().join("short.md");
    for args in [
        vec!["review", "--short", "--since", "7"],
        vec!["review", "--short", "--out", out_path.to_str().unwrap()],
    ] {
        c.run(&args).assert_fails().says("cannot be used with");
    }
    assert!(!out_path.exists(), "a refused --out must not write a file");
    c.run(&["review", "--tag", "physics"])
        .assert_fails()
        .says("--short");
}

/// Both clap spellings must reach the same day-threshold validator.
#[test]
fn review_since_negative_spellings_have_identical_validation() {
    let c = Corpus::new();
    for value in ["-1", "-9223372036854775808"] {
        let joined = c.run(&["review", &format!("--since={value}")]);
        let separate = c.run(&["review", "--since", value]);
        assert_eq!(joined.out.status.code(), Some(2));
        assert_eq!(separate.out.status.code(), Some(2));
        assert!(
            joined
                .stderr()
                .contains("must be a whole number, at least 0")
        );
        assert_eq!(separate.stderr(), joined.stderr());
        assert_eq!(separate.stdout(), "");
    }
}

/// A negative `--since` would call a seed made today untouched for -1 days
/// and propose abandoning it. It is a usage error; zero days is a threshold.
#[test]
fn review_since_negative_is_a_usage_error() {
    let c = Corpus::new();
    c.seed("made today", "Made today");
    for since in ["--since=-1", "--since=-9223372036854775808"] {
        let refused = c.run(&["review", since]);
        assert_eq!(refused.out.status.code(), Some(2), "{}", refused.stderr());
        assert_eq!(refused.stdout(), "");
        assert!(
            refused.stderr().contains("--since") && refused.stderr().contains("at least 0"),
            "{}",
            refused.stderr()
        );
    }
    c.run(&["review", "--since", "0"])
        .assert_ok()
        .says("## Seeds untouched for 0 days");
}

/// `open` is kept for one release so routines that call it keep working: the
/// same stdout as `review --short`, in text and JSON, plus one deprecation
/// line on stderr, and no row in `--help`.
#[test]
fn deprecated_open_matches_review_short_and_warns_once_on_stderr() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&["sharpen", &bare, "--kill", "if X"]).assert_ok();
    c.run(&["tag", &bare, "--add", "physics"]).assert_ok();
    set_created(&c.node_file(&bare), &date_days_ago(14));
    let old = c.seed("an old seed", "An old seed");
    set_updated(&c.node_file(&old), &date_days_ago(90));

    for extra in [
        [].as_slice(),
        &["--json"],
        &["--tag", "physics"],
        &["--json", "--tag", "physics"],
    ] {
        let short = c
            .run(&[["review", "--short"].as_slice(), extra].concat())
            .assert_ok();
        let open = c.run(&[["open"].as_slice(), extra].concat()).assert_ok();
        assert!(
            short.stdout().contains(&bare),
            "{extra:?}: {}",
            short.stdout()
        );
        assert_eq!(open.stdout(), short.stdout(), "{extra:?}");
        assert_eq!(
            short.stderr(),
            "",
            "{extra:?}: review --short is not deprecated"
        );
        assert_eq!(
            open.stderr().lines().collect::<Vec<_>>(),
            ["warning: `neb open` is deprecated; use `neb review --short`"],
            "{extra:?}"
        );
    }

    let help = c.run(&["--help"]).assert_ok().stdout();
    assert!(!help.contains("\n  open "), "open is hidden:\n{help}");
    assert!(help.contains("\n  review "), "{help}");
}

#[test]
fn review_reports_stale_hypotheses_on_both_sides_of_thirty_days() {
    let c = Corpus::new();
    let stale = c.seed("an old hypothesis", "An old hypothesis");
    c.run(&["sharpen", &stale, "--kill", "if X"]).assert_ok();
    c.run(&["cite", &stale, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&stale), &date_days_ago(45));

    let fresh = c.seed("a fresh hypothesis", "A fresh hypothesis");
    c.run(&["sharpen", &fresh, "--kill", "if Y"]).assert_ok();
    c.run(&["cite", &fresh, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&fresh), &date_days_ago(15));

    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains(&format!("`{stale}`")),
        "stale hypothesis missing:\n{out}"
    );
    assert!(
        !out.contains(&format!("`{fresh}`")),
        "fresh hypothesis should not be flagged:\n{out}"
    );
}

#[test]
fn review_reports_untouched_seeds_on_both_sides_of_ninety_days() {
    let c = Corpus::new();
    let stale = c.seed("an old seed", "An old seed");
    c.run(&["cite", &stale, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&stale), &date_days_ago(120));

    let fresh = c.seed("a fresh seed", "A fresh seed");
    c.run(&["cite", &fresh, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&fresh), &date_days_ago(60));

    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains(&format!("`{stale}`")) && out.contains("propose: status abandoned"),
        "stale seed missing:\n{out}"
    );
    assert!(
        !out.contains(&format!("`{fresh}`")),
        "fresh seed should not be flagged:\n{out}"
    );
}

#[test]
fn review_and_its_short_form_report_only_aged_hypotheses_with_no_references() {
    let c = Corpus::new();
    let seed = c.seed("an old seed", "An old seed");
    set_created(&c.node_file(&seed), &date_days_ago(30));

    let hypothesis = c.seed("an old hypothesis", "An old hypothesis");
    c.run(&["sharpen", &hypothesis, "--kill", "if X"])
        .assert_ok();
    set_created(&c.node_file(&hypothesis), &date_days_ago(30));

    for args in [["review", "--short"].as_slice(), &["review"]] {
        let out = c.run(args).assert_ok().stdout();
        assert!(
            !out.contains(&seed),
            "{args:?} incorrectly reported an old seed with no references:\n{out}"
        );
        assert!(
            out.contains(&hypothesis),
            "{args:?} missed an old hypothesis with no references:\n{out}"
        );
    }
}

#[test]
fn review_reports_stale_inbox_entries_on_both_sides_of_fourteen_days() {
    let c = Corpus::new();
    c.run(&["capture", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains("1 capture waiting over fourteen days; promote or drop them"),
        "{out}"
    );

    let fresh = Corpus::new();
    fresh.run(&["capture", "a recent capture"]).assert_ok();
    set_inbox_stamp_for(&fresh.root, "a recent capture", &stamp_days_ago(10));
    let out = fresh.run(&["review"]).assert_ok().stdout();
    assert!(!out.contains("waiting over fourteen days"), "{out}");
}

#[test]
fn review_json_emits_all_four_rule_names() {
    let c = Corpus::new();

    let stale_hyp = c.seed("stale hypothesis", "Stale hypothesis");
    c.run(&["sharpen", &stale_hyp, "--kill", "if X"])
        .assert_ok();
    c.run(&[
        "cite",
        &stale_hyp,
        "--uri",
        "http://example.com",
        "--note",
        "n",
    ])
    .assert_ok();
    set_updated(&c.node_file(&stale_hyp), &date_days_ago(45));

    let stale_seed = c.seed("stale seed", "Stale seed");
    c.run(&[
        "cite",
        &stale_seed,
        "--uri",
        "http://example.com",
        "--note",
        "n",
    ])
    .assert_ok();
    set_updated(&c.node_file(&stale_seed), &date_days_ago(120));

    let bare_hypothesis = c.seed("bare hypothesis", "Bare hypothesis");
    c.run(&["sharpen", &bare_hypothesis, "--kill", "if Z"])
        .assert_ok();
    set_created(&c.node_file(&bare_hypothesis), &date_days_ago(14));

    c.run(&["capture", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&json).expect("review --json is valid JSON");
    let rules: std::collections::HashSet<&str> =
        items.iter().map(|i| i["rule"].as_str().unwrap()).collect();
    for rule in [
        "stale-hypothesis",
        "untouched-seed",
        "no-references",
        "stale-inbox",
    ] {
        assert!(rules.contains(rule), "missing rule `{rule}` in {json}");
    }
    for item in &items {
        assert!(item["id"].is_string());
        assert!(item["title"].is_string());
        assert!(item["reason"].is_string());
    }
}

/// `--out` writes the report to the file and names it on stderr, in both
/// modes, so stdout stays empty and the write says what it wrote (STD-01
/// §R30).
#[test]
fn review_out_names_the_file() {
    let c = Corpus::new();
    let bare = c.seed("a hypothesis needing a look", "A hypothesis needing a look");
    c.run(&["sharpen", &bare, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&bare), &date_days_ago(14));
    let out_path = c.workdir().join("review.md");
    let run = c
        .run(&["review", "--out", out_path.to_str().unwrap()])
        .assert_ok();
    assert_eq!(
        run.stdout(),
        "",
        "stdout must stay empty when writing to a file"
    );
    assert_eq!(run.stderr(), format!("wrote {}\n", out_path.display()));
    let report = std::fs::read_to_string(&out_path).unwrap();
    assert!(report.contains(&format!("`{bare}`")));
    assert!(report.contains("## Nodes with no references"));

    let json_path = c.workdir().join("review.json");
    let run = c
        .run(&["--json", "review", "--out", json_path.to_str().unwrap()])
        .assert_ok();
    assert_eq!(run.stdout(), "");
    assert_eq!(run.stderr(), format!("wrote {}\n", json_path.display()));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert!(
        report.as_array().is_some_and(|items| !items.is_empty()),
        "{report}"
    );
}

/// A report that cannot be written is refused naming the path it was for.
#[test]
fn report_out_write_failure_names_the_path() {
    let c = Corpus::new();
    let out_path = c.workdir().join("no-such-dir").join("review.md");
    let refused = c
        .run(&["--json", "review", "--out", out_path.to_str().unwrap()])
        .refusal();
    assert_eq!(refused["code"], "io_at", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains(&out_path.display().to_string()),
        "{refused}"
    );
    assert!(!out_path.exists());
}

/// The real entry point must never turn a diagnostic report into corpus data.
#[test]
fn review_out_refuses_corpus_targets_without_writing() {
    let c = Corpus::new();
    let id = c.seed("keep this idea", "Keep this idea");
    let node = c.node_file(&id);
    let config = c.root.join("config.yaml");
    let before_node = std::fs::read(&node).unwrap();
    let before_config = std::fs::read(&config).unwrap();
    for target in [
        node.clone(),
        config.clone(),
        c.root.join("nodes/review.md"),
        c.root.join("nodes/../review.md"),
        c.root.clone(),
    ] {
        let run = c.run(&["review", "--json", "--out", target.to_str().unwrap()]);
        assert_eq!(run.refusal()["code"], "report_in_corpus");
        assert_eq!(run.stdout(), "");
    }
    // Match the original relative-path repro with root discovery from cwd.
    for target in ["config.yaml", "nodes/review.md", "nodes/../review.md"] {
        let mut cmd = crate::harness::neb_command(c.workdir());
        cmd.current_dir(&c.root)
            .env("PWD", &c.root)
            .args(["review", "--json", "--out", target]);
        let run = crate::harness::run_command(&mut cmd, format!("review --out {target}"));
        assert_eq!(run.refusal()["code"], "report_in_corpus");
        assert_eq!(run.stdout(), "");
    }
    assert_eq!(std::fs::read(node).unwrap(), before_node);
    assert_eq!(std::fs::read(config).unwrap(), before_config);
    assert!(!c.root.join("nodes/review.md").exists());
    assert!(!c.root.join("review.md").exists());
    c.run(&["check"]).assert_ok();
}

#[test]
fn review_out_read_only_refuses_inside_and_outside_but_allows_stdout() {
    let c = Corpus::new();
    for target in [c.root.join("config.yaml"), c.workdir().join("review.md")] {
        let before = std::fs::read(&target).ok();
        let run = c.run_with_env(
            &["review", "--json", "--out", target.to_str().unwrap()],
            &[("NEBULA_READ_ONLY", "1")],
        );
        assert_eq!(run.refusal()["code"], "read_only");
        assert_eq!(run.stdout(), "");
        assert_eq!(std::fs::read(&target).ok(), before);
    }
    c.run_with_env(&["review", "--json"], &[("NEBULA_READ_ONLY", "1")])
        .assert_ok();
}

#[cfg(unix)]
#[test]
fn review_out_refuses_corpus_aliases_and_final_symlinks() {
    use std::os::unix::fs::symlink;
    let mut c = Corpus::new();
    let real_root = c.root.clone();
    let config = real_root.join("config.yaml");
    let before = std::fs::read(&config).unwrap();
    let alias = c.workdir().join("alias");
    symlink(&real_root, &alias).unwrap();
    c.root = alias.clone(); // Corpus discovery itself may go through a symlink.
    for target in [
        alias.join("nodes/review.md"),
        real_root.join("review.md"),
        alias.join("nodes/../config.yaml"),
    ] {
        let run = c.run(&["review", "--json", "--out", target.to_str().unwrap()]);
        assert_eq!(run.refusal()["code"], "report_in_corpus");
    }
    let link = c.workdir().join("report-link");
    symlink(&config, &link).unwrap();
    let run = c.run(&["review", "--json", "--out", link.to_str().unwrap()]);
    assert_eq!(run.refusal()["code"], "report_in_corpus");
    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read(&config).unwrap(), before);
    assert!(!real_root.join("review.md").exists());
    assert!(!real_root.join("nodes/review.md").exists());

    let outside = c.workdir().join("outside.md");
    crate::harness::write(&outside, "keep outside");
    let inside_link = real_root.join("report-link");
    symlink(&outside, &inside_link).unwrap();
    let run = c.run(&["review", "--json", "--out", inside_link.to_str().unwrap()]);
    assert_eq!(run.refusal()["code"], "report_in_corpus");
    assert!(
        std::fs::symlink_metadata(&inside_link)
            .unwrap()
            .is_symlink()
    );
    let outside_link = c.workdir().join("outside-link");
    symlink(&outside, &outside_link).unwrap();
    let dangling = c.workdir().join("dangling");
    symlink(c.workdir().join("missing.md"), &dangling).unwrap();
    for target in [&outside_link, &dangling] {
        let run = c.run(&["review", "--json", "--out", target.to_str().unwrap()]);
        assert_eq!(run.refusal()["code"], "not_regular_file");
        assert!(std::fs::symlink_metadata(target).unwrap().is_symlink());
    }
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "keep outside");
    assert!(!c.workdir().join("missing.md").exists());
}

#[cfg(unix)]
#[test]
fn review_out_replaces_atomically_with_private_mode() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let c = Corpus::new();
    // A prefix sibling is outside, and a symlinked parent outside is allowed.
    let dir = c.workdir().join("corpus-reports");
    std::fs::create_dir(&dir).unwrap();
    let alias = c.workdir().join("reports");
    symlink(&dir, &alias).unwrap();
    let target = dir.join("review.md");
    let hardlink = dir.join("old-report.md");
    crate::harness::write(&target, "old report");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::hard_link(&target, &hardlink).unwrap();
    let expected = c.run(&["review"]).assert_ok().stdout();
    c.run(&["review", "--out", alias.join("review.md").to_str().unwrap()])
        .assert_ok();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), expected);
    assert_eq!(std::fs::read_to_string(&hardlink).unwrap(), "old report");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    let fresh = dir.join("fresh.json");
    c.run(&["review", "--json", "--out", fresh.to_str().unwrap()])
        .assert_ok();
    assert_eq!(
        std::fs::metadata(fresh).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
