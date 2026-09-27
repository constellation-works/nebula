//! The corpus config: a missing, symlinked or deleted `config.yaml`, the
//! legacy observatory root, and settings rewritten under the lock.

use crate::harness::{
    committed, committed_paths, corpus, every_file, git, git_init, head, mkfifo, not_regular,
    process_locations, within_two_seconds,
};
use crate::support;
use nebula_core::{CommitOutcome, Corpus, Error, ops};

/// A current-schema node file, as `neb new` would write it.
const V2_NODE: &str = "---\nid: an-idea\ntitle: An idea\nstatus: seed\ncreated: 2026-09-01\nupdated: 2026-09-01\n---\n\nThe idea.\n";

/// A corpus written by v0.1, before `config.yaml` existed: one node in the
/// v1 shape and nothing else.
pub(super) const V1_NODE: &str = "---\nid: first\ntitle: First\ndomain: Physics\nstatus: testing\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe first.\n";

/// A root with `nodes/` holding `node` as `<id>.md`, and no `config.yaml`.
pub(super) fn configless(dir: &std::path::Path, id: &str, node: &str) -> std::path::PathBuf {
    let root = dir.join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::write(root.join("nodes").join(format!("{id}.md")), node).unwrap();
    root
}

/// A missing `config.yaml` is refused, not synthesized: opening is a read,
/// and a read that wrote a fresh config would stamp this build's schema over
/// files that may predate it and mint an id the corpus never had.
#[test]
fn open_never_writes_a_missing_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "an-idea", V2_NODE);
    let before = every_file(&root);

    let refused = Corpus::open(&process_locations(), Some(root.clone()));
    assert!(
        matches!(&refused, Err(Error::MissingConfig { path }) if *path == root.join("config.yaml")),
        "{refused:?}"
    );
    assert_eq!(before, every_file(&root));
}

/// The corpora this refusal exists for are still one command from working:
/// `migrate` reads a missing config as a corpus from before the file.
#[test]
fn v1_corpus_without_config_still_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "first", V1_NODE);
    let before = every_file(&root);

    assert!(matches!(
        Corpus::open(&process_locations(), Some(root.clone())),
        Err(Error::MissingConfig { .. })
    ));
    assert_eq!(before, every_file(&root), "a refused open wrote something");

    let report = nebula_core::migrate::run(&process_locations(), Some(root.clone())).unwrap();
    assert!(report.config_rewritten);
    assert!(report.minted_corpus_id.is_some(), "{report:?}");
    assert_eq!(
        report
            .rewritten
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    let node = std::fs::read_to_string(root.join("nodes").join("first.md")).unwrap();
    assert!(node.contains("tags:\n- physics\n"), "{node}");
    assert!(node.contains("status: hypothesis"), "{node}");

    let corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert_eq!(corpus.load_all().unwrap().len(), 1);
}

/// Absent means `NotFound`. A `config.yaml` that is there but cannot be
/// followed is a different fault, named as the path it is, and the symlink is
/// left for whoever made it.
#[cfg(unix)]
#[test]
fn a_dangling_config_symlink_is_not_a_missing_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "an-idea", V2_NODE);
    let config = root.join("config.yaml");
    std::os::unix::fs::symlink(dir.path().join("gone.yaml"), &config).unwrap();

    let refused = Corpus::open(&process_locations(), Some(root.clone()))
        .expect_err("a dangling config is refused");
    assert!(
        !matches!(refused, Error::MissingConfig { .. }),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("config.yaml"), "{refused}");
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!dir.path().join("gone.yaml").exists());
}

/// A symlinked `config.yaml` handed the corpus another file's settings, and
/// the next config write replaced the link; a FIFO there hung every verb.
/// Both are refused by name, before anything is read, and a config write
/// that re-reads under the lock refuses the same way.
#[cfg(unix)]
#[test]
fn a_symlinked_or_fifo_config_yaml_is_refused() {
    use nebula_core::fs::EntryKind;
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    let outside = dir.path().join("outside.yaml");
    std::fs::rename(&config, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &config).unwrap();
    let before = std::fs::read(&outside).unwrap();

    let opened = Corpus::open(&process_locations(), Some(root.clone()));
    assert!(
        not_regular(&opened, &config, EntryKind::Symlink),
        "{opened:?}"
    );
    let set = ops::set_commit(&mut corpus, true);
    assert!(not_regular(&set, &config, EntryKind::Symlink), "{set:?}");
    let initialized = Corpus::init(&process_locations(), &root);
    assert!(
        not_regular(&initialized, &config, EntryKind::Symlink),
        "{initialized:?}"
    );
    // Found, not walked past: opening from inside refuses it by name rather
    // than opening some other corpus further up.
    assert_eq!(Corpus::discover(&root.join("nodes")), Some(root.clone()));
    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );

    std::fs::remove_file(&config).unwrap();
    mkfifo(&config);
    let opened = within_two_seconds({
        let root = root.clone();
        move || Corpus::open(&process_locations(), Some(root))
    });
    assert!(not_regular(&opened, &config, EntryKind::Fifo), "{opened:?}");
    let discovered = within_two_seconds({
        let nodes = root.join("nodes");
        move || Corpus::discover(&nodes)
    });
    assert_eq!(discovered, Some(root));
}

