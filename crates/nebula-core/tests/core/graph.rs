//! The graph queries: trace up and down through diamonds, depth bounds,
//! review cuts, impact and export, and the scan and model they read.

use crate::harness::{corpus, seed};
use nebula_core::{
    Corpus, Direction, EdgeType, Error, Graph, ReviewItem, ReviewReport, ReviewRule, Status,
    TraceHop, Via, graph, ops,
};

#[test]
fn current_model_refuses_unknown_fields_in_nested_node_data() {
    for nested in [
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n",
        "references:\n- id: r1\n  kind: study\n  added: 2026-09-22\n  origin:\n    task: FIXTURE-1\n    future_field: IRREPLACEABLE\n",
        "edges:\n- type: derives-from\n  to: parent\n  future_field: IRREPLACEABLE\n",
    ] {
        let (dir, corpus) = corpus();
        let id = seed(&corpus, "Nested data", &[]);
        let path = dir.path().join("corpus/nodes").join(format!("{id}.md"));
        let original = std::fs::read_to_string(&path).unwrap();
        let edited = original.replacen("---\n\n", &format!("{nested}---\n\n"), 1);
        assert_ne!(original, edited);
        std::fs::write(&path, &edited).unwrap();

        let error = corpus.load_all().unwrap_err().to_string();
        assert!(error.contains("future_field"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    }
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::print_stderr,
    reason = "report why the unreadable-file test is skipped as root"
)]
fn scan_reports_every_bad_file_and_returns_the_rest() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root can read mode-000 files");
        return;
    }
    let (_dir, corpus) = corpus();
    let good = seed(&corpus, "Good idea", &[]);
    let unreadable = corpus.node_path("unreadable").unwrap();
    let malformed = corpus.node_path("malformed").unwrap();
    std::fs::write(&unreadable, "---\nid: unreadable\n---\n").unwrap();
    std::fs::write(&malformed, "no frontmatter\n").unwrap();
    let original = std::fs::metadata(&unreadable).unwrap().permissions();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    let scan = corpus.scan().unwrap();
    std::fs::set_permissions(&unreadable, original).unwrap();
    assert_eq!(scan.docs.len(), 1);
    assert_eq!(scan.docs[0].node.id, good);
    assert_eq!(scan.unreadable.len(), 2);
    assert!(scan.unreadable.iter().any(|entry| entry.path == unreadable));
    assert!(scan.unreadable.iter().any(|entry| entry.path == malformed));
}

/// A diamond: `d` descends from `b` and `c`, both of which descend from `a`.
pub(super) fn diamond(corpus: &Corpus) -> [String; 4] {
    let a = seed(corpus, "A", &[]);
    let b = seed(corpus, "B", &[&a]);
    let c = seed(corpus, "C", &[&a]);
    let d = seed(corpus, "D", &[&b, &c]);
    [a, b, c, d]
}

#[test]
fn trace_up_reports_each_ancestor_once_nearest_first() {
    let (_dir, corpus) = corpus();
    let [a, b, c, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let ids: Vec<_> = graph::trace(&graph, &d, Direction::Up)
        .unwrap()
        .0
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(ids.first(), Some(&d), "the walk starts at the node itself");
    assert_eq!(
        ids.len(),
        4,
        "a diamond reaches `a` twice but reports it once"
    );
    assert!(ids.contains(&a) && ids.contains(&b) && ids.contains(&c));
}

#[test]
fn trace_down_walks_descendants() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let walk = graph::trace(&graph, &a, Direction::Down).unwrap();
    assert_eq!(walk.0.len(), 4);
    assert_eq!(walk.0[0].id, a);
    assert!(walk.0.iter().any(|n| n.id == d));
}

