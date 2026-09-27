//! The point-of-action guards on links and status: self-links, genealogy
//! loops, contradictions, kill conditions, refuting and reopening.

use crate::graph::diamond;
use crate::harness::{corpus, plant_edges, seed};
use nebula_core::{Corpus, EdgeType, Error, Graph, NewNode, Status, graph, ops};

#[test]
fn a_node_cannot_link_to_itself() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::link(&corpus, &a, EdgeType::Refines, &a, None),
        Err(Error::SelfLoop)
    ));
}

#[test]
fn a_genealogy_edge_that_closes_a_loop_is_refused() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let err = ops::link(&corpus, &a, EdgeType::DerivesFrom, &d, None).unwrap_err();
    assert!(
        matches!(&err, Error::Cycle { from, to } if from == &a && to == &d),
        "got {err:?}"
    );
    // Refused means not written.
    let docs = corpus.load_all().unwrap();
    assert_eq!(
        graph::export(&Graph::build(&docs).unwrap())
            .unwrap()
            .edges
            .len(),
        4
    );
}

#[test]
fn a_contradiction_is_not_genealogy_so_it_may_point_anywhere() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let changed = ops::link(&corpus, &a, EdgeType::Contradicts, &d, None).unwrap();
    assert_eq!(changed.len(), 2, "recorded on both ends");
}

/// `alpha contradicts beta` recorded on `alpha` alone: what a crash between
/// `link`'s two saves leaves, or a hand edit.
fn half_recorded_contradiction(corpus: &Corpus) -> (String, String) {
    let alpha = seed(corpus, "Alpha idea", &[]);
    let beta = seed(corpus, "Beta idea", &[]);
    plant_edges(
        corpus,
        &alpha,
        &format!("  - type: contradicts\n    to: {beta}\n    by: claude"),
    );
    (alpha, beta)
}

#[test]
fn rerunning_a_half_recorded_contradicts_link_completes_it() {
    let (_dir, corpus) = corpus();
    let (alpha, beta) = half_recorded_contradiction(&corpus);
    let alpha_before = std::fs::read(corpus.node_path(&alpha).unwrap()).unwrap();

    let changed = ops::link(&corpus, &alpha, EdgeType::Contradicts, &beta, None).unwrap();

    let changed: Vec<&str> = changed.iter().map(|d| d.node.id.as_str()).collect();
    assert_eq!(changed, [beta.as_str()], "only the missing half is written");
    assert_eq!(
        std::fs::read(corpus.node_path(&alpha).unwrap()).unwrap(),
        alpha_before,
        "the half already recorded is left exactly as it was"
    );
    let reverse: Vec<_> = corpus
        .load(&beta)
        .unwrap()
        .node
        .edges
        .into_iter()
        .map(|e| (e.kind, e.to, e.by))
        .collect();
    assert_eq!(
        reverse,
        [(EdgeType::Contradicts, alpha.clone(), Some("claude".into()))],
        "the reverse is the other half of the recorded claim, credited to its author"
    );
    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);

    // Both halves present: a duplicate like any other, from either end.
    for (from, to) in [(&alpha, &beta), (&beta, &alpha)] {
        let refused = ops::link(&corpus, from, EdgeType::Contradicts, to, None);
        assert!(
            matches!(refused, Err(Error::DuplicateEdge { .. })),
            "{refused:?}"
        );
    }
    // A genealogy edge asked for twice is refused too, whatever `to` holds.
    ops::link(&corpus, &alpha, EdgeType::Refines, &beta, None).unwrap();
    let refused = ops::link(&corpus, &alpha, EdgeType::Refines, &beta, None);
    assert!(
        matches!(refused, Err(Error::DuplicateEdge { .. })),
        "{refused:?}"
    );
}

#[test]
fn check_names_the_repair_for_a_one_sided_contradiction() {
    let (_dir, corpus) = corpus();
    let (alpha, beta) = half_recorded_contradiction(&corpus);
    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    let rule4: Vec<_> = report.findings.iter().filter(|f| f.rule == 4).collect();
    assert_eq!(rule4.len(), 1, "{:?}", report.findings);
    assert_eq!(rule4[0].node.as_deref(), Some(alpha.as_str()));
    let repair = format!("neb link {alpha} contradicts {beta}");
    assert!(rule4[0].message.contains(&repair), "{}", rule4[0].message);
}

#[test]
fn a_hypothesis_needs_a_kill_condition() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Hypothesis, None),
        Err(Error::NeedsKill(Status::Hypothesis))
    ));
    ops::sharpen(&corpus, &a, "it would fail if …", None).unwrap();
    assert_eq!(corpus.load(&a).unwrap().node.status, Status::Hypothesis);
}

