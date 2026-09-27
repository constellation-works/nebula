//! The write lock across processes: concurrent writers, waiting it out,
//! and the lock and pending files never committed.

use crate::harness::{Corpus, corpus_repo, dirt, git, git_init, head_paths, log, write};
use crate::observatory::with_legacy_observatory_root;
use crate::supervised_git::pre_commit_hook;
use std::path::PathBuf;

/// Two `neb` processes editing one node's tags at the same time. Each is a
/// load, an edit and a save, so without `<root>/.lock` the second reads the
/// node as it was before the first saved and the later write wins: one tag
/// lands and the other is gone without a word.
///
/// Separate processes deliberately. `flock` is held by the open file
/// description, so only a second *process* exercises it;
/// `crates/nebula-core/tests/core/lock.rs` runs the same race across two threads,
/// which is what exercises the in-process half.
#[test]
fn two_concurrent_tag_writes_from_separate_processes_both_land() {
    let c = Corpus::new();
    let id = c.seed("a node two writers will tag", "Contended node");

    let first = c.spawn(&["tag", &id, "--add", "alpha"]);
    let second = c.spawn(&["tag", &id, "--add", "beta"]);
    first.wait().assert_ok();
    second.wait().assert_ok();

    let shown = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let shown: serde_json::Value = serde_json::from_str(&shown).expect("show --json");
    let mut tags: Vec<&str> = shown["node"]["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t.as_str().expect("a tag"))
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["alpha", "beta"], "one writer's tag was lost");
    c.run(&["check"]).assert_ok();
}

/// Past the bounded wait the writer refuses, naming who holds the lock and
/// saying what to do. The write never happens, so retrying is safe.
///
/// The lock is held here in the test process and contended by a spawned
/// `neb`, so this is a real cross-process `flock` and not the in-process
/// table standing in for one, and the holder `neb` names is read from the
/// record this process wrote.
#[test]
fn a_writer_that_waits_out_the_lock_refuses_with_a_hint_and_writes_nothing() {
    let c = Corpus::new();
    let id = c.seed("a node nobody else gets to edit", "Locked node");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    let held = nebula_core::CorpusLock::acquire_as(&c.root, nebula_core::LOCK_WAIT, "test holder")
        .expect("holding the lock");
    let named = format!("(`test holder`, pid {}, for ", std::process::id());

    // It waits the full five seconds before giving up, which is the bound
    // under test.
    let refused = c
        .run(&["tag", &id, "--add", "never"])
        .assert_fails()
        .says("another nebula writer is holding")
        .says(&named)
        .says("mid-write")
        .says(&format!("ps -p {}", std::process::id()));
    assert!(
        !refused.stderr().contains("run it again in a moment"),
        "{}",
        refused.stderr()
    );

    // The same refusal under `--json`: the holder is in `error`, and the
    // hint names it too.
    let envelope = c
        .run(&["--json", "tag", &id, "--add", "never"])
        .assert_fails()
        .refusal();
    assert_eq!(envelope["code"], "locked", "{envelope}");
    let error = envelope["error"].as_str().expect("error");
    assert!(error.contains(&named), "{error}");
    let hint = envelope["hint"].as_str().expect("hint");
    assert!(
        hint.contains(&format!("`test holder` (pid {})", std::process::id()))
            && !hint.contains("in a moment"),
        "{hint}"
    );

    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "the refusal came before the write"
    );
    // A read never waits on a writer, whoever is holding it. The reading
    // forms of `neb config` are reads too: they answer from the config the
    // open snapshotted and take no lock at all.
    c.run(&["show", &id]).assert_ok();
    c.run(&["list"]).assert_ok();
    c.run(&["check"]).assert_ok();
    c.run(&["config", "commit"]).assert_ok().says("off");
    c.run(&["config", "observatory-root"]).assert_ok();

    // And once it is free, the same write goes through.
    drop(held);
    c.run(&["tag", &id, "--add", "never"]).assert_ok();
    assert!(
        std::fs::read_to_string(c.node_file(&id))
            .unwrap()
            .contains("never")
    );
}

