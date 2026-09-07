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
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Corpus {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("corpus");
        let me = Self { _dir: dir, root };
        me.run(&["init"]).assert_ok();
        me
    }

    fn run(&self, args: &[&str]) -> Run {
        let out = Command::new(bin())
            .arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("NEBULA_ROOT")
            .output()
            .expect("running neb");
        Run {
            args: args.join(" "),
            out,
        }
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
fn a_supports_cycle_is_a_warning_not_an_error() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["link", &a, "supports", &b]).assert_ok();
    c.run(&["link", &b, "supports", &a]).assert_ok();
    // Circular reasoning is worth noticing and not worth blocking on: sometimes
    // two ideas really do lean on each other and saying so is the honest record.
    c.run(&["check"])
        .assert_ok()
        .says("circular reasoning")
        .says("0 errors");
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
fn impact_reports_what_leans_on_a_node() {
    let c = Corpus::new();
    let base = c.seed("foundation", "Foundation");
    let dep = c.seed("built on it", "Dependent");
    c.run(&["link", &dep, "depends-on", &base]).assert_ok();
    c.run(&["impact", &base])
        .assert_ok()
        .says(&dep)
        .says("dies with it");
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
fn a_verdict_must_rest_on_evidence() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["status", &id, "supported"])
        .assert_fails()
        .says("nothing supports");

    c.run(&[
        "evidence",
        &id,
        "--verdict",
        "supports",
        "--source",
        "sim://run-1",
    ])
    .assert_ok();
    c.run(&["status", &id, "supported"]).assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_refuted_idea_cannot_quietly_come_back() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&[
        "evidence",
        &id,
        "--verdict",
        "undermines",
        "--source",
        "sim://run-2",
    ])
    .assert_ok();
    c.run(&["status", &id, "refuted"]).assert_ok();

    // Reviving it takes a new node with a `reopens` edge, so the fact that it
    // was once ruled out stays visible in the graph.
    c.run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("cannot simply reopen");
    c.run(&[
        "new",
        "Second attempt",
        "--kill",
        "if Y",
        "--status",
        "hypothesis",
    ])
    .assert_ok();
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
fn weighing_a_reference_keeps_the_reading_history() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "doi:10.1000/x",
        "--kind",
        "paper",
        "--note",
        "read this",
    ])
    .assert_ok();
    c.run(&[
        "weigh",
        &id,
        "r1",
        "--verdict",
        "undermines",
        "--strength",
        "strong",
    ])
    .assert_ok();

    let shown = c.run(&["show", &id]).assert_ok().stdout();
    assert!(shown.contains("r1"), "the reference stays put");
    assert!(
        shown.contains("ev1"),
        "and the evidence it became appears too"
    );
    c.run(&["weigh", &id, "r1", "--verdict", "supports"])
        .assert_fails()
        .says("already weighed");
}

#[test]
fn evidence_ids_are_never_reused() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["evidence", &id, "--verdict", "supports", "--source", "a"])
        .assert_ok();
    c.run(&["evidence", &id, "--verdict", "supports", "--source", "b"])
        .assert_ok();

    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace("  id: ev1\n", "  id: evX\n"); // simulate a removal
    write(&c.node_file(&id), &raw);
    c.run(&["evidence", &id, "--verdict", "supports", "--source", "c"])
        .assert_ok();

    let shown = c.run(&["show", &id]).assert_ok().stdout();
    assert!(
        shown.contains("ev3"),
        "the next id counts past the highest ever issued:\n{shown}"
    );
}

#[test]
fn graduating_must_say_where_the_idea_went() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["status", &id, "graduated"])
        .assert_fails()
        .says("--to");
    // Downstream demands a falsifier, so graduating without one is refused here
    // rather than exporting the gap to principia.
    c.run(&["graduate", &id, "--to", "principia://x"])
        .assert_fails()
        .says("no kill condition");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["graduate", &id, "--to", "principia://theory/an-idea"])
        .assert_ok();
    c.run(&["show", &id])
        .assert_ok()
        .says("principia://theory/an-idea");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn malformed_task_ids_are_caught() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["task", &id, "ORB-oops"]).assert_ok();
    c.run(&["check"])
        .assert_fails()
        .says("malformed Orbit task id")
        .says("expected PREFIX-nnnnn");
    for task_id in ["orb-1", "ORB-", "-123", "ORB-12-3"] {
        let malformed = Corpus::new();
        let malformed_id = malformed.seed("another idea", "Another idea");
        malformed
            .run(&["task", &malformed_id, "--", task_id])
            .assert_ok();
        malformed
            .run(&["check"])
            .assert_fails()
            .says("malformed Orbit task id");
    }
    let valid = Corpus::new();
    let valid_id = valid.seed("another idea", "Another idea");
    valid
        .run(&["task", &valid_id, "DANI-10293", "--state", "open"])
        .assert_ok();
    valid.run(&["check"]).assert_ok().says("0 errors");
    valid
        .run(&["task", &valid_id, "ORB-11440", "--state", "open"])
        .assert_ok();
    valid.run(&["check"]).assert_ok().says("0 errors");
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
fn open_finds_the_hypothesis_with_nothing_running() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["open"])
        .assert_ok()
        .says("no evidence and no task running");

    c.run(&["task", &id, "ORB-11440", "--why", "run the sim"])
        .assert_ok();
    let after = c.run(&["open"]).assert_ok().stdout();
    assert!(
        !after.contains("no task running"),
        "a spawned task closes the gap:\n{after}"
    );
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
    c.run(&["open", "--all"])
        .assert_ok()
        .says("1 captures waiting over fourteen days; promote or drop them");
    c.run(&["open", "--domain", "general"])
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

