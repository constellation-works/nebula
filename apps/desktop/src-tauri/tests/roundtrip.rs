//! `capture` followed by `inbox`, through a temporary corpus, using the same
//! `session` functions the commands call. The line written is the one
//! `neb capture` writes, so `neb inbox` lists it too.

use nebula_core::Corpus;
use nebula_desktop::session;

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
