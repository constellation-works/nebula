//! Node ids: the stored id and file name must agree before any write, and
//! aliases, symlinks, FIFOs, devices and traversal are refused.

use crate::harness::{corpus, mkfifo, not_regular, process_locations, seed, within_two_seconds};
use nebula_core::{Corpus, Error, Graph, NewNode, ops};

/// Rewrite the id a node file stores, the way a hand edit would.
fn rewrite_stored_id(path: &std::path::Path, to: &str) {
    let raw = std::fs::read_to_string(path).expect("node file");
    let (first, rest) = raw.split_once('\n').expect("frontmatter");
    assert!(first == "---", "a node file starts with `---`");
    let rewritten = rest
        .lines()
        .map(|line| {
            if line.starts_with("id: ") {
                format!("id: {to}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("---\n{rewritten}\n")).expect("rewrite");
}

/// A stored id that is not one file name never becomes a `Doc`, so no verb
/// downstream can derive a destination from it. The file that proved this
/// necessary said `id: ../../escaped` and a `note` wrote `escaped.md` beside
/// the corpus root.
#[test]
fn a_stored_id_that_leaves_nodes_fails_to_load_and_writes_nothing() {
    for escape in ["../../escaped", "../escaped", "nodes/elsewhere", ".."] {
        let (dir, corpus) = corpus();
        let id = seed(&corpus, "Safe", &[]);
        let path = corpus.node_path(&id).unwrap();
        rewrite_stored_id(&path, escape);

        let refused = ops::note(&corpus, &id, "a fixture note", None);
        assert!(
            matches!(&refused, Err(Error::IdMismatch { path: p, id: stored }) if p == &path && stored == escape),
            "`{escape}` got {refused:?}"
        );
        assert!(corpus.load(&id).is_err(), "`{escape}` still loads");
        assert!(corpus.load_all().is_err(), "`{escape}` survives a scan");
        let outside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        assert_eq!(outside, ["corpus"], "`{escape}` wrote beside the corpus");
    }
}

/// An absolute id is the same escape spelled differently, and is refused the
/// same way rather than replacing the join wholesale.
#[test]
fn a_stored_id_that_is_an_absolute_path_fails_to_load() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "Safe", &[]);
    let elsewhere = dir.path().join("escaped");
    rewrite_stored_id(
        &corpus.node_path(&id).unwrap(),
        &elsewhere.display().to_string(),
    );

    assert!(ops::note(&corpus, &id, "a fixture note", None).is_err());
    assert!(!elsewhere.with_extension("md").exists(), "wrote outside");
}

/// The quieter half of the same bug: a stored id that *is* a valid id, but
/// another node's. `save` writes where the id says, so loading this file and
/// noting on it overwrote the node it named.
#[test]
fn a_node_file_that_claims_another_nodes_id_is_refused_before_the_write() {
    let (_dir, corpus) = corpus();
    let safe = seed(&corpus, "Safe", &[]);
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let before = std::fs::read_to_string(&victim_path).unwrap();
    rewrite_stored_id(&corpus.node_path(&safe).unwrap(), &victim);

    let refused = ops::note(&corpus, &safe, "a fixture note", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &corpus.node_path(&safe).unwrap() && id == &victim),
        "got {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        before,
        "the other node is untouched"
    );
    // A scan finds nodes by path, so it refuses from the other direction
    // rather than answering every query from the wrong file.
    assert!(matches!(
        corpus.load_all(),
        Err(Error::UnreadableNodes { count: 1, .. })
    ));
    assert!(
        corpus
            .scan()
            .unwrap()
            .unreadable
            .iter()
            .any(|entry| entry.code == "id_mismatch")
    );
}

/// The same bug without the hand edit. Copying a node file leaves two files
/// whose text is equal down to the byte, and agreement was once decided by
/// comparing that text — so `nodes/safe.md`, a copy of `nodes/victim.md`,
/// vouched for the `victim` it stored and `note safe` appended to the other
/// node. Equal contents are not one file, and never were the question.
#[test]
fn a_copy_of_another_node_file_does_not_authorize_the_id_it_stores() {
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let safe_path = corpus.node_path("safe").unwrap();
    std::fs::copy(&victim_path, &safe_path).expect("copy");
    let before = std::fs::read_to_string(&victim_path).unwrap();
    assert_eq!(
        std::fs::read_to_string(&safe_path).unwrap(),
        before,
        "the fixture is a byte-for-byte copy"
    );

    let refused = ops::note(&corpus, "safe", "a fixture note", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &safe_path && id == &victim),
        "got {refused:?}"
    );
    // Both doors: the id names the copy, and a scan finds it by path.
    assert!(
        matches!(corpus.load("safe"), Err(Error::IdMismatch { .. })),
        "a read answered from the wrong file"
    );
    assert!(
        matches!(
            corpus.load_all(),
            Err(Error::UnreadableNodes { count: 1, .. })
        ),
        "a scan read it as the node it claims"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        before,
        "the node the copy named is untouched"
    );
    assert_eq!(
        std::fs::read_to_string(&safe_path).unwrap(),
        before,
        "and so is the file that was asked for"
    );
}