#[test]
fn refuting_needs_a_reason_and_is_final() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    ops::sharpen(&corpus, &a, "kill", None).unwrap();
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Refuted, None),
        Err(Error::RefutedNeedsWhy)
    ));
    let change = ops::set_status(&corpus, &a, Status::Refuted, Some("it fired")).unwrap();
    assert_eq!(change.from, Status::Hypothesis);
    assert_eq!(
        change.doc.node.closed.as_ref().map(|c| c.why.as_str()),
        Some("it fired")
    );
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Seed, None),
        Err(Error::RefutedCannotReopen)
    ));
    assert!(
        matches!(
            ops::sharpen(&corpus, &a, "a different falsifier", None),
            Err(Error::RefutedCannotReopen)
        ),
        "rewriting the kill of a refuted node would orphan closed.why"
    );
    assert!(
        matches!(
            ops::confirm_kill(&corpus, &a),
            Err(Error::RefutedCannotReopen)
        ),
        "confirming a kill on a refuted node is the same quiet rewrite"
    );
    let after = corpus.load(&a).unwrap().node;
    assert_eq!(after.kill.as_deref(), Some("kill"));
    assert_eq!(after.status, Status::Refuted);
}

/// A refuted idea, ready to be revived.
pub(super) fn refuted(corpus: &Corpus, title: &str) -> String {
    let id = seed(corpus, title, &[]);
    ops::sharpen(corpus, &id, "kill", None).unwrap();
    ops::set_status(corpus, &id, Status::Refuted, Some("it fired")).unwrap();
    id
}

/// How many node files are on disk, so a refusal can be shown to write none.
fn node_count(corpus: &Corpus) -> usize {
    corpus.load_all().unwrap().len()
}

#[test]
fn a_new_node_can_reopen_a_refuted_one_with_exactly_one_edge() {
    let (_dir, corpus) = corpus();
    let dead = refuted(&corpus, "Dead");
    let before = corpus.load(&dead).unwrap();
    let created = ops::new_node(
        &corpus,
        &NewNode {
            title: "Take two".into(),
            reopens: Some(dead.clone()),
            by: Some("agent:crew-alpha".into()),
            ..NewNode::default()
        },
    )
    .unwrap();
    let edges = &corpus.load(&created.doc.node.id).unwrap().node.edges;
    assert_eq!(edges.len(), 1, "{edges:?}");
    assert_eq!(edges[0].kind, EdgeType::Reopens);
    assert_eq!(edges[0].to, dead);
    assert_eq!(edges[0].by.as_deref(), Some("agent:crew-alpha"));
    // The refuted node is the record of the verdict, and reopening it from
    // a new node leaves it exactly as it was.
    let after = corpus.load(&dead).unwrap();
    assert_eq!(after.node.status, Status::Refuted);
    assert_eq!(after.node.edges, before.node.edges);
    assert_eq!(after.node.updated, before.node.updated);
}

#[test]
fn a_new_node_refuses_an_edge_to_a_missing_node_and_writes_nothing() {
    let (_dir, corpus) = corpus();
    seed(&corpus, "Present", &[]);
    for (reopens, contradicts) in [(Some("absent"), vec![]), (None, vec!["absent".to_string()])] {
        let refused = ops::new_node(
            &corpus,
            &NewNode {
                title: "Orphan".into(),
                reopens: reopens.map(String::from),
                contradicts,
                ..NewNode::default()
            },
        );
        assert!(
            matches!(&refused, Err(Error::NoSuchNode(id)) if id == "absent"),
            "{refused:?}"
        );
        assert_eq!(node_count(&corpus), 1);
    }
}

#[test]
fn a_new_node_refuses_a_parent_it_also_reopens() {
    let (_dir, corpus) = corpus();
    let dead = refuted(&corpus, "Dead");
    let refused = ops::new_node(
        &corpus,
        &NewNode {
            title: "Take two".into(),
            parents: vec![dead.clone()],
            reopens: Some(dead.clone()),
            ..NewNode::default()
        },
    );
    assert!(
        matches!(&refused, Err(Error::ParentAndReopens(id)) if *id == dead),
        "{refused:?}"
    );
    assert_eq!(node_count(&corpus), 1, "nothing was written");
}

#[test]
fn a_new_node_refuses_the_same_edge_twice() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    for spec in [
        NewNode {
            parents: vec![a.clone(), a.clone()],
            ..NewNode::default()
        },
        NewNode {
            contradicts: vec![a.clone(), a.clone()],
            ..NewNode::default()
        },
    ] {
        let refused = ops::new_node(
            &corpus,
            &NewNode {
                title: "Twice".into(),
                ..spec
            },
        );
        assert!(
            matches!(&refused, Err(Error::DuplicateEdge { from, to, .. }) if from == "twice" && *to == a),
            "{refused:?}"
        );
        assert_eq!(node_count(&corpus), 1);
        assert!(corpus.load(&a).unwrap().node.edges.is_empty());
    }
}

