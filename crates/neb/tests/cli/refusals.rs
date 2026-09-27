//! Refusals: the `--json` error envelope on stderr, its codes and hints, and
//! the exit codes of usage and argument-shape errors.

use crate::harness::{
    Corpus, break_head, corpus_repo, git_init, output, snapshot_corpus_files, write,
};
use crate::support;
use std::path::Path;

/// Run printed shell commands against recording functions so their parsed
/// arguments and any unexpected command are visible without doing the work.
#[cfg(unix)]
fn shell_calls(commands: &[String], scratch: &Path) -> Vec<Vec<String>> {
    let trace = scratch.join("shell-calls");
    write(&trace, "");
    let script = format!(
        r#"record() {{
  printf 'CALL|%s' "$1" >> "$NEBULA_TEST_TRACE"
  shift
  for arg do printf '|%s' "$arg" >> "$NEBULA_TEST_TRACE"; done
  printf '\n' >> "$NEBULA_TEST_TRACE"
}}
neb() {{ record neb "$@"; }}
git() {{ record git "$@"; }}
cat() {{ record cat "$@"; }}
rm() {{ record rm "$@"; }}
touch() {{ record unexpected_touch "$@"; }}
{}
"#,
        commands.join("\n")
    );
    let mut command = support::command("sh", scratch);
    command
        .current_dir(scratch)
        .env("NEBULA_TEST_TRACE", &trace)
        .env("NEBULA_TEST_EXPANSION", "expanded")
        .args(["-c", &script]);
    let out = output(&mut command);
    assert!(
        out.status.success(),
        "shell command failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(trace)
        .expect("recorded shell calls")
        .lines()
        .map(|line| {
            line.strip_prefix("CALL|")
                .expect("recorded call marker")
                .split('|')
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

#[cfg(unix)]
#[test]
fn refusal_remedies_quote_shell_sensitive_paths_and_keep_the_root_selected() {
    let dir = tempfile::tempdir().expect("fixture directory");
    let root = dir
        .path()
        .join("my corpus 'draft' $NEBULA_TEST_EXPANSION; touch marker");
    let corpus = Corpus { dir, root };
    let root_text = corpus.root.to_string_lossy().into_owned();

    let no_corpus = corpus.run(&["--json", "list"]).refusal();
    assert_eq!(no_corpus["code"], "no_corpus");
    let command = no_corpus["hint"]
        .as_str()
        .unwrap()
        .strip_prefix("Create one with:  ")
        .expect("init remedy");
    assert_eq!(
        shell_calls(&[command.to_owned()], corpus.dir.path()),
        vec![vec!["neb".to_owned(), "init".to_owned(), root_text.clone()]]
    );

    corpus.run(&["init"]).assert_ok();
    let id = corpus.seed("an uncommitted past", "An uncommitted past");
    let not_git = corpus.run(&["--json", "log", &id]).refusal();
    assert_eq!(not_git["code"], "not_git_work_tree");
    let hint = not_git["hint"].as_str().unwrap();
    let commands = hint
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("git -C ") || line.starts_with("neb "))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        shell_calls(&commands, corpus.dir.path()),
        vec![
            vec![
                "git".to_owned(),
                "-C".to_owned(),
                root_text.clone(),
                "init".to_owned()
            ],
            vec![
                "neb".to_owned(),
                "--root".to_owned(),
                root_text.clone(),
                "config".to_owned(),
                "commit".to_owned(),
                "on".to_owned(),
            ],
        ]
    );

    let pending_path = corpus.root.join(".pending");
    write(&pending_path, "{");
    let pending = corpus
        .run(&["--json", "capture", "pending probe"])
        .refusal();
    assert_eq!(pending["code"], "pending_write_unreadable");
    let hint = pending["hint"].as_str().unwrap();
    let commands = ["cat", "rm"]
        .iter()
        .map(|command| {
            let path = hint
                .split_once(&format!("{command} "))
                .expect("pending remedy command")
                .1
                .lines()
                .next()
                .unwrap();
            format!("{command} {path}")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shell_calls(&commands, corpus.dir.path()),
        vec![
            vec![
                "cat".to_owned(),
                pending_path.to_string_lossy().into_owned()
            ],
            vec!["rm".to_owned(), pending_path.to_string_lossy().into_owned()],
        ]
    );

    std::fs::remove_file(&pending_path).expect("clear malformed fixture record");
    let lock_path = corpus.root.join(nebula_core::LOCK_FILE);
    std::fs::remove_file(&lock_path).expect("replace fixture lock with a symlink");
    let target = corpus.dir.path().join("outside-lock-target");
    std::fs::write(&target, b"keep this target").expect("symlink target");
    std::os::unix::fs::symlink(&target, &lock_path).expect("symlink lock fixture");
    let locked = corpus.run(&["--json", "capture", "lock probe"]).refusal();
    assert_eq!(locked["code"], "not_regular_file");
    let command = locked["hint"]
        .as_str()
        .unwrap()
        .split_once("rm ")
        .expect("lock removal remedy")
        .1;
    assert_eq!(
        shell_calls(&[format!("rm {command}")], corpus.dir.path()),
        vec![vec![
            "rm".to_owned(),
            lock_path.to_string_lossy().into_owned()
        ]]
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"keep this target");
}
use crate::migrate::v1_corpus;

/// Under `--json` a refusal is data: one envelope on stderr, stdout left to
/// the payload, and the exit code it always had. Without `--json` nothing
/// changed, byte for byte.
#[test]
fn a_json_refusal_is_one_envelope_on_stderr_and_the_prose_is_unchanged() {
    let c = Corpus::new();
    let run = c.run(&["show", "nope", "--json"]);
    assert_eq!(
        run.refusal(),
        serde_json::json!({
            "error": "no node `nope`",
            "code": "no_such_node",
            "hint": "List what exists with:  neb list",
        })
    );
    assert!(run.stdout().is_empty(), "{}", run.stdout());

    let run = c.run(&["show", "nope"]).assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(
        run.stderr(),
        "error: no node `nope`\n\nList what exists with:  neb list\n"
    );
    assert!(run.stdout().is_empty(), "{}", run.stdout());
}

/// Each point-of-action guard reports its core variant as `code`.
#[test]
fn json_refusals_name_the_link_or_cite_that_was_refused() {
    let c = Corpus::new();
    let a = c.seed("an idea", "An idea");
    let b = c
        .run(&["new", "A child", "--parent", &a])
        .assert_ok()
        .stdout_trim();

    let self_loop = c
        .run(&["--json", "link", &a, "refines", &a])
        .usage_refusal();
    assert_eq!(self_loop["code"], "self_loop");
    assert_eq!(self_loop["error"], "a node cannot link to itself");
    assert_eq!(self_loop["hint"], serde_json::Value::Null);

    let cycle = c.run(&["--json", "link", &a, "derives-from", &b]).refusal();
    assert_eq!(cycle["code"], "cycle");
    assert_eq!(
        cycle["error"],
        format!("that edge would make `{a}` its own ancestor")
    );

    let unknown = c
        .run(&[
            "--json",
            "cite",
            &b,
            "--kind",
            "bogus",
            "--uri",
            "https://example.org",
            "--note",
            "n",
        ])
        .usage_refusal();
    assert_eq!(unknown["code"], "unknown_reference_kind");
    assert_eq!(
        unknown["error"],
        "`bogus` is not an accepted reference kind; accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other"
    );

    let unresolved = c
        .run(&[
            "--json",
            "cite",
            &b,
            "--uri",
            "./notes/missing.md",
            "--note",
            "n",
        ])
        .refusal();
    assert_eq!(unresolved["code"], "unresolved_uri");
    let message = unresolved["error"].as_str().unwrap();
    assert!(
        message.starts_with("`./notes/missing.md` does not resolve from")
            && message.ends_with("local references are relative to nodes/"),
        "{message}"
    );
}

/// The guards whose hint names the node report a clean message and hint
/// under `--json`, and keep their prose exactly without it.
#[test]
fn json_refusals_about_a_node_split_message_and_hint() {
    // (arguments, exit, code, message, hint, the prose without --json)
    type Guard<'a> = (&'a [&'a str], i32, &'a str, String, String, String);
    let c = Corpus::new();
    let a = c.seed("an idea", "An idea");
    let b = c.seed("a child", "A child");
    c.run(&["sharpen", &b, "--kill", "if X"]).assert_ok();
    let dead = c.seed("a dead idea", "A dead idea");
    c.run(&["sharpen", &dead, "--kill", "if Y"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "Y happened"])
        .assert_ok();
    let guards: [Guard; 3] = [
        (
            &["status", &a, "hypothesis"],
            1,
            "needs_kill",
            "`hypothesis` needs a kill condition first".to_string(),
            format!("neb sharpen {a} --kill \"...\""),
            format!(
                "error: `hypothesis` needs a kill condition first:\n\n  neb sharpen {a} --kill \"...\"\n"
            ),
        ),
        (
            &["status", &b, "refuted"],
            2,
            "refuted_needs_why",
            "refuted needs --why: say how the kill condition fired".to_string(),
            format!("neb status {b} refuted --why \"...\""),
            format!(
                "error: refuted needs --why: say how the kill condition fired\n\n  neb status {b} refuted --why \"...\"\n"
            ),
        ),
        (
            &["status", &dead, "seed"],
            1,
            "refuted_cannot_reopen",
            format!("`{dead}` is refuted and cannot simply reopen"),
            // The hint is the one command to run, and nothing else.
            format!("neb new \"...\" --reopens {dead}"),
            format!(
                "error: `{dead}` is refuted and cannot simply reopen.\n\nRevive it as a new node that reopens it:\n  neb new \"...\" --reopens {dead}\n"
            ),
        ),
    ];
    for (args, exit, code, message, hint, prose) in guards {
        let mut with_json = vec!["--json"];
        with_json.extend_from_slice(args);
        let refused = c.run(&with_json).refusal_exiting(exit);
        assert_eq!(
            refused,
            serde_json::json!({"error": message, "code": code, "hint": hint}),
            "neb {}",
            args.join(" ")
        );
        let run = c.run(args).assert_fails();
        assert_eq!(run.out.status.code(), Some(exit));
        assert_eq!(run.stderr(), prose);
    }
}

/// A schema this build does not read is one kind either way; the hint says
/// which way to go.
#[test]
fn json_refusals_for_a_schema_too_old_or_too_new_carry_the_direction_in_the_hint() {
    let old = v1_corpus();
    let refused = old.run(&["list", "--json"]).refusal();
    assert_eq!(refused["code"], "schema_mismatch");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .ends_with("is schema_version 1, and this build understands 2"),
        "{refused}"
    );
    assert_eq!(
        refused["hint"],
        "Bring the corpus forward with:  neb migrate"
    );

    let new = Corpus::new();
    let config = new.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &raw.replacen("schema_version: 2", "schema_version: 3", 1),
    );
    let refused = new.run(&["list", "--json"]).refusal();
    assert_eq!(refused["code"], "schema_mismatch");
    assert_eq!(
        refused["hint"],
        "This corpus was written by a newer nebula. Upgrade this build."
    );
}