// ----------------------------------------------------------------- domains --

#[test]
fn a_fresh_corpus_has_one_domain_and_asks_nothing() {
    let c = Corpus::new();
    c.run(&["domain", "list"])
        .assert_ok()
        .says("general")
        .says("(default)");
    let id = c.seed("an idea", "An idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("domain: general"), "{raw}");
    // One domain: no scope footer, nothing to cross.
    let out = c.run(&["list"]).assert_ok().stdout();
    assert!(!out.contains("--all"), "{out}");
}

#[test]
fn several_domains_without_a_default_make_new_a_decision() {
    let c = Corpus::new();
    c.run(&["domain", "add", "work"]).assert_ok();
    // A fresh corpus defaults to `general`, so a second domain alone is
    // still decision-free. Remove the default by hand to reach the case.
    let cfg = c.root.join("config.yaml");
    let raw = std::fs::read_to_string(&cfg).unwrap();
    let kept: Vec<&str> = raw
        .lines()
        .filter(|l| !l.starts_with("default_domain"))
        .collect();
    write(&cfg, &(kept.join("\n") + "\n"));
    c.run(&["new", "Undecided"]).assert_fails().says("--domain");
    c.run(&["new", "Placed", "--domain", "work"]).assert_ok();
    c.run(&["new", "Nowhere", "--domain", "nope"])
        .assert_fails()
        .says("no domain `nope`")
        .says("general, work");
    c.run(&["domain", "default", "work"]).assert_ok();
    let id = c.run(&["new", "Defaulted"]).assert_ok().stdout_trim();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("domain: work"), "{raw}");
}

#[test]
fn list_scopes_to_the_default_domain_and_all_crosses() {
    let c = Corpus::new();
    c.run(&["domain", "add", "work"]).assert_ok();
    c.run(&["domain", "default", "general"]).assert_ok();
    c.run(&["new", "Personal thing"]).assert_ok();
    c.run(&["new", "Work thing", "--domain", "work"])
        .assert_ok();

    let scoped = c
        .run(&["list"])
        .assert_ok()
        .says("domain: general")
        .stdout();
    assert!(
        scoped.contains("personal-thing") && !scoped.contains("work-thing"),
        "{scoped}"
    );

    let all = c.run(&["list", "--all"]).assert_ok().stdout();
    assert!(
        all.contains("personal-thing") && all.contains("work-thing"),
        "{all}"
    );
    assert!(all.contains("[work]") && all.contains("[general]"), "{all}");

    let work = c.run(&["list", "--domain", "work"]).assert_ok().stdout();
    assert!(
        !work.contains("personal-thing") && work.contains("work-thing"),
        "{work}"
    );

    c.run(&["list", "--domain", "work", "--all"]).assert_fails();
}

#[test]
fn edges_cross_domains_because_a_domain_is_a_view_not_a_wall() {
    let c = Corpus::new();
    c.run(&["domain", "add", "physics"]).assert_ok();
    c.run(&["domain", "default", "general"]).assert_ok();
    c.run(&["new", "Ranking decay"]).assert_ok();
    c.run(&[
        "new",
        "Dissipation analogy",
        "--domain",
        "physics",
        "--parent",
        "ranking-decay",
    ])
    .assert_ok();
    c.run(&["trace", "dissipation-analogy"])
        .assert_ok()
        .says("ranking-decay");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_node_without_a_domain_fails_check_until_placed() {
    let c = Corpus::new();
    let id = c.seed("a legacy idea", "A legacy idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(&c.node_file(&id), &raw.replace("domain: general\n", ""));
    c.run(&["check"]).assert_fails().says("no domain");
    c.run(&["domain", "list"])
        .assert_ok()
        .says("1 nodes name no declared domain");

    c.run(&["domain", "set", &id, "general"])
        .assert_ok()
        .says("(none) -> general");
    c.run(&["check"]).assert_ok().says("0 errors");

    // An undeclared spelling is caught the same way.
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("domain: general", "domain: General"),
    );
    c.run(&["check"]).assert_fails().says("not declared");
}

#[test]
fn unplaced_domain_set_bulk_places_only_empty_nodes() {
    let c = Corpus::new();
    let unplaced = c.seed("a legacy idea", "A legacy idea");
    let placed = c.seed("an existing idea", "An existing idea");
    let raw = std::fs::read_to_string(c.node_file(&unplaced)).unwrap();
    write(
        &c.node_file(&unplaced),
        &raw.replace("domain: general\n", ""),
    );

    c.run(&["domain", "set", "--unplaced", "general"])
        .assert_ok()
        .says("(none) -> general")
        .says("placed 1 nodes");
    let unplaced_raw = std::fs::read_to_string(c.node_file(&unplaced)).unwrap();
    let placed_raw = std::fs::read_to_string(c.node_file(&placed)).unwrap();
    assert!(unplaced_raw.contains("domain: general"), "{unplaced_raw}");
    assert!(placed_raw.contains("domain: general"), "{placed_raw}");
    c.run(&["check"]).assert_ok().says("0 errors");

    c.run(&["domain", "set", &placed, "--unplaced", "general"])
        .assert_fails()
        .says("cannot be used with");
    c.run(&["domain", "set", "--unplaced", "missing"])
        .assert_fails()
        .says("no domain `missing`");
}

#[test]
fn domain_names_are_slugs_and_never_declared_twice() {
    let c = Corpus::new();
    c.run(&["domain", "add", "Principia"])
        .assert_fails()
        .says("lowercase");
    c.run(&["domain", "add", "principia"]).assert_ok();
    c.run(&["domain", "add", "principia"])
        .assert_fails()
        .says("already");
    c.run(&["domain", "default", "nope"]).assert_fails();
}