/// One file under two names in `nodes/`. The names once opened the same inode
/// and so agreed, but the write that followed replaced `victim.md` through a
/// rename: `safe.md` kept the old bytes under a name that no longer matched
/// anything, and the next load of either refused. The alias is refused before
/// the write, whichever name the node is reached through.
#[cfg(unix)]
#[test]
fn a_hard_linked_alias_is_refused_before_a_write_can_split_it() {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &std::path::Path| {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let safe_path = corpus.node_path("safe").unwrap();
    std::fs::hard_link(&victim_path, &safe_path).expect("hard link");
    let before = std::fs::read_to_string(&victim_path).unwrap();
    let linked = identity(&victim_path);

    let names_the_alias = |refused: &nebula_core::Result<_>| matches!(refused, Err(Error::IdMismatch { path, id }) if path == &safe_path && id == &victim);
    let through_alias = ops::note(&corpus, "safe", "ALIAS-NOTE", None);
    assert!(names_the_alias(&through_alias), "got {through_alias:?}");
    let through_own_name = ops::note(&corpus, &victim, "OWN-NAME-NOTE", None);
    assert!(
        names_the_alias(&through_own_name),
        "got {through_own_name:?}"
    );
    let scanned = corpus.load_all();
    assert!(
        matches!(&scanned, Err(Error::UnreadableNodes { count: 2, details }) if details.contains(&safe_path.display().to_string()) && details.contains(&victim)),
        "got {scanned:?}"
    );
    assert!(matches!(corpus.load("safe"), Err(Error::IdMismatch { .. })));
    assert!(matches!(
        corpus.load(&victim),
        Err(Error::IdMismatch { .. })
    ));

    assert_eq!(identity(&victim_path), linked, "no write replaced the node");
    assert_eq!(identity(&safe_path), linked, "the pair was not split");
    assert_eq!(std::fs::read_to_string(&victim_path).unwrap(), before);
}

/// A symlink alias stays linked across a write, but a scan read the node
/// twice and refused a duplicate id while `load` through the alias succeeded.
/// Both doors now refuse the alias itself, as a mismatch, before any write.
#[cfg(unix)]
#[test]
fn a_symlinked_alias_is_refused_by_load_and_by_a_scan_alike() {
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let alias_path = corpus.node_path("alias").unwrap();
    std::os::unix::fs::symlink(format!("{victim}.md"), &alias_path).expect("symlink");
    let before = std::fs::read_to_string(&victim_path).unwrap();

    let refused = ops::note(&corpus, "alias", "SYM-NOTE", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &alias_path && id == &victim),
        "got {refused:?}"
    );
    let scanned = corpus.load_all();
    assert!(
        matches!(&scanned, Err(Error::UnreadableNodes { count: 1, details }) if details.contains(&alias_path.display().to_string()) && details.contains(&victim)),
        "a scan refused something other than the alias: {scanned:?}"
    );
    assert_eq!(std::fs::read_to_string(&victim_path).unwrap(), before);
    assert_eq!(corpus.load(&victim).unwrap().node.id, victim);
}

