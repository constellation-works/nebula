//! `neb handoff`: citing an Observatory record and closing the node in one
//! write, its JSON, and the hints on stderr.

use crate::harness::{Corpus, date_days_ago, run_from_home, write};
use crate::observatory::observatory;
use std::path::{Path, PathBuf};

/// [`observatory`], plus the hypothesis record `H012` a hand-off goes to.
pub(super) fn observatory_with_h012(at: &Path) -> (PathBuf, PathBuf) {
    let root = observatory(at);
    let record = root.join("hypotheses").join("H012-scarcity-wake.md");
    write(&record, "# H012\n");
    (root, record)
}

/// The one-step hand-off: one `observatory` reference and the node closed as
/// abandoned for it, in a single write that `check` passes. `show` and
/// `trace` say where the idea went.
#[test]
fn handoff_cites_the_record_and_closes_the_node_in_one_write() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    let parent = c.seed("gravity as scarcity", "Gravity as scarcity");
    let id = c
        .run(&["new", "Scarcity wake", "--parent", &parent])
        .assert_ok()
        .stdout_trim();

    c.run(&[
        "handoff",
        &id,
        "h012",
        "--note",
        "the hypothesis this became",
    ])
    .assert_ok()
    .says(&format!("{id} seed -> abandoned, handed off to H012 (r1)"))
    .says(record.to_str().unwrap());

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(raw.matches("\n- id: r").count(), 1, "one reference:\n{raw}");
    assert!(raw.contains("kind: observatory"), "{raw}");
    assert!(raw.contains("uri: H012"), "{raw}");
    assert!(raw.contains("note: the hypothesis this became"), "{raw}");
    assert!(raw.contains("status: abandoned"), "{raw}");
    assert!(raw.contains("why: handed off to H012"), "{raw}");
    assert!(
        !raw.contains(obs.to_str().unwrap()),
        "no machine path may reach the corpus:\n{raw}"
    );
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // `show` puts the record's location under the reason it closed.
    let shown = c.run(&["show", &id]).assert_ok().stdout();
    let location = format!(
        "closed: handed off to H012 {}\n        {}\n",
        date_days_ago(0),
        record.display()
    );
    assert!(shown.contains(&location), "{shown}");
    let v: serde_json::Value =
        serde_json::from_str(&c.run(&["--json", "show", &id]).assert_ok().stdout()).unwrap();
    assert_eq!(v["handed_off_to"], "H012");
    assert_eq!(v["observatory"][0]["path"], record.to_str().unwrap());

    // `trace` names the hand-off in its payload; the tree a terminal gets
    // shows it on the node's own line (`render::tree`'s unit tests).
    let walk: serde_json::Value =
        serde_json::from_str(&c.run(&["--json", "trace", &id]).assert_ok().stdout()).unwrap();
    assert_eq!(walk[0]["handed_off_to"], "H012");
    assert!(
        walk[1]["handed_off_to"].is_null(),
        "null for a node never handed off: {walk}"
    );
}

/// `--json` is the core `HandedOff`: the node as written, the new reference,
/// the record as stored, and the status it left.
#[test]
fn handoff_json_is_the_node_the_reference_the_record_and_the_old_status() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    let id = c
        .run(&[
            "new",
            "Scarcity wake",
            "--kill",
            "if the wake is frame-independent",
        ])
        .assert_ok()
        .stdout_trim();
    let out = c
        .run_with_env(
            &[
                "--json", "handoff", &id, "H012", "--note", "n", "--by", "agent:x",
            ],
            &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("handoff --json is JSON");
    assert_eq!(v["reference"], "r1");
    assert_eq!(v["record"], "H012");
    assert_eq!(v["from"], "hypothesis");
    assert_eq!(v["doc"]["node"]["status"], "abandoned");
    assert_eq!(v["doc"]["node"]["closed"]["why"], "handed off to H012");
    assert_eq!(v["doc"]["node"]["references"][0]["kind"], "observatory");
    assert_eq!(v["doc"]["node"]["references"][0]["uri"], "H012");
    assert_eq!(v["doc"]["node"]["references"][0]["by"], "agent:x");
    assert!(
        !out.contains("committed") && !out.contains("resolve"),
        "the payload is all stdout holds:\n{out}"
    );
}

