//! A closed stdout: reads, triage and writes exit quietly and still commit.

use crate::handoff::observatory_with_h012;
use crate::harness::{Corpus, Run, closed_stdout, corpus_repo, dirt, git, log, neb_command, write};
use crate::migrate::v1_corpus;
use crate::observatory::with_legacy_observatory_root;

/// Linux exercises the reported full-device failure directly. macOS has no
/// /dev/full, so an unconnected local datagram socket supplies a non-EPIPE
/// write failure without filling the fixture's disk.
fn failed_stdout(c: &Corpus, args: &[&str]) -> Run {
    use crate::support::{ChildGuard, DEADLINE};
    use std::process::Stdio;

    #[cfg(target_os = "linux")]
    let sink = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    #[cfg(not(target_os = "linux"))]
    let sink = std::os::fd::OwnedFd::from(std::os::unix::net::UnixDatagram::unbound().unwrap());
    let mut cmd = c.command(args);
    cmd.stdin(Stdio::null()).stdout(sink).stderr(Stdio::piped());
    Run {
        args: args.join(" "),
        out: ChildGuard::spawn(&mut cmd)
            .unwrap()
            .wait_with_output(DEADLINE)
            .unwrap(),
    }
}

fn assert_write_landed(run: &Run, id: &str, json: bool) {
    assert_eq!(run.out.status.code(), Some(1), "{}", run.stderr());
    let message = if json {
        let stderr = run.stderr();
        let refusal: serde_json::Value =
            serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        assert_eq!(refusal["code"], "stdout");
        refusal["error"].as_str().unwrap().to_owned()
    } else {
        run.stderr()
    };
    assert!(message.contains("write landed"), "{message}");
    assert!(message.contains(id), "{message}");
}

fn failed_stdout_after_new(json: bool) {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = log(&c.root).len();
    let mut args = vec!["new", "Output failed", "--id", "landed-idea"];
    if json {
        args.push("--json");
    }
    let run = failed_stdout(&c, &args);
    // Verify the durable outcome before checking its diagnostic.
    assert_eq!(log(&c.root).len(), before + 1);
    assert!(dirt(&c.root).is_empty());
    c.run(&["show", "landed-idea"]).assert_ok();
    assert_write_landed(&run, "landed-idea", json);
}

#[test]
fn failed_stdout_after_new_text_names_the_landed_write() {
    failed_stdout_after_new(false);
}

#[test]
fn failed_stdout_after_new_json_names_the_landed_write() {
    failed_stdout_after_new(true);
}

#[test]
fn failed_stdout_names_other_completed_record_writes() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    c.run(&["new", "First"]).assert_ok();
    c.run(&["new", "Second"]).assert_ok();
    for args in [
        vec!["note", "first", "A note"],
        vec!["sharpen", "first", "--kill", "A counterexample", "--json"],
        vec!["tag", "first", "--add", "physics", "--json"],
        vec![
            "cite",
            "first",
            "--uri",
            "https://example.com",
            "--note",
            "Source",
            "--json",
        ],
        vec!["link", "first", "contradicts", "second", "--json"],
    ] {
        let before = log(&c.root).len();
        let run = failed_stdout(&c, &args);
        assert_eq!(log(&c.root).len(), before + 1, "{}", run.stderr());
        assert!(dirt(&c.root).is_empty());
        assert_write_landed(&run, "first", args.contains(&"--json"));
        if args[0] == "link" {
            assert!(run.stderr().contains("second"));
        }
    }
    let run = failed_stdout(&c, &["capture", "-q", "A captured thought", "--json"]);
    let inbox: serde_json::Value =
        serde_json::from_str(&c.run(&["inbox", "--json"]).assert_ok().stdout()).unwrap();
    let id = inbox[0]["id"].as_str().unwrap();
    assert!(dirt(&c.root).is_empty());
    assert_write_landed(&run, id, true);
    let run = failed_stdout(&c, &["promote", id, "--title", "Promoted", "--json"]);
    assert!(dirt(&c.root).is_empty());
    assert_write_landed(&run, "promoted", true);
}

