//! Where a write came from: Orbit provenance from the environment and flags,
//! `--by` under an Orbit run, and the read-only environment.

use crate::edit::{corpus_bytes, shown};
use crate::harness::{Corpus, editor_script};

#[test]
fn origin_is_filled_from_orbit_env() {
    let c = Corpus::new();
    let env = [("ORBIT_TASK_ID", "ORB-1"), ("ORBIT_RUN_ID", "jrun-1")];
    c.run_with_env(&["new", "Orbit made idea", "--by", "agent:x"], &env)
        .assert_ok();
    assert_eq!(
        shown(&c, "orbit-made-idea")["node"]["origin"]["task"],
        "ORB-1"
    );
    assert_eq!(
        shown(&c, "orbit-made-idea")["node"]["origin"]["run"],
        "jrun-1"
    );

    let capture = c
        .run(&["capture", "human capture"])
        .assert_ok()
        .stdout_trim();
    c.run_with_env(
        &[
            "promote",
            &capture,
            "--title",
            "Orbit promoted",
            "--by",
            "agent:x",
        ],
        &env,
    )
    .assert_ok();
    assert_eq!(
        shown(&c, "orbit-promoted")["node"]["origin"]["task"],
        "ORB-1"
    );
    assert_eq!(
        shown(&c, "orbit-promoted")["node"]["origin"]["run"],
        "jrun-1"
    );

    c.run_with_env(
        &[
            "cite",
            "orbit-made-idea",
            "--uri",
            "https://example.org",
            "--by",
            "agent:x",
        ],
        &env,
    )
    .assert_ok();
    c.run_with_env(
        &["handoff", "orbit-made-idea", "H012", "--by", "agent:x"],
        &env,
    )
    .assert_ok();
    let refs = &shown(&c, "orbit-made-idea")["node"]["references"];
    for reference in refs.as_array().unwrap() {
        assert_eq!(reference["origin"]["task"], "ORB-1");
        assert_eq!(reference["origin"]["run"], "jrun-1");
    }
}

#[test]
fn explicit_task_and_run_flags_outrank_orbit_env() {
    let c = Corpus::new();
    c.run_with_env(
        &[
            "new",
            "Explicit provenance",
            "--by",
            "human",
            "--task",
            "ORB-explicit",
            "--run",
            "jrun-explicit",
        ],
        &[("ORBIT_TASK_ID", "ORB-env"), ("ORBIT_RUN_ID", "jrun-env")],
    )
    .assert_ok();
    let origin = &shown(&c, "explicit-provenance")["node"]["origin"];
    assert_eq!(origin["task"], "ORB-explicit");
    assert_eq!(origin["run"], "jrun-explicit");
    c.run_with_env(
        &[
            "new",
            "Mixed provenance",
            "--by",
            "human",
            "--task",
            "ORB-explicit",
        ],
        &[("ORBIT_TASK_ID", "ORB-env"), ("ORBIT_RUN_ID", "jrun-env")],
    )
    .assert_ok();
    let mixed = &shown(&c, "mixed-provenance")["node"]["origin"];
    assert_eq!(mixed["task"], "ORB-explicit");
    assert_eq!(mixed["run"], "jrun-env");
    let refused = c.run_with_env(
        &["--json", "new", "Blank author", "--by", "  "],
        &[("ORBIT_RUN_ID", "jrun-env")],
    );
    assert_eq!(refused.refusal()["code"], "by_required");
}

#[test]
fn mutating_verb_under_orbit_run_without_by_is_refused() {
    for verb in [
        "new", "sharpen", "link", "note", "cite", "handoff", "edit", "promote",
    ] {
        for by in [None, Some("human"), Some("agent:x")] {
            let c = Corpus::new();
            let node = c.seed("first thought", "First thought");
            let other = c.seed("second thought", "Second thought");
            let capture = c
                .run(&["capture", "third thought"])
                .assert_ok()
                .stdout_trim();
            let editor = editor_script(
                &c,
                "orbit-editor.sh",
                "#!/bin/sh\nprintf 'edited words\\n' > \"$1\"\n",
            );
            let mut args: Vec<&str> = match verb {
                "new" => vec!["new", "Authored idea"],
                "sharpen" => vec!["sharpen", &node, "--kill", "if false"],
                "link" => vec!["link", &node, "refines", &other],
                "note" => vec!["note", &node, "my reasoning"],
                "cite" => vec!["cite", &node, "--uri", "https://example.org"],
                "handoff" => vec!["handoff", &node, "H012"],
                "edit" => vec!["edit", &node],
                "promote" => vec!["promote", &capture, "--title", "Authored title"],
                _ => unreachable!(),
            };
            if let Some(by) = by {
                args.extend(["--by", by]);
            }
            let before = corpus_bytes(&c.root);
            let mut env = vec![("ORBIT_RUN_ID", "jrun-test")];
            if verb == "edit" {
                env.push(("EDITOR", editor.to_str().unwrap()));
            }
            let mut json_args = vec!["--json"];
            json_args.extend(args);
            let result = c.run_with_env(&json_args, &env);
            if by.is_none() {
                assert_eq!(result.refusal()["code"], "by_required", "{verb}");
                assert_eq!(corpus_bytes(&c.root), before, "{verb} changed files");
            } else {
                result.assert_ok();
            }
        }
    }
}

