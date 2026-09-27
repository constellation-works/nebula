//! Citations and handoffs: reference kinds and local paths, and a handoff
//! that cites an Observatory record and closes the node in one write.

use crate::harness::{corpus, seed};
use crate::ops::refuted;
use nebula_core::{
    Citation, Direction, Error, Graph, Handoff, ObservatoryLink, Status, graph, ops,
};

pub(super) fn citing(uri: &str) -> Citation {
    Citation {
        uri: Some(uri.to_string()),
        kind: "study".to_string(),
        note: Some("why".to_string()),
        ..Citation::default()
    }
}

/// Rule 8 at the point of action, as a typed refusal: an absolute path or a
/// `file:` URI is refused even when it exists on this machine, before any
/// resolving, and nothing is written. URLs and scheme handles pass.
#[test]
fn cite_refuses_an_absolute_local_path_as_a_typed_error() {
    let (dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let existing = dir.path().join("corpus").join("config.yaml");
    let existing = existing.to_str().unwrap();
    let path = corpus.node_path(&a).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    for uri in [
        existing.to_string(),
        format!("file://{existing}"),
        "/nonexistent/x.md".to_string(),
        "C:\\studies\\x.md".to_string(),
    ] {
        let refused = ops::cite(&corpus, &a, &citing(&uri));
        assert!(
            matches!(&refused, Err(Error::AbsoluteUri(written)) if *written == uri),
            "{uri}: {refused:?}"
        );
    }
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());

    assert!(matches!(
        ops::cite(&corpus, &a, &citing("./missing.md")),
        Err(Error::UnresolvedUri { .. })
    ));
    for uri in [
        "https://example.org",
        "http://example.org",
        "mailto:someone@example.org",
        "orbit:ORB-13049",
        "neb:another-idea",
        "../config.yaml",
    ] {
        ops::cite(&corpus, &a, &citing(uri)).unwrap_or_else(|e| panic!("{uri}: {e}"));
    }
}

#[test]
fn a_reference_kind_outside_the_vocabulary_is_refused_by_name() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let path = corpus.node_path(&a).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let refused = ops::cite(
        &corpus,
        &a,
        &Citation {
            uri: Some("https://example.org".into()),
            kind: "bogus".into(),
            note: Some("n".into()),
            ..Citation::default()
        },
    );
    assert!(
        matches!(&refused, Err(Error::UnknownReferenceKind(kind)) if kind == "bogus"),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing written"
    );
}

pub(super) fn handing(record: &str) -> Handoff {
    Handoff {
        record: record.to_string(),
        note: Some("the hypothesis this became".to_string()),
        ..Handoff::default()
    }
}

/// An Observatory checkout carrying the one record `H012`, as a file under
/// `hypotheses/` the way research layout v2 files it.
fn observatory_with_h012(at: &std::path::Path) -> std::path::PathBuf {
    let root = at.join("observatory");
    std::fs::create_dir_all(root.join("hypotheses")).unwrap();
    std::fs::write(root.join("hypotheses").join("H012-wake.md"), "# H012\n").unwrap();
    root
}

/// The hand-off is one write carrying both halves: exactly one `observatory`
/// reference, and the node abandoned with the record named as the reason.
/// Trace and show read the hand-off back from those two halves.
#[test]
fn a_handoff_cites_the_record_and_closes_the_node_in_one_write() {
    let (dir, corpus) = corpus();
    let parent = seed(&corpus, "Gravity as scarcity", &[]);
    let id = seed(&corpus, "Scarcity wake", &[&parent]);
    let root = observatory_with_h012(dir.path());

    let done = ops::handoff(&corpus, &id, &handing("h012"), Some(&root)).unwrap();
    assert_eq!(done.from, Status::Seed);
    assert_eq!(done.record, "H012", "case is normalised up, as cite does");
    assert_eq!(done.reference, "r1");
    assert_eq!(
        done.observatory,
        ObservatoryLink {
            reference: "r1".into(),
            record: "H012".into(),
            path: Some(root.join("hypotheses").join("H012-wake.md")),
        },
        "where the record is, as `show` would say"
    );

    let node = corpus.load(&id).unwrap().node;
    assert_eq!(node.status, Status::Abandoned);
    assert_eq!(
        node.closed, done.doc.node.closed,
        "what was returned is on disk"
    );
    let closed = node.closed.as_ref().expect("closed");
    assert_eq!(closed.why, "handed off to H012");
    assert_eq!(node.references.len(), 1, "exactly one reference");
    let reference = &node.references[0];
    assert_eq!(reference.kind, "observatory");
    assert_eq!(reference.uri.as_deref(), Some("H012"));
    assert_eq!(
        reference.note.as_deref(),
        Some("the hypothesis this became")
    );
    assert_eq!(node.handed_off_to(), Some("H012"));

    let docs = corpus.load_all().unwrap();
    let g = Graph::build(&docs).unwrap();
    assert_eq!(
        graph::node(&g, &id).unwrap().handed_off_to.as_deref(),
        Some("H012")
    );
    let walk = graph::trace(&g, &parent, Direction::Down).unwrap().0;
    let handed: Vec<_> = walk
        .iter()
        .map(|n| (n.id.as_str(), n.handed_off_to.as_deref()))
        .collect();
    assert_eq!(
        handed,
        [(parent.as_str(), None), (id.as_str(), Some("H012"))]
    );
}

/// With no observatory root on this machine there is nothing to resolve
/// against, so the id is accepted on its shape alone, as `cite` accepts it.
#[test]
fn a_handoff_with_no_observatory_root_accepts_the_record_id() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "An idea", &[]);
    let done = ops::handoff(&corpus, &id, &handing("H404"), None).unwrap();
    assert_eq!(done.doc.node.handed_off_to(), Some("H404"));
    assert_eq!(
        (done.observatory.record.as_str(), done.observatory.path),
        ("H404", None)
    );
}

