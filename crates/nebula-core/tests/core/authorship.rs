//! Authorship and notes: who wrote each field, a human confirming a kill,
//! and notes accumulating in order.

use crate::harness::{corpus, seed};
use nebula_core::{
    Corpus, EdgeType, Error, Graph, HUMAN, NewNode, Promotion, ReviewRule, graph, ops,
};

/// Authorship as a consumer that is not a terminal sees it: stored per field,
/// the human by omission, and confirmable without touching the kill text.
#[test]
fn authorship_is_per_field_and_a_human_can_confirm_a_kill() {
    let (_dir, corpus) = corpus();
    let by = Some("agent:crew-alpha");
    let parent = seed(&corpus, "Parent", &[]);

    let mine = seed(&corpus, "Mine", &[&parent]);
    let node = corpus.load(&mine).unwrap().node;
    assert_eq!(node.title_by, None, "an unattributed write is the human's");
    assert_eq!(node.edges[0].by, None);

    let theirs = ops::new_node(
        &corpus,
        &NewNode {
            title: "Theirs".into(),
            parents: vec![parent.clone()],
            kill: Some("if X".into()),
            by: by.map(String::from),
            ..NewNode::default()
        },
    )
    .unwrap()
    .doc
    .node;
    assert_eq!(theirs.title_by.as_deref(), by);
    assert_eq!(theirs.kill_by.as_deref(), by);
    assert_eq!(theirs.edges[0].by.as_deref(), by);

    // `review` asks the human to stand behind a kill somebody else wrote.
    let flagged = |corpus: &Corpus| -> Vec<String> {
        let docs = corpus.load_all().unwrap();
        graph::review(
            &Graph::build(&docs).unwrap(),
            &corpus.inbox().unwrap(),
            None,
        )
        .unwrap()
        .0
        .into_iter()
        .filter(|i| i.rule == ReviewRule::UnconfirmedKill)
        .map(|i| i.id)
        .collect()
    };
    assert_eq!(flagged(&corpus), std::slice::from_ref(&theirs.id));

    let confirmed = ops::confirm_kill(&corpus, &theirs.id).unwrap().node;
    assert_eq!(
        confirmed.kill.as_deref(),
        Some("if X"),
        "the text is as it was"
    );
    assert_eq!(confirmed.kill_by, None);
    assert_eq!(
        confirmed.title_by.as_deref(),
        by,
        "only the kill is confirmed"
    );
    assert!(flagged(&corpus).is_empty());

    // The queries state the default the file leaves out.
    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &theirs.id).unwrap();
    assert_eq!(view.node.kill_by.as_deref(), Some(HUMAN));
    assert_eq!(view.node.title_by.as_deref(), by);

    assert!(
        ops::sharpen(&corpus, &mine, "if Y", Some("crew (alpha)")).is_err(),
        "a label that would break a note line is refused"
    );
}

#[test]
fn notes_accumulate_in_order_and_unknown_nodes_are_refused() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the original thought").unwrap();
    let id = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            title: Some("A".into()),
            tags: vec!["physics".into()],
            ..Promotion::default()
        },
        0,
    )
    .unwrap()
    .doc
    .node
    .id;
    let parent = seed(&corpus, "Parent", &[]);
    ops::link(&corpus, &id, EdgeType::DerivesFrom, &parent, None).unwrap();

    let before = corpus.load(&id).unwrap();
    let status = before.node.status;
    let edges = before.node.edges.clone();
    let tags = before.node.tags.clone();
    let original = before.body.clone();

    ops::note(&corpus, &id, "first thought", None).unwrap();
    let second = ops::note(&corpus, &id, "second thought", None).unwrap();

    assert!(
        second.body.contains(original.trim()),
        "the capture is still there:\n{}",
        second.body
    );
    let first_at = second.body.find("first thought").expect("first note");
    let second_at = second.body.find("second thought").expect("second note");
    assert!(first_at < second_at, "notes accumulate in order");
    assert!(second.body.contains("## Notes"));
    assert_eq!(second.node.status, status);
    assert_eq!(second.node.edges, edges);
    assert_eq!(second.node.tags, tags);

    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &id).unwrap();
    assert_eq!(
        view.notes
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["first thought", "second thought"]
    );

    assert!(matches!(
        ops::note(&corpus, "nope", "lost", None),
        Err(Error::NoSuchNode(missing)) if missing == "nope"
    ));
}
