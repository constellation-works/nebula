//! `neb check`: every rule's findings, their levels and JSON, unreadable
//! nodes, dangling edges and invalid dates.

use crate::harness::{Corpus, date_days_ago, set_created, set_updated, write};
use crate::lock::plant_debris;
use crate::observatory::observatory;

/// Rule 17: a temporary file an interrupted write left behind is named, with
/// the command that removes it, and left exactly where it is.
#[test]
fn check_warns_about_stale_temp_files() {
    let c = Corpus::new();
    let debris = plant_debris(&c);

    let text = c
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 2 warnings")
        .stdout();
    for name in [
        "nodes/x.md.1f2e-0-18d8.tmp",
        "inbox/2026-09.md.1f2e-0-18d8.tmp",
    ] {
        text.lines()
            .find(|line| line.contains(name))
            .filter(|line| line.contains("[17]") && line.contains("rm '"))
            .unwrap_or_else(|| panic!("no rule-17 warning for {name}:\n{text}"));
    }

    let json = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{findings:?}");
    for (finding, path) in findings.iter().zip(&debris) {
        assert_eq!(finding["rule"], 17);
        assert_eq!(finding["level"], "warn");
        let message = finding["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("rm '{}'", path.display())),
            "{message}"
        );
    }
    for path in &debris {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "half of a write that never landed\n",
            "check never deletes or rewrites debris"
        );
    }
}

#[test]
fn check_json_exposes_rule_and_level() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\ntags:\n- Sims\n- sim"),
    );

    let out = c.run(&["--json", "check"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json is valid JSON");
    let findings = v["findings"].as_array().expect("findings array");

    let rule10 = findings
        .iter()
        .find(|f| f["rule"] == 10)
        .expect("rule 10 finding present");
    assert_eq!(rule10["level"], "warn");
    assert_eq!(rule10["node"], id);

    let rule11 = findings
        .iter()
        .find(|f| f["rule"] == 11)
        .expect("rule 11 finding present");
    assert_eq!(rule11["level"], "warn");
    assert!(rule11["node"].is_null(), "tag drift is a corpus finding");
}

fn corpus_with_two_bad_nodes() -> (Corpus, String, String) {
    let c = Corpus::new();
    let first = c.seed("first thought", "First thought");
    let second = c.seed("second thought", "Second thought");
    let raw = std::fs::read_to_string(c.node_file(&first)).unwrap();
    write(
        &c.node_file(&first),
        &raw.replacen(
            "status: seed\n",
            &format!("status: seed\nedges:\n- type: contradicts\n  to: {second}\n"),
            1,
        ),
    );
    write(&c.node_file("broken"), "---\nid: broken\n");
    write(&c.node_file("zzz-bad"), "no frontmatter\n");
    (c, first, second)
}

#[test]
fn check_reports_every_unparsable_node_and_checks_the_rest() {
    let (c, _, _) = corpus_with_two_bad_nodes();
    let run = c.run(&["--json", "check"]).assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    assert_eq!(report["nodes"], 2);
    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 2);
    for name in ["broken.md", "zzz-bad.md"] {
        let item = unreadable
            .iter()
            .find(|item| item["path"].as_str().unwrap().ends_with(name))
            .unwrap();
        assert_eq!(item["code"], "malformed_frontmatter", "{item}");
        assert!(item["message"].is_string(), "{item}");
    }
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == 4),
        "{report}"
    );
    let human = c.run(&["check"]).assert_fails();
    let text = human.stdout();
    assert!(text.find("broken.md") < text.find("contradicts"), "{text}");
    assert!(human.stderr().contains("2 unreadable files"));
}

#[test]
fn check_reports_an_unknown_field_as_an_unreadable_node() {
    let c = Corpus::new();
    let first = c.seed("first thought", "First thought");
    c.seed("second thought", "Second thought");
    let raw = std::fs::read_to_string(c.node_file(&first)).unwrap();
    write(
        &c.node_file(&first),
        &raw.replace("status: seed", "status: seed\nfuture_field: unknown"),
    );
    let run = c.run(&["--json", "check"]).assert_fails();
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    assert_eq!(report["nodes"], 1);
    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 1);
    assert!(
        unreadable[0]["path"]
            .as_str()
            .unwrap()
            .ends_with(&format!("{first}.md"))
    );
    assert!(
        unreadable[0]["message"]
            .as_str()
            .unwrap()
            .contains("future_field")
    );
}

#[test]
fn strict_verbs_name_every_unreadable_node() {
    let (c, first, _) = corpus_with_two_bad_nodes();
    for args in [vec!["list"], vec!["show", &first], vec!["trace", &first]] {
        let mut full = vec!["--json"];
        full.extend(args);
        let refusal = c.run(&full).refusal();
        assert_eq!(refusal["code"], "unreadable_nodes");
        let error = refusal["error"].as_str().unwrap();
        assert!(error.contains("2 unreadable"), "{error}");
        assert!(
            error.contains("broken.md") && error.contains("zzz-bad.md"),
            "{error}"
        );
        assert!(refusal["hint"].as_str().unwrap().contains("neb check"));
    }
}