/// A node file that is a symlink to a valid node outside the root was read
/// as a node, and a write through it replaced the link. Every door now
/// refuses it by name, and the file outside is never read or changed.
#[cfg(unix)]
#[test]
fn a_node_file_symlinked_outside_the_corpus_is_refused_by_load_scan_and_check() {
    use nebula_core::fs::EntryKind;
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "Outside node", &[]);
    let node = corpus.node_path(&id).unwrap();
    let outside = dir.path().join("outside.md");
    std::fs::rename(&node, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &node).unwrap();
    let before = std::fs::read(&outside).unwrap();

    assert!(not_regular(&corpus.load(&id), &node, EntryKind::Symlink));
    assert!(matches!(
        corpus.load_all(),
        Err(Error::UnreadableNodes { count: 1, .. })
    ));
    let scan = corpus.scan().unwrap();
    assert_eq!(scan.unreadable[0].path, node);
    assert_eq!(scan.unreadable[0].code, "not_regular_file");
    let checked = nebula_core::check::run_scanned(&scan, &corpus).unwrap();
    assert_eq!(checked.unreadable.len(), 1);
    let noted = ops::note(&corpus, &id, "a fixture note", None);
    assert!(not_regular(&noted, &node, EntryKind::Symlink), "{noted:?}");

    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&node)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );

    // A link to nowhere is a symlink too, not a missing node, and creating
    // a node over it refuses rather than replacing it.
    std::fs::remove_file(&outside).unwrap();
    assert!(not_regular(&corpus.load(&id), &node, EntryKind::Symlink));
    let created = ops::new_node(
        &corpus,
        &NewNode {
            title: "Outside node".into(),
            ..NewNode::default()
        },
    );
    assert!(
        not_regular(&created, &node, EntryKind::Symlink),
        "{created:?}"
    );
    assert!(!outside.exists(), "nothing was created through the link");
}

/// A FIFO named like a node hung every scan: a read of it waits for a
/// writer that never comes. It is refused without being opened.
#[cfg(unix)]
#[test]
fn a_fifo_under_nodes_is_refused_without_blocking() {
    let (_dir, corpus) = corpus();
    seed(&corpus, "Healthy", &[]);
    let fifo = corpus.node_path("fifo").unwrap();
    mkfifo(&fifo);

    let scanned = within_two_seconds({
        let corpus = corpus.clone();
        move || corpus.load_all()
    });
    assert!(
        matches!(&scanned, Err(Error::UnreadableNodes { count: 1, details }) if details.contains(&fifo.display().to_string()) && details.contains("FIFO")),
        "{scanned:?}"
    );
    let loaded = within_two_seconds(move || corpus.load("fifo"));
    assert!(
        not_regular(&loaded, &fifo, nebula_core::fs::EntryKind::Fifo),
        "{loaded:?}"
    );
}

/// `nodes/zero.md -> /dev/zero` read until memory ran out. The link is
/// refused before anything is read through it.
#[cfg(unix)]
#[test]
fn a_device_symlink_under_nodes_is_refused_without_reading() {
    let (_dir, corpus) = corpus();
    let zero = corpus.node_path("zero").unwrap();
    std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();

    let scanned = within_two_seconds({
        let corpus = corpus.clone();
        move || corpus.load_all()
    });
    assert!(
        matches!(&scanned, Err(Error::UnreadableNodes { count: 1, details }) if details.contains(&zero.display().to_string()) && details.contains("symlink")),
        "{scanned:?}"
    );
    let loaded = within_two_seconds(move || corpus.load("zero"));
    assert!(
        not_regular(&loaded, &zero, nebula_core::fs::EntryKind::Symlink),
        "{loaded:?}"
    );
}

