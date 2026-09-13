//! End-to-end tests over a real corpus in a temporary directory.
//!
//! These drive the built binary rather than library functions, because the
//! things most likely to break are the transition guards and the exit codes,
//! and both live at the edge.
//!
//! Paths are used exactly as the temporary directory reports them and are never
//! canonicalized or compared against a resolved form. On macOS the temporary
//! directory sits under a symlink, and a checker that resolved paths would pass
//! on Linux and fail here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("neb")
}

struct Corpus {
    dir: tempfile::TempDir,
    root: PathBuf,
}

impl Corpus {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("corpus");
        let me = Self { dir, root };
        me.run(&["init"]).assert_ok();
        me
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with_env(args, &[])
    }

    fn run_with_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut cmd = Command::new(bin());
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("NEBULA_ROOT");
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("running neb");
        Run {
            args: args.join(" "),
            out,
        }
    }

    fn workdir(&self) -> &Path {
        self.dir.path()
    }

    fn node_file(&self, id: &str) -> PathBuf {
        self.root.join("nodes").join(format!("{id}.md"))
    }

    /// Create a seed by capturing then promoting, the normal path.
    fn seed(&self, text: &str, title: &str) -> String {
        let id = self.run(&["capture", text]).stdout_trim();
        self.run(&["promote", &id, "--title", title])
            .assert_ok()
            .stdout_trim()
    }
}

struct Run {
    args: String,
    out: Output,
}

impl Run {
    fn assert_ok(self) -> Self {
        assert!(
            self.out.status.success(),
            "`neb {}` failed:\n{}\n{}",
            self.args,
            String::from_utf8_lossy(&self.out.stdout),
            String::from_utf8_lossy(&self.out.stderr)
        );
        self
    }
    fn assert_fails(self) -> Self {
        assert!(
            !self.out.status.success(),
            "`neb {}` unexpectedly succeeded",
            self.args
        );
        self
    }
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.out.stdout).to_string()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).to_string()
    }
    fn stdout_trim(&self) -> String {
        self.stdout()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    }
    fn says(self, needle: &str) -> Self {
        let all = format!("{}{}", self.stdout(), self.stderr());
        assert!(
            all.contains(needle),
            "expected `{needle}` in output of `neb {}`:\n{all}",
            self.args
        );
        self
    }
}

fn write(path: &Path, s: &str) {
    std::fs::write(path, s).expect("writing fixture");
}

/// `YYYY-MM-DD` for `days` ago, computed the same way `store::days_since`
/// computes "now", so fixtures land unambiguously on one side of a threshold
/// regardless of what day the suite actually runs.
fn date_days_ago(days: i64) -> String {
    use time::{OffsetDateTime, macros::format_description};
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = now.date() - time::Duration::days(days);
    date.format(format_description!("[year]-[month]-[day]"))
        .expect("formatting a date")
}

/// The inbox stamp format is the date plus a time-of-day, but only the date
/// half feeds the fourteen-day rule.
fn stamp_days_ago(days: i64) -> String {
    format!("{}T00:00", date_days_ago(days))
}

/// Back-date a node's `updated` field in place, to land it on a chosen side
/// of a staleness threshold without waiting for real time to pass.
fn set_updated(path: &Path, date: &str) {
    let raw = std::fs::read_to_string(path).unwrap();
    let needle = "\nupdated: ";
    let start = raw.find(needle).unwrap() + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, date);
    write(path, &raw);
}

/// Back-date one capture's timestamp in place, found by its own text rather
/// than by position, since `Corpus::seed` leaves earlier settled captures in
/// the same monthly inbox file ahead of whichever one a test cares about.
fn set_inbox_stamp_for(root: &Path, capture_text: &str, stamp: &str) {
    let inbox_dir = root.join("inbox");
    for entry in std::fs::read_dir(&inbox_dir).unwrap() {
        let path = entry.unwrap().path();
        let mut raw = std::fs::read_to_string(&path).unwrap();
        let Some(text_at) = raw.find(capture_text) else {
            continue;
        };
        let line_start = raw[..text_at].rfind('\n').map_or(0, |i| i + 1);
        let stamp_start = raw[line_start..].find("] ").unwrap() + line_start + 2;
        let stamp_end = stamp_start + "2020-01-01T00:00".len();
        raw.replace_range(stamp_start..stamp_end, stamp);
        std::fs::write(&path, raw).unwrap();
        return;
    }
    panic!("no inbox entry contains `{capture_text}`");
}

