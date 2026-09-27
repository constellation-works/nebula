//! Tags: normalisation on every write path, no-op retags, and drift between
//! near variants.

use crate::harness::{Corpus, corpus_repo, date_days_ago, git, log, set_updated, write};

#[test]
fn tags_are_normalised_on_every_write_path() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "Direct",
        "--tag",
        "Physics",
        "--tag",
        "Machine Learning",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("direct")).unwrap();
    assert!(
        raw.contains("tags:\n- physics\n- machine-learning\n"),
        "{raw}"
    );

    let entry = c.run(&["capture", "promoted with a tag"]).stdout_trim();
    c.run(&["promote", &entry, "--title", "Promoted", "--tag", "ORRERY"])
        .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("promoted")).unwrap();
    assert!(raw.contains("tags:\n- orrery\n"), "{raw}");

    c.run(&["tag", "promoted", "--add", "Physics", "--add", "physics"])
        .assert_ok()
        .says("orrery, physics");
    c.run(&["tag", "promoted", "--remove", "ORRERY"])
        .assert_ok()
        .says("physics");
    let raw = std::fs::read_to_string(c.node_file("promoted")).unwrap();
    assert!(raw.contains("tags:\n- physics\n"), "{raw}");
    c.run(&["tag", "promoted"])
        .assert_fails()
        .says("--add <tag> or --remove <tag>");

    c.run(&["tag", "list"])
        .assert_ok()
        .says("machine-learning\t1\n")
        .says("physics\t2\n");
    let json = c.run(&["--json", "tag", "list"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["tag"] == "physics" && i["count"] == 2),
        "{json}"
    );

    // `--tag` filters are AND, and normalised the same way as the writes.
    let out = c.run(&["list", "--tag", "Physics"]).assert_ok().stdout();
    assert!(out.contains("direct") && out.contains("promoted"), "{out}");
    let out = c
        .run(&["list", "--tag", "physics", "--tag", "machine-learning"])
        .assert_ok()
        .stdout();
    assert!(out.contains("direct") && !out.contains("promoted"), "{out}");
    c.run(&["list", "--tag", "nope"])
        .assert_ok()
        .says("no nodes match");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// A tag not there to remove, or already there to add, is noted on stderr
/// naming the tag and the node; when that leaves the tags as they were,
/// the node is not written and nothing is committed (STD-01 §R30). A call
/// with one real change in it writes that change and notes the rest.
#[test]
fn tag_noop_changes_nothing_and_says_so() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Tagged", "--tag", "physics"])
        .assert_ok()
        .stdout_trim();
    set_updated(&c.node_file(&id), &date_days_ago(7));
    git(&c.root, &["commit", "-qam", "back-dated"]);
    let before = std::fs::read(c.node_file(&id)).unwrap();
    let commits = log(&c.root);

    for (flag, tag, says) in [
        ("--remove", "absent", "nothing to remove"),
        ("--add", "physics", "nothing to add"),
    ] {
        let run = c.run(&["tag", &id, flag, tag]).assert_ok();
        let stderr = run.stderr();
        assert!(stderr.contains(&format!("`{tag}`")), "{stderr}");
        assert!(stderr.contains(&id) && stderr.contains(says), "{stderr}");
        assert!(stderr.contains("no change"), "{stderr}");
        assert!(!stderr.contains("committed"), "{stderr}");
        assert_eq!(std::fs::read(c.node_file(&id)).unwrap(), before, "{flag}");
        assert_eq!(log(&c.root), commits, "{flag}: no commit");
    }

    let run = c
        .run(&["tag", &id, "--add", "new", "--remove", "absent"])
        .assert_ok();
    let stderr = run.stderr();
    assert!(
        stderr.contains("`absent`") && stderr.contains(&id),
        "{stderr}"
    );
    assert!(!stderr.contains("no change"), "{stderr}");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("- new"), "{raw}");
    assert!(
        raw.contains(&format!("updated: {}", date_days_ago(0))),
        "{raw}"
    );
    assert_eq!(log(&c.root)[0], format!("neb tag {id}"));
}

#[test]
fn tag_drift_by_case_or_plural_is_a_warning() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["tag", &a, "--add", "physics", "--add", "sim"])
        .assert_ok();
    c.run(&["tag", &b, "--add", "physics", "--add", "sims"])
        .assert_ok();
    // Writes normalise case, so the case collision has to be a hand edit.
    let raw = std::fs::read_to_string(c.node_file(&b)).unwrap();
    write(&c.node_file(&b), &raw.replace("- physics", "- Physics"));
    c.run(&["check"])
        .assert_ok()
        .says("[11]")
        .says(&format!(
            "tags `Physics` (on {b}) and `physics` (on {a}) differ only by case"
        ))
        .says(&format!(
            "tags `sim` (on {a}) and `sims` (on {b}) differ only by a trailing `s`"
        ))
        .says("0 errors, 2 warnings");

    let json = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&json).expect("check --json");
    let drift: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == 11)
        .map(|f| f["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        drift,
        [
            format!("tags `Physics` (on {b}) and `physics` (on {a}) differ only by case"),
            format!("tags `sim` (on {a}) and `sims` (on {b}) differ only by a trailing `s`"),
        ],
        "{json}"
    );
}

/// Drift is noted at the write that introduces it, on stderr, and never
/// refused: there is no declared list to refuse against.
#[test]
fn a_write_introducing_a_near_variant_of_a_tag_notes_it_and_succeeds() {
    let c = Corpus::new();
    c.run(&["new", "First", "--tag", "physics"]).assert_ok();
    c.run(&["new", "Second", "--tag", "Physics"]).assert_ok();

    let run = c.run(&["new", "Third", "--tag", "physic"]).assert_ok();
    assert_eq!(run.stdout_trim(), "third");
    assert_eq!(
        run.stderr(),
        "note: tag physic is close to physics (2 nodes)\n"
    );
    let raw = std::fs::read_to_string(c.node_file("third")).unwrap();
    assert!(
        raw.contains("tags:\n- physic\n"),
        "the tag is written as given: {raw}"
    );

    // `tag --add` says the same, and `--json` keeps stdout the payload.
    let run = c
        .run(&["--json", "tag", "first", "--add", "Orrerys"])
        .assert_ok();
    assert!(
        run.stderr().is_empty(),
        "nothing close yet: {}",
        run.stderr()
    );
    let run = c
        .run(&["--json", "tag", "second", "--add", "orrery"])
        .assert_ok();
    serde_json::from_str::<serde_json::Value>(&run.stdout()).expect("stdout stays JSON");
    assert_eq!(
        run.stderr(),
        "note: tag orrery is close to orrerys (1 node)\n"
    );
    let run = c.run(&["tag", "list"]).assert_ok();
    assert!(run.stderr().is_empty(), "a read notes nothing");

    // A tag someone else already carries is not new to the corpus, so the
    // write adds nothing to what `check` already says.
    let run = c.run(&["tag", "first", "--add", "physic"]).assert_ok();
    assert!(run.stderr().is_empty(), "{}", run.stderr());
    let run = c.run(&["new", "Fourth", "--tag", "design"]).assert_ok();
    assert!(run.stderr().is_empty(), "{}", run.stderr());
}

#[test]
fn promote_notes_a_tag_close_to_one_in_use() {
    let c = Corpus::new();
    c.run(&["new", "First", "--tag", "sims"]).assert_ok();
    let entry = c.run(&["capture", "a promoted thought"]).stdout_trim();
    c.run(&["promote", &entry, "--title", "Promoted", "--tag", "SIM"])
        .assert_ok()
        .says("note: tag sim is close to sims (1 node)");
}