/// A corpus with no `config.yaml` is refused by every verb, the reads and
/// capture alike, and none of them puts a config back: a synthesized one
/// would stamp this build's schema over files that may predate it.
#[test]
fn list_on_corpus_without_config_refuses_and_leaves_no_file() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let config = c.root.join("config.yaml");
    std::fs::remove_file(&config).unwrap();
    let before = snapshot_corpus_files(&c.root);

    for args in [
        &["--json", "list"][..],
        &["--json", "show", &id],
        &["--json", "check"],
        &["--json", "capture", "x"],
    ] {
        let refused = c.run(args).refusal();
        assert_eq!(refused["code"], "missing_config", "{args:?}: {refused}");
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains(config.to_str().unwrap()),
            "{args:?}: {refused}"
        );
        assert!(
            refused["hint"]
                .as_str()
                .unwrap()
                .contains(&format!("neb --root {} migrate", c.root.display())),
            "{args:?}: {refused}"
        );
        assert!(!config.exists(), "`neb {}` wrote a config", args.join(" "));
    }
    assert_eq!(before, snapshot_corpus_files(&c.root), "a refusal wrote");
}

/// A refusal is written by the output layer to stderr: under `--json`, one
/// JSON object there and nothing on stdout.
#[test]
fn json_refusal_goes_to_stderr_through_the_output_layer() {
    let c = Corpus::new();
    let run = c.run(&["--json", "show", "nope"]);
    assert_eq!(run.refusal()["code"], "no_such_node");
    assert_eq!(run.stdout(), "");
}