// ------------------------------------------------------------------ capture --

#[test]
fn capture_then_promote_leaves_the_original_text_in_the_node() {
    let c = Corpus::new();
    let entry = c
        .run(&[
            "capture",
            "ranking decay looks like a half-life, not a cliff",
        ])
        .assert_ok();
    let id = entry.stdout_trim();
    c.run(&["inbox"]).assert_ok().says(&id).says("half-life");

    let node = c
        .run(&["promote", &id, "--title", "Ranking decay half-life"])
        .assert_ok();
    let node_id = node.stdout_trim();
    assert_eq!(node_id, "ranking-decay-half-life");

    let body = std::fs::read_to_string(c.node_file(&node_id)).unwrap();
    assert!(
        body.contains("half-life, not a cliff"),
        "capture text should survive promotion"
    );
}

#[test]
fn promoted_and_dropped_entries_leave_the_inbox_but_stay_on_disk() {
    let c = Corpus::new();
    let keep = c.run(&["capture", "worth keeping"]).stdout_trim();
    let toss = c.run(&["capture", "not worth keeping"]).stdout_trim();

    c.run(&["promote", &keep, "--title", "Worth keeping"])
        .assert_ok();
    c.run(&["drop", &toss]).assert_ok();

    let listing = c.run(&["inbox"]).assert_ok().stdout();
    assert!(
        !listing.contains(&keep) && !listing.contains(&toss),
        "both should be settled"
    );

    // Nothing is deleted: the record of what was captured, and what became of
    // it, is the part that stops you re-treading ground.
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let raw = std::fs::read_to_string(month.path()).unwrap();
    assert!(
        raw.contains("not worth keeping"),
        "dropped text must remain on disk"
    );
    assert!(raw.contains("dropped"), "drop should be recorded");
    assert!(
        raw.contains("worth-keeping"),
        "promotion should name the node it became"
    );
}

#[test]
fn capture_works_before_a_corpus_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");
    let out = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .args(["capture", "the thought arrives before the setup"])
        .env_remove("NEBULA_ROOT")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "capture must never be blocked by missing setup"
    );
    assert!(root.join("inbox").is_dir());
}

#[test]
fn zsh_completions_include_the_cli_commands() {
    let c = Corpus::new();
    c.run(&["completions", "zsh"])
        .assert_ok()
        .says("#compdef neb")
        .says("capture");
}

// ------------------------------------------------------------------- graph --

