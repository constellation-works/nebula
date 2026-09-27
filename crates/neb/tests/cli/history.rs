//! History: `neb log` and `show --at` over git, and the verbs outside a
//! work tree or over a broken repository.

use crate::harness::{
    Corpus, break_head, corpus_repo, git, git_command, git_commit_at, git_init, output,
};
use crate::support;
use std::path::Path;

/// Commit everything with `stamp`, a full ISO 8601 date and time with its
/// offset, as both author and committer date.
pub(super) fn git_commit_stamped(dir: &Path, stamp: &str, message: &str) {
    git(dir, &["add", "-A"]);
    let out = output(
        git_command(dir, support::home())
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_DATE", stamp)
            .env("GIT_COMMITTER_DATE", stamp),
    );
    assert!(
        out.status.success(),
        "git commit failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn log_and_show_at_read_a_node_sharpened_across_two_commits() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "History has a shape", "--id", "history-shape"])
        .assert_ok()
        .stdout_trim();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", &format!("neb new {id}"));
    let created = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    c.run(&["sharpen", &id, "--kill", "the past cannot be read"])
        .assert_ok();
    git_commit_at(&c.root, "2020-01-02", &format!("neb sharpen {id}"));
    let sharpened = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    let text = c.run(&["log", &id]).assert_ok().stdout();
    let mut lines = text.lines();
    let newest = lines.next().expect("sharpen history line");
    let oldest = lines.next().expect("creation history line");
    assert_eq!(
        newest,
        format!("{}\t2020-01-02\tneb sharpen history-shape", &sharpened[..7]),
        "{text}"
    );
    assert_eq!(
        oldest,
        format!("{}\t2020-01-01\tneb new history-shape", &created[..7]),
        "{text}"
    );
    assert!(lines.next().is_none(), "{text}");

    let json = c.run(&["--json", "log", &id]).assert_ok().stdout();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json).expect("log --json");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["hash"], sharpened);
    assert_eq!(entries[0]["date"], "2020-01-02");
    assert_eq!(entries[0]["message"], "neb sharpen history-shape");
    assert_eq!(entries[1]["hash"], created);

    for at in [&created, "2020-01-01"] {
        let shown = c
            .run(&["--json", "show", &id, "--at", at])
            .assert_ok()
            .stdout();
        let view: serde_json::Value = serde_json::from_str(&shown).expect("historical NodeView");
        assert_eq!(view["node"]["status"], "seed");
        assert_eq!(view["node"]["title_by"], "human");
        assert_eq!(view["body"], "");
        assert_eq!(view["observatory"], serde_json::json!([]));
    }
    for at in [&sharpened, "2020-01-02"] {
        let shown = c
            .run(&["--json", "show", &id, "--at", at])
            .assert_ok()
            .stdout();
        let view: serde_json::Value = serde_json::from_str(&shown).expect("historical NodeView");
        assert_eq!(view["node"]["status"], "hypothesis");
        assert_eq!(view["node"]["kill"], "the past cannot be read");
    }
    c.run(&["show", &id, "--at", "2019-12-31"])
        .assert_fails()
        .says("no node `history-shape` at `2019-12-31`");

    for at in ["not-a-date", "2026-99-99", "2026-13-01", "2027-02-29"] {
        c.run(&["show", &id, "--at", at])
            .assert_fails()
            .says(&format!(
                "`{at}` is not a YYYY-MM-DD date or a git revision"
            ))
            .says("--at");
    }

    // A real leap date is a date, not malformed.
    c.run(&["show", &id, "--at", "2028-02-29"]).assert_ok();
}