/// `init` finishes a root it was interrupted in, where nothing has been
/// written yet, and refuses one that already holds content: minting a
/// config there is the same fabrication `open` no longer does.
#[test]
fn init_completes_an_interrupted_init_but_refuses_a_configless_corpus() {
    let dir = tempfile::tempdir().unwrap();

    let interrupted = dir.path().join("interrupted");
    std::fs::create_dir_all(interrupted.join("nodes")).unwrap();
    Corpus::init(&process_locations(), &interrupted).unwrap();
    assert!(interrupted.join("inbox").is_dir());
    let config = std::fs::read_to_string(interrupted.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 2"), "{config}");
    assert!(config.contains("corpus_id: neb-"), "{config}");
    Corpus::open(&process_locations(), Some(interrupted)).unwrap();

    let with_content = configless(&dir.path().join("content"), "an-idea", V2_NODE);
    let before = every_file(&with_content);
    let refused = Corpus::init(&process_locations(), &with_content);
    assert!(
        matches!(&refused, Err(Error::MissingConfig { path }) if *path == with_content.join("config.yaml")),
        "{refused:?}"
    );
    assert_eq!(
        before,
        every_file(&with_content),
        "a refused init created something"
    );

    // Captures are content too: a corpus that has only ever been captured
    // into has an identity of its own to lose.
    let captured = dir.path().join("captured");
    std::fs::create_dir_all(captured.join("nodes")).unwrap();
    std::fs::create_dir_all(captured.join("inbox")).unwrap();
    std::fs::write(
        captured.join("inbox").join("2026-09.md"),
        "- [0a1b] 2026-09-01T10:00:00Z a thought\n",
    )
    .unwrap();
    let before = every_file(&captured);
    assert!(matches!(
        Corpus::init(&process_locations(), &captured),
        Err(Error::MissingConfig { .. })
    ));
    assert_eq!(before, every_file(&captured));
}

/// A writer re-reads `config.yaml` under its lock. When it is gone, that is
/// a refusal: bringing it back with the id in hand would still turn a
/// `commit: true` it no longer holds into off.
#[test]
fn a_config_deleted_under_a_writer_is_refused_not_recreated() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    ops::set_commit(&mut corpus, true).unwrap();
    let mut corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert!(corpus.commit_setting().enabled);
    std::fs::remove_file(&config).unwrap();

    let committed = ops::commit(&corpus, "capture", &["x"]);
    assert!(
        matches!(&committed, Err(Error::MissingConfig { path }) if *path == config),
        "{committed:?}"
    );
    assert!(!config.exists());

    for enabled in [true, false] {
        let set = ops::set_commit(&mut corpus, enabled);
        assert!(
            matches!(&set, Err(Error::MissingConfig { path }) if *path == config),
            "{set:?}"
        );
        assert!(!config.exists());
    }
}

/// Give the corpus at `root` the `observatory_root` key an older `neb` wrote
/// into `config.yaml`: another machine's absolute path. Written by hand
/// because nothing in this build writes it any more. Returns the path.
pub(super) fn with_legacy_observatory_root(root: &std::path::Path) -> std::path::PathBuf {
    let foreign = std::path::PathBuf::from("/Users/someone-else/workspace/observatory");
    let config = root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{raw}observatory_root: {}\n", foreign.display()),
    )
    .unwrap();
    foreign
}

/// A corpus written before the setting moved out of `config.yaml` still
/// opens, and the key is reported as the legacy setting it is, whichever
/// setting wins on this machine.
#[test]
fn a_legacy_observatory_root_in_config_yaml_still_loads_and_is_reported() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    let corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    let setting = corpus.observatory_root().unwrap();
    assert_eq!(setting.legacy, Some(foreign));
    assert!(setting.root.is_some(), "the legacy key is still a fallback");
}

/// Dropping the key rewrites `config.yaml` without it and keeps the rest;
/// with no key there it writes nothing at all.
#[test]
fn dropping_the_legacy_observatory_root_keeps_every_other_setting() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    let pristine = std::fs::read(&config).unwrap();

    let dropped = ops::drop_legacy_observatory_root(&mut corpus).unwrap();
    assert_eq!((dropped.removed, dropped.setting.legacy), (None, None));
    assert_eq!(pristine, std::fs::read(&config).unwrap(), "a no-op wrote");

    let foreign = with_legacy_observatory_root(&root);
    let mut corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    let dropped = ops::drop_legacy_observatory_root(&mut corpus).unwrap();
    assert_eq!(dropped.removed, Some(foreign), "it names what it removed");
    assert_eq!(pristine, std::fs::read(&config).unwrap());
}