#[test]
fn orbit_provenance_is_recorded_only_when_supplied() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "Orbit-produced idea",
        "--task",
        "ORB-12345",
        "--run",
        "jrun-20260907-0001",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("orbit-produced-idea")).unwrap();
    assert!(raw.contains("origin:\n  task: ORB-12345\n  run: jrun-20260907-0001"));

    c.run(&["new", "Unattributed idea"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file("unattributed-idea")).unwrap();
    assert!(!raw.contains("origin:"));

    let cited = c.seed("context", "Context");
    c.run(&[
        "cite",
        &cited,
        "--uri",
        "https://example.com",
        "--task",
        "ORB-12345",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&cited)).unwrap();
    assert!(raw.contains("references:"));
    assert!(raw.contains("task: ORB-12345"));

    let entry = c
        .run(&["capture", "promoted with provenance"])
        .stdout_trim();
    c.run(&[
        "promote",
        &entry,
        "--title",
        "Promoted with provenance",
        "--run",
        "jrun-20260907-0002",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("promoted-with-provenance")).unwrap();
    assert!(raw.contains("run: jrun-20260907-0002"));
}

#[test]
fn a_node_can_descend_from_two_parents_and_trace_shows_the_diamond() {
    let c = Corpus::new();
    let a = c.seed("gravity might be about scarcity", "Gravity as scarcity");
    let b = c.seed(
        "a moving source should drag the field",
        "Moving source drag",
    );

    c.run(&[
        "new",
        "Retardation in the wake",
        "--parent",
        &a,
        "--parent",
        &b,
    ])
    .assert_ok();

    let tree = c
        .run(&["trace", "retardation-in-the-wake"])
        .assert_ok()
        .stdout();
    assert!(
        tree.contains(&a) && tree.contains(&b),
        "both parents belong in the trace:\n{tree}"
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_shared_ancestor_is_reached_by_both_branches_and_expanded_once() {
    let c = Corpus::new();
    let root_id = c.seed("space might have a density of something", "Density hunch");
    c.run(&["new", "Left branch", "--parent", &root_id])
        .assert_ok();
    c.run(&["new", "Right branch", "--parent", &root_id])
        .assert_ok();
    c.run(&[
        "new",
        "Synthesis",
        "--parent",
        "left-branch",
        "--parent",
        "right-branch",
    ])
    .assert_ok();

    let tree = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        tree.matches(&root_id).count(),
        2,
        "the shared ancestor appears on both branches"
    );
    assert!(
        tree.contains("shown above"),
        "but it is only expanded once:\n{tree}"
    );
}

#[test]
fn genealogy_cycles_are_refused_at_the_point_of_linking() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    c.run(&["new", "Second", "--parent", &a]).assert_ok();
    c.run(&["link", &a, "derives-from", "second"])
        .assert_fails()
        .says("its own ancestor");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn contradicts_is_recorded_on_both_nodes() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["link", &a, "contradicts", &b]).assert_ok();
    c.run(&["show", &b])
        .assert_ok()
        .says("contradicts")
        .says(&a);
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn impact_reports_descendants_and_contradictions() {
    let c = Corpus::new();
    let base = c.seed("foundation", "Foundation");
    c.run(&["new", "Child", "--parent", &base]).assert_ok();
    c.run(&["new", "Grandchild", "--parent", "child"])
        .assert_ok();
    let rival = c.seed("the other way round", "Rival");
    c.run(&["link", &base, "contradicts", &rival]).assert_ok();
    let unrelated = c.seed("nothing to do with it", "Unrelated");

    let out = c.run(&["impact", &base]).assert_ok().stdout();
    assert!(out.contains("child") && out.contains("grandchild"), "{out}");
    assert!(out.contains(&rival), "{out}");
    assert!(!out.contains(&unrelated), "{out}");

    let json = c.run(&["--json", "impact", &base]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["id"] == "grandchild" && i["via"] == "descends"),
        "{json}"
    );
    assert!(
        items
            .iter()
            .any(|i| i["id"] == rival && i["via"] == "contradicts"),
        "{json}"
    );
    c.run(&["impact", &unrelated])
        .assert_ok()
        .says("nothing descends from or contradicts");
}

// -------------------------------------------------------------- discipline --

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
        .says("--why only applies");
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
    // goes with it.
    c.run(&["status", &reasoned, "seed"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&reasoned)).unwrap();
    assert!(!raw.contains("closed:"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors");
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
    c.run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("cannot simply reopen");
    c.run(&["status", &id, "abandoned"])
        .assert_fails()
        .says("cannot simply reopen");
    c.run(&["new", "Second attempt", "--kill", "if Y"])
        .assert_ok();
    c.run(&["show", "second-attempt"])
        .assert_ok()
        .says("hypothesis");
    c.run(&["link", "second-attempt", "reopens", &id])
        .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_reference_cannot_smuggle_in_a_verdict() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace(
        "status: seed",
        "status: seed\nreferences:\n- id: r1\n  kind: paper\n  uri: http://example.com\n  added: 2026-09-06\n  verdict: supports",
    );
    write(&c.node_file(&id), &raw);
    // The separation between context and evidence is the discipline the whole
    // system exists to impose, so it fails to parse rather than merely warning.
    c.run(&["check"])
        .assert_fails()
        .says("unknown field `verdict`");
}

#[test]
fn a_reference_with_no_note_warns_and_a_noted_one_does_not() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &bare,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "",
    ])
    .assert_ok();
    c.run(&["check"])
        .assert_ok()
        .says("[9]")
        .says("reference `r1` has no note saying why it is here")
        .says(&bare)
        .says("0 errors, 1 warnings");

    let omitted = Corpus::new();
    let omitted_id = omitted.seed("another idea", "Another idea");
    omitted
        .run(&[
            "cite",
            &omitted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
        ])
        .assert_ok();
    omitted
        .run(&["check"])
        .assert_ok()
        .says("[9]")
        .says("0 errors, 1 warnings");

    let noted = Corpus::new();
    let noted_id = noted.seed("a third idea", "A third idea");
    noted
        .run(&[
            "cite",
            &noted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
            "--note",
            "explains the mechanism",
        ])
        .assert_ok();
    noted
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

