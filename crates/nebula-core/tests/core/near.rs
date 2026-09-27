//! Lexical neighbours: ranking, bands, links already made, the cap, and the
//! suggestions capture and promote make without linking.

use crate::harness::{corpus, process_locations, seed};
use nebula_core::{
    Band, Corpus, EdgeType, Error, Graph, NEAR_DEFAULT, NewNode, Promotion, graph, ops,
};

pub(super) fn lexical_fixture(corpus: &Corpus) {
    for (title, tags, kill) in [
        (
            "Tags beat domains",
            vec!["design", "corpus"],
            Some("a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer"),
        ),
        ("A single global taxonomy", vec!["design"], None),
        ("Ranking decay half-life", vec!["search"], None),
        ("Proper time is a count", vec!["physics"], None),
    ] {
        ops::new_node(
            corpus,
            &NewNode {
                title: title.to_string(),
                tags: tags.into_iter().map(String::from).collect(),
                kill: kill.map(String::from),
                ..NewNode::default()
            },
        )
        .unwrap();
    }
    // Body text counts too: the ranking node argues in words a query can hit.
    ops::note(
        corpus,
        "ranking-decay-half-life",
        "a search result should lose rank as it ages, on a half-life",
        None,
    )
    .unwrap();
}

#[test]
fn near_ranks_by_shared_vocabulary_best_first() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags and domains beat a taxonomy", NEAR_DEFAULT).unwrap();
    let ids: Vec<&str> = near.0.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        ["tags-beat-domains", "a-single-global-taxonomy"],
        "the two nodes sharing words, the one sharing more first; \
         the search and physics nodes share none and are left out"
    );
    assert!(
        near.0.windows(2).all(|w| w[0].score >= w[1].score),
        "best first: {near:?}"
    );
    assert!(
        near.0.iter().all(|n| n.score > 0.0 && n.score <= 1.0),
        "scores sit in 0..=1: {near:?}"
    );
    assert_eq!(
        near.0[0].tags,
        ["design", "corpus"],
        "a neighbour carries its tags, since a promotion reuses the parent's"
    );

    // Body text is read, not just the title: only the note mentions ageing.
    let near = graph::near(&graph, "results that age", NEAR_DEFAULT).unwrap();
    assert_eq!(
        near.0.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );
}

#[test]
fn near_takes_a_node_id_and_leaves_that_node_out() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags-beat-domains", NEAR_DEFAULT).unwrap();
    let ids: Vec<&str> = near.0.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !ids.contains(&"tags-beat-domains"),
        "a node is not its own neighbour: {ids:?}"
    );
    assert_eq!(
        ids.first(),
        Some(&"a-single-global-taxonomy"),
        "the other design node shares the `design` tag; nothing else does: {ids:?}"
    );
}

#[test]
fn near_bands_every_score_and_a_word_for_word_copy_reads_strong() {
    // The cut-offs, at and either side of each.
    for (score, band) in [
        (1.0, Band::Strong),
        (graph::STRONG_FROM, Band::Strong),
        (0.249, Band::Some),
        (graph::SOME_FROM, Band::Some),
        (0.069, Band::Weak),
        (0.0, Band::Weak),
    ] {
        assert_eq!(Band::of(score), band, "{score}");
    }

    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    // The case the raw score misleads on: a copy of a node, word for word,
    // scores well under `1` against it.
    let original = corpus.load("ranking-decay-half-life").unwrap();
    ops::new_node(
        &corpus,
        &NewNode {
            title: original.node.title.clone(),
            body: original.body.clone(),
            tags: original.node.tags.clone(),
            id: Some("ranking-decay-again".into()),
            ..NewNode::default()
        },
    )
    .unwrap();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "ranking-decay-again", NEAR_DEFAULT).unwrap();
    let copy = &near.0[0];
    assert_eq!(copy.id, "ranking-decay-half-life", "{near:?}");
    assert!(copy.score < 0.7, "a copy is nowhere near 1: {near:?}");
    assert_eq!(copy.band, Band::Strong, "but it reads as strong: {near:?}");

    for query in ["ranking-decay-again", "tags taxonomy ranking", "a search"] {
        let near = graph::near(&graph, query, NEAR_DEFAULT).unwrap();
        assert!(
            near.0.iter().all(|n| n.band == Band::of(n.score)),
            "the band is the score's, as rounded: {near:?}"
        );
    }
}

/// One edge a neighbour is linked by, as `(from, kind, to)`.
type Link = (String, EdgeType, String);

