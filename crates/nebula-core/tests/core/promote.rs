//! Promotion ids and interrupted promotions: short ids from long captures,
//! fallbacks, and a pending promotion completed or discarded on the next
//! write.

use crate::harness::{corpus, live_inbox};
use nebula_core::{Corpus, Error, Graph, InboxEntry, NewNode, Promotion, Settlement, ops};
use std::path::Path;

/// The capture from the v0.2 evaluation, whose sentence slug is 60 characters.
const LONG_CAPTURE: &str = "gravity might be a scarcity gradient in some shared resource";

const LONG_CAPTURE_SLUG: &str = "gravity-might-be-a-scarcity-gradient-in-some-shared-resource";

const LONG_CAPTURE_ID: &str = "gravity-scarcity-gradient-shared-resource";

/// A capture promoted with neither a title nor an id keeps the sentence as
/// its title but takes its id from the first five significant words.
#[test]
fn a_long_capture_promoted_as_captured_gets_a_short_id() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let created = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap();

    assert_eq!(created.doc.node.id, LONG_CAPTURE_ID);
    assert!(created.doc.node.id.split('-').count() <= 5);
    assert_eq!(
        created.doc.node.title, LONG_CAPTURE,
        "the title is untouched"
    );
    assert_eq!(created.path, corpus.node_path(LONG_CAPTURE_ID).unwrap());
    assert_eq!(
        corpus.load(LONG_CAPTURE_ID).unwrap().node.title,
        LONG_CAPTURE
    );
    assert!(corpus.inbox().unwrap().0.is_empty(), "the entry is settled");
}

/// A taken short id falls back to a longer id from the same text, never
/// onto the existing node. Once every candidate is taken, the promotion is
/// refused as a duplicate exactly as before, and nothing is written.
#[test]
fn a_taken_capture_id_falls_back_and_never_touches_the_existing_node() {
    let (_dir, corpus) = corpus();
    let existing = ops::new_node(
        &corpus,
        &NewNode {
            title: "An unrelated earlier idea".into(),
            id: Some(LONG_CAPTURE_ID.into()),
            ..NewNode::default()
        },
    )
    .unwrap();
    let before = std::fs::read(&existing.path).unwrap();

    // One more significant word than the bound: the next candidate.
    let longer = format!("{LONG_CAPTURE} pool");
    let entry = ops::capture(&corpus, &longer).unwrap();
    let created = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap();
    assert_eq!(
        created.doc.node.id,
        "gravity-scarcity-gradient-shared-resource-pool"
    );

    // Exactly five significant words: the fallback is the full slug.
    let entry = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let created = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap();
    assert_eq!(created.doc.node.id, LONG_CAPTURE_SLUG);

    // The same sentence again: the short id is still another idea's, but the
    // full slug holds this very thought, so it is refused by that id.
    let entry = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let error = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap_err();
    assert!(
        matches!(&error, Error::NodeExists(id) if id == LONG_CAPTURE_SLUG),
        "{error:?}"
    );
    assert_eq!(
        live_inbox(&corpus, &entry.id).text,
        LONG_CAPTURE,
        "a refused promotion leaves the capture waiting"
    );

    assert_eq!(
        std::fs::read(&existing.path).unwrap(),
        before,
        "an existing node is never rewritten"
    );
    assert_eq!(corpus.load_all().unwrap().len(), 3);
}

/// A thought already promoted is refused, not promoted a second time under
/// a fallback id: the collision names the node that already holds it.
#[test]
fn a_capture_already_promoted_is_refused_rather_than_duplicated() {
    let (_dir, corpus) = corpus();
    let first = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let second = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let created = ops::promote(&corpus, &first.id, &Promotion::default(), 0).unwrap();
    assert_eq!(created.doc.node.id, LONG_CAPTURE_ID);
    let before = std::fs::read(&created.path).unwrap();

    let error = ops::promote(&corpus, &second.id, &Promotion::default(), 0).unwrap_err();
    assert!(
        matches!(&error, Error::NodeExists(id) if id == LONG_CAPTURE_ID),
        "{error:?}"
    );
    assert_eq!(
        live_inbox(&corpus, &second.id).text,
        LONG_CAPTURE,
        "a refused promotion leaves the capture waiting"
    );
    assert_eq!(corpus.load_all().unwrap().len(), 1, "no duplicate node");
    assert_eq!(std::fs::read(&created.path).unwrap(), before);
}