#[test]
fn a_local_uri_must_resolve_and_external_urls_never_trip_it() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    // Rule 8 at the point of action: `cite` refuses a path to nothing.
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_fails()
        .says("does not resolve");

    std::fs::create_dir_all(c.root.join("nodes").join("notes")).unwrap();
    write(
        &c.root.join("nodes").join("notes").join("missing.md"),
        "here now",
    );
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // And in `check`, for the file that went missing later: an error, since
    // a citation to nothing is a broken record rather than an untidy one.
    std::fs::remove_file(c.root.join("nodes").join("notes").join("missing.md")).unwrap();
    c.run(&["check"])
        .assert_fails()
        .says("[8]")
        .says("points at a path that does not resolve: ./notes/missing.md")
        .says("1 errors");

    // Schemes and URLs are never resolved, so none of these trip rule 8.
    let online = Corpus::new();
    let online_id = online.seed("an online idea", "An online idea");
    for uri in [
        "https://example.org/paper",
        "doi:10.1000/x",
        "orbit:DANI-10345",
        "[[almanac/some-page]]",
    ] {
        online
            .run(&["cite", &online_id, "--uri", uri, "--note", "n"])
            .assert_ok();
    }
    online
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
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

    let rule9 = findings
        .iter()
        .find(|f| f["rule"] == 9)
        .expect("rule 9 finding present");
    assert_eq!(rule9["level"], "warn");
    assert_eq!(rule9["node"], id);

    let rule10 = findings
        .iter()
        .find(|f| f["rule"] == 10)
        .expect("rule 10 finding present");
    assert_eq!(rule10["level"], "warn");
    assert!(rule10["node"].is_null(), "tag drift is a corpus finding");
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

// ------------------------------------------------------------------ triage --

#[test]
fn open_finds_the_hypothesis_with_no_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["open"])
        .assert_ok()
        .says(&id)
        .says("hypothesis with no references");

    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "n"])
        .assert_ok();
    let after = c.run(&["open"]).assert_ok().stdout();
    assert!(
        !after.contains("no references"),
        "a reference closes the gap:\n{after}"
    );
}

#[test]
fn open_finds_seeds_untouched_for_ninety_days_and_filters_by_tag() {
    let c = Corpus::new();
    let old = c.seed("an old seed", "An old seed");
    c.run(&["tag", &old, "--add", "physics"]).assert_ok();
    set_updated(&c.node_file(&old), &date_days_ago(90));
    let fresh = c.seed("a fresh seed", "A fresh seed");
    set_updated(&c.node_file(&fresh), &date_days_ago(89));

    let out = c.run(&["open"]).assert_ok().stdout();
    assert!(
        out.contains(&old) && out.contains("seed untouched for ninety days"),
        "{out}"
    );
    assert!(!out.contains(&fresh), "{out}");

    let out = c.run(&["open", "--tag", "physics"]).assert_ok().stdout();
    assert!(out.contains(&old), "{out}");
    let out = c.run(&["open", "--tag", "orrery"]).assert_ok().stdout();
    assert!(!out.contains(&old), "{out}");
}