/// The interleave this was found by. A config write rewrites `config.yaml`
/// whole, and the writer doing it opened — and so read the file — before the
/// writer ahead of it in the queue had finished its own config write. Without
/// a reload under the lock the waiter exits 0 and quietly takes the other
/// writer's setting back out.
///
/// A real `flock` held in this process and contended by a spawned `neb`, so
/// this is the cross-process shape; `crates/nebula-core/tests/core/config.rs`
/// sequences the same staleness by hand, which is what makes it deterministic.
#[test]
fn a_config_write_that_waited_out_the_lock_keeps_the_setting_written_meanwhile() {
    let (c, _remote) = corpus_repo();
    let foreign = with_legacy_observatory_root(&c);

    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    // The waiter opens — snapshotting a config with no `commit` key — and
    // then polls for the lock this thread is holding.
    let waiting = c.spawn(&["config", "observatory-root", "--drop-legacy"]);

    // Long enough for the spawned `neb` to be past its open and into the
    // wait, and far short of the five seconds it would wait in total. On a
    // machine slow enough that it has not opened yet, the snapshot it takes
    // is simply a fresh one and this passes without having raced — the core
    // test is the one that cannot miss.
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The writer ahead finishes its config write and lets go. Taking the
    // lock again underneath is the re-entry the CLI relies on too.
    let mut ahead =
        nebula_core::Corpus::open(&nebula_core::Locations::default(), Some(c.root.clone()))
            .expect("open");
    assert!(
        nebula_core::ops::set_commit(&mut ahead, true)
            .unwrap()
            .enabled
    );
    drop(held);

    waiting.wait().assert_ok();

    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(
        raw.contains("commit: true"),
        "the waiter rewrote the config from its pre-lock snapshot: {raw}"
    );
    assert!(
        !raw.contains(&foreign),
        "and the waiter's own change must still have landed: {raw}"
    );
    c.run(&["config", "commit"]).assert_ok().says("on");
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says("no observatory root");
}

/// The lock file is the one thing under the root that is not corpus content,
/// so `neb commit` never stages it and `check` never reads it — nor the
/// holder record written into it, which is there for the whole of every
/// write and the commit that records it.
#[test]
fn the_lock_file_is_never_staged_and_never_checked() {
    let c = Corpus::new();
    git_init(&c.root);
    c.run(&["config", "commit", "on"]).assert_ok();
    // A record a crashed holder left behind: present from before the first
    // write, overwritten by each writer, and emptied as each lets go.
    let stale = r#"{"pid":1,"since":"2026-09-26T00:00:00Z","label":"a crashed neb"}"#;
    write(&c.root.join(".lock"), stale);
    let id = c.seed("a write that takes the lock", "Locked write");

    assert!(c.root.join(".lock").exists(), "the write took the lock");
    assert_eq!(
        std::fs::read_to_string(c.root.join(".lock")).unwrap(),
        "",
        "the writer replaced the stale record and cleared its own on release"
    );
    assert!(
        !git(&c.root, &["ls-files"]).contains(".lock"),
        "the lock file is not tracked"
    );
    assert!(
        !git(&c.root, &["status", "--porcelain"]).contains(".lock"),
        "the ignored lock is absent from repository status"
    );
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
    let paths = head_paths(&c.root);
    assert!(
        paths.contains(&format!("nodes/{id}.md")) && paths.iter().all(|p| p != ".lock"),
        "the commit is the promotion and nothing beside it: {paths:?}"
    );

    // Checked while a holder record sits in the file.
    write(&c.root.join(".lock"), stale);
    c.run(&["check"]).assert_ok().says("0 errors");
    let report = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["nodes"], 1, "the lock file is not read as a node");
    assert_eq!(report["findings"].as_array().unwrap().len(), 0);
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