/// The machine setting is read from any directory, so a relative one is
/// refused before anything is written.
#[test]
fn a_relative_observatory_root_is_refused_before_it_is_written() {
    let (_dir, corpus) = corpus();
    let relative = std::path::Path::new("observatory");
    assert!(matches!(
        ops::set_observatory_root(&corpus, relative),
        Err(Error::RelativeObservatoryRoot { root, setting: None }) if root == relative
    ));
    // The machine setting lives under `HOME`, which here is this process's
    // temporary one, never the developer's.
    assert_eq!(std::env::var_os("HOME"), Some(support::home().into()));
    assert!(
        !support::home()
            .join(".config/nebula/observatory-root")
            .exists()
    );
}

/// `Corpus::open` reads `config.yaml` without the lock, because a read must
/// never wait on a writer. A writer that then waits its turn is holding a
/// snapshot from before the writer ahead of it finished, and every config
/// write rewrites the file whole — so without a reload under the lock the
/// waiter carries the stale copy of every key it is not setting back to disk
/// and the other writer's setting is gone without a word.
///
/// Two `Corpus` handles on one thread rather than two threads: the interleave
/// under test is a *stale open*, not contention, and sequencing it by hand is
/// what makes the loss deterministic. `crates/neb/tests/cli/lock.rs` runs the same
/// shape across two processes against a really held `flock`.
#[test]
fn a_setting_landed_since_open_survives_the_next_writers_rewrite() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    // The waiter opens — and so snapshots the config — before the writer
    // ahead of it in the queue has written anything.
    let mut waiting = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(!waiting.commit_setting().enabled);

    // The writer ahead finishes its own config write and lets go.
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(ops::set_commit(&mut ahead, true).unwrap().enabled);

    // The waiter gets in and removes a different key.
    let setting = ops::drop_legacy_observatory_root(&mut waiting)
        .unwrap()
        .setting;
    assert_eq!(setting.legacy, None);

    let reopened = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the commit setting written after the waiter opened was erased by its rewrite"
    );
    assert_eq!(
        reopened.observatory_root().unwrap().legacy,
        None,
        "and the waiter's own change must still have landed"
    );
    let raw = std::fs::read_to_string(root.join("config.yaml")).unwrap();
    assert!(!raw.contains(foreign.to_str().unwrap()), "{raw}");
}

/// The same loss in the other direction, so neither setting is merely the one
/// that happens to be written last: a stale snapshot that still holds the
/// legacy key must not write it back.
#[test]
fn a_stale_commit_write_does_not_restore_the_legacy_key_dropped_since_it_opened() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    let mut waiting = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert_eq!(
        waiting.observatory_root().unwrap().legacy,
        Some(foreign.clone())
    );

    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::drop_legacy_observatory_root(&mut ahead).unwrap();

    assert!(ops::set_commit(&mut waiting, true).unwrap().enabled);

    let reopened = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert_eq!(
        reopened.observatory_root().unwrap().legacy,
        None,
        "the legacy key dropped after the waiter opened came back"
    );
    assert!(reopened.commit_setting().enabled);
}

/// Whether a write is recorded is a question about the configuration in
/// force, not the one this corpus happened to open with. A verb that waited
/// out a writer who turned the setting on commits; one that waited out a
/// writer who turned it off does not.
#[test]
fn the_commit_decision_follows_the_setting_on_disk_not_the_one_at_open() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);

    // Opened while the setting was off.
    let stale_off = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(!stale_off.commit_setting().enabled);

    // Another writer turns it on and records that.
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, true).unwrap();
    committed(ops::commit(&ahead, "config", &["commit"]));

    let entry = ops::capture(&stale_off, "a thought the setting now says to record").unwrap();
    let done = committed(ops::commit(&stale_off, "capture", &[&entry.id]));
    assert_eq!(done.message, format!("neb capture {}", entry.id));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    // And the other way: opened while it was on, but off by the time the
    // verb lands.
    let stale_on = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(stale_on.commit_setting().enabled);
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, false).unwrap();

    let before = head(&root);
    let entry = ops::capture(&stale_on, "a thought the setting now says to leave").unwrap();
    assert!(matches!(
        ops::commit(&stale_on, "capture", &[&entry.id]),
        Ok(CommitOutcome::Disabled)
    ));
    assert_eq!(head(&root), before, "nothing was committed");
    assert!(
        git(&root, &["status", "--porcelain"]).contains(" M inbox/"),
        "the capture is on disk, just not recorded"
    );
}
