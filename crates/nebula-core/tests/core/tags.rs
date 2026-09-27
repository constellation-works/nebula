//! Tags and citation kinds: drift between near variants of a tag, and
//! lowercased reference kinds.

use crate::harness::{corpus, seed};
use nebula_core::{Citation, Corpus, Error, Graph, ops};

fn tag(corpus: &Corpus, id: &str, tags: &[&str]) {
    let tags: Vec<String> = tags.iter().map(ToString::to_string).collect();
    ops::tag_add(corpus, id, &tags).expect("tag");
}

fn close(corpus: &Corpus, id: &str, tags: &[&str]) -> Vec<(String, String, usize)> {
    let tags: Vec<String> = tags.iter().map(ToString::to_string).collect();
    ops::close_tags(corpus, id, &tags)
        .expect("close tags")
        .into_iter()
        .map(|c| (c.tag, c.near, c.nodes))
        .collect()
}

/// A tag this write introduced is compared with the tags already in use,
/// and the existing variant comes back with how many nodes carry it. A tag
/// someone else already carries is not new, so the drift it shares is
/// `check`'s to report, not this write's.
#[test]
fn a_tag_new_to_the_corpus_is_reported_close_to_a_variant_in_use() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let b = seed(&corpus, "B", &[]);
    let c = seed(&corpus, "C", &[]);
    tag(&corpus, &a, &["physics", "orrery"]);
    tag(&corpus, &b, &["physics"]);

    tag(&corpus, &c, &["physic", "orrery", "design"]);
    assert_eq!(
        close(&corpus, &c, &["Physic", "orrery", "design"]),
        [("physic".to_string(), "physics".to_string(), 2)],
        "only the new tag is compared, and it is normalised first"
    );
    // The plural the other way round, and on a node that is its only carrier.
    assert_eq!(
        close(&corpus, &a, &["orrerys"]),
        [("orrerys".to_string(), "orrery".to_string(), 2)]
    );
    // `physic` now has a carrier other than `a`, so adding it there
    // introduces nothing.
    tag(&corpus, &a, &["physic"]);
    assert_eq!(close(&corpus, &a, &["physic"]), []);
    assert_eq!(close(&corpus, &c, &["design"]), []);
    assert_eq!(close(&corpus, &c, &[]), []);

    // A case variant can only be a hand edit, and is caught the same way.
    let path = corpus.node_path(&b).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, raw.replace("- physics", "- Sim")).unwrap();
    let d = seed(&corpus, "D", &[]);
    tag(&corpus, &d, &["sim"]);
    assert_eq!(
        close(&corpus, &d, &["sim"]),
        [("sim".to_string(), "Sim".to_string(), 1)]
    );
}

/// Rule 11 names the nodes carrying each variant, since those are the files
/// someone has to retag.
#[test]
fn tag_drift_findings_name_the_nodes_carrying_each_variant() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let b = seed(&corpus, "B", &[]);
    let c = seed(&corpus, "C", &[]);
    tag(&corpus, &a, &["physics"]);
    tag(&corpus, &b, &["physics"]);
    tag(&corpus, &c, &["physic"]);

    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    let drift: Vec<_> = report.findings.iter().filter(|f| f.rule == 11).collect();
    assert_eq!(drift.len(), 1, "{:?}", report.findings);
    assert_eq!(drift[0].node, None, "drift belongs to the corpus");
    assert_eq!(
        drift[0].message,
        format!("tags `physic` (on {c}) and `physics` (on {a}, {b}) differ only by a trailing `s`")
    );
}

fn citation(kind: &str, uri: &str) -> Citation {
    Citation {
        kind: kind.to_string(),
        uri: Some(uri.to_string()),
        note: Some("context".to_string()),
        ..Citation::default()
    }
}

/// A kind is a label like a tag: its case is not a second value. Anything
/// still outside the vocabulary after lowercasing is refused, and writes
/// nothing.
#[test]
fn cite_lowercases_the_kind_and_still_refuses_one_outside_the_vocabulary() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Cited", &[]);

    for (given, stored) in [("Paper", "paper"), (" BOOK ", "book"), ("paper", "paper")] {
        let cited = ops::cite(&corpus, &id, &citation(given, "https://example.org")).unwrap();
        let reference = cited
            .doc
            .node
            .references
            .iter()
            .find(|r| r.id == cited.reference)
            .unwrap();
        assert_eq!(reference.kind, stored, "`{given}`");
    }
    // Normalised before the kind decides anything else: an Observatory
    // record id is still shape-checked, and a discussion still needs no URI.
    let cited = ops::cite(&corpus, &id, &citation("Observatory", "q002")).unwrap();
    let observatory = cited.doc.node.references.last().unwrap();
    assert_eq!(
        (observatory.kind.as_str(), observatory.uri.as_deref()),
        ("observatory", Some("Q002"))
    );
    assert_eq!(
        cited
            .observatory
            .as_ref()
            .map(|l| (l.record.as_str(), &l.path)),
        Some(("Q002", &None)),
        "unlocated until a caller asks"
    );
    assert!(matches!(
        ops::cite(&corpus, &id, &citation("OBSERVATORY", "not-an-id")),
        Err(Error::InvalidObservatoryId(record)) if record == "NOT-AN-ID"
    ));
    ops::cite(
        &corpus,
        &id,
        &Citation {
            uri: None,
            ..citation("Discussion", "")
        },
    )
    .unwrap();

    let before = std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap();
    for kind in ["Bogus", "VERDICT", "", "not a kind"] {
        assert!(
            matches!(
                ops::cite(&corpus, &id, &citation(kind, "https://example.org")),
                Err(Error::UnknownReferenceKind(given)) if given == kind
            ),
            "`{kind}` is refused as given"
        );
    }
    assert_eq!(
        before,
        std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap(),
        "a refused kind must not change the node"
    );
}