/// `near <node>` names the edges a neighbour already has to that node, so
/// a parent in the answer is not mistaken for a link still to make.
#[test]
fn near_marks_neighbours_already_linked_to_the_node() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let parent = "tags-beat-domains";
    let child = seed(&corpus, "Tags beat domains for corpus design", &[parent]);
    // A second kind to the same parent: one neighbour, both kinds.
    ops::link(&corpus, &child, EdgeType::Refines, parent, None).unwrap();
    // Saved on both ends, so it is found from either.
    ops::link(
        &corpus,
        parent,
        EdgeType::Contradicts,
        "a-single-global-taxonomy",
        None,
    )
    .unwrap();
    let before = corpus.load_all().unwrap();
    let graph = Graph::build(&before).unwrap();

    let links = |query: &str| -> Vec<(String, Option<Vec<Link>>)> {
        graph::near(&graph, query, 10)
            .unwrap()
            .0
            .into_iter()
            .map(|n| {
                let linked = n.linked.map(|es| {
                    es.into_iter()
                        .map(|e| (e.from, e.kind, e.to))
                        .collect::<Vec<_>>()
                });
                (n.id, linked)
            })
            .collect()
    };
    let edge = |from: &str, kind, to: &str| (from.to_string(), kind, to.to_string());

    let from_parent = links(parent);
    assert!(
        from_parent.contains(&(
            child.clone(),
            Some(vec![
                edge(&child, EdgeType::DerivesFrom, parent),
                edge(&child, EdgeType::Refines, parent),
            ])
        )),
        "a child, with every kind it declares: {from_parent:?}"
    );
    assert!(
        from_parent.contains(&(
            "a-single-global-taxonomy".to_string(),
            Some(vec![
                edge(parent, EdgeType::Contradicts, "a-single-global-taxonomy"),
                edge("a-single-global-taxonomy", EdgeType::Contradicts, parent),
            ])
        )),
        "a contradiction, from both ends: {from_parent:?}"
    );

    let from_child = links(&child);
    assert!(
        from_child.contains(&(
            parent.to_string(),
            Some(vec![
                edge(&child, EdgeType::DerivesFrom, parent),
                edge(&child, EdgeType::Refines, parent),
            ])
        )),
        "a parent, the same edges seen from the other end: {from_child:?}"
    );
    assert!(
        from_child.contains(&("a-single-global-taxonomy".to_string(), None)),
        "a neighbour with no edge to the node is not linked, whatever its \
         edges to others: {from_child:?}"
    );

    // Free text is no node, so nothing is linked to it.
    let free = links("tags beat domains taxonomy");
    assert!(free.len() >= 3, "{free:?}");
    assert!(free.iter().all(|(_, l)| l.is_none()), "{free:?}");

    // Reading the links wrote none.
    let after = corpus.load_all().unwrap();
    let edges = |docs: &[nebula_core::Doc]| -> Vec<_> {
        docs.iter()
            .map(|d| (d.node.id.clone(), d.node.edges.clone()))
            .collect()
    };
    assert_eq!(edges(&before), edges(&after), "near writes nothing");
}

#[test]
fn near_is_capped_at_k_and_empty_for_a_thought_unlike_anything() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags taxonomy ranking", 2).unwrap();
    assert_eq!(near.0.len(), 2, "k caps the answer: {near:?}");
    assert!(
        graph::near(&graph, "tags taxonomy ranking", 0)
            .unwrap()
            .0
            .is_empty(),
        "k of zero asks for nothing"
    );

    let none = graph::near(&graph, "quantum gravity", NEAR_DEFAULT).unwrap();
    assert!(none.0.is_empty(), "no shared word, no neighbour: {none:?}");
    let stop = graph::near(&graph, "the and of", NEAR_DEFAULT).unwrap();
    assert!(stop.0.is_empty(), "stopwords alone are no query: {stop:?}");

    let empty = tempfile::tempdir().unwrap();
    let empty = Corpus::init(&process_locations(), &empty.path().join("corpus")).unwrap();
    let docs = empty.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    assert!(
        graph::near(&graph, "anything", NEAR_DEFAULT)
            .unwrap()
            .0
            .is_empty(),
        "an empty corpus has no neighbours"
    );
}

/// The count beside the answer is every node that shared a word with the
/// query, before `k` cut it: what a caller needs to say the answer is capped.
#[test]
fn near_counted_reports_the_matches_before_the_cut() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let (all, matched) = graph::near_counted(&graph, "tags taxonomy ranking", 100).unwrap();
    assert!(
        matched > 2,
        "the fixture has more than two matches: {all:?}"
    );
    assert_eq!(all.0.len(), matched, "an uncut answer is every match");

    let (cut, total) = graph::near_counted(&graph, "tags taxonomy ranking", 2).unwrap();
    assert_eq!(cut.0.len(), 2);
    assert_eq!(total, matched, "the count is taken before the cut");
    let ids = |near: &nebula_core::Near| near.0.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&cut), ids(&all)[..2], "the cut keeps the best");
    assert_eq!(
        ids(&cut),
        ids(&graph::near(&graph, "tags taxonomy ranking", 2).unwrap()),
        "`near` is the same answer without the count"
    );

    let (none, total) = graph::near_counted(&graph, "tags taxonomy ranking", 0).unwrap();
    assert!(none.0.is_empty());
    assert_eq!(total, matched, "k of zero still counts what it left out");
    assert_eq!(
        graph::near_counted(&graph, "quantum gravity", NEAR_DEFAULT)
            .unwrap()
            .1,
        0
    );
}