/// A citation of an Observatory record says where the record is once the
/// caller asks with a root, as `show` does; other kinds carry no link.
#[test]
fn a_cited_observatory_record_is_located_under_the_root_given() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "An idea", &[]);
    let root = observatory_with_h012(dir.path());
    let args = Citation {
        kind: "observatory".into(),
        uri: Some("H012".into()),
        ..Citation::default()
    };

    let located = ops::cite(&corpus, &id, &args)
        .unwrap()
        .with_observatory(Some(&root))
        .unwrap();
    assert_eq!(
        located.observatory,
        Some(ObservatoryLink {
            reference: "r1".into(),
            record: "H012".into(),
            path: Some(root.join("hypotheses").join("H012-wake.md")),
        })
    );
    let missing = Citation {
        uri: Some("H404".into()),
        ..args.clone()
    };
    let unresolved = ops::cite(&corpus, &id, &missing)
        .unwrap()
        .with_observatory(Some(&root))
        .unwrap();
    assert_eq!(unresolved.observatory.map(|l| l.path), Some(None));
    let paper = Citation {
        kind: "paper".into(),
        uri: Some("https://example.org".into()),
        ..Citation::default()
    };
    let cited = ops::cite(&corpus, &id, &paper)
        .unwrap()
        .with_observatory(Some(&root))
        .unwrap();
    assert_eq!(cited.observatory, None);
}

/// Every refusal comes before the write: the node file is byte for byte
/// what it was.
#[test]
fn a_handoff_refuses_a_closed_node_an_unknown_one_and_an_unresolved_record() {
    let (dir, corpus) = corpus();
    let root = observatory_with_h012(dir.path());
    let unchanged = |id: &str, before: &str, refused: &Result<_, Error>| {
        let after = std::fs::read_to_string(corpus.node_path(id).unwrap()).unwrap();
        assert_eq!(after, before, "nothing written after {refused:?}");
    };

    let dead = refuted(&corpus, "Dead idea");
    let before = std::fs::read_to_string(corpus.node_path(&dead).unwrap()).unwrap();
    let refused = ops::handoff(&corpus, &dead, &handing("H012"), Some(&root)).map(|_| ());
    assert!(
        matches!(&refused, Err(Error::AlreadyClosed { id, status: Status::Refuted }) if *id == dead),
        "{refused:?}"
    );
    unchanged(&dead, &before, &refused);

    let dropped = seed(&corpus, "Dropped idea", &[]);
    ops::set_status(&corpus, &dropped, Status::Abandoned, Some("lost interest")).unwrap();
    let before = std::fs::read_to_string(corpus.node_path(&dropped).unwrap()).unwrap();
    let refused = ops::handoff(&corpus, &dropped, &handing("H012"), Some(&root)).map(|_| ());
    assert!(
        matches!(
            &refused,
            Err(Error::AlreadyClosed {
                status: Status::Abandoned,
                ..
            })
        ),
        "{refused:?}"
    );
    unchanged(&dropped, &before, &refused);

    // Handed off once is closed too: a second hand-off would replace the
    // first one's reason.
    let once = seed(&corpus, "Handed off once", &[]);
    ops::handoff(&corpus, &once, &handing("H012"), Some(&root)).unwrap();
    let before = std::fs::read_to_string(corpus.node_path(&once).unwrap()).unwrap();
    let refused = ops::handoff(&corpus, &once, &handing("H012"), Some(&root)).map(|_| ());
    assert!(
        matches!(refused, Err(Error::AlreadyClosed { .. })),
        "{refused:?}"
    );
    unchanged(&once, &before, &refused);

    let refused = ops::handoff(&corpus, "nope", &handing("H012"), Some(&root)).map(|_| ());
    assert!(
        matches!(&refused, Err(Error::NoSuchNode(id)) if id == "nope"),
        "{refused:?}"
    );

    let open = seed(&corpus, "Still open", &[]);
    let before = std::fs::read_to_string(corpus.node_path(&open).unwrap()).unwrap();
    let refused = ops::handoff(&corpus, &open, &handing("H999"), Some(&root)).map(|_| ());
    assert!(
        matches!(
            &refused,
            Err(Error::UnresolvedObservatoryRecord { record, root: at })
                if record == "H999" && *at == root
        ),
        "{refused:?}"
    );
    unchanged(&open, &before, &refused);

    let refused = ops::handoff(&corpus, &open, &handing("notes/x.md"), None).map(|_| ());
    assert!(
        matches!(&refused, Err(Error::InvalidObservatoryId(_))),
        "{refused:?}"
    );
    unchanged(&open, &before, &refused);
}

/// A hand-off is read from both halves it writes, so a reason that merely
/// names a record, with no reference to it, is not one.
#[test]
fn a_reason_alone_is_not_a_handoff() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "An idea", &[]);
    let changed =
        ops::set_status(&corpus, &id, Status::Abandoned, Some("handed off to H012")).unwrap();
    assert_eq!(changed.doc.node.handed_off_to(), None);

    // The two-verb form, with both halves in place, reads the same as the
    // one-step verb.
    let other = seed(&corpus, "Another idea", &[]);
    ops::cite(
        &corpus,
        &other,
        &Citation {
            uri: Some("H012".into()),
            kind: "observatory".into(),
            note: Some("n".into()),
            ..Citation::default()
        },
    )
    .unwrap();
    let changed = ops::set_status(
        &corpus,
        &other,
        Status::Abandoned,
        Some("handed off to H012"),
    )
    .unwrap();
    assert_eq!(changed.doc.node.handed_off_to(), Some("H012"));
}
