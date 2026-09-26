//! `capture` followed by `inbox`, through a temporary corpus, using the same
//! `session` functions the commands call. The line written is the one
//! `neb capture` writes, so `neb inbox` lists it too.

use nebula_core::{CommitOutcome, Corpus, Error, ops};
use nebula_desktop::session;
use std::path::Path;
use std::time::{Duration, Instant};

// This process runs with a temporary `HOME` and git environment, set before
// any test thread starts, and every child comes from its builder.
#[path = "../../../../crates/nebula-core/tests/support/mod.rs"]
mod support;

mod lock_holder;

use lock_holder::LockHolder;

/// Run git in `root`, asserting it succeeded; stdout as text.
fn git_in(root: &Path, args: &[&str]) -> String {
    let output = support::output(
        support::git_command(root, support::home()).args(args),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

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
    let git = |args: &[&str]| git_in(&root, args);
    git(&["init", "-q"]);
    git(&["config", "user.name", "neb-test"]);
    git(&["config", "user.email", "neb-test@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    ops::set_commit(&mut corpus, true).unwrap();
    assert!(matches!(
        ops::commit(&corpus, "config", &["commit"]).unwrap(),
        CommitOutcome::Committed(_)
    ));

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
    let git = |args: &[&str]| git_in(&root, args);
    git(&["init", "-q"]);
    git(&["config", "user.name", "neb-test"]);
    git(&["config", "user.email", "neb-test@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    ops::set_commit(&mut corpus, true).unwrap();
    assert!(matches!(
        ops::commit(&corpus, "config", &["commit"]).unwrap(),
        CommitOutcome::Committed(_)
    ));

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

/// The desktop opens the corpus at launch, and a launch is a read: a root
/// with `nodes/` and no `config.yaml` is the missing-config error, never a
/// corpus quietly stamped with a config it did not have.
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the fixture plants a config-less corpus directly; the code under test only reads"
)]
fn opening_a_configless_corpus_errors_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::write(
        root.join("nodes").join("an-idea.md"),
        "---\nid: an-idea\ntitle: An idea\nstatus: seed\ncreated: 2026-09-01\nupdated: 2026-09-01\n---\n\nThe idea.\n",
    )
    .unwrap();
    let listing = |dir: &Path| {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    let (root_before, nodes_before) = (listing(&root), listing(&root.join("nodes")));

    let refused = session::open(&root);
    assert!(
        matches!(&refused, Err(Error::MissingConfig { path }) if *path == root.join("config.yaml")),
        "{refused:?}"
    );
    assert_eq!(root_before, listing(&root));
    assert_eq!(nodes_before, listing(&root.join("nodes")));
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
    let holder = LockHolder::start(&root);

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

    holder.release();
    let corpus = session::open(&root).unwrap();
    assert_eq!(session::inbox(&corpus).unwrap()[0].id, pending.id);
    session::drop_entry(&corpus, &pending.id).unwrap();
    assert!(session::inbox(&corpus).unwrap().is_empty());
    let entry = session::capture(&corpus, "retry me").unwrap();
    assert_eq!(session::inbox(&corpus).unwrap()[0].id, entry.id);
}

/// Every child this suite starts comes from the isolating builder in
/// `support`.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    let strays = support::commands_outside(include_str!("roundtrip.rs"), &[]);
    assert!(
        strays.is_empty(),
        "roundtrip.rs creates a child outside `support::command`:\n{}",
        strays.join("\n")
    );
}