#[test]
fn capture_and_promote_suggest_but_never_link() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);

    let captured = ops::capture_near(&corpus, "a taxonomy for tags", NEAR_DEFAULT).unwrap();
    assert_eq!(captured.entry.text, "a taxonomy for tags");
    assert_eq!(
        captured
            .near
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["a-single-global-taxonomy", "tags-beat-domains"],
        "capture names the nearest nodes: {:?}",
        captured.near
    );
    let quiet = ops::capture_near(&corpus, "a taxonomy for tags, again", 0).unwrap();
    assert!(quiet.near.is_empty(), "k of zero is the quiet path");

    // Promoted as a root: the suggestions come back, and the node has no edge.
    let created = ops::promote(
        &corpus,
        &captured.entry.id,
        &Promotion::default(),
        NEAR_DEFAULT,
    )
    .unwrap();
    assert_eq!(
        created.near.first().map(|n| n.id.as_str()),
        Some("a-single-global-taxonomy"),
        "{:?}",
        created.near
    );
    assert!(
        !created.near.iter().any(|n| n.id == created.doc.node.id),
        "a promotion is not its own neighbour: {:?}",
        created.near
    );
    assert!(
        created.doc.node.edges.is_empty(),
        "suggesting never links: {:?}",
        created.doc.node.edges
    );
    let on_disk = corpus.load(&created.doc.node.id).unwrap();
    assert!(on_disk.node.edges.is_empty(), "nor on disk");

    // With a parent named, there is nothing to suggest.
    let entry = ops::capture(&corpus, "ranking decay again").unwrap();
    let created = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            parents: vec!["ranking-decay-half-life".into()],
            ..Promotion::default()
        },
        NEAR_DEFAULT,
    )
    .unwrap();
    assert!(created.near.is_empty(), "{:?}", created.near);
    assert_eq!(
        created.doc.node.parents().collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );
}

/// `suggest` and `close_tags` scan every node for advice, and a writer that
/// ran them under the lock would make every other writer wait on advice
/// (STD-03 §R1). A debug build refuses that outright, so any caller that
/// regresses fails its own tests; `capture_and_promote_suggest_but_never_link`
/// above and the CLI's capture, promote and tag tests passing is the proof
/// that the real callers run them with the lock released.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must not run under the corpus lock")]
fn advisory_reads_refuse_to_run_under_the_write_lock() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let _held = corpus.lock().expect("holding the lock");

    let close_tags = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ops::close_tags(&corpus, "tags-beat-domains", &["designs".to_string()])
    }));
    assert!(close_tags.is_err(), "close_tags ran under the lock");

    // With nothing to look for, nothing is scanned, so that is allowed:
    // the desktop's promote runs it that way inside its own lock.
    assert!(ops::suggest(&corpus, "tags", 0).unwrap().is_empty());
    let _ = ops::suggest(&corpus, "a taxonomy for tags", NEAR_DEFAULT);
}

/// `promote` reads its suggestions before it takes the lock; a caller that
/// holds the lock across the promotion and its commit reads them first with
/// `promotion_near` and hands them to `promote_with`, which returns them as
/// given.
#[test]
fn promotion_near_is_read_ahead_of_the_lock_and_promote_with_carries_it() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let entry = ops::capture(&corpus, "a taxonomy for tags").unwrap();

    let near =
        ops::promotion_near(&corpus, &entry.id, &Promotion::default(), NEAR_DEFAULT).unwrap();
    assert_eq!(
        near.first().map(|n| n.id.as_str()),
        Some("a-single-global-taxonomy"),
        "{near:?}"
    );
    let with_parent = Promotion {
        parents: vec!["tags-beat-domains".into()],
        ..Promotion::default()
    };
    assert!(
        ops::promotion_near(&corpus, &entry.id, &with_parent, NEAR_DEFAULT)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        ops::promotion_near(&corpus, "nope", &Promotion::default(), NEAR_DEFAULT),
        Err(Error::NoSuchInboxEntry(_))
    ));

    let _held = corpus.lock().expect("the caller's own lock");
    let created = ops::promote_with(&corpus, &entry.id, &Promotion::default(), near.clone())
        .expect("promoted under the caller's lock");
    assert_eq!(
        created.near.iter().map(|n| &n.id).collect::<Vec<_>>(),
        near.iter().map(|n| &n.id).collect::<Vec<_>>()
    );
}