#[test]
fn failed_stdout_distinguishes_reads_noops_and_uncommitted_writes() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    c.run(&["new", "Existing", "--tag", "physics"]).assert_ok();
    let before = log(&c.root).len();
    for args in [
        vec!["show", "existing", "--json"],
        vec!["tag", "existing", "--add", "physics", "--json"],
    ] {
        let run = failed_stdout(&c, &args);
        assert_eq!(run.out.status.code(), Some(1));
        assert!(run.stderr().contains("could not write to stdout"));
        assert!(!run.stderr().contains("write landed"));
        assert_eq!(log(&c.root).len(), before);
        assert!(dirt(&c.root).is_empty());
    }
    let run = failed_stdout(&c, &["new", "Uncommitted", "--no-commit", "--json"]);
    assert_eq!(log(&c.root).len(), before);
    assert!(!dirt(&c.root).is_empty());
    assert_write_landed(&run, "uncommitted", true);
}

#[test]
fn failed_stdout_after_init_names_created_corpus_and_machine_setting() {
    for (json, set_root) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("corpus");
        let c = Corpus { dir, root };
        let mut args = vec!["init"];
        if set_root {
            args.push("--set-root");
        }
        if json {
            args.push("--json");
        }
        let run = failed_stdout(&c, &args);
        let config = c.root.join("config.yaml");
        let setting = c.workdir().join(".config/nebula/root");
        assert!(config.is_file(), "{}", run.stderr());
        if set_root {
            assert_eq!(
                std::fs::read_to_string(&setting).unwrap().trim(),
                c.root.to_str().unwrap()
            );
        } else {
            assert!(!setting.exists());
        }
        assert_write_landed(&run, c.root.to_str().unwrap(), json);
        if set_root {
            assert!(
                run.stderr().contains(setting.to_str().unwrap()),
                "{}",
                run.stderr()
            );
        }
    }
}

#[test]
fn failed_stdout_after_config_writes_names_effect_and_file() {
    for json in [false, true] {
        let (c, _remote) = corpus_repo();
        let config = c.root.join("config.yaml");
        let mut args = vec!["config", "commit", "on"];
        if json {
            args.push("--json");
        }
        let run = failed_stdout(&c, &args);
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("commit: true")
        );
        assert_write_landed(&run, config.to_str().unwrap(), json);
        assert!(run.stderr().contains("commit setting"), "{}", run.stderr());

        let obs = c.workdir().join("observatory");
        let setting = c.workdir().join(".config/nebula/observatory-root");
        let mut args = vec!["config", "observatory-root", obs.to_str().unwrap()];
        if json {
            args.push("--json");
        }
        let run = failed_stdout(&c, &args);
        assert_eq!(
            std::fs::read_to_string(&setting).unwrap().trim(),
            obs.to_str().unwrap()
        );
        assert_write_landed(&run, setting.to_str().unwrap(), json);
        assert!(
            run.stderr().contains("observatory root setting"),
            "{}",
            run.stderr()
        );

        with_legacy_observatory_root(&c);
        let mut args = vec!["config", "observatory-root", "--drop-legacy"];
        if json {
            args.push("--json");
        }
        let run = failed_stdout(&c, &args);
        assert!(
            !std::fs::read_to_string(&config)
                .unwrap()
                .contains("observatory_root:")
        );
        assert_write_landed(&run, config.to_str().unwrap(), json);
        assert!(
            run.stderr().contains("legacy observatory root"),
            "{}",
            run.stderr()
        );
    }
}