/// `cite --json` carries where an `observatory` reference's record is, as
/// its prose does (STD-01 §R6): `path` is the resolved file under the root,
/// `null` with no root, and the whole `observatory` is `null` for any other
/// kind.
#[test]
fn cite_json_carries_observatory_resolution() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    let id = c.seed("an idea", "An idea");
    let cite = |root: Option<&Path>| {
        let mut env = Vec::new();
        if let Some(root) = root {
            env.push(("OBSERVATORY_ROOT", root.to_str().unwrap()));
        }
        let out = c
            .run_with_env(
                &[
                    "--json",
                    "cite",
                    &id,
                    "--kind",
                    "observatory",
                    "--uri",
                    "H012",
                    "--note",
                    "n",
                ],
                &env,
            )
            .assert_ok()
            .stdout();
        serde_json::from_str::<serde_json::Value>(&out).unwrap()
    };

    let v = cite(Some(&obs));
    assert_eq!(v["observatory"]["reference"], v["reference"]);
    assert_eq!(v["observatory"]["record"], "H012");
    assert_eq!(v["observatory"]["path"], record.to_str().unwrap());
    let v = cite(None);
    assert_eq!(v["observatory"]["record"], "H012");
    assert!(v["observatory"]["path"].is_null(), "{v}");

    let out = c
        .run(&[
            "--json",
            "cite",
            &id,
            "--kind",
            "paper",
            "--uri",
            "https://example.org",
            "--note",
            "n",
        ])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.as_object().unwrap().contains_key("observatory"), "{v}");
    assert!(v["observatory"].is_null(), "{v}");
}

/// `handoff --json` carries where the record is, as its prose does
/// (STD-01 §R6), and `path` is `null` with no root.
#[test]
fn handoff_json_carries_observatory_resolution() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    let located = c.seed("an idea", "An idea");
    let out = c
        .run_with_env(
            &["--json", "handoff", &located, "H012", "--note", "n"],
            &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["observatory"]["reference"], "r1");
    assert_eq!(v["observatory"]["record"], "H012");
    assert_eq!(v["observatory"]["path"], record.to_str().unwrap());

    let unlocated = c.seed("another idea", "Another idea");
    let out = c
        .run(&["--json", "handoff", &unlocated, "H012", "--note", "n"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["observatory"]["record"], "H012");
    assert!(
        v["observatory"].as_object().unwrap().contains_key("path")
            && v["observatory"]["path"].is_null(),
        "{v}"
    );
}

/// With no observatory root on this machine, the id is accepted on its
/// shape and the hand-off warns exactly as `cite --kind observatory` does.
#[test]
fn handoff_with_no_observatory_root_accepts_the_id_and_warns_as_cite_does() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let other = c.seed("another idea", "Another idea");
    let cited = c
        .run(&[
            "cite",
            &other,
            "--kind",
            "observatory",
            "--uri",
            "H012",
            "--note",
            "n",
        ])
        .assert_ok();
    assert_eq!(
        cited.stdout(),
        format!("{other} r1\n"),
        "the reference alone"
    );
    let warning = cited.stderr();
    assert!(warning.starts_with("No observatory root set"), "{warning}");
    assert_eq!(warning.lines().count(), 1, "{warning}");

    let out = c.run(&["handoff", &id, "H012", "--note", "n"]).assert_ok();
    assert!(
        !out.stdout().contains("No observatory root"),
        "{}",
        out.stdout()
    );
    assert_eq!(out.stderr(), warning, "the same one line on stderr");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("why: handed off to H012"), "{raw}");
}

