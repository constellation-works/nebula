//! A closed stdout: reads, triage and writes exit quietly and still commit.

use crate::handoff::observatory_with_h012;
use crate::harness::{Corpus, Run, closed_stdout, corpus_repo, dirt, git, log, neb_command};

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