/// The state a crash between a promotion's two renames leaves: the pending
/// record written, the node written, and the inbox line still live. Built
/// from a real promotion whose strike is then put back, and a record in the
/// shape `promote` writes. Returns the entry and the node id.
fn interrupted_promotion(corpus: &Corpus, root: &Path, text: &str) -> (InboxEntry, String) {
    let entry = ops::capture(corpus, text).unwrap();
    let node = ops::promote(corpus, &entry.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let live = format!("- [{}] {} {}", entry.id, entry.at, entry.text);
    let struck = format!("- ~~[{}] {} {}~~ -> {node}", entry.id, entry.at, entry.text);
    let month = std::fs::read_to_string(&entry.file).unwrap();
    assert!(month.contains(&struck), "{month}");
    std::fs::write(&entry.file, month.replace(&struck, &live)).unwrap();
    write_pending(root, &entry, &node);
    (entry, node)
}

/// A pending promotion record, as `promote` writes it before its first write.
fn write_pending(root: &Path, entry: &InboxEntry, node: &str) {
    let record = serde_json::json!({
        "op": "promote",
        "entry": entry.id,
        "stamp": entry.at,
        "node": node,
    });
    std::fs::write(root.join(".pending"), record.to_string()).unwrap();
}

/// The inbox line for `entry`, as it now reads on disk.
fn inbox_line(entry: &InboxEntry) -> String {
    let prefixes = [format!("- [{}] ", entry.id), format!("- ~~[{}] ", entry.id)];
    std::fs::read_to_string(&entry.file)
        .unwrap()
        .lines()
        .find(|line| prefixes.iter().any(|p| line.starts_with(p.as_str())))
        .unwrap_or_else(|| panic!("no line for {}", entry.id))
        .to_string()
}

#[test]
fn a_promotion_interrupted_before_the_strike_completes_on_the_next_write() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    let (entry, node) = interrupted_promotion(&corpus, &root, "an interrupted promotion idea");

    ops::capture(&corpus, "the next thought").unwrap();

    assert!(
        inbox_line(&entry).ends_with(&format!("~~ -> {node}")),
        "{}",
        inbox_line(&entry)
    );
    let ids: Vec<String> = corpus
        .load_all()
        .unwrap()
        .into_iter()
        .map(|d| d.node.id)
        .collect();
    assert_eq!(ids, [node.as_str()], "exactly one node");
    assert!(!root.join(".pending").exists(), "the record is settled");
    assert!(
        corpus.inbox().unwrap().0.iter().all(|e| e.id != entry.id),
        "the entry reads as promoted, not waiting"
    );
}

/// The desktop takes the lock under a label of its own, and that take
/// settles an interrupted write like any other outermost one.
#[test]
fn a_labelled_lock_settles_an_interrupted_promotion_too() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    let (entry, node) = interrupted_promotion(&corpus, &root, "an interrupted promotion idea");

    drop(
        corpus
            .lock_as(nebula_core::LOCK_WAIT, "desktop capture")
            .expect("the lock"),
    );

    assert!(!root.join(".pending").exists(), "the record is settled");
    assert!(
        inbox_line(&entry).ends_with(&format!("~~ -> {node}")),
        "{}",
        inbox_line(&entry)
    );
}

#[test]
fn a_pending_promotion_whose_node_was_never_written_is_discarded() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    let entry = ops::capture(&corpus, "a promotion that never began").unwrap();
    write_pending(&root, &entry, "a-promotion-that-never-began");
    let live = inbox_line(&entry);

    ops::capture(&corpus, "the next thought").unwrap();

    assert_eq!(inbox_line(&entry), live, "the line stays live");
    assert!(!root.join(".pending").exists(), "the record is discarded");
    assert!(
        corpus.load_all().unwrap().is_empty(),
        "no node by guesswork"
    );
    assert_eq!(live_inbox(&corpus, &entry.id).text, entry.text);
    // It is an ordinary waiting capture again, and promotes as one.
    let created = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap();
    assert_eq!(created.doc.node.id, "a-promotion-that-never-began");
}

#[test]
fn an_interrupted_promotion_is_not_listed_as_waiting() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    let waiting = ops::capture(&corpus, "a thought still waiting").unwrap();
    let (entry, node) = interrupted_promotion(&corpus, &root, "an interrupted promotion idea");

    let listed: Vec<String> = corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        listed,
        [waiting.id],
        "only the capture that is really waiting"
    );
    assert!(
        inbox_line(&entry).starts_with("- ["),
        "no write has run since"
    );

    // Asking to promote it again is the next write: it finishes the
    // promotion, then says what became of the entry.
    let refused = ops::promote(&corpus, &entry.id, &Promotion::default(), 0);
    assert!(
        matches!(
            &refused,
            Err(Error::InboxEntrySettled { settlement: Settlement::Promoted(n), .. }) if *n == node
        ),
        "{refused:?}"
    );
    assert!(!root.join(".pending").exists());
}

