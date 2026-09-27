//! Supervised git: inherited `GIT_DIR` and index ignored, hooks in their own
//! process group, and git timing out.

use crate::harness::{Corpus, corpus_repo, dirt, git, git_init, log, write};
use crate::migrate::v1_corpus;
use crate::support::ChildGuard;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// A second repository beside the corpus, with one commit of its own: the
/// one an inherited `GIT_DIR` points at.
fn other_repo(c: &Corpus) -> PathBuf {
    let other = c.workdir().join("other");
    std::fs::create_dir(&other).unwrap();
    git_init(&other);
    write(&other.join("theirs.txt"), "theirs\n");
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "theirs"]);
    other
}

/// A `neb` commit run from inside another repository's hook, or any shell
/// that exported git's own variables, still lands in the corpus repository
/// and leaves the other one exactly as it was.
#[test]
fn a_commit_ignores_an_inherited_git_dir_and_index() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let other = other_repo(&c);
    let other_git = other.join(".git");
    let head = git(&other, &["rev-parse", "HEAD"]);
    let index = std::fs::read(other_git.join("index")).unwrap();
    let history = git(&other, &["log", "--stat"]);

    let run = c
        .run_with_env(
            &["capture", "probe gitdir"],
            &[
                ("GIT_DIR", other_git.to_str().unwrap()),
                ("GIT_WORK_TREE", other.to_str().unwrap()),
                ("GIT_INDEX_FILE", other_git.join("index").to_str().unwrap()),
            ],
        )
        .assert_ok()
        .says("committed ");
    let id = run.stdout_trim();

    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read(other_git.join("index")).unwrap(), index);
    assert_eq!(git(&other, &["log", "--stat"]), history);
    assert!(!other.join("inbox").exists() && !other.join("config.yaml").exists());
}

/// A history query answers from the corpus repository, not from whichever
/// repository an inherited `GIT_DIR` names, where the node has no history
/// and the answer would be a confident "none".
#[test]
fn history_ignores_an_inherited_git_dir() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Git dir", "--id", "gd"])
        .assert_ok()
        .stdout_trim();
    let other = other_repo(&c);

    c.run_with_env(
        &["log", &id],
        &[("GIT_DIR", other.join(".git").to_str().unwrap())],
    )
    .assert_ok()
    .says(&format!("neb new {id}"));
}