/// A refused commit comes after the write, so under `--json` stdout still
/// holds the write's payload and stderr the refusal: two streams, each one
/// JSON document.
#[test]
fn json_commit_refusals_leave_the_payload_on_stdout() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    break_head(&c.root);

    let run = c.run(&["new", "An idea", "--json"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "git");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .starts_with("git rev-parse failed in"),
        "{refused}"
    );
    let created: serde_json::Value =
        serde_json::from_str(&run.stdout()).expect("the write's payload is still JSON");
    assert_eq!(created["doc"]["node"]["id"], "an-idea");

    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    git_init(&outer);
    write(&outer.join(".gitignore"), "corpus/\n");
    let run = c.run(&["config", "commit", "on", "--json"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "corpus_ignored");
    assert!(
        refused["hint"].as_str().unwrap().contains(&format!(
            "neb --root {} config commit off",
            c.root.display()
        )),
        "{refused}"
    );
    serde_json::from_str::<serde_json::Value>(&run.stdout())
        .expect("the setting's payload is still JSON");
}

/// The CLI's own refusals use the same envelope, a usage error among them
/// exiting 2; only clap's usage errors, raised before `neb` knows it was
/// asked for JSON, stay prose, with the same exit 2.
#[test]
fn json_covers_the_clis_own_refusals_but_not_clap_usage_errors() {
    let c = Corpus::new();
    let empty = c.run(&["capture", "  ", "--json"]).usage_refusal();
    assert_eq!(
        empty,
        serde_json::json!({"error": "nothing to capture", "code": "empty_capture", "hint": null})
    );

    let id = c.seed("an idea", "An idea");
    let editor = c.run(&["edit", &id, "--json"]).refusal();
    assert_eq!(editor["code"], "editor_not_configured");

    let run = c.run(&["show", "--json"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert!(run.stderr().starts_with("error:"), "{}", run.stderr());
    assert!(
        serde_json::from_str::<serde_json::Value>(&run.stderr()).is_err(),
        "{}",
        run.stderr()
    );
}

/// `args` refused as a usage error in both modes: exit 2 and nothing on
/// stdout, `error:` prose on stderr without `--json` and the envelope with
/// `code` under it. Returns the envelope.
pub(super) fn assert_usage_error(c: &Corpus, args: &[&str], code: &str) -> serde_json::Value {
    let run = c.run(args);
    assert_eq!(run.out.status.code(), Some(2), "neb {}", run.args);
    assert_eq!(run.stdout(), "", "neb {}", run.args);
    assert!(
        run.stderr().starts_with("error: "),
        "neb {}: {}",
        run.args,
        run.stderr()
    );
    let mut with_json = vec!["--json"];
    with_json.extend_from_slice(args);
    let run = c.run(&with_json);
    let refused = run.usage_refusal();
    assert_eq!(run.stdout(), "", "neb {}", run.args);
    assert_eq!(refused["code"], code, "neb {}", run.args);
    refused
}

/// Each refusal the CLI makes of arguments that parsed but ask for nothing
/// is a usage error, exit 2, like clap's own (STD-01 §R20).
#[test]
fn usage_refusals_exit_two() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = snapshot_corpus_files(&c.root);
    for (args, message) in [
        (
            vec!["tag", &id],
            "nothing to do; pass --add <tag> or --remove <tag>",
        ),
        (vec!["near", "  "], "nothing to look near"),
    ] {
        let refused = assert_usage_error(&c, &args, "usage");
        assert_eq!(refused["error"], message, "neb {}", args.join(" "));
    }
    // Core's own argument rules, with core's codes, exit 2 as well.
    for (args, code, message) in [
        (
            vec!["status", &id, "seed", "--why", "x"],
            "reason_on_open_status",
            "a reason only applies to refuted or abandoned, not `seed`",
        ),
        (vec!["capture", "  "], "empty_capture", "nothing to capture"),
        (
            vec!["note", &id, "  "],
            "empty_note",
            "a note cannot be empty",
        ),
    ] {
        let refused = assert_usage_error(&c, &args, code);
        assert_eq!(refused["error"], message, "neb {}", args.join(" "));
    }
    // `triage` has no JSON form: `--json` is a flag the verb does not take.
    let run = c.run_with_stdin(&["--json", "triage"], "d\n");
    assert_eq!(run.usage_refusal()["code"], "interactive");
    assert_eq!(run.stdout(), "");
    // `sharpen`'s `--kill`-or-`--confirm` is clap's to refuse, first.
    let run = c.run(&["sharpen", &id]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    assert_eq!(
        snapshot_corpus_files(&c.root),
        before,
        "nothing was written"
    );
}

/// A core refusal of an argument no corpus could accept, whatever it holds,
/// is a usage error too, decided per variant in one place.
#[test]
fn core_argument_shape_refusals_have_a_decided_exit_code() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let hypothesis = c.seed("a hypothesis", "A hypothesis");
    c.run(&["sharpen", &hypothesis, "--kill", "if X"])
        .assert_ok();
    let dead = c.seed("a dead idea", "A dead idea");
    c.run(&["sharpen", &dead, "--kill", "if Y"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "Y happened"])
        .assert_ok();
    let before = snapshot_corpus_files(&c.root);
    for (args, code) in [
        (vec!["handoff", &id, "nonsense"], "invalid_observatory_id"),
        (
            vec![
                "cite",
                &id,
                "--kind",
                "bogus",
                "--uri",
                "https://example.org",
                "--note",
                "n",
            ],
            "unknown_reference_kind",
        ),
        (vec!["new", "X", "--id", "Bad Id"], "invalid_id"),
        (vec!["new", "X", "--kill", "  "], "empty_kill"),
        (
            vec!["new", "X", "--parent", &dead, "--reopens", &dead],
            "parent_and_reopens",
        ),
        (vec!["status", &hypothesis, "refuted"], "refuted_needs_why"),
        (vec!["new", "   "], "unusable_title"),
        (
            vec!["cite", &id, "--uri", "/abs/x", "--note", "n"],
            "absolute_uri",
        ),
    ] {
        assert_usage_error(&c, &args, code);
    }
    assert_eq!(
        snapshot_corpus_files(&c.root),
        before,
        "nothing was written"
    );
    // What the corpus holds decides a failure, which stays exit 1.
    c.run(&["--json", "show", "nope"]).refusal();
}