#[test]
fn open_finds_inbox_captures_waiting_over_fourteen_days() {
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

    c.run(&["open"])
        .assert_ok()
        .says("1 captures waiting over fourteen days; promote or drop them");

    let json = c.run(&["open", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items.iter().any(|item| item["id"] == "inbox"),
        "inbox finding missing: {json}"
    );
}

#[test]
fn json_output_is_machine_readable() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["id"], id);
    assert_eq!(v["body"], "an idea");

    let empty = c.run(&["new", "Empty prose"]).assert_ok().stdout_trim();
    let out = c.run(&["--json", "show", &empty]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["body"], "");

    let out = c.run(&["--json", "check"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json is valid JSON");
    assert_eq!(v["nodes"], 2);
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

// -------------------------------------------------------------------- tags --

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
        .says("machine-learning 1")
        .says("physics 2");
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
        .says("[10]")
        .says("tags `Physics` and `physics` differ only by case")
        .says("tags `sim` and `sims` differ only by a trailing `s`")
        .says("0 errors, 2 warnings");
}

// ------------------------------------------------------------------ review --

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
fn review_reports_nodes_with_no_references() {
    let c = Corpus::new();
    let bare = c.seed("a bare idea", "A bare idea");

    let cited = c.seed("a cited idea", "A cited idea");
    c.run(&["cite", &cited, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();

    let abandoned = c.seed("an abandoned idea", "An abandoned idea");
    c.run(&["status", &abandoned, "abandoned"]).assert_ok();

    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(out.contains(&format!("`{bare}`")), "{out}");
    assert!(!out.contains(&format!("`{cited}`")), "{out}");
    assert!(!out.contains(&format!("`{abandoned}`")), "{out}");
}

#[test]
fn review_reports_stale_inbox_entries_on_both_sides_of_fourteen_days() {
    let c = Corpus::new();
    c.run(&["capture", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains("1 captures waiting over fourteen days; promote or drop them"),
        "{out}"
    );

    let fresh = Corpus::new();
    fresh.run(&["capture", "a recent capture"]).assert_ok();
    set_inbox_stamp_for(&fresh.root, "a recent capture", &stamp_days_ago(10));
    let out = fresh.run(&["review"]).assert_ok().stdout();
    assert!(
        !out.contains("captures waiting over fourteen days"),
        "{out}"
    );
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

    c.seed("bare idea", "Bare idea");

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

#[test]
fn review_out_writes_the_report_and_prints_nothing_else() {
    let c = Corpus::new();
    let bare = c.seed("an idea needing a look", "An idea needing a look");
    let out_path = c.workdir().join("review.md");
    let run = c
        .run(&["review", "--out", out_path.to_str().unwrap()])
        .assert_ok();
    assert_eq!(
        run.stdout(),
        "",
        "stdout must stay empty when writing to a file"
    );
    let report = std::fs::read_to_string(&out_path).unwrap();
    assert!(report.contains(&format!("`{bare}`")));
    assert!(report.contains("## Nodes with no references"));
}

#[test]
fn an_empty_corpus_produces_an_empty_review() {
    let c = Corpus::new();
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(out.contains("## Hypotheses untouched for 30 days"));
    assert!(out.contains("## Seeds untouched for 90 days"));
    assert!(out.contains("## Nodes with no references"));
    assert!(out.contains("## Inbox entries waiting 14 days or more"));
    assert_eq!(
        out.matches("_none_").count(),
        4,
        "every section is empty:\n{out}"
    );

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(items.is_empty(), "{json}");
}

/// Every file under `nodes/` and `inbox/`, by path, for a before/after diff.
fn snapshot_corpus_files(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    let mut map = std::collections::BTreeMap::new();
    for sub in ["nodes", "inbox"] {
        let dir = root.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let content = std::fs::read_to_string(&path).unwrap();
            map.insert(path, content);
        }
    }
    map
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

// ----------------------------------------------------------------- migrate --

/// A corpus in v1 form: declared domains, evidence with verdicts and
/// strengths, a task link, a weighed reference, the removed edge kinds, and
/// the statuses v2 collapses. Written by hand so the fixture is exactly what
/// v0.1 wrote and nothing in the current binary can shape it.
const V1_CONFIG: &str = r"# nebula corpus configuration. `neb domain` edits this.
schema_version: 1
corpus_id: neb-abc123
domains:
- general
- physics
default_domain: physics
";

const V1_WAKE: &str = r"---
id: wake-retardation
title: Retardation in the scarcity wake
domain: physics
status: supported
created: 2026-09-07
updated: 2026-09-10
kill: If the wake timescale is frame-independent, this is dead.
tags:
- Orrery
edges:
- type: derives-from
  to: gravity-as-scarcity
- type: depends-on
  to: gravity-as-scarcity
evidence:
- id: ev1
  verdict: supports
  strength: strong
  source: sim://boosted-source/run-3
  date: 2026-09-08
  note: |-
    Boosted source shows a lag.
    Frame dependence still to be checked.
  origin:
    task: DANI-10001
- id: ev2
  verdict: undermines
  strength: anecdote
  source: https://example.org/objection
  date: 2026-09-09
references:
- id: r1
  kind: study
  uri: https://example.org/time-dilation
  title: Gravitational time dilation
  note: The constraint any wake timescale has to survive.
  added: 2026-09-07
  promoted_to: ev1
tasks:
- id: DANI-10001
  state: open
  why: run the boosted-source sim at three velocities
---

The wake lags the source.
";

const V1_GRAVITY: &str = r"---
id: gravity-as-scarcity
title: Gravity as scarcity
domain: physics
status: testing
created: 2026-09-01
updated: 2026-09-05
kill: If a dense region shows no pull at all.
evidence:
- id: ev1
  verdict: inconclusive
  strength: suggestive
  source: doi:10.1000/scarcity
  date: 2026-09-04
  note: Looked, learned nothing.
---

Space might have a density of something.
";

const V1_OLD: &str = r"---
id: old-idea
title: Old idea
domain: general
status: graduated
created: 2026-08-01
updated: 2026-08-20
kill: If nobody downstream wants it.
graduated_to: principia://theory/old-idea
---

It went downstream.
";

const V1_DEAD: &str = r"---
id: dead-idea
title: Dead idea
domain: general
status: refuted
created: 2026-08-01
updated: 2026-08-15
kill: If the effect vanishes under control.
edges:
- type: undermines
  to: old-idea
evidence:
- id: ev1
  verdict: undermines
  strength: strong
  source: https://example.org/control-run
  date: 2026-08-15
  note: It vanished under control.
---

The effect vanished.
";

fn v1_corpus() -> Corpus {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    write(&root.join("config.yaml"), V1_CONFIG);
    for (id, text) in [
        ("wake-retardation", V1_WAKE),
        ("gravity-as-scarcity", V1_GRAVITY),
        ("old-idea", V1_OLD),
        ("dead-idea", V1_DEAD),
    ] {
        write(&root.join("nodes").join(format!("{id}.md")), text);
    }
    Corpus { dir, root }
}

#[test]
fn a_v1_corpus_refuses_to_open_until_migrated() {
    let c = v1_corpus();
    c.run(&["list"])
        .assert_fails()
        .says("schema_version 1")
        .says("neb migrate");
    c.run(&["check"]).assert_fails().says("neb migrate");
}

#[test]
fn migrate_relabels_evidence_tasks_and_edges_into_references() {
    let c = v1_corpus();
    let first = c
        .run(&["migrate"])
        .assert_ok()
        .says("wake-retardation")
        .says("domain `physics` -> tag `physics`")
        .says("evidence ev1 -> reference r2")
        .says("evidence ev2 -> reference r3")
        .says("task DANI-10001 -> reference r4")
        .says("edge depends-on gravity-as-scarcity -> reference r5")
        .says("status supported -> hypothesis")
        .says("status testing -> hypothesis")
        .says("status graduated -> abandoned (graduated to principia://theory/old-idea)")
        .says("config.yaml schema_version -> 2")
        .says("4 of 4 nodes rewritten")
        .stdout();
    assert!(!first.contains("nothing changed"), "{first}");

    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 2"), "{config}");
    assert!(config.contains("corpus_id: neb-abc123"), "{config}");
    assert!(
        !config.contains("domains") && !config.contains("default_domain"),
        "{config}"
    );

    let wake = std::fs::read_to_string(c.node_file("wake-retardation")).unwrap();
    for gone in [
        "domain:",
        "evidence:",
        "tasks:",
        "promoted_to",
        "type: depends-on",
        "verdict:",
        "strength:",
    ] {
        assert!(!wake.contains(gone), "`{gone}` survived migration:\n{wake}");
    }
    assert!(wake.contains("status: hypothesis"), "{wake}");
    assert!(wake.contains("tags:\n- orrery\n- physics\n"), "{wake}");
    assert!(
        wake.contains("- type: derives-from\n  to: gravity-as-scarcity\n"),
        "{wake}"
    );
    // The verdict and strength survive as a prefix on the note, the note's
    // first line becomes the title, and the evidence provenance is kept.
    assert!(
        wake.contains(
            "- id: r2\n  kind: other\n  uri: sim://boosted-source/run-3\n  \
             title: Boosted source shows a lag.\n  note: |-\n    \
             [supports/strong] Boosted source shows a lag.\n    \
             Frame dependence still to be checked.\n  added: 2026-09-08\n  \
             origin:\n    task: DANI-10001\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r3\n  kind: other\n  uri: https://example.org/objection\n  \
             note: '[undermines/anecdote]'\n  added: 2026-09-09\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r4\n  kind: other\n  uri: orbit:DANI-10001\n  title: DANI-10001\n  \
             note: '[open] run the boosted-source sim at three velocities'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r5\n  kind: other\n  uri: neb:gravity-as-scarcity\n  \
             title: gravity-as-scarcity\n  note: '[depends-on] gravity-as-scarcity'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains("The wake lags the source."),
        "prose survives:\n{wake}"
    );
    // The migration never masquerades as an edit.
    assert!(wake.contains("updated: 2026-09-10"), "{wake}");
    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
}

#[test]
fn migrate_maps_statuses_and_is_a_no_op_the_second_time() {
    let c = v1_corpus();
    c.run(&["migrate"]).assert_ok();

    let gravity = std::fs::read_to_string(c.node_file("gravity-as-scarcity")).unwrap();
    assert!(gravity.contains("status: hypothesis"), "{gravity}");
    assert!(
        gravity.contains("[inconclusive/suggestive] Looked, learned nothing."),
        "{gravity}"
    );
    assert!(gravity.contains("tags:\n- physics\n"), "{gravity}");

    let old = std::fs::read_to_string(c.node_file("old-idea")).unwrap();
    assert!(old.contains("status: abandoned"), "{old}");
    assert!(
        old.contains(
            "closed:\n  why: graduated to principia://theory/old-idea\n  at: 2026-08-20\n"
        ),
        "{old}"
    );
    assert!(!old.contains("graduated_to"), "{old}");

    let dead = std::fs::read_to_string(c.node_file("dead-idea")).unwrap();
    assert!(dead.contains("status: refuted"), "{dead}");
    assert!(dead.contains("closed:\n  why: refuted under v1"), "{dead}");
    assert!(
        dead.contains("[undermines/strong] It vanished under control."),
        "{dead}"
    );
    assert!(dead.contains("note: '[undermines] old-idea'"), "{dead}");

    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
    let listed = c.run(&["list", "--tag", "physics"]).assert_ok().stdout();
    assert!(
        listed.contains("wake-retardation") && listed.contains("gravity-as-scarcity"),
        "{listed}"
    );

    // Second run: nothing to do, nothing touched.
    let before = snapshot_corpus_files(&c.root);
    let before_config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(
        before_config,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
}

#[test]
fn migrate_refuses_a_dirty_git_tree() {
    let c = v1_corpus();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&c.root)
            .args(args)
            .output()
            .expect("running git");
        assert!(
            out.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    c.run(&["migrate"])
        .assert_fails()
        .says("uncommitted changes");
    // Refused before anything was written.
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 1"), "{config}");

    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=neb-test",
        "-c",
        "user.email=neb-test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "-m",
        "v1 corpus",
    ]);
    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}
