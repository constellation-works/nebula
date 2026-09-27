//! The write lock: concurrent tag adds, no-op rewrites, conditional body
//! sets, kept edits, and the lock file itself.

use crate::harness::{
    committed, committed_paths, corpus, every_file, git, git_init, mkfifo, not_regular,
    process_locations, seed, within_two_seconds,
};
use crate::support;
use nebula_core::{Corpus, CorpusLock, Error, Graph, ops};

/// Two writers editing one node's tags at the same time. Each op is a load,
/// an edit and a save; without the lock the second load sees the corpus as it
/// was before the first save, and the later rename wins — one tag survives and
/// the other is silently gone.
///
/// Threads rather than processes here: this is what the in-process half of
/// the lock is for, since `flock` is held by the open file description and
/// would let a second thread of one process straight through.
/// `crates/neb/tests/cli/lock.rs` runs the same race across two spawned binaries,
/// which is what exercises `flock` itself.
#[test]
fn two_concurrent_tag_adds_on_one_node_both_survive() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "A node two writers will tag", &[]);
    let root = dir.path().join("corpus");

    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for tag in ["alpha", "beta"] {
            let (root, start, id) = (root.clone(), &start, id.clone());
            scope.spawn(move || {
                let corpus = Corpus::open(&process_locations(), Some(root)).expect("open");
                start.wait();
                ops::tag_add(&corpus, &id, &[tag.to_string()]).expect("tag");
            });
        }
    });

    let mut tags = corpus.load(&id).unwrap().node.tags;
    tags.sort();
    assert_eq!(tags, ["alpha", "beta"], "one writer's tag was lost");
}

/// A tag not there to remove, or already there to add, is named in the
/// result; when the tags come out as they were stored, nothing is written
/// (STD-01 §R30). A request with one real change in it is written.
#[test]
fn a_retag_that_changes_nothing_writes_nothing_and_names_each_tag() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Tagged", &[]);
    ops::tag_add(&corpus, &id, &["physics".to_string()]).unwrap();
    let path = corpus.node_path(&id).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let tags = |t: &[&str]| t.iter().map(ToString::to_string).collect::<Vec<_>>();

    let done = ops::retag(&corpus, &id, &tags(&["Physics"]), &tags(&["absent"])).unwrap();
    assert!(!done.written);
    assert_eq!(done.already, ["physics"], "normalised as stored");
    assert_eq!(done.absent, ["absent"]);
    assert_eq!(done.doc.node.tags, ["physics"]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let done = ops::retag(&corpus, &id, &tags(&["new"]), &tags(&["absent"])).unwrap();
    assert!(done.written);
    assert_eq!((done.already.len(), done.absent), (0, tags(&["absent"])));
    assert_eq!(corpus.load(&id).unwrap().node.tags, ["physics", "new"]);
}

/// `neb edit` saves a body it read before the editor opened, with no lock
/// held while the person typed. The compare-and-set under the lock is what
/// stops that save erasing a note another writer appended meanwhile; a
/// frontmatter-only change is not a conflict, and the edit lands on top.
#[test]
fn set_body_if_refuses_a_changed_body_and_keeps_a_changed_frontmatter() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Edited elsewhere", &[]);
    let read = corpus.load(&id).unwrap().body;

    // A tag lands while the editor is open: the edit still goes through,
    // and keeps it.
    ops::tag_add(&corpus, &id, &["x".to_string()]).unwrap();
    let saved = ops::set_body_if(&corpus, &id, &read, "the new body\n")
        .unwrap()
        .expect("a changed body is written");
    assert_eq!(saved.body, "the new body");
    let on_disk = corpus.load(&id).unwrap();
    assert_eq!(on_disk.body.trim(), "the new body");
    assert_eq!(on_disk.node.tags, ["x"]);

    // A note lands while the editor is open: the body is no longer the one
    // that was edited, so nothing is written.
    let read = on_disk.body;
    ops::note(&corpus, &id, "a concurrent note", None).unwrap();
    let before = std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap();
    let refused = ops::set_body_if(&corpus, &id, &read, "an edit made blind");
    assert!(
        matches!(&refused, Err(Error::EditConflict(node)) if *node == id),
        "got {refused:?}"
    );
    assert_eq!(refused.unwrap_err().code(), "edit_conflict");
    assert_eq!(
        std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap(),
        before,
        "the concurrent note survives"
    );
}

