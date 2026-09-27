//! Supervised git: every git child runs in its own process group with a
//! deadline, and a hook that hangs or ignores SIGTERM cannot hold the lock.

use crate::harness::{committed, corpus, git, git_init, head};
use nebula_core::{Corpus, CorpusLock, Error, ops};

//
// Every git child runs in its own process group with a deadline. A hook is
// the part of `git commit` nebula does not control, so these drive the
// runner through one: it records its process group, then misbehaves.

/// The deadline the timeout tests shorten every git command to.
#[cfg(all(unix, debug_assertions))]
const SHORT: std::time::Duration = std::time::Duration::from_secs(2);

/// How much later than planned a supervised stop may finish on a loaded CI
/// machine.
#[cfg(unix)]
const SLACK: std::time::Duration = std::time::Duration::from_secs(5);

/// A corpus that is its own repository, with commits on and that setting
/// already committed, so the next commit is the one under test.
#[cfg(unix)]
pub(super) fn committing_corpus() -> (tempfile::TempDir, Corpus, std::path::PathBuf) {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));
    (dir, corpus, root)
}

/// Give the repository at `repo` a `pre-commit` hook that writes its process
/// group to `pgid_file`, atomically, and then runs `body`. The hooks
/// directory is named in the repository's own config, so a developer's
/// global `core.hooksPath` cannot replace it.
#[cfg(unix)]
pub(super) fn pre_commit_hook(repo: &std::path::Path, pgid_file: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let hook = hooks.join("pre-commit");
    let recorded = pgid_file.display();
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n\
             ps -o pgid= -p $$ | tr -d ' ' > '{recorded}.tmp' && mv '{recorded}.tmp' '{recorded}'\n\
             {body}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The process group a hook recorded.
#[cfg(unix)]
fn recorded_group(pgid_file: &std::path::Path) -> rustix::process::Pid {
    let raw = std::fs::read_to_string(pgid_file).expect("the hook ran and recorded its group");
    rustix::process::Pid::from_raw(raw.trim().parse().expect("a process group id"))
        .expect("a positive process group id")
}

/// Whether `group` is empty, by the group's own answer to signal 0: anything
/// but "no such group" counts as a member left. Members the sweep killed are
/// orphans, reaped by init rather than by nebula, so they may linger as
/// zombies for a moment; that moment is bounded here, and a live process
/// never leaves on its own within it.
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

/// Whether another thread could take the corpus lock right now, without
/// waiting. Another thread, because the lock is re-entrant on this one.
#[cfg(unix)]
fn lock_is_free(root: &std::path::Path) -> bool {
    let root = root.to_path_buf();
    std::thread::spawn(move || CorpusLock::acquire_within(&root, std::time::Duration::ZERO).is_ok())
        .join()
        .unwrap()
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn a_hung_commit_hook_times_out_and_releases_the_lock() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    pre_commit_hook(&root, &pgid_file, "sleep 30");
    let before = head(&root);

    let _short = nebula_core::GitDeadlineOverride::new(SHORT);
    let entry = ops::capture(&corpus, "a thought the hook holds up").unwrap();
    let started = std::time::Instant::now();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let took = started.elapsed();

    assert!(
        matches!(&err, Error::GitTimedOut { root: r, context, after }
            if r == &root && context == "commit" && *after == SHORT),
        "got {err:?}"
    );
    assert_eq!(err.code(), "git_timed_out");
    assert!(
        took < SHORT + nebula_core::GIT_TERMINATION_GRACE + SLACK,
        "took {took:?}"
    );
    assert!(
        corpus.inbox().unwrap().0.iter().any(|e| e.id == entry.id),
        "the write stays on disk whatever git does"
    );
    assert_eq!(head(&root), before, "nothing was committed");
    assert!(lock_is_free(&root), "the lock went with the verb");
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook's group outlived the timeout"
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn a_hook_that_ignores_sigterm_is_killed_after_the_grace_period() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    // `sleep` inherits the ignored SIGTERM, and holds git's stderr open.
    pre_commit_hook(&root, &pgid_file, "trap '' TERM\nsleep 30");

    let _short = nebula_core::GitDeadlineOverride::new(SHORT);
    let entry = ops::capture(&corpus, "a thought the hook will not let go").unwrap();
    let started = std::time::Instant::now();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let took = started.elapsed();

    assert!(matches!(err, Error::GitTimedOut { .. }), "got {err:?}");
    let grace = nebula_core::GIT_TERMINATION_GRACE;
    assert!(
        took >= SHORT + grace,
        "SIGTERM was ignored, so only SIGKILL after the grace ends it: took {took:?}"
    );
    assert!(took < SHORT + grace + SLACK, "took {took:?}");
    assert!(group_gone(recorded_group(&pgid_file)));
    assert!(lock_is_free(&root));
}

#[cfg(unix)]
#[test]
fn a_hook_descendant_does_not_outlive_a_clean_commit() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    // The background `sleep` keeps git's stderr open for a minute after git
    // itself is done.
    pre_commit_hook(&root, &pgid_file, "sleep 60 &\nexit 0");

    let entry = ops::capture(&corpus, "a thought with a straggler").unwrap();
    let started = std::time::Instant::now();
    let done = committed(ops::commit(&corpus, "capture", &[&entry.id]));
    let took = started.elapsed();

    assert_eq!(done.message, format!("neb capture {}", entry.id));
    assert_eq!(done.hash, head(&root));
    assert!(took < SLACK, "the commit waited on the straggler: {took:?}");
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook's background sleep outlived a clean commit"
    );
}

#[cfg(unix)]
#[test]
fn oversized_git_stderr_is_truncated_and_marked() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    pre_commit_hook(
        &root,
        &pgid_file,
        "head -c 5000000 /dev/zero | tr '\\0' x >&2\nexit 1",
    );

    let entry = ops::capture(&corpus, "a thought the hook shouts at").unwrap();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let Error::Git {
        context, stderr, ..
    } = &err
    else {
        panic!("got {err:?}");
    };
    assert_eq!(context, "commit");
    let cap = nebula_core::GIT_OUTPUT_CAP;
    let (shown, marker) = stderr
        .rsplit_once(" … ")
        .expect("a truncation marker after the text");
    assert!(shown.len() <= cap, "{} bytes shown", shown.len());
    assert!(
        shown.bytes().all(|b| b == b'x'),
        "the head of what the hook said"
    );
    let hidden: u64 = marker
        .strip_prefix("[truncated ")
        .and_then(|rest| rest.strip_suffix(" bytes]"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("`{marker}` is not the truncation marker"));
    assert!(hidden >= 5_000_000 - cap as u64, "{hidden}");
    assert!(stderr.len() <= cap + marker.len() + " … ".len());
    assert!(lock_is_free(&root));
}