#[test]
fn promote_as_captured_under_orbit_run_needs_no_by() {
    let c = Corpus::new();
    let capture = c.run(&["capture", "human words"]).assert_ok().stdout_trim();
    c.run_with_env(&["promote", &capture], &[("ORBIT_RUN_ID", "jrun-test")])
        .assert_ok();
    let second = c
        .run(&["capture", "another human thought"])
        .assert_ok()
        .stdout_trim();
    let before = corpus_bytes(&c.root);
    let refused = c.run_with_env(
        &["--json", "promote", &second, "--body", ""],
        &[("ORBIT_RUN_ID", "jrun-test")],
    );
    assert_eq!(refused.refusal()["code"], "by_required");
    assert_eq!(corpus_bytes(&c.root), before);
}

#[test]
fn confirm_kill_under_orbit_run_is_refused() {
    let c = Corpus::new();
    c.run(&["new", "Agent kill", "--kill", "if false", "--by", "agent:x"])
        .assert_ok();
    let before = corpus_bytes(&c.root);
    let refused = c.run_with_env(
        &["--json", "sharpen", "agent-kill", "--confirm"],
        &[("ORBIT_RUN_ID", "jrun-test")],
    );
    assert_eq!(refused.refusal()["code"], "human_only");
    assert_eq!(corpus_bytes(&c.root), before);
    assert_eq!(shown(&c, "agent-kill")["node"]["kill_by"], "agent:x");
}

#[test]
fn read_only_env_refuses_capture_and_promote_but_allows_list() {
    let c = Corpus::new();
    let node = c.seed("first thought", "First thought");
    let other = c.seed("second thought", "Second thought");
    let capture = c
        .run(&["capture", "third thought"])
        .assert_ok()
        .stdout_trim();
    let editor = editor_script(
        &c,
        "read-only-editor.sh",
        "#!/bin/sh\nprintf 'edited\\n' > \"$1\"\n",
    );
    let cases: Vec<Vec<&str>> = vec![
        vec!["init"],
        vec!["migrate"],
        vec!["config", "commit", "on"],
        vec!["config", "observatory-root", c.workdir().to_str().unwrap()],
        vec!["config", "observatory-root", "--drop-legacy"],
        vec!["capture", "new capture"],
        vec!["promote", &capture],
        vec!["drop", &capture],
        vec!["triage"],
        vec!["new", "new node"],
        vec!["edit", &node],
        vec!["sharpen", &node, "--kill", "if false"],
        vec!["status", &node, "abandoned", "--why", "done"],
        vec!["link", &node, "refines", &other],
        vec!["tag", &node, "--add", "alpha"],
        vec!["note", &node, "words"],
        vec!["cite", &node, "--uri", "https://example.org"],
        vec!["handoff", &node, "H012"],
    ];
    let before = corpus_bytes(&c.root);
    for case in cases {
        let mut args = vec!["--json"];
        args.extend(case.clone());
        let result = c.run_with_env(
            &args,
            &[
                ("NEBULA_READ_ONLY", "1"),
                ("EDITOR", editor.to_str().unwrap()),
            ],
        );
        assert_eq!(result.refusal()["code"], "read_only", "{case:?}");
        assert_eq!(corpus_bytes(&c.root), before, "{case:?} changed files");
    }
    for args in [
        vec!["list"],
        vec!["show", &node],
        vec!["inbox"],
        vec!["check"],
        vec!["review"],
    ] {
        c.run_with_env(&args, &[("NEBULA_READ_ONLY", "1")])
            .assert_ok();
    }
}

#[test]
fn read_only_env_with_an_unrecognised_value_is_refused() {
    let c = Corpus::new();
    let result = c.run_with_env(&["--json", "list"], &[("NEBULA_READ_ONLY", "yes")]);
    assert_eq!(result.refusal()["code"], "invalid_read_only_environment");
    assert!(result.stderr().contains("NEBULA_READ_ONLY"));
}