/// A refused repeat names the edge, so the refusal can say which one.
#[test]
fn linking_an_edge_twice_names_both_ends_and_the_kind() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let b = seed(&corpus, "B", &[]);
    ops::link(&corpus, &b, EdgeType::DerivesFrom, &a, None).unwrap();
    let refused = ops::link(&corpus, &b, EdgeType::DerivesFrom, &a, None);
    let Err(e @ Error::DuplicateEdge { .. }) = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(
        e.to_string(),
        "the edge `b` derives-from `a` already exists"
    );
    assert!(
        matches!(e, Error::DuplicateEdge { from, kind: EdgeType::DerivesFrom, to } if *from == b && *to == a),
        "{e:?}"
    );
}

#[test]
fn a_new_node_refuses_a_genealogy_edge_that_closes_a_loop() {
    // Nothing points at a node that does not exist yet, except an edge
    // written by hand ahead of it. Reopening the node that carries it would
    // make the new node its own ancestor.
    let (_dir, corpus) = corpus();
    let dead = refuted(&corpus, "Dead");
    let unrelated = seed(&corpus, "Unrelated", &[]);
    plant_edges(&corpus, &dead, "  - type: derives-from\n    to: take-two");

    let refused = ops::new_node(
        &corpus,
        &NewNode {
            title: "Take two".into(),
            parents: vec![unrelated],
            reopens: Some(dead.clone()),
            ..NewNode::default()
        },
    );
    assert!(
        matches!(&refused, Err(Error::Cycle { from, to }) if from == "take-two" && *to == dead),
        "the refusal names the edge that closes the loop: {refused:?}"
    );
    assert_eq!(node_count(&corpus), 2, "nothing was written");
}

#[test]
fn a_new_node_that_contradicts_one_records_it_on_both() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    let b = seed(&corpus, "B", &[]);
    let created = ops::new_node(
        &corpus,
        &NewNode {
            title: "Rival".into(),
            contradicts: vec![a.clone(), b.clone()],
            by: Some("agent:crew-alpha".into()),
            ..NewNode::default()
        },
    )
    .unwrap();
    let rival = created.doc.node.id;
    let mine = corpus.load(&rival).unwrap().node;
    assert_eq!(
        mine.edges_of(EdgeType::Contradicts).collect::<Vec<_>>(),
        [a.as_str(), b.as_str()]
    );
    for other in [&a, &b] {
        let node = corpus.load(other).unwrap().node;
        assert_eq!(node.edges.len(), 1, "{:?}", node.edges);
        assert_eq!(node.edges[0].kind, EdgeType::Contradicts);
        assert_eq!(node.edges[0].to, rival);
        assert_eq!(node.edges[0].by.as_deref(), Some("agent:crew-alpha"));
    }
}

#[test]
fn a_node_with_a_kill_condition_reopens_as_a_hypothesis_not_a_seed() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    ops::sharpen(&corpus, &a, "kill", None).unwrap();

    // hypothesis -> seed would keep the kill, since nothing is deleted, and
    // leave a seed that `check` reads as a hand edit.
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Seed, None),
        Err(Error::SeedWithKill)
    ));
    let after = corpus.load(&a).unwrap().node;
    assert_eq!(after.status, Status::Hypothesis);
    assert_eq!(after.kill.as_deref(), Some("kill"));

    // abandoned -> seed is the same move, and the refusal leaves the closing
    // exactly as it was.
    ops::set_status(&corpus, &a, Status::Abandoned, Some("moved on")).unwrap();
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Seed, None),
        Err(Error::SeedWithKill)
    ));
    let after = corpus.load(&a).unwrap().node;
    assert_eq!(after.status, Status::Abandoned);
    assert_eq!(after.kill.as_deref(), Some("kill"));
    assert_eq!(
        after.closed.as_ref().map(|c| c.why.as_str()),
        Some("moved on")
    );

    // abandoned -> hypothesis is the honest reopen, and keeps the kill.
    let change = ops::set_status(&corpus, &a, Status::Hypothesis, None).unwrap();
    assert_eq!(change.from, Status::Abandoned);
    assert_eq!(change.doc.node.status, Status::Hypothesis);
    assert_eq!(change.doc.node.kill.as_deref(), Some("kill"));
    assert!(change.doc.node.closed.is_none());

    // A node without a kill still goes back to seed from abandoned.
    let b = seed(&corpus, "B", &[]);
    ops::set_status(&corpus, &b, Status::Abandoned, None).unwrap();
    let change = ops::set_status(&corpus, &b, Status::Seed, None).unwrap();
    assert_eq!(change.doc.node.status, Status::Seed);

    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn an_empty_kill_condition_is_refused() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::sharpen(&corpus, &a, "   ", None),
        Err(Error::EmptyKill)
    ));
}