/// Only a name a scan would read is an alias. A hard link from outside
/// `nodes/`, as a `cp -al` backup makes, leaves the node one entry there.
#[cfg(unix)]
#[test]
fn a_hard_link_outside_the_nodes_directory_does_not_refuse_the_node() {
    let (dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    std::fs::hard_link(
        corpus.node_path(&victim).unwrap(),
        dir.path().join("backup.md"),
    )
    .expect("hard link");

    ops::note(&corpus, &victim, "a fixture note", None).expect("note");
    assert_eq!(corpus.load_all().expect("scan").len(), 1);
}

/// The one reason two spellings may name one node, pinned to the case it was
/// written for. A volume may store a file name in a different Unicode
/// normalization than the id the file stores, and a scan reading that name
/// back must still recognize the node rather than refuse it. macOS does this
/// and Linux does not, so the case is *detected* rather than assumed: where
/// the volume answers both spellings with one file, the node still loads.
#[test]
fn a_file_name_stored_in_another_normalization_still_loads_and_scans() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Ünïcode título → ok", &[]);
    assert_eq!(id, "ünïcode-título-ok");
    let composed = corpus.node_path(&id).unwrap();
    // The same word decomposed: each accent written as a combining mark.
    let decomposed = composed.with_file_name("u\u{308}ni\u{308}code-ti\u{301}tulo-ok.md");
    std::fs::rename(&composed, &decomposed).expect("rename to the decomposed spelling");

    if !composed.exists() {
        // A byte-exact volume: the two spellings are two names, the node the
        // id points at is simply gone, and there is nothing to agree with.
        assert!(matches!(corpus.load(&id), Err(Error::NoSuchNode(_))));
        return;
    }

    assert_eq!(
        corpus
            .load(&id)
            .expect("the id still opens the node")
            .node
            .id,
        id,
        "the stored id is the composed spelling the volume did not keep"
    );
    let docs = corpus
        .load_all()
        .expect("a scan accepts the stored spelling");
    assert_eq!(docs.len(), 1, "one file, read once");
    assert_eq!(docs[0].node.id, id);
    ops::note(&corpus, &id, "a fixture note", None).expect("and a write still lands");
}

/// A caller-supplied id is a path the moment it is used, whether it came from
/// a terminal, the desktop, or an agent.
#[test]
fn a_caller_supplied_id_that_is_not_one_file_name_is_refused() {
    let (dir, corpus) = corpus();
    seed(&corpus, "Safe", &[]);
    // A real node file outside the corpus: the refusal must not depend on
    // there being nothing to find.
    let secret = dir.path().join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(
        secret.join("leak.md"),
        "---\nid: leak\ntitle: fixture node outside the corpus\nstatus: seed\n\
         created: 2026-09-22\nupdated: 2026-09-22\n---\n\nfixture body\n",
    )
    .unwrap();
    let absolute = secret.join("leak").display().to_string();

    for id in ["../../secret/leak", "../secret/leak", &absolute, "..", ""] {
        assert!(
            matches!(corpus.node_path(id), Err(Error::UnsafeId(_))),
            "node_path accepted `{id}`"
        );
        for refused in [
            ops::note(&corpus, id, "a fixture note", None).err(),
            ops::sharpen(&corpus, id, "a fixture kill", None).err(),
            ops::tag_add(&corpus, id, &["fixture".to_string()]).err(),
            corpus.load(id).err(),
            corpus.history(id).err(),
        ] {
            assert!(
                matches!(refused, Some(Error::UnsafeId(_) | Error::NotGitWorkTree(_))),
                "`{id}` got {refused:?}"
            );
        }
    }
    assert!(
        !corpus.node_path("leak").unwrap().exists(),
        "nothing outside the root was read into the corpus"
    );
}