/// Hints and advice are stderr's, so stdout holds only what a verb did or
/// read (STD-01 §R12): `init`'s `--set-root` hint, `cite`'s and `handoff`'s
/// notes about the record and a missing note, and `config`'s advice.
#[test]
#[allow(clippy::too_many_lines)] // One walk through every hint is easier to audit than five.
fn init_cite_handoff_and_config_hints_are_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = dir.path().join("scratch");
    let init = run_from_home(&dir.path().join("home"), Some(&scratch), &["init"], None).assert_ok();
    assert_eq!(
        init.stdout(),
        format!("corpus ready at {}\n", scratch.display())
    );
    assert!(init.stderr().contains("--set-root"), "{}", init.stderr());

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let other = c.seed("another idea", "Another idea");

    let cited = c
        .run(&["cite", &id, "--kind", "observatory", "--uri", "H012"])
        .assert_ok();
    assert_eq!(cited.stdout(), format!("{id} r1\n"));
    let stderr = cited.stderr();
    let said: Vec<&str> = stderr.lines().collect();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[0].starts_with("No observatory root set"), "{said:?}");
    assert!(said[1].starts_with("No note."), "{said:?}");

    let handed = c.run(&["handoff", &other, "H012"]).assert_ok();
    assert!(
        handed
            .stdout()
            .starts_with(&format!("{other} seed -> abandoned")),
        "{}",
        handed.stdout()
    );
    assert_eq!(handed.stdout().lines().count(), 1, "{}", handed.stdout());
    assert_eq!(handed.stderr(), cited.stderr(), "the same two lines");

    let (obs, record) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap();
    let env = [("OBSERVATORY_ROOT", obs)];
    let resolved = c
        .run_with_env(
            &[
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                "H012",
                "--note",
                "n",
            ],
            &env,
        )
        .assert_ok();
    assert_eq!(
        resolved.stdout(),
        format!("{id} r2\n{}\n", record.display()),
        "the record's path is the result"
    );
    assert_eq!(resolved.stderr(), "");
    let unresolved = c
        .run_with_env(
            &[
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                "H999",
                "--note",
                "n",
            ],
            &env,
        )
        .assert_ok();
    assert_eq!(unresolved.stdout(), format!("{id} r3\n"));
    assert!(
        unresolved
            .stderr()
            .starts_with("`H999` does not resolve under"),
        "{}",
        unresolved.stderr()
    );

    let commit = c.run(&["config", "commit"]).assert_ok();
    assert_eq!(commit.stdout(), "off\n");
    assert!(
        commit.stderr().contains("neb config commit on"),
        "{}",
        commit.stderr()
    );
    let commit = c.run(&["--json", "config", "commit"]).assert_ok();
    assert_eq!(commit.stderr(), "", "advice is for a person");

    let unset = c.run(&["config", "observatory-root"]).assert_ok();
    assert_eq!(unset.stdout(), "");
    assert!(
        unset.stderr().starts_with("no observatory root; set one"),
        "{}",
        unset.stderr()
    );
    let saved = c
        .run_with_env(&["config", "observatory-root", obs], &env)
        .assert_ok();
    assert!(saved.stdout().starts_with(obs), "{}", saved.stdout());
    assert_eq!(saved.stdout().lines().count(), 1, "{}", saved.stdout());
    assert!(
        saved.stderr().contains("outranks it while exported"),
        "{}",
        saved.stderr()
    );
}

/// Each refusal exits non-zero, writes nothing, and under `--json` is the
/// typed envelope.
#[test]
fn handoff_refuses_a_closed_node_an_unknown_one_and_an_unresolved_record() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    let snapshot = |c: &Corpus| {
        let mut files: Vec<(String, String)> = std::fs::read_dir(c.root.join("nodes"))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                let raw = std::fs::read_to_string(&path).unwrap();
                (path.display().to_string(), raw)
            })
            .collect();
        files.sort();
        files
    };

    let dead = c
        .run(&["new", "Dead idea", "--kill", "k"])
        .assert_ok()
        .stdout_trim();
    c.run(&["status", &dead, "refuted", "--why", "it fired"])
        .assert_ok();
    let dropped = c.seed("dropped idea", "Dropped idea");
    c.run(&["status", &dropped, "abandoned", "--why", "lost interest"])
        .assert_ok();
    let open = c.seed("open idea", "Open idea");
    let before = snapshot(&c);

    c.run(&["handoff", &dead, "H012"])
        .assert_fails()
        .says(&format!("`{dead}` is already refuted"))
        .says(&format!("neb new \"...\" --reopens {dead}"));
    let refused = c.run(&["--json", "handoff", &dead, "H012"]).refusal();
    assert_eq!(refused["code"], "already_closed");

    c.run(&["handoff", &dropped, "H012"])
        .assert_fails()
        .says(&format!("`{dropped}` is already abandoned"))
        .says(&format!("neb show {dropped}"));
    let refused = c.run(&["--json", "handoff", &dropped, "H012"]).refusal();
    assert_eq!(refused["code"], "already_closed");

    c.run(&["handoff", "nope", "H012"])
        .assert_fails()
        .says("no node `nope`");
    let refused = c.run(&["--json", "handoff", "nope", "H012"]).refusal();
    assert_eq!(refused["code"], "no_such_node");

    c.run(&["handoff", &open, "H999"])
        .assert_fails()
        .says("Observatory record `H999` does not resolve under")
        .says(obs.to_str().unwrap());
    let refused = c.run(&["--json", "handoff", &open, "H999"]).refusal();
    assert_eq!(refused["code"], "unresolved_observatory_record");
    assert!(refused["hint"].is_string(), "{refused}");

    let refused = c
        .run(&["--json", "handoff", &open, "hypotheses/H012.md"])
        .usage_refusal();
    assert_eq!(refused["code"], "invalid_observatory_id");

    assert_eq!(snapshot(&c), before, "no refusal wrote anything");
}