/// A body saved as it already is changes nothing, so nothing is written and
/// `updated` stays put (STD-01 §R30): not with the compare-and-set, even when
/// another writer changed the body meanwhile, and not without it.
#[test]
fn a_body_set_to_what_it_already_is_writes_nothing() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Left alone", &[]);
    ops::set_body(&corpus, &id, "the body")
        .unwrap()
        .expect("a new body is written");
    // Back-dated, so a save would show in `updated` as well as in the bytes.
    let path = corpus.node_path(&id).unwrap();
    let updated = corpus.load(&id).unwrap().node.updated;
    let pristine = std::fs::read_to_string(&path)
        .unwrap()
        .replace(&format!("updated: {updated}"), "updated: 2020-01-01");
    std::fs::write(&path, &pristine).unwrap();
    let read = corpus.load(&id).unwrap().body;

    assert!(ops::body_unchanged(&read, "the body"));
    assert!(
        ops::body_unchanged(&read, "\n the body \n\n"),
        "stored trimmed"
    );
    assert!(!ops::body_unchanged(&read, "the body, changed"));
    assert!(ops::set_body(&corpus, &id, "the body\n").unwrap().is_none());
    assert!(
        ops::set_body_if(&corpus, &id, &read, &read)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), pristine);

    // An edit that changed nothing has nothing to write over a note that
    // landed meanwhile, so it is not a conflict either.
    ops::note(&corpus, &id, "a concurrent note", None).unwrap();
    let noted = std::fs::read_to_string(&path).unwrap();
    assert!(
        ops::set_body_if(&corpus, &id, &read, &read)
            .unwrap()
            .is_none()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), noted);
}

/// What a refused edit is kept as: a new owner-only file outside the corpus,
/// never one that replaces an earlier kept edit.
#[test]
fn a_kept_edit_is_a_new_owner_only_file_outside_the_corpus() {
    let dir = Corpus::kept_edits_dir(&process_locations()).unwrap();
    assert_eq!(
        dir,
        support::home().join(".local/state/nebula/edits"),
        "with no XDG_STATE_HOME, under ~/.local/state"
    );
    let first =
        Corpus::keep_edit(&process_locations(), "kept-edit-fixture", "the first text").unwrap();
    let second =
        Corpus::keep_edit(&process_locations(), "kept-edit-fixture", "the second text").unwrap();
    assert_ne!(first, second, "a second edit never replaces the first");
    for (path, text) in [(&first, "the first text"), (&second, "the second text")] {
        assert_eq!(path.parent(), Some(dir.as_path()));
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("kept-edit-fixture-") && name.ends_with(".md"),
            "{name}"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", path.display());
        }
    }
    assert!(matches!(
        Corpus::keep_edit(&process_locations(), "../escape", "text"),
        Err(Error::UnsafeId(_))
    ));
}

/// Standard input is read up to a ceiling and no further: exactly the limit
/// is taken whole, one byte more is a typed refusal naming the limit.
#[test]
fn read_bounded_takes_the_limit_and_refuses_one_byte_more() {
    let at = "a".repeat(16);
    assert_eq!(
        ops::read_bounded(at.as_bytes(), "a capture", 16).unwrap(),
        at
    );
    let over = "a".repeat(17);
    let refused = ops::read_bounded(over.as_bytes(), "a capture", 16).unwrap_err();
    assert!(
        matches!(
            refused,
            Error::InputTooLarge {
                what: "a capture",
                limit: 16
            }
        ),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "input_too_large");
    assert!(refused.to_string().contains("16 bytes"), "{refused}");
    // An endless source is refused after limit + 1 bytes rather than read
    // to the end that never comes.
    let endless = std::io::repeat(b'a');
    assert!(matches!(
        ops::read_bounded(endless, "a body", ops::BODY_INPUT_LIMIT),
        Err(Error::InputTooLarge { .. })
    ));
    assert!(matches!(
        ops::read_bounded(&[0xff, 0xfe][..], "a body", 16),
        Err(Error::StdinNotUtf8 { what: "a body" })
    ));
}