/// A commit that does not exist and a commit from before the node existed
/// are different answers: the first is a mistyped or foreign hash, the
/// second a true fact about the node's history.
#[test]
fn show_at_unknown_revision_is_not_no_node_at_revision() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();

    let unknown = c
        .run(&["--json", "show", &id, "--at", "deadbeef"])
        .refusal();
    assert_eq!(unknown["code"], "unknown_revision", "{unknown}");
    assert!(
        unknown["error"].as_str().unwrap().contains("deadbeef"),
        "{unknown}"
    );
    assert!(
        unknown["hint"].as_str().unwrap().contains("neb log"),
        "{unknown}"
    );

    let absent = c.run(&["--json", "show", &id, "--at", &before]).refusal();
    assert_eq!(absent["code"], "no_node_at_revision", "{absent}");
}

/// A date given to `show --at` names the day `log` prints, which is each
/// commit's own day in the offset it was recorded with, so the reader's
/// timezone never moves it. The commits sit where the end of a day in the
/// reader's zone used to cut them wrongly: at UTC+14 the end of 2020-01-01
/// falls before its noon-UTC commit, and at UTC-11 or UTC-12 it falls after
/// the early-morning commit dated 2020-01-02.
#[test]
fn show_at_a_date_is_timezone_independent() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Days are the log's", "--id", "days-are-the-logs"])
        .assert_ok()
        .stdout_trim();
    git_init(&c.root);
    git_commit_stamped(&c.root, "2020-01-01T12:00:00Z", &format!("neb new {id}"));
    let created = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    c.run(&["sharpen", &id, "--kill", "a zone moves the day"])
        .assert_ok();
    git_commit_stamped(
        &c.root,
        "2020-01-02T05:00:00Z",
        &format!("neb sharpen {id}"),
    );
    let sharpened = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    // A commit recorded in its author's own far-east offset is dated by it:
    // 2020-01-03 at UTC+14, which is still 2020-01-02 in UTC.
    c.run(&["note", &id, "written a day ahead"]).assert_ok();
    git_commit_stamped(
        &c.root,
        "2020-01-03T08:00:00+14:00",
        &format!("neb note {id}"),
    );
    let noted = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    let log: Vec<serde_json::Value> =
        serde_json::from_str(&c.run(&["--json", "log", &id]).assert_ok().stdout()).unwrap();
    let dates: Vec<_> = log.iter().map(|entry| entry["date"].clone()).collect();
    assert_eq!(dates, ["2020-01-03", "2020-01-02", "2020-01-01"], "{log:?}");

    // POSIX zones, so the offsets hold without tz data: UTC+14, UTC-11,
    // UTC-12 and UTC itself.
    let zones = ["XYZ-14", "XYZ+11", "XYZ+12", "UTC0"];
    for (date, revision) in [
        ("2020-01-01", &created),
        ("2020-01-02", &sharpened),
        ("2020-01-03", &noted),
    ] {
        let by_hash = c
            .run(&["--json", "show", &id, "--at", revision])
            .assert_ok()
            .stdout();
        for zone in zones {
            let by_date = c
                .run_with_env(&["--json", "show", &id, "--at", date], &[("TZ", zone)])
                .assert_ok()
                .stdout();
            assert_eq!(
                by_date, by_hash,
                "`--at {date}` under TZ={zone} is not the revision `log` dates {date}"
            );
        }
    }
    for zone in zones {
        c.run_with_env(&["show", &id, "--at", "2019-12-31"], &[("TZ", zone)])
            .assert_fails()
            .says("no node `days-are-the-logs` at `2019-12-31`");
    }
}

#[test]
fn log_reports_when_an_uncommitted_node_has_no_history() {
    let c = Corpus::new();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "initial repository");
    let id = c
        .run(&["new", "Never committed", "--id", "never-committed"])
        .assert_ok()
        .stdout_trim();

    c.run(&["log", &id])
        .assert_ok()
        .says("no commits touched this node");
    assert_eq!(c.run(&["--json", "log", &id]).assert_ok().stdout(), "[]\n");
}