#[test]
fn check_reports_a_broken_observatory_setting_as_a_finding() {
    let c = Corpus::new();
    let id = c.seed("a thought", "A thought");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replacen(
            "status: seed\n",
            "status: seed\nedges:\n- type: derives-from\n  to: ghost\n",
            1,
        ),
    );
    let setting = c.workdir().join(".config/nebula/observatory-root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    write(&setting, "relative-observatory\n");
    let run = c.run(&["--json", "check"]).assert_fails();
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    let findings = report["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["level"] == "error"
            && f["message"]
                .as_str()
                .unwrap()
                .contains(&setting.display().to_string())),
        "{report}"
    );
    assert!(findings.iter().any(|f| f["rule"] == 3), "{report}");
    assert!(!run.stdout().contains("no observatory root is set"));
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::print_stderr,
    reason = "report why the unreadable-directory test is skipped as root"
)]
fn an_unreadable_observatory_directory_is_not_reported_as_unresolved() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root can read mode-000 directories");
        return;
    }
    let c = Corpus::new();
    let id = c.seed("a thought", "A thought");
    let obs = observatory(c.workdir());
    let questions = obs.join("questions");
    let env = [("OBSERVATORY_ROOT", obs.to_str().unwrap())];
    c.run_with_env(
        &["cite", &id, "--uri", "Q002", "--kind", "observatory"],
        &env,
    )
    .assert_ok();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let mode = std::fs::metadata(&questions).unwrap().permissions();
    std::fs::set_permissions(&questions, std::fs::Permissions::from_mode(0o000)).unwrap();
    let checked = c.run_with_env(&["--json", "check"], &env).assert_fails();
    let refused = c
        .run_with_env(&["--json", "handoff", &id, "Q002"], &env)
        .refusal();
    std::fs::set_permissions(&questions, mode).unwrap();
    let report: serde_json::Value = serde_json::from_str(&checked.stdout()).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |f| f["message"].as_str().unwrap().contains("could not read")
                    && f["message"].as_str().unwrap().contains("questions")
            ),
        "{report}"
    );
    assert!(!checked.stdout().contains("does not resolve"));
    assert_eq!(refused["code"], "io_at");
    assert!(refused["error"].as_str().unwrap().contains("questions"));
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

#[test]
fn dangling_edges_are_caught() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace(
        "status: seed",
        "status: seed\nedges:\n- type: derives-from\n  to: ghost",
    );
    write(&c.node_file(&id), &raw);
    c.run(&["check"]).assert_fails().says("missing node");
}

/// `status` always clears `closed` the moment a node leaves refuted or
/// abandoned, so a `closed` block on a seed or hypothesis is not a state any
/// verb produces — only a hand edit that reopened the node outside `status`
/// leaves one behind.
#[test]
fn a_closed_block_on_an_open_node_is_a_rule_12_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace(
            "status: hypothesis",
            "status: hypothesis\nclosed:\n  why: it was abandoned once\n  at: 2026-09-01",
        ),
    );
    c.run(&["check"])
        .assert_fails()
        .says("[12]")
        .says("status is `hypothesis` but a `closed` block is still set")
        .says("1 error,");
}

/// `new --kill` and `sharpen` always move status to `hypothesis` together
/// with writing `kill`, so a `seed` carrying one was set by hand without the
/// guard. Not wrong by itself, so it is a warning rather than an error.
#[test]
fn a_seed_with_a_kill_condition_is_a_rule_13_warning() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\nkill: if X"),
    );
    c.run(&["check"])
        .assert_ok()
        .says("[13]")
        .says("status is seed but a kill condition is set")
        .says("0 errors, 1 warning");
}

/// `ops.rs` only ever stamps `created`/`updated` from `stamp::today()`, so
/// either field failing to parse, or `updated` landing before `created`, is
/// a hand edit — and `review`/`open` then silently treat the node as never
/// stale, since `days_since` returns `None` for a date it cannot parse.
#[test]
fn an_unparsable_created_or_updated_date_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), "not-a-date");
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("created `not-a-date` is not a YYYY-MM-DD date")
        .says("1 error,");
}

/// Calendar-looking strings must still name real Gregorian dates. Otherwise
/// hand edits evade rule 14 while `review` and `open` silently ignore them.
#[test]
fn impossible_calendar_dates_are_rule_14_errors() {
    for (field, date) in [
        ("created", "2026-99-99"),
        ("updated", "2026-04-31"),
        ("created", "2026-02-29"),
    ] {
        let c = Corpus::new();
        let id = c.seed("an idea", "An idea");
        match field {
            "created" => set_created(&c.node_file(&id), date),
            "updated" => set_updated(&c.node_file(&id), date),
            _ => unreachable!("test fields are explicit"),
        }
        c.run(&["check"])
            .assert_fails()
            .says("[14]")
            .says(&format!("{field} `{date}` is not a YYYY-MM-DD date"))
            .says("1 error,");
    }

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "2026-02-29");
    write(&c.node_file(&id), &raw);
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("reference `r1` has an added date `2026-02-29` that does not parse")
        .says("1 error,");
}

#[test]
fn leap_day_dates_are_valid_for_nodes_and_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), "2024-02-29");
    set_updated(&c.node_file(&id), "2024-02-29");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "2024-02-29");
    write(&c.node_file(&id), &raw);
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn updated_earlier_than_created_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), &date_days_ago(1));
    set_updated(&c.node_file(&id), &date_days_ago(2));
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("updated")
        .says("earlier than created")
        .says("1 error,");
}

/// A reference's `added` date is stamped the same way and can go wrong the
/// same way, so rule 14 covers it too.
#[test]
fn an_unparsable_reference_added_date_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "not-a-date");
    write(&c.node_file(&id), &raw);
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("reference `r1` has an added date `not-a-date` that does not parse")
        .says("1 error,");
}