/// The rule is about path structure, not about the alphabet: a Unicode id
/// still captures, loads, saves and checks. macOS stores a file name in a
/// normalization of its own choosing, so this is also where a scan that
/// compared names byte for byte would fail on one platform and pass on the
/// other.
#[test]
fn a_unicode_id_still_round_trips_through_a_write() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Ünïcode título → ok", &[]);
    assert_eq!(id, "ünïcode-título-ok");
    let korean = seed(&corpus, "시간은 프레임의 수다", &[]);

    ops::note(&corpus, &id, "a fixture note", None).expect("note");
    ops::note(&corpus, &korean, "a fixture note", None).expect("note");
    assert_eq!(corpus.load(&id).unwrap().node.id, id);
    let docs = corpus.load_all().expect("a scan reads both back");
    assert_eq!(docs.len(), 2);
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

/// macOS temporary directories sit under a symlink, so a corpus reached
/// through one is the normal case there rather than an exotic one. Nothing
/// in the id rules canonicalizes, so the path is used as given and the same
/// verbs work.
#[cfg(unix)]
#[test]
fn a_corpus_reached_through_a_symlinked_root_still_writes_and_loads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let real = dir.path().join("real");
    let corpus = Corpus::init(&process_locations(), &real).expect("init");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    let linked =
        Corpus::open(&process_locations(), Some(link.clone())).expect("open through the link");
    let id = seed(&linked, "Reached through a link", &[]);
    ops::note(&linked, &id, "a fixture note", None).expect("note");

    assert_eq!(
        linked.node_path(&id).unwrap(),
        link.join("nodes").join(format!("{id}.md")),
        "the path is the one given, not a resolved one"
    );
    assert!(
        corpus.load(&id).is_ok(),
        "the same node through the real path"
    );
    assert_eq!(linked.load_all().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn a_symlinked_nodes_directory_refuses_open_init_reads_and_writes() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "Victim", &[]);
    let root = dir.path().join("corpus");
    let nodes = root.join("nodes");
    let outside = dir.path().join("outside-nodes");
    std::fs::rename(&nodes, &outside).unwrap();
    let outside_node = outside.join(format!("{id}.md"));
    let before = std::fs::read(&outside_node).unwrap();
    std::os::unix::fs::symlink(&outside, &nodes).unwrap();

    for result in [
        Corpus::open(&process_locations(), Some(root.clone())),
        Corpus::init(&process_locations(), &root),
        Corpus::open_or_init(&process_locations(), Some(root.clone())).map(|(corpus, _)| corpus),
    ] {
        assert!(matches!(result, Err(Error::NodesSymlink(path)) if path == nodes));
    }
    assert!(matches!(corpus.load(&id), Err(Error::NodesSymlink(path)) if path == nodes));
    assert!(matches!(corpus.load_all(), Err(Error::NodesSymlink(path)) if path == nodes));
    assert!(
        matches!(ops::note(&corpus, &id, "escape", None), Err(Error::NodesSymlink(path)) if path == nodes)
    );
    assert!(
        matches!(ops::new_node(&corpus, &NewNode { title: "New".into(), ..NewNode::default() }), Err(Error::NodesSymlink(path)) if path == nodes)
    );
    assert!(
        matches!(nebula_core::migrate::run(&process_locations(), Some(root)), Err(Error::NodesSymlink(path)) if path == nodes)
    );

    assert_eq!(std::fs::read(&outside_node).unwrap(), before);
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    assert!(
        std::fs::symlink_metadata(&nodes)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn init_refuses_a_dangling_nodes_symlink_without_creating_a_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir(&root).unwrap();
    let nodes = root.join("nodes");
    std::os::unix::fs::symlink(dir.path().join("missing"), &nodes).unwrap();

    assert!(
        matches!(Corpus::init(&process_locations(), &root), Err(Error::NodesSymlink(path)) if path == nodes)
    );
    assert!(
        std::fs::symlink_metadata(&nodes)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!root.join("config.yaml").exists());
    assert!(!root.join("inbox").exists());
}