#[test]
fn trace_records_the_edge_kinds_of_each_step_through_a_diamond() {
    let (_dir, corpus) = corpus();
    let [a, b, c, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    let hop = |from: &str, kinds: &[EdgeType]| {
        Some(TraceHop {
            from: from.to_string(),
            kinds: kinds.to_vec(),
        })
    };

    let up = graph::trace(&graph, &d, Direction::Up).unwrap().0;
    let steps: Vec<_> = up.iter().map(|n| (n.id.as_str(), n.via.clone())).collect();
    assert_eq!(
        steps,
        [
            (d.as_str(), None),
            (b.as_str(), hop(&d, &[EdgeType::DerivesFrom])),
            (a.as_str(), hop(&b, &[EdgeType::DerivesFrom])),
            (c.as_str(), hop(&d, &[EdgeType::DerivesFrom])),
        ],
        "the start has no step; the shared ancestor is reached once, by the first branch"
    );

    let down = graph::trace(&graph, &a, Direction::Down).unwrap().0;
    let steps: Vec<_> = down
        .iter()
        .map(|n| (n.id.as_str(), n.via.clone()))
        .collect();
    assert_eq!(
        steps,
        [
            (a.as_str(), None),
            (b.as_str(), hop(&a, &[EdgeType::DerivesFrom])),
            (d.as_str(), hop(&b, &[EdgeType::DerivesFrom])),
            (c.as_str(), hop(&a, &[EdgeType::DerivesFrom])),
        ]
    );
}

#[test]
fn trace_merges_parallel_edges_into_one_step_with_both_kinds() {
    let (_dir, corpus) = corpus();
    let old = seed(&corpus, "Old", &[]);
    let new = seed(&corpus, "New", &[&old]);
    ops::link(&corpus, &new, EdgeType::Reopens, &old, None).unwrap();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    let both = vec![EdgeType::DerivesFrom, EdgeType::Reopens];

    let up = graph::trace(&graph, &new, Direction::Up).unwrap().0;
    assert_eq!(up.len(), 2, "two edges to one parent reach it once");
    assert_eq!(
        up[0].parents,
        std::slice::from_ref(&old),
        "and name it as a parent once"
    );
    assert_eq!(up[1].id, old);
    let via = up[1].via.as_ref().expect("reached by a step");
    assert_eq!((via.from.as_str(), &via.kinds), (new.as_str(), &both));

    let down = graph::trace(&graph, &old, Direction::Down).unwrap().0;
    assert_eq!(down.len(), 2, "{down:?}");
    assert_eq!(down[1].id, new);
    let via = down[1].via.as_ref().expect("reached by a step");
    assert_eq!((via.from.as_str(), &via.kinds), (old.as_str(), &both));

    // Parallel edges are one relation, so nothing downstream counts it twice.
    let touched = graph::impact(&graph, &old).unwrap().0;
    assert_eq!(touched.len(), 1, "{touched:?}");
}

#[test]
fn trace_of_an_unknown_node_is_a_typed_error() {
    let (_dir, corpus) = corpus();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    assert!(matches!(
        graph::trace(&graph, "nope", Direction::Up),
        Err(Error::NoSuchNode(id)) if id == "nope"
    ));
}

/// A chain with a shortcut: `root <- branch <- cut <- deep`, and `cut` also
/// descends from `root` directly. Walking down from `root`, `cut` is met two
/// steps out through `branch` before it is met one step out on its own.
fn shortcut(corpus: &Corpus) -> [String; 4] {
    let root = seed(corpus, "Root", &[]);
    let branch = seed(corpus, "Branch", &[&root]);
    let cut = seed(corpus, "Cut", &[&branch, &root]);
    let deep = seed(corpus, "Deep", &[&cut]);
    [root, branch, cut, deep]
}

#[test]
fn trace_within_stops_at_the_depth_but_keeps_every_node_that_close() {
    let (_dir, corpus) = corpus();
    let [root, branch, cut, deep] = shortcut(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    let ids = |from: &str, direction, depth| -> Vec<String> {
        graph::trace_within(&graph, from, direction, depth)
            .unwrap()
            .0
            .into_iter()
            .map(|n| n.id)
            .collect()
    };

    assert_eq!(
        ids(&root, Direction::Down, Some(0)),
        std::slice::from_ref(&root)
    );
    assert_eq!(
        ids(&root, Direction::Down, Some(1)),
        [root.clone(), branch.clone(), cut.clone()],
        "one step down is the children and nothing below them"
    );
    // Through `branch`, `cut` sits at the bound and `deep` beyond it; through
    // the shortcut, `deep` is two steps out, so it belongs in the walk.
    let two = ids(&root, Direction::Down, Some(2));
    assert_eq!(two.len(), 4, "{two:?}");
    assert!(two.contains(&deep), "{two:?}");
    assert_eq!(
        two.iter().filter(|id| **id == cut).count(),
        1,
        "met twice, reported once"
    );

    assert_eq!(
        ids(&deep, Direction::Up, Some(1)),
        [deep.clone(), cut.clone()]
    );
    assert_eq!(ids(&deep, Direction::Up, Some(2)).len(), 4);
}

#[test]
fn an_unbounded_or_unreached_bound_walks_exactly_as_trace_does() {
    let (_dir, corpus) = corpus();
    let [root, ..] = shortcut(&corpus);
    let [a, _, _, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    let steps = |walk: graph::Trace| -> Vec<(String, Option<TraceHop>)> {
        walk.0.into_iter().map(|n| (n.id, n.via)).collect()
    };
    for (from, direction) in [
        (&root, Direction::Down),
        (&a, Direction::Down),
        (&d, Direction::Up),
    ] {
        let whole = steps(graph::trace(&graph, from, direction).unwrap());
        for depth in [None, Some(3), Some(100)] {
            assert_eq!(
                steps(graph::trace_within(&graph, from, direction, depth).unwrap()),
                whole,
                "{from} {direction:?} {depth:?}"
            );
        }
    }
    assert!(matches!(
        graph::trace_within(&graph, "nope", Direction::Up, Some(1)),
        Err(Error::NoSuchNode(id)) if id == "nope"
    ));
}

#[test]
fn a_review_is_cut_per_rule_and_says_how_much_each_rule_lost() {
    let item = |rule, id: &str| ReviewItem {
        rule,
        id: id.into(),
        title: id.into(),
        reason: String::new(),
    };
    let full = ReviewReport(vec![
        item(ReviewRule::StaleHypothesis, "h1"),
        item(ReviewRule::StaleHypothesis, "h2"),
        item(ReviewRule::StaleHypothesis, "h3"),
        item(ReviewRule::UntouchedSeed, "s1"),
        item(ReviewRule::NoReferences, "n1"),
        item(ReviewRule::NoReferences, "n2"),
        item(ReviewRule::StaleInbox, "inbox"),
    ]);
    let ids = |r: &ReviewReport| r.0.iter().map(|i| i.id.clone()).collect::<Vec<_>>();

    let mut one = full.clone();
    let omitted = one.truncate_per_rule(1);
    assert_eq!(
        ids(&one),
        ["h1", "s1", "n1", "inbox"],
        "the first of each rule, in order"
    );
    assert_eq!(
        omitted,
        [
            (ReviewRule::StaleHypothesis, 2),
            (ReviewRule::NoReferences, 1)
        ],
        "only the rules that lost something, with how much"
    );

    let mut roomy = full.clone();
    assert!(roomy.truncate_per_rule(3).is_empty());
    assert_eq!(
        ids(&roomy),
        ids(&full),
        "a bound nobody reaches cuts nothing"
    );

    let mut none = full.clone();
    let omitted = none.truncate_per_rule(0);
    assert!(none.0.is_empty());
    assert_eq!(omitted.iter().map(|(_, n)| n).sum::<usize>(), full.0.len());
}

#[test]
fn review_reasons_count_in_the_singular_for_one() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "Cold seed", &[]);
    let path = dir.path().join("corpus/nodes").join(format!("{id}.md"));
    let raw = std::fs::read_to_string(&path).unwrap();
    let at = raw.find("\nupdated: ").unwrap() + "\nupdated: ".len();
    let mut raw = raw;
    raw.replace_range(at..at + 10, "2000-01-01");
    std::fs::write(&path, raw).unwrap();
    let docs = corpus.load_all().unwrap();
    let report = graph::review(
        &Graph::build(&docs).unwrap(),
        &corpus.inbox().unwrap(),
        Some(1),
    )
    .unwrap();
    let seed = report
        .0
        .iter()
        .find(|i| i.rule == ReviewRule::UntouchedSeed)
        .expect("the seed is cold");
    assert_eq!(
        seed.reason,
        "seed untouched for 1 day; propose: status abandoned"
    );
}

#[test]
fn impact_lists_descendants_then_contradictions() {
    let (_dir, corpus) = corpus();
    let [a, b, c, d] = diamond(&corpus);
    let rival = seed(&corpus, "Rival", &[]);
    ops::link(&corpus, &a, EdgeType::Contradicts, &rival, None).unwrap();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let touched = graph::impact(&graph, &a).unwrap().0;
    let descendants: Vec<_> = touched
        .iter()
        .filter(|t| t.via == Via::Descends)
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(descendants.len(), 3);
    for id in [&b, &c, &d] {
        assert!(descendants.contains(&id.as_str()), "{id} descends from a");
    }
    assert!(
        touched
            .iter()
            .any(|t| t.via == Via::Contradicts && t.id == rival)
    );
}

#[test]
fn export_carries_every_node_and_edge() {
    let (_dir, corpus) = corpus();
    let [a, b, _, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let out = graph::export(&graph).unwrap();
    assert_eq!(out.nodes.len(), 4);
    assert_eq!(out.edges.len(), 4, "b→a, c→a, d→b, d→c");
    assert!(out.edges.iter().all(|e| e.kind == EdgeType::DerivesFrom));
    assert!(out.edges.iter().any(|e| e.from == b && e.to == a));
    assert!(out.edges.iter().any(|e| e.from == d && e.to == b));
    assert!(out.nodes.iter().all(|n| n.status == Status::Seed));
}

#[test]
fn build_refuses_duplicate_ids() {
    let (_dir, corpus) = corpus();
    seed(&corpus, "Twin", &[]);
    let mut docs = corpus.load_all().unwrap();
    let copy = docs[0].clone();
    docs.push(copy);
    assert!(matches!(
        Graph::build(&docs),
        Err(Error::DuplicateId(id)) if id == "twin"
    ));
}