#[test]
fn check_names_a_pending_promotion_and_refuses_one_it_cannot_read() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    let (entry, node) = interrupted_promotion(&corpus, &root, "an interrupted promotion idea");
    let report = |corpus: &Corpus| {
        let docs = corpus.load_all().unwrap();
        nebula_core::check::run(&Graph::build(&docs).unwrap(), corpus).unwrap()
    };

    let findings: Vec<_> = report(&corpus)
        .findings
        .into_iter()
        .filter(|f| f.rule == 17)
        .collect();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].level, nebula_core::Severity::Warn);
    for part in [entry.id.as_str(), node.as_str(), ".pending"] {
        assert!(
            findings[0].message.contains(part),
            "{}",
            findings[0].message
        );
    }
    assert!(root.join(".pending").exists(), "check never settles it");

    // A record this build cannot read keeps every writer out, and the read
    // that would present its entry too, until a person has looked at it.
    let garbled = r#"{"op":"promote","entry":"#;
    std::fs::write(root.join(".pending"), garbled).unwrap();
    let unknown = r#"{"op":"merge","nodes":["a","b"]}"#;
    for record in [garbled, unknown] {
        std::fs::write(root.join(".pending"), record).unwrap();
        let refused = ops::capture(&corpus, "a thought that must wait");
        assert!(
            matches!(&refused, Err(Error::PendingWriteUnreadable { path, .. }) if *path == root.join(".pending")),
            "{refused:?}"
        );
        assert!(matches!(
            corpus.inbox(),
            Err(Error::PendingWriteUnreadable { .. })
        ));
        let findings = report(&corpus).findings;
        let unreadable = findings.iter().find(|f| f.rule == 17).unwrap();
        assert_eq!(unreadable.level, nebula_core::Severity::Error);
        assert!(
            unreadable.message.contains("rm '"),
            "{}",
            unreadable.message
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".pending")).unwrap(),
            record,
            "nothing deleted or rewrote it"
        );
    }
    assert!(inbox_line(&entry).starts_with("- ["), "no write ran");

    // A field a later build adds is not a reason to refuse.
    std::fs::remove_file(root.join(".pending")).unwrap();
    write_pending(&root, &entry, &node);
    let raw = std::fs::read_to_string(root.join(".pending")).unwrap();
    std::fs::write(
        root.join(".pending"),
        raw.replacen('{', r#"{"future_field":1,"#, 1),
    )
    .unwrap();
    ops::capture(&corpus, "the next thought").unwrap();
    assert!(inbox_line(&entry).ends_with(&format!("-> {node}")));
}

/// Every candidate held by some other idea: refused by the full slug, and
/// nothing is written.
#[test]
fn a_capture_whose_every_candidate_is_another_idea_is_refused() {
    let (_dir, corpus) = corpus();
    for (title, id) in [
        ("One unrelated idea", LONG_CAPTURE_ID),
        ("Another unrelated idea", LONG_CAPTURE_SLUG),
    ] {
        ops::new_node(
            &corpus,
            &NewNode {
                title: title.into(),
                id: Some(id.into()),
                ..NewNode::default()
            },
        )
        .unwrap();
    }
    let entry = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let error = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap_err();
    assert!(
        matches!(&error, Error::NodeExists(id) if id == LONG_CAPTURE_SLUG),
        "{error:?}"
    );
    assert_eq!(live_inbox(&corpus, &entry.id).id, entry.id);
    assert_eq!(corpus.load_all().unwrap().len(), 2);
}

/// Only the id minted from a raw capture is shortened: a title still
/// slugifies whole, and an explicit id is taken as given.
#[test]
fn an_explicit_title_or_id_decides_the_promoted_id_as_before() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a thought to retitle").unwrap();
    let created = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            title: Some(LONG_CAPTURE.into()),
            ..Promotion::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(created.doc.node.id, LONG_CAPTURE_SLUG);

    let entry = ops::capture(&corpus, LONG_CAPTURE).unwrap();
    let created = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            id: Some("gravity-as-scarcity".into()),
            ..Promotion::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(created.doc.node.id, "gravity-as-scarcity");
    assert_eq!(created.doc.node.title, LONG_CAPTURE);

    // A capture of five words or fewer keeps the slug it always had.
    let entry = ops::capture(&corpus, "the human's own words").unwrap();
    let created = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap();
    assert_eq!(created.doc.node.id, "the-human-s-own-words");
}

#[test]
fn a_capture_with_no_usable_id_is_refused_as_before() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "→ … !!").unwrap();
    let error = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap_err();
    assert!(matches!(error, Error::UnusableTitle(_)), "{error:?}");
}