/// Past the bounded wait the writer refuses rather than proceeding, and
/// refuses *before* touching the node: the error is the whole outcome.
#[test]
fn a_write_against_a_held_lock_refuses_and_changes_nothing() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "A node nobody gets to edit", &[]);
    let before = std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap();
    let root = dir.path().join("corpus");

    // Held on another thread, because the lock is re-entrant on the thread
    // that already has it — which is what lets the CLI hold it across a verb
    // and the commit that records it.
    let held = CorpusLock::acquire(&root).unwrap();
    let refused = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let corpus = Corpus::open(&process_locations(), Some(root.clone())).expect("open");
                // The op uses the real bound, so this waits the full five
                // seconds before refusing. That wait is the thing under test.
                ops::tag_add(&corpus, &id, &["never".to_string()])
            })
            .join()
            .expect("the waiting writer did not panic")
    });

    assert!(
        matches!(&refused, Err(Error::Locked { root: r, .. }) if r == &root),
        "got {refused:?}"
    );
    // An unlabelled holder is named as `nebula`, with this process's id.
    assert!(
        matches!(
            &refused,
            Err(Error::Locked { holder: Some(h), .. })
                if h.pid == std::process::id() && h.label == "nebula"
        ),
        "got {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap(),
        before,
        "the refusal came before the write, so the node is untouched"
    );

    // And a reader never waited on any of it.
    assert!(corpus.load(&id).is_ok());
    assert!(Corpus::open(&process_locations(), Some(root.clone())).is_ok());
    drop(held);
}

/// `.lock` planted as a symlink to nowhere: the lock's open followed it,
/// created the file at the far end and took the `flock` there. The open is
/// `O_NOFOLLOW` now, so the writer refuses, naming `.lock`, and creates
/// nothing anywhere.
#[cfg(unix)]
#[test]
fn a_symlinked_lock_file_is_refused_and_creates_nothing_outside() {
    let (dir, corpus) = corpus();
    ops::capture(&corpus, "an earlier thought").unwrap();
    let root = dir.path().join("corpus");
    let lock = root.join(nebula_core::LOCK_FILE);
    let outside = dir.path().join("outside").join("created-by-lock");
    std::fs::create_dir(outside.parent().unwrap()).unwrap();
    std::fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(&outside, &lock).unwrap();
    let inbox = every_file(&root.join("inbox"));

    let refused = ops::capture(&corpus, "must not land");
    assert!(
        not_regular(&refused, &lock, nebula_core::fs::EntryKind::Symlink),
        "{refused:?}"
    );
    assert!(
        !outside.exists(),
        "the lock created a file outside the root"
    );
    assert_eq!(every_file(&root.join("inbox")), inbox);
    assert!(
        std::fs::symlink_metadata(&lock)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // A FIFO there would have held the open forever.
    std::fs::remove_file(&lock).unwrap();
    mkfifo(&lock);
    let refused = within_two_seconds(move || ops::capture(&corpus, "must not land either"));
    assert!(
        not_regular(&refused, &lock, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );
    assert_eq!(every_file(&root.join("inbox")), inbox);
}

/// The lock file is the one thing under the root that is not the corpus, so
/// nothing that reads or records the corpus may see it.
#[test]
fn the_lock_file_is_neither_committed_nor_checked() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));

    // Written and committed under a held lock, so the holder record is in
    // `.lock` for the whole of both.
    let held = corpus
        .lock_as(nebula_core::LOCK_WAIT, "core test")
        .expect("the lock");
    let record = std::fs::read_to_string(root.join(nebula_core::LOCK_FILE)).unwrap();
    assert!(record.contains("\"core test\""), "{record}");
    let id = seed(&corpus, "A node whose write took the lock", &[]);
    committed(ops::commit(&corpus, "new", &[&id]));
    assert!(
        root.join(nebula_core::LOCK_FILE).exists(),
        "the write took the lock, so the file is there to be excluded"
    );

    assert_eq!(committed_paths(&root, "HEAD"), [format!("nodes/{id}.md")]);
    assert!(
        !git(&root, &["ls-files"]).contains(nebula_core::LOCK_FILE),
        "the lock file is not tracked"
    );
    assert!(
        !git(&root, &["status", "--porcelain"]).contains(".lock"),
        "the ignored lock is absent from repository status"
    );
    assert_eq!(git(&root, &["check-ignore", ".lock"]), ".lock\n");

    // Checked with the record still there.
    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert_eq!(report.nodes, 1, "the lock file is not read as a node");
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    drop(held);
    assert_eq!(
        std::fs::read_to_string(root.join(nebula_core::LOCK_FILE)).unwrap(),
        "",
        "the record goes when the lock does"
    );
}