#[test]
fn failed_stdout_after_migration_names_rewritten_corpus() {
    for json in [false, true] {
        let c = v1_corpus();
        let config = c.root.join("config.yaml");
        let mut args = vec!["migrate"];
        if json {
            args.push("--json");
        }
        let run = failed_stdout(&c, &args);
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("schema_version: 2")
        );
        assert_write_landed(&run, c.root.to_str().unwrap(), json);
        assert!(run.stderr().contains("migration"), "{}", run.stderr());

        // A second migration validates the corpus but does not rewrite it.
        let run = failed_stdout(&c, &args);
        assert_eq!(run.out.status.code(), Some(i32::from(json)));
        assert!(!run.stderr().contains("write landed"), "{}", run.stderr());
    }
}

#[test]
fn failed_stdout_on_config_reads_and_noop_drop_reports_no_write() {
    let c = Corpus::new();
    for args in [
        vec!["config", "commit", "--json"],
        vec!["config", "observatory-root", "--json"],
        vec!["config", "observatory-root", "--drop-legacy", "--json"],
    ] {
        let run = failed_stdout(&c, &args);
        assert_eq!(run.out.status.code(), Some(1), "{}", run.stderr());
        assert!(!run.stderr().contains("write landed"), "{}", run.stderr());
    }
}

#[test]
fn failed_stdout_on_repeated_init_reports_no_write() {
    let c = Corpus::new();
    let config = c.root.join("config.yaml");
    let before = std::fs::read(&config).unwrap();
    for args in [vec!["init"], vec!["init", "--json"]] {
        let run = failed_stdout(&c, &args);
        assert_eq!(std::fs::read(&config).unwrap(), before);
        assert_eq!(run.out.status.code(), Some(1));
        assert!(!run.stderr().contains("write landed"), "{}", run.stderr());
    }
}

#[test]
fn failed_stdout_after_only_machine_root_write_names_setting() {
    let c = Corpus::new();
    let config = c.root.join("config.yaml");
    let before = std::fs::read(&config).unwrap();
    let run = failed_stdout(&c, &["init", "--set-root", "--json"]);
    let setting = c.workdir().join(".config/nebula/root");
    assert_eq!(std::fs::read(&config).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(&setting).unwrap().trim(),
        c.root.to_str().unwrap()
    );
    assert_write_landed(&run, setting.to_str().unwrap(), true);
    assert!(
        !run.stderr().contains("corpus initialization"),
        "{}",
        run.stderr()
    );
}

#[test]
fn failed_commit_error_takes_precedence_over_failed_stdout() {
    let (c, _remote) = corpus_repo();
    write(&c.root.join(".git/index.lock"), "held\n");
    let run = failed_stdout(&c, &["config", "commit", "on", "--json"]);
    assert!(
        std::fs::read_to_string(c.root.join("config.yaml"))
            .unwrap()
            .contains("commit: true")
    );
    assert_eq!(run.refusal()["code"], "git");
    assert!(
        !run.stderr().contains("could not write to stdout"),
        "{}",
        run.stderr()
    );
}

#[test]
fn closed_stdout_after_corpus_writes_stays_silent() {
    let (c, _remote) = corpus_repo();
    let run = c.run_closed_stdout(&["config", "commit", "on", "--json"], &[], "");
    assert_closed_quietly(&run, "");
    assert!(
        std::fs::read_to_string(c.root.join("config.yaml"))
            .unwrap()
            .contains("commit: true")
    );

    let c = v1_corpus();
    let run = c.run_closed_stdout(&["migrate", "--json"], &[], "");
    assert_closed_quietly(&run, "");
    assert!(
        std::fs::read_to_string(c.root.join("config.yaml"))
            .unwrap()
            .contains("schema_version: 2")
    );
}

/// A stdout closed under `neb`: exit 0, and on stderr exactly `stderr`,
/// what the verb says there with its stdout open. A reader that went away
/// adds no error, and takes no notice away: counts and the `committed` line
/// are stderr's, whoever reads stdout.
fn assert_closed_quietly(run: &Run, stderr: &str) {
    assert_eq!(
        run.out.status.code(),
        Some(0),
        "`neb {}` with a closed stdout:\n{}",
        run.args,
        run.stderr()
    );
    assert_eq!(run.stderr(), stderr, "`neb {}` said something", run.args);
}