/// A `neb` holding the lock names itself by its verb and target. Its
/// pre-commit hook stalls with the lock held, the way a slow commit does, and
/// a waiter in this process is told that `neb tag <id>`, at `neb`'s PID, is
/// the holder. The commit made under that record does not stage it.
#[cfg(unix)]
#[test]
fn a_neb_mid_write_is_named_by_its_verb_and_commits_without_its_record() {
    use std::time::{Duration, Instant};
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("a node a slow commit holds", "Slow commit");
    let pgid_file = c.workdir().join("hook.pgid");
    let release = c.workdir().join("release");
    // Bounded, so a test that fails before releasing it cannot wedge `neb`.
    pre_commit_hook(
        &c.root,
        &pgid_file,
        &format!(
            "i=0; while [ ! -e '{}' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done",
            release.display()
        ),
    );

    let tagging = c.spawn(&["tag", &id, "--add", "slow"]);
    let neb = tagging.child.id();
    let waiting = Instant::now();
    while !pgid_file.exists() {
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "the hook never ran"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let refused =
        nebula_core::CorpusLock::acquire_within(&c.root, Duration::from_millis(50)).map(|_| ());
    write(&release, "");
    tagging.wait().assert_ok();

    let Err(nebula_core::Error::Locked {
        holder: Some(holder),
        ..
    }) = &refused
    else {
        panic!("expected a named holder, got {refused:?}");
    };
    assert_eq!(holder.label, format!("neb tag {id}"));
    assert_eq!(holder.pid, neb, "the holder is the spawned neb");
    assert_eq!(
        std::fs::read_to_string(c.root.join(".lock")).unwrap(),
        "",
        "neb cleared its record on release"
    );
    let paths = head_paths(&c.root);
    assert_eq!(paths, [format!("nodes/{id}.md")], "{paths:?}");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

/// Plant the temporary files a write killed before its rename leaves, one
/// in each directory a commit stages whole.
pub(super) fn plant_debris(c: &Corpus) -> [PathBuf; 2] {
    let debris = [
        c.root.join("nodes/x.md.1f2e-0-18d8.tmp"),
        c.root.join("inbox/2026-09.md.1f2e-0-18d8.tmp"),
    ];
    for path in &debris {
        write(path, "half of a write that never landed\n");
    }
    debris
}

/// A killed write's temporary file sits in `nodes/` or `inbox/`, which a
/// commit stages whole. It stays on disk and out of the history, whether or
/// not the corpus `.gitignore` already carries the rule that hides it.
#[test]
fn a_stale_temp_file_is_never_committed() {
    let c = Corpus::new();
    git_init(&c.root);
    // As an older `init` left it: the lock rule and nothing for debris, so
    // the commit's own pathspec is what keeps the debris out.
    write(&c.root.join(".gitignore"), "/.lock\n");
    c.run(&["config", "commit", "on"]).assert_ok();
    let debris = plant_debris(&c);

    c.run(&["capture", "debris test"])
        .assert_ok()
        .says("committed ");

    let tracked = git(&c.root, &["ls-files"]);
    assert!(!tracked.contains(".tmp"), "{tracked}");
    assert!(
        tracked.contains("inbox/"),
        "the capture itself is committed: {tracked}"
    );
    for path in &debris {
        assert!(path.exists(), "{} was deleted", path.display());
    }

    // And with the rule `init` now writes, a person's `git add -A` skips it too.
    c.run(&["init"]).assert_ok();
    git(&c.root, &["add", "-A"]);
    let staged = git(&c.root, &["diff", "--cached", "--name-only"]);
    assert!(!staged.contains(".tmp"), "{staged}");
}

/// The pending-write record is a fact about a write in flight on one
/// machine, like the lock: ignored, never staged, and settled by the next
/// write whether or not that write commits.
#[test]
fn the_pending_record_is_never_staged() {
    let c = Corpus::new();
    git_init(&c.root);
    c.run(&["config", "commit", "on"]).assert_ok();
    let entry = c
        .run(&["capture", "an interrupted promotion idea"])
        .stdout_trim();
    let node = c.run(&["promote", &entry]).assert_ok().stdout_trim();
    // The state a crash between the node and the strike leaves.
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "md"))
        .unwrap();
    let text = std::fs::read_to_string(&month).unwrap();
    let struck = text
        .lines()
        .find(|l| l.contains(&entry))
        .unwrap()
        .to_string();
    let live = struck
        .replacen("- ~~[", "- [", 1)
        .split("~~ ->")
        .next()
        .unwrap()
        .to_string();
    write(&month, &text.replace(&struck, &live));
    let stamp = live.split(' ').nth(2).unwrap();
    write(
        &c.root.join(".pending"),
        &format!(r#"{{"op":"promote","entry":"{entry}","stamp":"{stamp}","node":"{node}"}}"#),
    );

    assert_eq!(git(&c.root, &["check-ignore", ".pending"]), ".pending\n");
    git(&c.root, &["add", "-A"]);
    assert!(
        !git(&c.root, &["diff", "--cached", "--name-only"]).contains(".pending"),
        "git add -A must not stage the pending record"
    );
    // `check` reads it and leaves it; `inbox` no longer offers the entry.
    c.run(&["check"]).assert_ok().says("did not finish");
    assert!(c.root.join(".pending").exists());
    assert!(!c.run(&["inbox"]).assert_ok().stdout().contains(&entry));

    c.run(&["capture", "the next thought"])
        .assert_ok()
        .says("committed ");

    assert!(
        !c.root.join(".pending").exists(),
        "the next write settled it"
    );
    assert!(
        std::fs::read_to_string(&month)
            .unwrap()
            .contains(&format!("~~ -> {node}")),
        "and finished the promotion"
    );
    assert!(!git(&c.root, &["ls-files"]).contains(".pending"));
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

#[test]
fn git_add_all_cannot_stage_the_lock_or_block_later_neb_commits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(&root).unwrap();
    git_init(&root);
    let c = Corpus { dir, root };

    c.run(&["init"]).assert_ok();
    assert!(c.root.join(".lock").exists());
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(
        !git(&c.root, &["status", "--porcelain"]).contains(".lock"),
        "init must not leave the runtime lock visible to git"
    );

    c.run(&["config", "commit", "on"])
        .assert_ok()
        .says("committed ");
    git(&c.root, &["add", "-A"]);
    assert!(
        !git(&c.root, &["diff", "--cached", "--name-only"]).contains(".lock"),
        "git add -A must not stage the runtime lock"
    );

    c.run(&["new", "Still commits", "--id", "still-commits"])
        .assert_ok()
        .says("committed ");
    assert_eq!(log(&c.root)[0], "neb new still-commits");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}