/// Give the repository at `repo` a `pre-commit` hook that writes its process
/// group to `pgid_file`, atomically, then runs `body`. The hooks directory is
/// named in the repository's own config, so no global `core.hooksPath` can
/// replace it.
#[cfg(unix)]
pub(super) fn pre_commit_hook(repo: &Path, pgid_file: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let hook = hooks.join("pre-commit");
    let recorded = pgid_file.display();
    write(
        &hook,
        &format!(
            "#!/bin/sh\n\
             ps -o pgid= -p $$ | tr -d ' ' > '{recorded}.tmp' && mv '{recorded}.tmp' '{recorded}'\n\
             {body}\n"
        ),
    );
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The process group a hook recorded.
#[cfg(unix)]
fn recorded_group(pgid_file: &Path) -> rustix::process::Pid {
    let raw = std::fs::read_to_string(pgid_file).expect("the hook ran and recorded its group");
    rustix::process::Pid::from_raw(raw.trim().parse().expect("a process group id"))
        .expect("a positive process group id")
}

/// Whether `group` is empty, by the group's own answer to signal 0. Killed
/// members are orphans that init reaps, so they may linger as zombies for a
/// moment; that moment is bounded here.
#[cfg(unix)]
fn group_gone(group: rustix::process::Pid) -> bool {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if matches!(
            rustix::process::test_kill_process_group(group),
            Err(rustix::io::Errno::SRCH)
        ) {
            return true;
        }
        if std::time::Instant::now() >= give_up {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Killing `neb` mid-commit used to orphan git, which then committed the
/// next writer's staged work under the dead verb's message. The signal now
/// stops git's whole group first, and `neb` still ends by that signal.
#[cfg(unix)]
#[test]
fn a_terminated_neb_takes_its_git_group_with_it() {
    use std::os::unix::process::ExitStatusExt;
    use std::time::{Duration, Instant};
    const HOOK_SLEEP: Duration = Duration::from_secs(5);

    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let pgid_file = c.workdir().join("hook.pgid");
    pre_commit_hook(&c.root, &pgid_file, "sleep 5");
    let before = log(&c.root);

    let mut capture = c.command(&["capture", "orphan probe"]);
    capture
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut neb = ChildGuard::spawn(&mut capture).unwrap_or_else(|e| panic!("{e}"));
    let waiting = Instant::now();
    while !pgid_file.exists() {
        assert!(neb.try_wait().is_none(), "neb finished before its hook ran");
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "the hook never ran"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let hook_started = Instant::now();
    let pid = rustix::process::Pid::from_raw(neb.id().try_into().unwrap()).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    let status = neb
        .wait(Duration::from_secs(20))
        .unwrap_or_else(|e| panic!("neb did not end after SIGTERM: {e}"));
    assert_eq!(
        status.signal(),
        Some(rustix::process::Signal::TERM.as_raw()),
        "neb ends by the signal it was sent: {status:?}"
    );
    assert!(
        hook_started.elapsed() < HOOK_SLEEP,
        "neb waited for the hook instead of stopping it"
    );
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "git or its hook outlived neb"
    );

    // Past the moment the hook would have returned and an orphaned git
    // would have committed.
    std::thread::sleep(
        (HOOK_SLEEP + Duration::from_secs(2)).saturating_sub(hook_started.elapsed()),
    );
    assert_eq!(log(&c.root), before, "a commit landed after neb was gone");
    assert!(!c.root.join(".git").join("index.lock").exists());

    // The next writer's commit is its own, and sweeps up the stopped one's
    // write, which stayed on disk.
    std::fs::remove_file(c.root.join(".git").join("hooks").join("pre-commit")).unwrap();
    let after = c.run(&["capture", "after"]).assert_ok().stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {after}"));
    let inbox = c.run(&["inbox"]).assert_ok().stdout();
    assert!(inbox.contains("orphan probe"), "{inbox}");
}

/// A commit that runs out of time is its own refusal, not a plain git
/// failure, and the write it was recording stays.
#[cfg(all(unix, debug_assertions))]
#[test]
fn git_timed_out_is_its_own_refusal_with_a_hook_hint() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let pgid_file = c.workdir().join("hook.pgid");
    pre_commit_hook(&c.root, &pgid_file, "sleep 30");
    let before = log(&c.root);
    // The debug-build seam: every git command gets two seconds.
    let short = [("NEBULA_TEST_GIT_DEADLINE_MS", "2000")];

    let run = c.run_with_env(&["--json", "capture", "held up"], &short);
    let envelope = run.refusal();
    assert_eq!(envelope["code"], "git_timed_out", "{envelope}");
    assert!(
        envelope["error"]
            .as_str()
            .unwrap()
            .starts_with("git commit did not finish within 2s"),
        "{envelope}"
    );
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook outlived the timeout"
    );
    assert_eq!(log(&c.root), before);
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    assert!(json.contains("\"held up\""), "the write stays: {json}");

    c.run_with_env(&["capture", "held up again"], &short)
        .assert_fails()
        .says("git commit did not finish within 2s")
        .says("A git hook is the likely cause")
        .says("hook run pre-commit")
        .says("--no-commit");
}

/// A migration is the write most worth its own commit. `migrate` reads the
/// setting through its lenient config model, keeps it, and commits itself.
#[test]
fn migrate_keeps_the_commit_setting_and_commits_itself() {
    let c = v1_corpus();
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    write(
        &c.root.join("config.yaml"),
        &format!("{config}commit: true\n"),
    );
    git_init(&c.root);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "v1 corpus"]);

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten")
        .says("committed ");
    assert_eq!(log(&c.root), ["neb migrate", "v1 corpus"]);
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(raw.contains("commit: true"), "{raw}");
    c.run(&["config", "commit"]).assert_ok().says("on");
}