/// `neb <read> | head -1` is normal use: a reader that stops early ends the
/// command silently with exit 0, never a panic (STD-01 §R13). The inbox is
/// made larger than any pipe buffer, so its writes cannot all land first.
#[test]
fn closed_stdout_exits_zero_silently_for_reads() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let root = c.seed("the root thought", "Root");
    let id = c
        .run(&["new", "A child", "--parent", &root, "--tag", "physics"])
        .assert_ok()
        .stdout_trim();
    let long = "a thought that goes on ".repeat(120);
    for n in 0..30 {
        c.run(&["capture", "-q", &format!("{n} {long}")])
            .assert_ok();
    }
    let inbox = c.run(&["inbox"]).assert_ok().stdout();
    assert!(inbox.len() > 64 * 1024, "{} bytes", inbox.len());

    for args in [
        vec!["list"],
        vec!["list", "--json"],
        vec!["inbox"],
        vec!["inbox", "--json"],
        vec!["show", &id],
        vec!["trace", &id],
        vec!["graph", "--json"],
        vec!["graph", "--mermaid"],
        vec!["review"],
        vec!["review", "--short"],
        vec!["tag", "list"],
        vec!["near", "child"],
        vec!["log", &id],
    ] {
        let open = c.run(&args).assert_ok().stderr();
        assert_closed_quietly(&c.run_closed_stdout(&args, &[], ""), &open);
    }
    // `completions` refuses `--root`, which every command above is given.
    let mut cmd = neb_command(c.workdir());
    cmd.args(["completions", "bash"]).env("NO_COLOR", "1");
    let run = Run {
        args: "completions bash".to_owned(),
        out: closed_stdout(&mut cmd, ""),
    };
    assert_closed_quietly(&run, "");
}

/// A triage whose screen has gone ends as `q` ends it, with exit 0 and no
/// refusal: nobody is there to see the next entry.
#[test]
fn closed_stdout_exits_zero_silently_for_triage() {
    let c = Corpus::new();
    let entry = c
        .run(&["capture", "-q", "a thought"])
        .assert_ok()
        .stdout_trim();
    assert_closed_quietly(&c.run_closed_stdout(&["triage"], &[], "d\nq\n"), "");
    assert!(
        c.run(&["inbox"]).assert_ok().stdout().contains(&entry),
        "a decision made with nobody watching was applied"
    );
}

/// The write a verb reports is committed even when nobody reads the report:
/// a closed stdout never stops a verb between its write and its commit.
#[test]
fn closed_stdout_still_commits_a_write() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();
    let (obs, _) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap();

    for (args, env, subject) in [
        (
            vec!["--json", "note", &id, "more", "text"],
            vec![],
            format!("neb note {id}"),
        ),
        (vec!["note", &id, "text"], vec![], format!("neb note {id}")),
        (vec!["new", "X", "--json"], vec![], "neb new x".to_string()),
        (
            vec!["capture", "-q", "text"],
            vec![],
            "neb capture ".to_string(),
        ),
        (
            vec!["handoff", &id, "H012", "--note", "n"],
            vec![("OBSERVATORY_ROOT", obs)],
            format!("neb handoff {id} H012"),
        ),
    ] {
        let before = log(&c.root).len();
        let run = c.run_closed_stdout(&args, &env, "");
        let head = git(&c.root, &["rev-parse", "HEAD"]);
        let notice = if args.contains(&"--json") {
            String::new()
        } else {
            format!("committed {}\n", &head[..7])
        };
        assert_closed_quietly(&run, &notice);
        assert_eq!(log(&c.root).len(), before + 1, "`neb {}`", run.args);
        let last = git(&c.root, &["log", "-1", "--format=%s"]);
        assert!(
            last.starts_with(&subject),
            "`neb {}` committed as {last:?}",
            run.args
        );
        assert!(
            dirt(&c.root).is_empty(),
            "`neb {}`: {}",
            run.args,
            dirt(&c.root)
        );
    }
}