/// A fixture whose temporary root sits inside someone's repository never
/// finds it: not to stage into it, and not to read history from it
/// (STD-04 §R7). Before the builder set `GIT_CEILING_DIRECTORIES`, running
/// the suite with `TMPDIR` in a work tree staged fixture files into it.
#[test]
fn fixtures_never_discover_an_enclosing_repository() {
    let outer = tempfile::tempdir().expect("tempdir");
    git_init(outer.path());
    let dir = tempfile::tempdir_in(outer.path()).expect("tempdir");
    let root = dir.path().join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("an idea inside someone else's repository", "Not theirs");
    c.run(&["log", &id])
        .assert_fails()
        .says("is not inside a git work tree");

    // With the fixture gone, anything it staged shows as added and deleted.
    drop(c);
    assert_eq!(git(outer.path(), &["status", "--porcelain"]), "");
    assert_eq!(git(outer.path(), &["ls-files"]), "");
}

#[test]
fn history_verbs_refuse_a_corpus_outside_a_git_work_tree() {
    let c = Corpus::new();
    let id = c.seed("an uncommitted past", "An uncommitted past");
    c.run(&["log", &id])
        .assert_fails()
        .says("is not inside a git work tree");
    c.run(&["show", &id, "--at", "2020-01-01"])
        .assert_fails()
        .says("is not inside a git work tree");
}

/// With `commit` on, a repository git cannot read is reported, never taken
/// for "no repository" and the commit silently skipped. The write stays.
#[test]
fn commit_on_with_broken_git_reports_instead_of_skipping() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    break_head(&c.root);

    let run = c.run(&["capture", "broken head probe"]).assert_fails();
    let run = run
        .says("git rev-parse failed in")
        .says(&c.root.display().to_string());
    assert!(
        !run.stdout_trim().is_empty(),
        "the verb reported its write before the refusal"
    );
    let inbox = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(inbox.contains("broken head probe"), "{inbox}");

    let run = c.run(&["--json", "capture", "second probe"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "git");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains(&c.root.display().to_string()),
        "{refused}"
    );
    serde_json::from_str::<serde_json::Value>(&run.stdout()).expect("the write's payload");
}

/// A history query in a repository git cannot read is a git failure, not a
/// corpus that is "not inside a git work tree", which would be false.
#[test]
fn history_on_a_broken_repository_is_a_git_error_not_not_a_work_tree() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();
    break_head(&c.root);
    let refused = c.run(&["--json", "log", &id]).refusal();
    assert_eq!(refused["code"], "git", "{refused}");
}

/// `commit` on with no repository anywhere above the corpus: the write lands
/// and the command succeeds, but it says once, on stderr, that nothing was
/// committed and why. The payload on stdout is what it always is.
#[test]
fn commit_on_outside_a_repository_says_it_did_not_commit() {
    let c = Corpus::new();
    c.run(&["config", "commit", "on"]).assert_ok();
    let run = c.run(&["capture", "x"]).assert_ok();
    let id = run.stdout_trim();
    assert_eq!(run.stdout(), format!("{id}\n"));
    assert_eq!(
        run.stderr(),
        format!(
            "note: not committed: {} is not inside a git work tree\n",
            c.root.display()
        )
    );
    // `--no-commit` asks for no commit, so there is nothing to say.
    let run = c.run(&["capture", "y", "--no-commit"]).assert_ok();
    assert_eq!(run.stderr(), "");
}

/// History needs git, and the refusal says how to start one for this corpus
/// and have later writes recorded in it.
#[test]
fn log_outside_git_hints_init() {
    let c = Corpus::new();
    let id = c.seed("an uncommitted past", "An uncommitted past");
    let run = c.run(&["log", &id]);
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.contains(&format!("git -C {} init", c.root.display())),
        "{stderr}"
    );
    assert!(stderr.contains("neb config commit on"), "{stderr}");
    let refused = c.run(&["--json", "log", &id]).refusal();
    assert_eq!(refused["code"], "not_git_work_tree");
    assert!(
        refused["hint"]
            .as_str()
            .is_some_and(|h| h.contains(&format!("git -C {} init", c.root.display()))),
        "{refused}"
    );
}
