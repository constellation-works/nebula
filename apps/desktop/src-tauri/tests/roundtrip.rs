//! `capture` followed by `inbox`, through a temporary corpus, using the same
//! `session` functions the commands call. The line written is the one
//! `neb capture` writes, so `neb inbox` lists it too.

use nebula_core::{Corpus, Error, ops};
use nebula_desktop::session;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn capture_then_inbox_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    Corpus::init(&root).unwrap();

    let corpus = session::open(&root).unwrap();
    assert!(session::inbox(&corpus).unwrap().is_empty());

    let entry = session::capture(&corpus, "  a thought from the menu bar  ").unwrap();
    assert_eq!(entry.text, "a thought from the menu bar");

    let listed = session::inbox(&corpus).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, entry.id);
    assert_eq!(listed[0].at, entry.at);
    assert_eq!(listed[0].text, entry.text);

    // One line, in the CLI's format, in this month's file.
    let month = &entry.at[..7];
    let file = std::fs::read_to_string(root.join("inbox").join(format!("{month}.md"))).unwrap();
    assert_eq!(
        file,
        format!(
            "- [{}] {} a thought from the menu bar\n",
            entry.id, entry.at
        )
    );
}

#[test]
fn capture_commits_each_entry_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "neb-test"]);
    git(&["config", "user.email", "neb-test@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    ops::set_commit(&mut corpus, true).unwrap();
    ops::commit(&corpus, "config", &["commit"])
        .unwrap()
        .unwrap();

    let first = session::capture(&corpus, "first thought").unwrap();
    assert_eq!(
        git(&["log", "-1", "--format=%s"]).trim(),
        format!("neb capture {}", first.id)
    );
    assert_eq!(git(&["rev-list", "--count", "HEAD"]).trim(), "2");
    let second = session::capture(&corpus, "second thought").unwrap();
    assert_eq!(
        git(&["log", "-1", "--format=%s"]).trim(),
        format!("neb capture {}", second.id)
    );
    assert_eq!(git(&["rev-list", "--count", "HEAD"]).trim(), "3");
    assert!(git(&["status", "--porcelain"]).is_empty());
}

#[test]
fn drop_and_promote_use_core_settlement_and_cli_commit_messages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "neb-test"]);
    git(&["config", "user.email", "neb-test@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    ops::set_commit(&mut corpus, true).unwrap();
    ops::commit(&corpus, "config", &["commit"]).unwrap();

    let dropped = session::capture(&corpus, "discard this").unwrap();
    assert_eq!(
        session::drop_entry(&corpus, &dropped.id).unwrap().id,
        dropped.id
    );
    assert_eq!(
        git(&["log", "-1", "--format=%s"]).trim(),
        format!("neb drop {}", dropped.id)
    );
    assert!(session::inbox(&corpus).unwrap().is_empty());

    let promoted = session::capture(&corpus, "keep this thought").unwrap();
    let created = session::promote_root(&corpus, &promoted.id).unwrap();
    assert!(created.doc.node.parents().next().is_none());
    assert_eq!(created.doc.node.title, promoted.text);
    assert_eq!(created.doc.body.trim(), promoted.text);
    assert_eq!(
        git(&["log", "-1", "--format=%s"]).trim(),
        format!("neb promote {} {}", promoted.id, created.doc.node.id)
    );
    assert!(session::inbox(&corpus).unwrap().is_empty());
    assert!(git(&["status", "--porcelain"]).is_empty());
}

#[test]
fn a_refused_settlement_leaves_the_inbox_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    let corpus = Corpus::init(&root).unwrap();
    let entry = session::capture(&corpus, "keep this thought").unwrap();
    let before = session::inbox(&corpus).unwrap();
    assert!(session::drop_entry(&corpus, "missing").is_err());
    assert!(session::promote_root(&corpus, "missing").is_err());
    let after = session::inbox(&corpus).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0].id, entry.id);
    assert_eq!(after[0].text, before[0].text);
}

#[test]
fn a_missing_root_is_an_error_naming_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("nowhere");
    let err = session::open(&root).unwrap_err().to_string();
    assert!(err.contains("nowhere"), "{err}");
}

#[test]
fn node_file_refuses_an_unknown_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    Corpus::init(&root).unwrap();
    let corpus = session::open(&root).unwrap();
    assert!(session::node_file(&corpus, "no-such-node").is_err());
    assert!(session::graph(&corpus).unwrap().nodes.is_empty());
}

#[test]
fn capture_refuses_a_busy_writer_quickly_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    let corpus = Corpus::init(&root).unwrap();
    let pending = session::capture(&corpus, "settle me").unwrap();
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hold_capture_lock_in_child", "--nocapture"])
        .env("NEBULA_TEST_CAPTURE_LOCK_ROOT", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(holder.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "lock holder exited early"
        );
        if line.contains("CAPTURE_LOCK_HELD") {
            break;
        }
    }

    let start = Instant::now();
    let result = session::capture(&corpus, "retry me");
    assert!(matches!(result, Err(Error::Locked { .. })), "{result:?}");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "capture waited as long as the CLI's five-second lock timeout"
    );
    let start = Instant::now();
    assert!(matches!(
        session::drop_entry(&corpus, &pending.id),
        Err(Error::Locked { .. })
    ));
    assert!(matches!(
        session::promote_root(&corpus, &pending.id),
        Err(Error::Locked { .. })
    ));
    assert!(start.elapsed() < Duration::from_secs(1));

    drop(holder.stdin.take());
    assert!(holder.wait().unwrap().success());
    let corpus = session::open(&root).unwrap();
    assert_eq!(session::inbox(&corpus).unwrap()[0].id, pending.id);
    session::drop_entry(&corpus, &pending.id).unwrap();
    assert!(session::inbox(&corpus).unwrap().is_empty());
    let entry = session::capture(&corpus, "retry me").unwrap();
    assert_eq!(session::inbox(&corpus).unwrap()[0].id, entry.id);
}

// This helper is a no-op in the regular test run. The contention test starts
// this test binary as a child so the lock is held by a different process.
#[test]
fn hold_capture_lock_in_child() {
    let Ok(root) = std::env::var("NEBULA_TEST_CAPTURE_LOCK_ROOT") else {
        return;
    };
    let corpus = session::open(std::path::Path::new(&root)).unwrap();
    let _lock = corpus.lock().unwrap();
    println!("CAPTURE_LOCK_HELD");
    std::io::stdout().flush().unwrap();
    let mut ignored = String::new();
    std::io::stdin().read_to_string(&mut ignored).unwrap();
}
