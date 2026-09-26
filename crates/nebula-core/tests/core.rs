#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Library-level tests: the graph queries and the point-of-action guards, as
//! the desktop app and the agent skill see them.
//!
//! `crates/neb/tests/cli.rs` covers the same rules through the binary and
//! asserts on messages and exit codes. These assert on the typed values, which
//! is what a consumer that is not a terminal matches on.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant and corrupt corpus files directly; only the code under test goes \
              through nebula_core's durable write helper"
)]

use nebula_core::InboxEntry;
use nebula_core::triage::{Action, Step, Tally};
use nebula_core::verb::{self, CommitPolicy, RootWarning, WriteOptions};
use nebula_core::{
    Band, Citation, CommitOutcome, Committed, Corpus, CorpusLock, Direction, EdgeType, Error,
    Graph, HUMAN, Handoff, Locations, NEAR_DEFAULT, NewNode, ObservatoryLink, Promotion,
    ReviewItem, ReviewReport, ReviewRule, Settlement, Status, TraceHop, Triage, Via, graph, ops,
    store,
};
use std::fmt::Write as _;
use std::path::Path;

// This process runs with a temporary `HOME` and git environment, set before
// any test thread starts, and every child comes from its builder.
mod support;

/// The resolved environment the existing tests run under: this process's,
/// which `support` isolated before any test thread started. The new
/// resolver tests build theirs by hand instead.
fn process_locations() -> Locations {
    Locations::from_reader(|name| std::env::var_os(name), std::env::current_dir().ok())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "enumerates every direct core write path"
)]
fn read_only_environment_refuses_every_write_op_before_any_write() {
    let (_dir, writable) = corpus();
    let node = seed(&writable, "First idea", &[]);
    let other = seed(&writable, "Second idea", &[]);
    let inbox = ops::capture(&writable, "Captured thought").unwrap();
    let root = writable.root().to_path_buf();
    let read_only = Locations {
        nebula_read_only: Some("1".into()),
        ..process_locations()
    };
    let mut c = Corpus::open(&read_only, Some(root.clone())).unwrap();
    let before_node = std::fs::read(c.node_path(&node).unwrap()).unwrap();
    let before_inbox = std::fs::read(&inbox.file).unwrap();
    let before_config = std::fs::read(root.join("config.yaml")).unwrap();
    let setting = Corpus::observatory_root_config_path(&read_only).unwrap();
    let before_setting = std::fs::read(&setting).ok();
    let before_entries = std::fs::read_dir(root.join("nodes")).unwrap().count();
    macro_rules! read_only {
        ($name:expr, $op:expr) => {
            assert!(matches!($op, Err(Error::ReadOnly)), "{}", $name);
        };
    }
    read_only!(
        "init",
        ops::init(&read_only, Some(root.join("new")), None, false, false)
    );
    read_only!(
        "migrate",
        nebula_core::migrate::run(&read_only, Some(root.clone()))
    );
    read_only!("capture", ops::capture(&c, "more thoughts"));
    read_only!("store capture", c.capture("more thoughts"));
    read_only!("drop", ops::drop(&c, &inbox.id));
    read_only!("store settlement", c.settle_inbox(&inbox, "dropped"));
    read_only!(
        "promote",
        ops::promote(&c, &inbox.id, &Promotion::default(), 0)
    );
    read_only!(
        "new",
        ops::new_node(
            &c,
            &NewNode {
                title: "Third idea".into(),
                ..NewNode::default()
            }
        )
    );
    read_only!("edit", ops::set_body(&c, &node, "edited"));
    let mut doc = c.load(&node).unwrap();
    read_only!("store save", c.save(&mut doc));
    read_only!("store create", c.create(&doc));
    read_only!("edit-if", ops::set_body_if(&c, &node, "", "edited"));
    read_only!("sharpen", ops::sharpen(&c, &node, "if false", None));
    read_only!("confirm", ops::confirm_kill(&c, &node));
    read_only!(
        "status",
        ops::set_status(&c, &node, Status::Abandoned, Some("done"))
    );
    read_only!(
        "link",
        ops::link(&c, &node, EdgeType::Refines, &other, None)
    );
    read_only!("note", ops::note(&c, &node, "more words", None));
    read_only!(
        "cite",
        ops::cite(
            &c,
            &node,
            &Citation {
                uri: Some("https://example.org".into()),
                kind: "other".into(),
                ..Citation::default()
            }
        )
    );
    read_only!(
        "handoff",
        ops::handoff(
            &c,
            &node,
            &Handoff {
                record: "H012".into(),
                ..Handoff::default()
            },
            None
        )
    );
    read_only!("tag", ops::retag(&c, &node, &["alpha".into()], &[]));
    read_only!("config commit", ops::set_commit(&mut c, true));
    read_only!("config observatory", ops::set_observatory_root(&c, &root));
    read_only!("config legacy", ops::drop_legacy_observatory_root(&mut c));
    read_only!("commit", ops::commit(&c, "test", &[]));
    assert_eq!(
        std::fs::read(c.node_path(&node).unwrap()).unwrap(),
        before_node
    );
    assert_eq!(std::fs::read(&inbox.file).unwrap(), before_inbox);
    assert_eq!(
        std::fs::read(root.join("config.yaml")).unwrap(),
        before_config
    );
    assert_eq!(std::fs::read(&setting).ok(), before_setting);
    assert_eq!(
        std::fs::read_dir(root.join("nodes")).unwrap().count(),
        before_entries
    );
    assert!(!root.join("new").exists());
}

#[cfg(unix)]
#[test]
fn invalid_orbit_origin_is_refused_without_lossy_provenance() {
    use std::os::unix::ffi::OsStringExt;
    let locations = Locations {
        orbit_run_id: Some(std::ffi::OsString::from_vec(vec![0xff])),
        ..Locations::default()
    };
    assert!(matches!(
        locations.origin(None, None),
        Err(Error::InvalidOriginEnvironment { .. })
    ));
    let explicit = locations
        .origin(None, Some("jrun-explicit".into()))
        .unwrap()
        .unwrap();
    assert_eq!(explicit.run.as_deref(), Some("jrun-explicit"));
}

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = Corpus::init(&process_locations(), &dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

fn seed(corpus: &Corpus, title: &str, parents: &[&str]) -> String {
    ops::new_node(
        corpus,
        &NewNode {
            title: title.to_string(),
            parents: parents.iter().map(ToString::to_string).collect(),
            ..NewNode::default()
        },
    )
    .expect("new node")
    .doc
    .node
    .id
}

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
fn diamond(corpus: &Corpus) -> [String; 4] {
    let a = seed(corpus, "A", &[]);
    let b = seed(corpus, "B", &[&a]);
    let c = seed(corpus, "C", &[&a]);
    let d = seed(corpus, "D", &[&b, &c]);
    [a, b, c, d]
}

// ------------------------------------------------------------------- graph --

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

// --------------------------------------------------------------------- ops --

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
    let mut doc = corpus.load(&alpha).unwrap();
    doc.node.edges.push(nebula_core::Edge {
        kind: EdgeType::Contradicts,
        to: beta.clone(),
        by: Some("claude".into()),
    });
    corpus.save(&mut doc).unwrap();
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
fn refuted(corpus: &Corpus, title: &str) -> String {
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
    let mut doc = corpus.load(&dead).unwrap();
    doc.node.edges.push(nebula_core::Edge {
        kind: EdgeType::DerivesFrom,
        to: "take-two".into(),
        by: None,
    });
    corpus.save(&mut doc).unwrap();

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

fn citing(uri: &str) -> Citation {
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

fn handing(record: &str) -> Handoff {
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

#[test]
fn settling_an_inbox_entry_is_atomic_and_leaves_no_temporary_file() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut tmp = entry.file.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);

    corpus.settle_inbox(&entry, "dropped").unwrap();

    assert!(!tmp.exists(), "successful settlement left a temporary file");
    let leftovers: Vec<_> = std::fs::read_dir(entry.file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name != entry.file.file_name().unwrap())
        .collect();
    assert!(
        leftovers.is_empty(),
        "successful settlement left {leftovers:?} beside the month file"
    );
    assert!(
        std::fs::read_to_string(&entry.file)
            .unwrap()
            .contains(&format!("- ~~[{}]", entry.id))
    );
}

#[test]
fn an_interrupted_settlement_copy_does_not_resurrect_an_entry() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut tmp = entry.file.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let unsettled = std::fs::read(&entry.file).unwrap();

    corpus.settle_inbox(&entry, "dropped").unwrap();
    std::fs::write(&tmp, unsettled).unwrap();

    assert!(corpus.inbox().unwrap().0.is_empty());
    assert!(matches!(
        corpus.inbox_entry(&entry.id),
        Err(Error::InboxEntrySettled { id, settlement: Settlement::Dropped }) if id == entry.id
    ));
}

/// Every write in the corpus goes through one atomic replacement, so a
/// symlink planted where that replacement's temporary file would go must be
/// refused for a node, for the inbox and for the config alike. These three
/// name the same guard from the three callers that reach it.
#[cfg(unix)]
#[test]
fn a_node_write_cannot_reach_a_file_outside_the_corpus_through_its_temporary() {
    let (dir, corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let id = seed(&corpus, "Safe", &[]);
    let node = dir.path().join("corpus").join("nodes").join("safe.md");
    let planted = dir.path().join("corpus").join("nodes").join("safe.md.tmp");
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    ops::note(&corpus, &id, "probe", None).unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the note was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&node)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became the node"
    );
    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &id).unwrap();
    assert_eq!(
        view.notes
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["probe"],
        "the note did not reach the node it named"
    );
}

#[cfg(unix)]
#[test]
fn settling_an_inbox_entry_cannot_reach_a_file_outside_the_corpus() {
    let (dir, corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut planted = entry.file.as_os_str().to_os_string();
    planted.push(".tmp");
    std::os::unix::fs::symlink(&outside, std::path::PathBuf::from(planted)).unwrap();

    corpus.settle_inbox(&entry, "dropped").unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the settlement was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&entry.file)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became the month file"
    );
    assert!(corpus.inbox().unwrap().0.is_empty());
}

#[cfg(unix)]
#[test]
fn capture_refuses_a_symlinked_active_month_without_changing_its_target() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    let outside = dir.path().join("outside-inbox.md");
    std::fs::rename(&entry.file, &outside).unwrap();
    let before = std::fs::read(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &entry.file).unwrap();

    let error = ops::capture(&corpus, "must stay inside").unwrap_err();

    assert!(matches!(error, Error::InboxSymlink(path) if path == entry.file));
    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&entry.file)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Capture reads the month file before it appends, and a FIFO there held
/// that read until something wrote to it. It is refused instead.
#[cfg(unix)]
#[test]
fn capture_refuses_a_fifo_month_file_without_blocking() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    std::fs::remove_file(&entry.file).unwrap();
    mkfifo(&entry.file);

    let refused = within_two_seconds(move || ops::capture(&corpus, "must not block"));
    assert!(
        not_regular(&refused, &entry.file, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );
}

/// `init` copies a symlinked `.gitignore`'s target on purpose, but only a
/// regular file's: a FIFO at `.gitignore`, or at the end of its link, is
/// refused rather than read.
#[cfg(unix)]
#[test]
fn init_refuses_a_fifo_gitignore_without_blocking() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let gitignore = root.join(".gitignore");
    std::fs::remove_file(&gitignore).unwrap();
    mkfifo(&gitignore);
    let refused = within_two_seconds({
        let root = root.clone();
        move || Corpus::init(&process_locations(), &root)
    });
    assert!(
        not_regular(&refused, &gitignore, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );

    let fifo = dir.path().join("outside-fifo");
    std::fs::rename(&gitignore, &fifo).unwrap();
    std::os::unix::fs::symlink(&fifo, &gitignore).unwrap();
    let refused = within_two_seconds(move || Corpus::init(&process_locations(), &root));
    assert!(
        not_regular(&refused, &gitignore, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );
    assert!(
        std::fs::symlink_metadata(&gitignore)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn inbox_writes_refuse_a_symlinked_directory_without_changing_its_target() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    let inbox_dir = entry.file.parent().unwrap();
    let outside_dir = dir.path().join("outside-inbox");
    std::fs::rename(inbox_dir, &outside_dir).unwrap();
    let outside_month = outside_dir.join(entry.file.file_name().unwrap());
    let before = std::fs::read(&outside_month).unwrap();
    std::os::unix::fs::symlink(&outside_dir, inbox_dir).unwrap();

    let capture_error = ops::capture(&corpus, "must stay inside").unwrap_err();
    let settle_error = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    let drop_error = ops::drop(&corpus, &entry.id).unwrap_err();
    let promote_error = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap_err();

    for error in [capture_error, settle_error, drop_error, promote_error] {
        assert!(
            matches!(&error, Error::InboxSymlink(path) if path == inbox_dir),
            "{error}"
        );
    }
    assert_eq!(std::fs::read(&outside_month).unwrap(), before);
    assert!(corpus.load_all().unwrap().is_empty());
}

/// An entry's file is the caller's to set, and settling writes it. Only
/// `<root>/inbox/<month>.md` is taken: a path that merely starts with the
/// inbox can climb out of it with `..`, and the entry's line copied there
/// would be struck in a file that is not the inbox's (STD-05 §R6).
#[test]
fn settling_refuses_an_entry_file_that_leaves_the_inbox_by_name() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let content = std::fs::read_to_string(&entry.file).unwrap();
    let root = dir.path().join("corpus");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir(&outside).unwrap();
    let month = entry.file.file_name().unwrap();
    std::fs::write(root.join(month), &content).unwrap();
    std::fs::write(outside.join(month), &content).unwrap();

    for file in [
        root.join("inbox").join("..").join(month),
        root.join("inbox")
            .join("..")
            .join("..")
            .join("elsewhere")
            .join(month),
        outside.join(month),
    ] {
        let mut foreign = entry.clone();
        foreign.file = file.clone();
        let error = corpus.settle_inbox(&foreign, "dropped").unwrap_err();
        assert!(
            matches!(&error, Error::InboxEntryForeign { file: named, .. } if *named == file),
            "{error}"
        );
    }
    assert_eq!(std::fs::read_to_string(root.join(month)).unwrap(), content);
    assert_eq!(
        std::fs::read_to_string(outside.join(month)).unwrap(),
        content
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), content);
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

/// `inbox/` is judged by `lstat`, and so is the month file, but nothing
/// between them is: an entry whose file sits in a symlinked directory under
/// the inbox is refused by its shape rather than followed out of the corpus.
#[cfg(unix)]
#[test]
fn settling_refuses_an_entry_file_below_a_symlinked_inbox_subdirectory() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let content = std::fs::read_to_string(&entry.file).unwrap();
    let outside = dir.path().join("outside-inbox");
    std::fs::create_dir(&outside).unwrap();
    let month = entry.file.file_name().unwrap();
    std::fs::write(outside.join(month), &content).unwrap();
    let planted = entry.file.parent().unwrap().join("sub");
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    let mut foreign = entry.clone();
    foreign.file = planted.join(month);
    let error = corpus.settle_inbox(&foreign, "dropped").unwrap_err();

    assert!(
        matches!(&error, Error::InboxEntryForeign { file, .. } if *file == foreign.file),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join(month)).unwrap(),
        content
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), content);
    corpus.settle_inbox(&entry, "dropped").unwrap();
    assert!(corpus.inbox().unwrap().0.is_empty());
}

#[cfg(unix)]
#[test]
fn a_config_write_cannot_reach_a_file_outside_the_corpus() {
    let (dir, mut corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let config = dir.path().join("corpus").join("config.yaml");
    let mut planted = config.as_os_str().to_os_string();
    planted.push(".tmp");
    std::os::unix::fs::symlink(&outside, std::path::PathBuf::from(planted)).unwrap();

    ops::set_commit(&mut corpus, true).unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the config was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became config.yaml"
    );
    let reopened = Corpus::open(&process_locations(), Some(dir.path().join("corpus"))).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the setting did not reach config.yaml"
    );
}

#[test]
fn inbox_ignores_everything_except_month_markdown_files() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let inbox_dir = entry.file.parent().unwrap();

    for name in ["notes.md", "2026-00.md", "2026-13.md", "2026-09.md.tmp"] {
        std::fs::write(inbox_dir.join(name), b"not utf-8: \xff").unwrap();
    }
    std::fs::create_dir(inbox_dir.join("2000-01.md")).unwrap();

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;

        let invalid_name = std::ffi::OsString::from_vec(b"2000-01-\xff.md".to_vec());
        std::fs::write(inbox_dir.join(invalid_name), "unrelated").unwrap();
    }

    let listed = corpus.inbox().unwrap();
    assert_eq!(listed.0.len(), 1);
    assert_eq!(listed.0[0].id, entry.id);

    let captured = ops::capture(&corpus, "another thought").unwrap();
    assert_ne!(captured.id, entry.id);
    assert_eq!(corpus.inbox().unwrap().0.len(), 2);
}

#[test]
fn settling_refuses_when_the_indexed_line_has_another_entry_id() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the original thought").unwrap();
    let replacement = "- [other] 2026-09-22T08:25 another thought\n";
    std::fs::write(&entry.file, replacement).unwrap();

    let error = corpus.settle_inbox(&entry, "dropped").unwrap_err();

    assert!(
        matches!(&error, Error::InboxEntryChanged { id, file, line: 0 } if *id == entry.id && *file == entry.file),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), replacement);
}

/// The line gone and the line changed are two failure modes, and each is
/// its own refusal naming the entry and its file (STD-02 §R26).
#[test]
fn a_missing_and_a_changed_inbox_line_are_distinct_refusals() {
    let (_dir, corpus) = corpus();
    ops::capture(&corpus, "first thought").unwrap();
    let entry = ops::capture(&corpus, "second thought").unwrap();
    assert_eq!(entry.line, 1);
    let before = std::fs::read_to_string(&entry.file).unwrap();

    // The file now ends before the entry's line.
    let first_line = format!("{}\n", before.lines().next().unwrap());
    std::fs::write(&entry.file, &first_line).unwrap();
    let missing = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    assert!(
        matches!(&missing, Error::InboxEntryMissing { id, file, line: 1 } if *id == entry.id && *file == entry.file),
        "{missing}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), first_line);

    // The line is there, holding other text.
    let changed_text = format!("{first_line}- [beef] 2026-09-22T08:25 someone else's\n");
    std::fs::write(&entry.file, &changed_text).unwrap();
    let changed = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    assert!(
        matches!(&changed, Error::InboxEntryChanged { id, file, line: 1 } if *id == entry.id && *file == entry.file),
        "{changed}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), changed_text);

    assert_ne!(missing.code(), changed.code());
    for error in [&missing, &changed] {
        let said = error.to_string();
        assert!(said.contains(&entry.id), "{said}");
        assert!(said.contains(&entry.file.display().to_string()), "{said}");
    }
}

/// A legacy stamp has no offset. It is read as local time, and settling the
/// entry strikes the line through with the stamp exactly as it was written.
#[test]
fn settling_a_legacy_entry_keeps_its_stamp_as_written() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a new thought").unwrap();
    let legacy = entry.file.with_file_name("2000-01.md");
    std::fs::write(&legacy, "- [abcd] 2026-09-01T08:00 old thought\n").unwrap();

    let old = corpus.inbox_entry("abcd").unwrap();
    assert!(old.at.starts_with("2026-09-01T08:00:00"), "{}", old.at);
    assert!(
        time::OffsetDateTime::parse(&old.at, &time::format_description::well_known::Rfc3339)
            .is_ok(),
        "{}",
        old.at
    );
    ops::drop(&corpus, "abcd").unwrap();

    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        "- ~~[abcd] 2026-09-01T08:00 old thought~~ dropped\n"
    );
    assert_eq!(corpus.inbox_entry(&entry.id).unwrap().at, entry.at);
}

#[test]
fn capture_line_joins_every_line_break_into_one_space() {
    for (text, line) in [
        ("one line", "one line"),
        ("  padded  ", "padded"),
        ("a\nb\n", "a b"),
        ("a\r\nb\r\n", "a b"),
        ("a\rb", "a b"),
        ("a  \n\n\t  b", "a b"),
        ("\n\nleading and trailing\n\n", "leading and trailing"),
        ("inner  spacing\tstays\nput", "inner  spacing\tstays put"),
        ("", ""),
        (" \n\r\n\t ", ""),
    ] {
        assert_eq!(store::capture_line(text), line, "joining {text:?}");
    }
}

#[test]
fn multiline_capture_is_stored_as_one_inbox_line() {
    let (_dir, corpus) = corpus();
    let existing = ops::capture(&corpus, "already here").unwrap();

    let entry = ops::capture(&corpus, "first line\nsecond line\r\nthird\rfourth\n").unwrap();

    assert_eq!(entry.text, "first line second line third fourth");
    let raw = std::fs::read_to_string(&entry.file).unwrap();
    assert_eq!(raw.lines().count(), 2, "one line per entry:\n{raw}");
    assert_eq!(entry.line, 1);
    assert_eq!(
        raw.lines().nth(entry.line).unwrap(),
        format!("- [{}] {} {}", entry.id, entry.at, entry.text)
    );
    assert_eq!(corpus.inbox().unwrap().0.len(), 2);
    assert_eq!(corpus.inbox_entry(&entry.id).unwrap().text, entry.text);
    assert_eq!(
        corpus.inbox_entry(&existing.id).unwrap().text,
        existing.text
    );
}

#[test]
fn whitespace_only_capture_is_refused_without_writing() {
    let (_dir, corpus) = corpus();
    let existing = ops::capture(&corpus, "already here").unwrap();
    let before = std::fs::read_to_string(&existing.file).unwrap();

    for text in ["", "   ", "\n", " \r\n\t\n "] {
        let error = ops::capture(&corpus, text).unwrap_err();
        assert!(
            matches!(&error, Error::EmptyCapture),
            "capturing {text:?}: {error}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&existing.file).unwrap(),
        before,
        "a refused capture must not change the inbox file"
    );
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

/// Each of these rules is spelled once, here, so every surface reports the
/// same code for it; `neb`'s test of the same name checks the CLI agrees
/// (STD-02 §R24). None of them touches the node.
#[test]
fn blank_capture_note_and_open_status_reason_are_typed_refusals() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "An idea", &[]);
    let node_file = dir.path().join("corpus/nodes").join(format!("{id}.md"));
    let before = std::fs::read_to_string(&node_file).unwrap();

    let capture = ops::capture(&corpus, "   ").unwrap_err();
    assert!(matches!(capture, Error::EmptyCapture), "{capture}");
    assert_eq!(capture.code(), "empty_capture");
    assert!(matches!(
        nebula_core::store::validate_capture(" \n\t "),
        Err(Error::EmptyCapture)
    ));
    assert_eq!(
        nebula_core::store::validate_capture(" a\n b ").unwrap(),
        "a b"
    );

    let note = ops::note(&corpus, &id, "  \n ", None).unwrap_err();
    assert!(matches!(note, Error::EmptyNote), "{note}");
    assert_eq!(note.code(), "empty_note");

    for open in [Status::Seed, Status::Hypothesis] {
        let reason = ops::set_status(&corpus, &id, open, Some("y")).unwrap_err();
        assert!(
            matches!(reason, Error::ReasonOnOpenStatus(status) if status == open),
            "{reason}"
        );
        assert_eq!(reason.code(), "reason_on_open_status");
    }
    // The arguments decide it, so a node that does not exist is no excuse.
    assert!(matches!(
        ops::set_status(&corpus, "nope", Status::Seed, Some("y")),
        Err(Error::ReasonOnOpenStatus(Status::Seed))
    ));

    assert_eq!(std::fs::read_to_string(&node_file).unwrap(), before);
    assert!(corpus.inbox().unwrap().0.is_empty());
}

#[test]
fn capture_refuses_an_exhausted_id_namespace_without_writing() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "locate the current inbox file").unwrap();
    let mut fixture = String::new();
    for id in 0..=u16::MAX {
        writeln!(fixture, "- [{id:04x}] 2000-01-01T00:00 occupied").unwrap();
    }
    std::fs::write(&entry.file, &fixture).unwrap();

    let error = ops::capture(&corpus, "there is no id left").unwrap_err();

    assert!(matches!(error, Error::InboxIdsExhausted));
    assert_eq!(
        std::fs::read_to_string(&entry.file).unwrap(),
        fixture,
        "exhaustion must be detected before appending"
    );
}

#[test]
fn capture_after_an_unterminated_record_stays_independent_and_promotes() {
    let (_dir, corpus) = corpus();
    let first = ops::capture(&corpus, "the first thought").unwrap();
    let unterminated = std::fs::read_to_string(&first.file)
        .unwrap()
        .trim_end_matches('\n')
        .to_string();
    std::fs::write(&first.file, unterminated).unwrap();

    let second = ops::capture(&corpus, "the second thought").unwrap();
    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), 2);
    assert_eq!(corpus.inbox_entry(&first.id).unwrap().text, first.text);
    assert_eq!(corpus.inbox_entry(&second.id).unwrap().text, second.text);

    let promoted = ops::promote(
        &corpus,
        &second.id,
        &Promotion {
            title: Some("Second thought".into()),
            ..Promotion::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(promoted.doc.body.trim(), "the second thought");
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
    assert_eq!(corpus.inbox_entry(&first.id).unwrap().text, first.text);
}

#[test]
fn a_repeated_capture_names_the_earliest_entry_still_waiting() {
    let (_dir, corpus) = corpus();
    let first = ops::capture(&corpus, "Tags beat  domains").unwrap();
    let unrelated = ops::capture(&corpus, "tags beat a domain").unwrap();
    let second = ops::capture(&corpus, "tags BEAT domains").unwrap();
    let third = ops::capture(&corpus, "  TAGS\tbeat domains ").unwrap();

    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), 4, "a duplicate is still captured");
    let same = |entry| inbox.same_as(entry).map(|e| e.id.clone());
    assert_eq!(same(&second), Some(first.id.clone()), "case is folded");
    assert_eq!(same(&third), Some(first.id.clone()), "whitespace is folded");
    assert_eq!(
        same(&unrelated),
        None,
        "a different word is a different thought"
    );
}

#[test]
fn a_settled_capture_is_not_a_duplicate() {
    let (_dir, corpus) = corpus();
    let dropped = ops::capture(&corpus, "a thought once dropped").unwrap();
    ops::drop(&corpus, &dropped.id).unwrap();
    let promoted = ops::capture(&corpus, "a thought once promoted").unwrap();
    ops::promote(&corpus, &promoted.id, &Promotion::default(), 0).unwrap();

    let again = [
        ops::capture(&corpus, "a thought once dropped").unwrap(),
        ops::capture(&corpus, "A thought once promoted").unwrap(),
    ];

    let inbox = corpus.inbox().unwrap();
    for entry in &again {
        assert!(
            inbox.same_as(entry).is_none(),
            "only waiting entries count: {entry:?}"
        );
    }
}

#[test]
fn promote_and_drop_on_a_settled_entry_say_how_it_was_settled() {
    let (dir, corpus) = corpus();
    let kept = ops::capture(&corpus, "worth keeping").unwrap();
    let node = ops::promote(&corpus, &kept.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let tossed = ops::capture(&corpus, "not worth keeping").unwrap();
    ops::drop(&corpus, &tossed.id).unwrap();
    let inbox_before = std::fs::read_to_string(&kept.file).unwrap();
    let nodes_before = corpus.load_all().unwrap().len();

    for error in [
        ops::promote(&corpus, &kept.id, &Promotion::default(), 0).unwrap_err(),
        ops::drop(&corpus, &kept.id).unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            format!("`{}` was already promoted to `{node}`", kept.id)
        );
        assert!(matches!(
            error,
            Error::InboxEntrySettled { id, settlement: Settlement::Promoted(to) }
                if id == kept.id && to == node
        ));
    }
    for error in [
        ops::promote(&corpus, &tossed.id, &Promotion::default(), 0).unwrap_err(),
        ops::drop(&corpus, &tossed.id).unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            format!("`{}` was already dropped", tossed.id)
        );
        assert!(matches!(
            error,
            Error::InboxEntrySettled { id, settlement: Settlement::Dropped } if id == tossed.id
        ));
    }
    assert!(matches!(
        ops::drop(&corpus, "zzzz"),
        Err(Error::NoSuchInboxEntry(id)) if id == "zzzz"
    ));

    assert_eq!(
        std::fs::read_to_string(&kept.file).unwrap(),
        inbox_before,
        "a refusal writes nothing to the inbox"
    );
    assert_eq!(corpus.load_all().unwrap().len(), nodes_before);
    assert!(
        dir.path()
            .join("corpus/nodes")
            .join(format!("{node}.md"))
            .is_file()
    );
}

/// Ids are unique among waiting entries only, so a struck-through line can
/// share its id with a later one, and a hand edit can leave an outcome that
/// no verb wrote.
#[test]
fn a_settled_id_resolves_to_its_latest_recorded_outcome() {
    let (_dir, corpus) = corpus();
    let live = ops::capture(&corpus, "waiting").unwrap();
    std::fs::write(
        live.file.with_file_name("2000-01.md"),
        format!(
            "- ~~[abcd] 2000-01-01T00:00 first~~ dropped\n\
             - ~~[abcd] 2000-01-02T00:00 a ~~struck~~ word~~ -> second-node\n\
             - ~~[beef] 2000-01-03T00:00 edited~~ merged elsewhere\n\
             - ~~[f00d] 2000-01-04T00:00 empty~~ ->\n\
             - ~~[{}] 2000-01-05T00:00 older~~ dropped\n",
            live.id
        ),
    )
    .unwrap();

    assert!(matches!(
        corpus.inbox_entry("abcd"),
        Err(Error::InboxEntrySettled { settlement: Settlement::Promoted(node), .. })
            if node == "second-node"
    ));
    for unrecognised in ["beef", "f00d"] {
        assert!(matches!(
            corpus.inbox_entry(unrecognised),
            Err(Error::NoSuchInboxEntry(id)) if id == unrecognised
        ));
    }
    assert_eq!(
        corpus.inbox_entry(&live.id).unwrap().text,
        "waiting",
        "a waiting entry wins over a settled one with its id"
    );
}

// -------------------------------------------------------------------- near --

/// A small corpus with vocabulary that overlaps in known ways: two nodes
/// about taxonomy and tags, one about search ranking, one about nothing
/// the queries below mention.
fn lexical_fixture(corpus: &Corpus) {
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
        corpus.inbox_entry(&entry.id).unwrap().text,
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
        corpus.inbox_entry(&second.id).unwrap().text,
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
        matches!(
            corpus.inbox_entry(&entry.id),
            Err(Error::InboxEntrySettled { settlement: Settlement::Promoted(n), .. }) if n == node
        ),
        "the entry reads as promoted, not dropped"
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
    assert_eq!(corpus.inbox_entry(&entry.id).unwrap().text, entry.text);
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
    assert!(corpus.inbox_entry(&entry.id).is_ok());
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

// ------------------------------------------------------------------ triage --

/// Rewrite one capture's stamp in place, found by its text, so a test can
/// put captures out of file order without waiting for real time to pass.
fn restamp(dir: &tempfile::TempDir, text: &str, stamp: &str) {
    let inbox = dir.path().join("corpus/inbox");
    for file in std::fs::read_dir(&inbox).unwrap() {
        let path = file.unwrap().path();
        let raw = std::fs::read_to_string(&path).unwrap();
        let Some(line) = raw.lines().find(|l| l.ends_with(text)) else {
            continue;
        };
        let (head, _) = line.split_once("] ").unwrap();
        let rewritten = format!("{head}] {stamp} {text}");
        std::fs::write(&path, raw.replace(line, &rewritten)).unwrap();
        return;
    }
    panic!("no capture `{text}`");
}

/// Every inbox file's text, settled lines included.
fn inbox_text(dir: &tempfile::TempDir) -> String {
    std::fs::read_dir(dir.path().join("corpus/inbox"))
        .unwrap()
        .map(|f| std::fs::read_to_string(f.unwrap().path()).unwrap())
        .collect()
}

/// The id of the entry triage is on now.
fn on(session: &mut Triage, corpus: &Corpus) -> Option<String> {
    session.current(corpus).unwrap().map(|w| w.entry.id.clone())
}

#[test]
fn triage_walks_the_inbox_oldest_first_through_promote_and_drop() {
    let (dir, corpus) = corpus();
    lexical_fixture(&corpus);
    // Captured in one order, stamped in another: triage follows the stamps.
    let newest = ops::capture(&corpus, "buy more coffee").unwrap();
    let oldest = ops::capture(&corpus, "a taxonomy for tags").unwrap();
    let middle = ops::capture(&corpus, "ranking decay again").unwrap();
    restamp(&dir, "buy more coffee", "2026-01-03T09:00");
    restamp(&dir, "a taxonomy for tags", "2026-01-01T09:00");
    restamp(&dir, "ranking decay again", "2026-01-02T09:00");

    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(session.total(), 3);
    let first = session.current(&corpus).unwrap().unwrap().clone();
    assert_eq!(first.entry.id, oldest.id);
    assert_eq!((first.position, first.total), (1, 3));
    assert!(first.days.is_some_and(|d| d > 0), "{:?}", first.days);
    assert_eq!(
        first.near.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["a-single-global-taxonomy", "tags-beat-domains"],
        "the candidates are `near` over the captured text"
    );

    // `p`: a root, whatever the candidates were.
    let Step::Promoted { entry, created } = session.apply(&corpus, Action::Promote).unwrap() else {
        panic!("expected a promotion")
    };
    assert_eq!(entry.id, oldest.id);
    assert!(created.doc.node.edges.is_empty(), "nothing links unasked");
    assert!(created.near.is_empty(), "the candidates were shown already");
    assert_eq!(
        corpus.load(&created.doc.node.id).unwrap().body.trim(),
        entry.text
    );

    // A number: promoted under that candidate, and only that one.
    let second = session.current(&corpus).unwrap().unwrap().clone();
    assert_eq!(second.entry.id, middle.id);
    assert_eq!(second.near[0].id, "ranking-decay-half-life");
    let Step::Promoted { created, .. } = session.apply(&corpus, Action::PromoteUnder(1)).unwrap()
    else {
        panic!("expected a promotion")
    };
    let on_disk = corpus.load(&created.doc.node.id).unwrap();
    assert_eq!(
        on_disk.node.parents().collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );

    // `d`: struck through, exactly as `drop` does.
    assert_eq!(on(&mut session, &corpus), Some(newest.id.clone()));
    let Step::Dropped { entry } = session.apply(&corpus, Action::Drop).unwrap() else {
        panic!("expected a drop")
    };
    assert_eq!(entry.id, newest.id);

    assert_eq!(on(&mut session, &corpus), None);
    assert!(corpus.inbox().unwrap().0.is_empty());
    let raw = inbox_text(&dir);
    assert!(raw.contains("~~ dropped"), "{raw}");
    assert!(
        raw.contains(&format!("~~ -> {}", created.doc.node.id)),
        "{raw}"
    );
    assert_eq!(
        session.tally(),
        Tally {
            promoted: 2,
            dropped: 1,
            skipped: 0,
            untouched: 0,
        }
    );
}

#[test]
fn triage_skip_and_quit_write_nothing_and_leave_entries_waiting() {
    let (dir, corpus) = corpus();
    let a = ops::capture(&corpus, "first thought").unwrap();
    let b = ops::capture(&corpus, "second thought").unwrap();
    let c = ops::capture(&corpus, "third thought").unwrap();
    let before = inbox_text(&dir);

    let mut session = Triage::start(&corpus, None).unwrap();
    let Step::Skipped { entry } = session.apply(&corpus, Action::Skip).unwrap() else {
        panic!("expected a skip")
    };
    assert_eq!(entry.id, a.id);
    assert_eq!(on(&mut session, &corpus), Some(b.id.clone()));
    assert!(matches!(
        session.apply(&corpus, Action::Quit).unwrap(),
        Step::Quit
    ));
    assert_eq!(on(&mut session, &corpus), None, "quit ends the session");
    assert!(
        matches!(session.apply(&corpus, Action::Drop).unwrap(), Step::Quit),
        "after quit there is nothing left to act on"
    );

    let after = inbox_text(&dir);
    assert_eq!(before, after, "skip and quit write nothing");
    let waiting: Vec<_> = corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(waiting, [a.id, b.id, c.id]);
    assert_eq!(
        session.tally(),
        Tally {
            promoted: 0,
            dropped: 0,
            skipped: 1,
            untouched: 2,
        }
    );
}

#[test]
fn triage_refuses_a_candidate_it_did_not_show_and_stays_on_the_entry() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let entry = ops::capture(&corpus, "a taxonomy for tags").unwrap();
    let mut session = Triage::start(&corpus, None).unwrap();
    let shown = session.current(&corpus).unwrap().unwrap().near.len();
    assert_eq!(shown, 2);

    for number in [0, shown + 1] {
        let refused = session
            .apply(&corpus, Action::PromoteUnder(number))
            .unwrap_err();
        assert!(
            matches!(refused, Error::NoSuchCandidate { number: n, shown: 2 } if n == number),
            "{refused:?}"
        );
    }
    assert_eq!(on(&mut session, &corpus), Some(entry.id.clone()));
    assert_eq!(corpus.load_all().unwrap().len(), 4, "nothing was written");
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

#[test]
fn triage_titles_the_promotion_and_a_refusal_keeps_the_entry_to_retry() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the captured words").unwrap();
    let mut session = Triage::start(&corpus, Some("agent-x".into())).unwrap();

    // A title that makes no id is `promote`'s own refusal, and nothing moves.
    assert!(matches!(
        session.apply(&corpus, Action::Title("  !!!  ".into())).unwrap(),
        Step::Titled { title: Some(t) } if t == "!!!"
    ));
    let refused = session.apply(&corpus, Action::Promote).unwrap_err();
    assert!(matches!(refused, Error::UnusableTitle(_)), "{refused:?}");
    let waiting = session.current(&corpus).unwrap().unwrap();
    assert_eq!(waiting.entry.id, entry.id);
    assert_eq!(waiting.title.as_deref(), Some("!!!"), "the title is kept");

    // A blank title goes back to the captured text; a real one is used.
    assert!(matches!(
        session.apply(&corpus, Action::Title("   ".into())).unwrap(),
        Step::Titled { title: None }
    ));
    session
        .apply(&corpus, Action::Title("A better title".into()))
        .unwrap();
    let Step::Promoted { created, .. } = session.apply(&corpus, Action::Promote).unwrap() else {
        panic!("expected a promotion")
    };
    let node = corpus.load(&created.doc.node.id).unwrap().node;
    assert_eq!(node.id, "a-better-title");
    assert_eq!(node.title, "A better title");
    assert_eq!(
        node.title_by.as_deref(),
        Some("agent-x"),
        "`by` is attributed as `promote --by` would"
    );
    assert_eq!(on(&mut session, &corpus), None);
}

#[test]
fn triage_moves_past_an_entry_settled_elsewhere_and_says_so() {
    let (_dir, corpus) = corpus();
    let gone = ops::capture(&corpus, "settled by someone else").unwrap();
    let next = ops::capture(&corpus, "still here").unwrap();
    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(on(&mut session, &corpus), Some(gone.id.clone()));

    ops::drop(&corpus, &gone.id).unwrap();
    let refused = session.apply(&corpus, Action::Drop).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::InboxEntrySettled { id, settlement: Settlement::Dropped } if *id == gone.id
        ),
        "{refused:?}"
    );
    assert_eq!(on(&mut session, &corpus), Some(next.id));
    assert_eq!(session.tally().dropped, 0, "it was not this session's drop");
}

#[test]
fn triage_of_an_empty_inbox_has_nothing_to_decide() {
    let (_dir, corpus) = corpus();
    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(session.total(), 0);
    assert!(session.current(&corpus).unwrap().is_none());
    assert_eq!(session.tally(), Tally::default());
}

#[test]
fn opening_a_missing_corpus_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere");
    assert!(matches!(
        Corpus::open(&process_locations(), Some(missing.clone())),
        Err(Error::NoCorpus(p)) if p == missing
    ));
}

#[test]
fn open_or_init_says_whether_it_created_the_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");

    let (_, created) =
        Corpus::open_or_init(&process_locations(), Some(root.clone())).expect("first open");
    assert!(created, "there was no corpus, so this call made one");
    assert!(root.join("nodes").is_dir() && root.join("inbox").is_dir());

    let (_, created) = Corpus::open_or_init(&process_locations(), Some(root)).expect("second open");
    assert!(!created, "an existing corpus is opened, not created");
}

#[test]
fn discover_finds_the_nearest_corpus_at_or_above_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let inner = outer.join("projects").join("inner");
    Corpus::init(&process_locations(), &outer).unwrap();
    Corpus::init(&process_locations(), &inner).unwrap();
    let deep = inner.join("notes").join("deep");
    std::fs::create_dir_all(&deep).unwrap();

    assert_eq!(Corpus::discover(&outer), Some(outer.clone()));
    assert_eq!(Corpus::discover(&outer.join("nodes")), Some(outer.clone()));
    assert_eq!(Corpus::discover(&outer.join("projects")), Some(outer));
    assert_eq!(Corpus::discover(&inner.join("nodes")), Some(inner.clone()));
    assert_eq!(Corpus::discover(&deep), Some(inner));
    assert_eq!(Corpus::discover(dir.path()), None);
    // The start need not exist: the walk is over the path, not the disk.
    assert_eq!(
        Corpus::discover(&deep.join("not-yet")),
        Corpus::discover(&deep)
    );
}

#[test]
fn discover_requires_nodes_beside_a_config_that_names_a_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let cases: [(&str, bool, Option<&str>); 6] = [
        ("nodes-only", true, None),
        (
            "config-only",
            false,
            Some("schema_version: 2\ncorpus_id: neb-000001\n"),
        ),
        ("no-corpus-id", true, Some("schema_version: 2\n")),
        (
            "empty-corpus-id",
            true,
            Some("schema_version: 2\ncorpus_id: ''\n"),
        ),
        ("not-yaml", true, Some("corpus_id: [unclosed\n")),
        // An older schema still counts, so `neb migrate` works from inside.
        (
            "v1",
            true,
            Some("schema_version: 1\ncorpus_id: neb-000001\ndomains: []\n"),
        ),
    ];
    for (name, nodes, config) in cases {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        if nodes {
            std::fs::create_dir(root.join("nodes")).unwrap();
        }
        if let Some(config) = config {
            std::fs::write(root.join("config.yaml"), config).unwrap();
        }
        let expected = (name == "v1").then(|| root.clone());
        assert_eq!(Corpus::discover(&root), expected, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn discover_walks_the_path_as_spelled_and_never_resolves_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("real").join("corpus");
    Corpus::init(&process_locations(), &root).unwrap();

    // Through a symlinked parent, the root comes back in the alias spelling.
    let alias = dir.path().join("alias");
    symlink(dir.path().join("real"), &alias).unwrap();
    assert_eq!(
        Corpus::discover(&alias.join("corpus").join("nodes")),
        Some(alias.join("corpus"))
    );

    // A link into the corpus from outside is a directory outside it, the way
    // `cd ..` from there leaves it: the walk does not follow the link back.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    symlink(root.join("nodes"), elsewhere.join("ideas")).unwrap();
    assert_eq!(Corpus::discover(&elsewhere.join("ideas")), None);
}

/// A symlinked `nodes/` is found, not walked past, so opening it refuses by
/// name rather than resolution quietly settling on some other corpus.
#[cfg(unix)]
#[test]
fn discover_finds_a_corpus_whose_nodes_is_a_symlink_so_open_can_refuse_it() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    Corpus::init(&process_locations(), &outer).unwrap();
    let root = outer.join("linked");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "schema_version: 2\ncorpus_id: neb-000001\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(outer.join("nodes"), root.join("nodes")).unwrap();

    assert_eq!(Corpus::discover(&root), Some(root.clone()));
    let refused =
        Corpus::open(&process_locations(), Some(root)).expect_err("a symlinked nodes/ is refused");
    assert!(refused.to_string().contains("is a symlink"), "{refused}");
}

#[cfg(unix)]
#[test]
fn a_corpus_that_cannot_be_created_names_the_root_it_aimed_at() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("read-only");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let root = parent.join("corpus");

    let init = Corpus::init(&process_locations(), &root).map(|_| ());
    let open_or_init = Corpus::open_or_init(&process_locations(), Some(root.clone())).map(|_| ());
    let ops_init =
        ops::init(&process_locations(), Some(root.clone()), None, false, false).map(|_| ());
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    for result in [init, open_or_init, ops_init] {
        assert!(
            matches!(
                &result,
                Err(Error::IoAt { action: "creating", path, source })
                    if *path == root && source.kind() == std::io::ErrorKind::PermissionDenied
            ),
            "expected a creation failure naming {}, got {result:?}",
            root.display()
        );
    }
    assert!(!root.exists());
}

/// `init` given a root and a path that name different directories refuses
/// before creating either (STD-01 §R28). One directory spelled two ways, with
/// a `.` segment or a trailing slash, is one corpus.
#[test]
fn init_refuses_a_root_and_a_path_that_name_different_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));

    let refused = ops::init(
        &process_locations(),
        Some(a.clone()),
        Some(b.clone()),
        false,
        false,
    )
    .map(|_| ());
    assert!(
        matches!(&refused, Err(Error::RootAndPathDiffer { root, path }) if *root == a && *path == b),
        "{refused:?}"
    );
    assert_eq!(refused.unwrap_err().code(), "root_and_path_differ");
    assert!(!a.exists() && !b.exists(), "neither is created");

    let spelled = dir.path().join(".").join("a").join("");
    assert_eq!(
        ops::init_target(&process_locations(), Some(a.clone()), Some(spelled.clone())).unwrap(),
        spelled
    );
    ops::init(
        &process_locations(),
        Some(a.clone()),
        Some(spelled),
        false,
        false,
    )
    .unwrap();
    assert!(a.join("nodes").is_dir());
}

/// Set in a child copy of this test binary that [`in_own_process`] started.
const IN_OWN_PROCESS: &str = "NEBULA_CORE_TEST_IN_OWN_PROCESS";

/// Whether the caller is the child copy of this test binary that runs `test`
/// alone. In the parent, start that child, require it to pass, and say no.
///
/// Machine settings live under `HOME`, and every test in this process shares
/// its one isolated home (`support`), so a test that writes
/// `~/.config/nebula` would race the tests beside it. `support` gives each
/// test process a fresh home of its own, so the child's settings are nobody
/// else's (STD-03 §R20). The child comes from the isolating builder and is
/// waited for under its guard and deadline.
#[cfg(unix)]
fn in_own_process(test: &str) -> bool {
    if std::env::var_os(IN_OWN_PROCESS).is_some() {
        return true;
    }
    let mut cmd = support::command(
        std::env::current_exe().expect("test binary"),
        support::home(),
    );
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(IN_OWN_PROCESS, "1");
    let out = support::output(&mut cmd, support::DEADLINE).unwrap_or_else(|e| panic!("{e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "the child run of {test} did not pass:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// `~/.config/nebula/root` is replaced whole, like every other file nebula
/// writes. Written by name, as it once was, a symlink there carried the new
/// path into whatever file it pointed at and stayed a symlink.
#[cfg(unix)]
#[test]
fn set_root_replaces_a_symlinked_setting_instead_of_writing_through() {
    use std::os::unix::fs::PermissionsExt;

    if !in_own_process("set_root_replaces_a_symlinked_setting_instead_of_writing_through") {
        return;
    }

    let settings = support::home().join(".config").join("nebula");
    std::fs::create_dir_all(&settings).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside-target");
    std::fs::write(&outside, "/an/older/corpus\n").unwrap();
    let setting = settings.join("root");
    std::os::unix::fs::symlink(&outside, &setting).unwrap();
    let target = dir.path().join("new-corpus");

    ops::init(&process_locations(), None, Some(target.clone()), true, true)
        .expect("init --set-root --force");

    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"/an/older/corpus\n",
        "the write went through the symlink"
    );
    let metadata = std::fs::symlink_metadata(&setting).unwrap();
    assert!(
        metadata.file_type().is_file(),
        "the setting is still a symlink"
    );
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        std::fs::read_to_string(&setting).unwrap(),
        format!("{}\n", target.display())
    );
}

/// Every entry under `root`, not following symlinks: a file's bytes, a
/// directory as `dir`, a symlink as `-> target`. What a test compares before
/// and after a call that must write nothing.
fn every_file(root: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            let content = if kind.is_symlink() {
                format!("-> {}", std::fs::read_link(&path).unwrap().display()).into_bytes()
            } else if kind.is_dir() {
                pending.push(path.clone());
                b"dir".to_vec()
            } else {
                std::fs::read(&path).unwrap()
            };
            out.insert(path, content);
        }
    }
    out
}

/// A current-schema node file, as `neb new` would write it.
const V2_NODE: &str = "---\nid: an-idea\ntitle: An idea\nstatus: seed\ncreated: 2026-09-01\nupdated: 2026-09-01\n---\n\nThe idea.\n";

/// A corpus written by v0.1, before `config.yaml` existed: one node in the
/// v1 shape and nothing else.
const V1_NODE: &str = "---\nid: first\ntitle: First\ndomain: Physics\nstatus: testing\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe first.\n";

/// A root with `nodes/` holding `node` as `<id>.md`, and no `config.yaml`.
fn configless(dir: &std::path::Path, id: &str, node: &str) -> std::path::PathBuf {
    let root = dir.join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::write(root.join("nodes").join(format!("{id}.md")), node).unwrap();
    root
}

/// A missing `config.yaml` is refused, not synthesized: opening is a read,
/// and a read that wrote a fresh config would stamp this build's schema over
/// files that may predate it and mint an id the corpus never had.
#[test]
fn open_never_writes_a_missing_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "an-idea", V2_NODE);
    let before = every_file(&root);

    let refused = Corpus::open(&process_locations(), Some(root.clone()));
    assert!(
        matches!(&refused, Err(Error::MissingConfig { path }) if *path == root.join("config.yaml")),
        "{refused:?}"
    );
    assert_eq!(before, every_file(&root));
}

/// The corpora this refusal exists for are still one command from working:
/// `migrate` reads a missing config as a corpus from before the file.
#[test]
fn v1_corpus_without_config_still_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "first", V1_NODE);
    let before = every_file(&root);

    assert!(matches!(
        Corpus::open(&process_locations(), Some(root.clone())),
        Err(Error::MissingConfig { .. })
    ));
    assert_eq!(before, every_file(&root), "a refused open wrote something");

    let report = nebula_core::migrate::run(&process_locations(), Some(root.clone())).unwrap();
    assert!(report.config_rewritten);
    assert!(report.minted_corpus_id.is_some(), "{report:?}");
    assert_eq!(
        report
            .rewritten
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    let node = std::fs::read_to_string(root.join("nodes").join("first.md")).unwrap();
    assert!(node.contains("tags:\n- physics\n"), "{node}");
    assert!(node.contains("status: hypothesis"), "{node}");

    let corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert_eq!(corpus.load_all().unwrap().len(), 1);
}

/// Absent means `NotFound`. A `config.yaml` that is there but cannot be
/// followed is a different fault, named as the path it is, and the symlink is
/// left for whoever made it.
#[cfg(unix)]
#[test]
fn a_dangling_config_symlink_is_not_a_missing_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "an-idea", V2_NODE);
    let config = root.join("config.yaml");
    std::os::unix::fs::symlink(dir.path().join("gone.yaml"), &config).unwrap();

    let refused = Corpus::open(&process_locations(), Some(root.clone()))
        .expect_err("a dangling config is refused");
    assert!(
        !matches!(refused, Error::MissingConfig { .. }),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("config.yaml"), "{refused}");
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!dir.path().join("gone.yaml").exists());
}

/// A symlinked `config.yaml` handed the corpus another file's settings, and
/// the next config write replaced the link; a FIFO there hung every verb.
/// Both are refused by name, before anything is read, and a config write
/// that re-reads under the lock refuses the same way.
#[cfg(unix)]
#[test]
fn a_symlinked_or_fifo_config_yaml_is_refused() {
    use nebula_core::fs::EntryKind;
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    let outside = dir.path().join("outside.yaml");
    std::fs::rename(&config, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &config).unwrap();
    let before = std::fs::read(&outside).unwrap();

    let opened = Corpus::open(&process_locations(), Some(root.clone()));
    assert!(
        not_regular(&opened, &config, EntryKind::Symlink),
        "{opened:?}"
    );
    let set = ops::set_commit(&mut corpus, true);
    assert!(not_regular(&set, &config, EntryKind::Symlink), "{set:?}");
    let initialized = Corpus::init(&process_locations(), &root);
    assert!(
        not_regular(&initialized, &config, EntryKind::Symlink),
        "{initialized:?}"
    );
    // Found, not walked past: opening from inside refuses it by name rather
    // than opening some other corpus further up.
    assert_eq!(Corpus::discover(&root.join("nodes")), Some(root.clone()));
    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );

    std::fs::remove_file(&config).unwrap();
    mkfifo(&config);
    let opened = within_two_seconds({
        let root = root.clone();
        move || Corpus::open(&process_locations(), Some(root))
    });
    assert!(not_regular(&opened, &config, EntryKind::Fifo), "{opened:?}");
    let discovered = within_two_seconds({
        let nodes = root.join("nodes");
        move || Corpus::discover(&nodes)
    });
    assert_eq!(discovered, Some(root));
}

/// `init` finishes a root it was interrupted in, where nothing has been
/// written yet, and refuses one that already holds content: minting a
/// config there is the same fabrication `open` no longer does.
#[test]
fn init_completes_an_interrupted_init_but_refuses_a_configless_corpus() {
    let dir = tempfile::tempdir().unwrap();

    let interrupted = dir.path().join("interrupted");
    std::fs::create_dir_all(interrupted.join("nodes")).unwrap();
    Corpus::init(&process_locations(), &interrupted).unwrap();
    assert!(interrupted.join("inbox").is_dir());
    let config = std::fs::read_to_string(interrupted.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 2"), "{config}");
    assert!(config.contains("corpus_id: neb-"), "{config}");
    Corpus::open(&process_locations(), Some(interrupted)).unwrap();

    let with_content = configless(&dir.path().join("content"), "an-idea", V2_NODE);
    let before = every_file(&with_content);
    let refused = Corpus::init(&process_locations(), &with_content);
    assert!(
        matches!(&refused, Err(Error::MissingConfig { path }) if *path == with_content.join("config.yaml")),
        "{refused:?}"
    );
    assert_eq!(
        before,
        every_file(&with_content),
        "a refused init created something"
    );

    // Captures are content too: a corpus that has only ever been captured
    // into has an identity of its own to lose.
    let captured = dir.path().join("captured");
    std::fs::create_dir_all(captured.join("nodes")).unwrap();
    std::fs::create_dir_all(captured.join("inbox")).unwrap();
    std::fs::write(
        captured.join("inbox").join("2026-09.md"),
        "- [0a1b] 2026-09-01T10:00:00Z a thought\n",
    )
    .unwrap();
    let before = every_file(&captured);
    assert!(matches!(
        Corpus::init(&process_locations(), &captured),
        Err(Error::MissingConfig { .. })
    ));
    assert_eq!(before, every_file(&captured));
}

/// A writer re-reads `config.yaml` under its lock. When it is gone, that is
/// a refusal: bringing it back with the id in hand would still turn a
/// `commit: true` it no longer holds into off.
#[test]
fn a_config_deleted_under_a_writer_is_refused_not_recreated() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    ops::set_commit(&mut corpus, true).unwrap();
    let mut corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert!(corpus.commit_setting().enabled);
    std::fs::remove_file(&config).unwrap();

    let committed = ops::commit(&corpus, "capture", &["x"]);
    assert!(
        matches!(&committed, Err(Error::MissingConfig { path }) if *path == config),
        "{committed:?}"
    );
    assert!(!config.exists());

    for enabled in [true, false] {
        let set = ops::set_commit(&mut corpus, enabled);
        assert!(
            matches!(&set, Err(Error::MissingConfig { path }) if *path == config),
            "{set:?}"
        );
        assert!(!config.exists());
    }
}

// ------------------------------------------------------------------ commit --

/// Run git in `dir`, asserting it succeeded; stdout as text.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = support::output(
        support::git_command(dir, support::home()).args(args),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository at `dir` with an identity, so `neb`'s commits do not depend
/// on the developer's global git configuration.
fn git_init(dir: &std::path::Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "neb-test"]);
    git(dir, &["config", "user.email", "neb-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Every child this suite starts comes from the isolating builder in
/// `support`, which is the one place a `Command` is created.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    for (file, source, allowed) in [
        ("core.rs", include_str!("core.rs"), &[][..]),
        (
            "support/mod.rs",
            include_str!("support/mod.rs"),
            &["command"][..],
        ),
    ] {
        let strays = support::commands_outside(source, allowed);
        assert!(
            strays.is_empty(),
            "{file} creates a child outside `support::command`:\n{}",
            strays.join("\n")
        );
    }
}

/// The paths a commit touched, relative to the repository's top level.
fn committed_paths(dir: &std::path::Path, rev: &str) -> Vec<String> {
    git(dir, &["show", "--name-only", "--format=", rev])
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn head(dir: &std::path::Path) -> String {
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

/// The commit a write made, failing the test on any other outcome.
fn committed(outcome: Result<CommitOutcome, Error>) -> Committed {
    match outcome {
        Ok(CommitOutcome::Committed(done)) => done,
        other => panic!("expected a commit, got {other:?}"),
    }
}

/// Each way a write can end up uncommitted is its own outcome, and a real
/// change is a commit named for the verb and the ids it touched.
#[test]
fn commit_outcome_distinguishes_disabled_not_a_repo_and_nothing_to_commit() {
    // Outside any repository: off is `Disabled`, on is `NotARepository`.
    let (_plain, mut plain) = corpus();
    let a = seed(&plain, "A", &[]);
    assert!(matches!(
        ops::commit(&plain, "new", &[&a]),
        Ok(CommitOutcome::Disabled)
    ));
    ops::set_commit(&mut plain, true).unwrap();
    assert!(matches!(
        ops::commit(&plain, "new", &[&a]),
        Ok(CommitOutcome::NotARepository)
    ));

    // In a repository: a real change commits, and the same call again finds
    // nothing left to record.
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    let b = seed(&corpus, "B", &[]);
    let done = committed(ops::commit(&corpus, "new", &[&b]));
    assert_eq!(done.message, format!("neb new {b}"));
    assert_eq!(done.hash, head(&root));
    assert!(matches!(
        ops::commit(&corpus, "new", &[&b]),
        Ok(CommitOutcome::NothingToCommit)
    ));
    assert_eq!(head(&root), done.hash);
}

#[test]
fn commit_is_off_by_default_and_off_means_nothing_is_committed() {
    // Not a repository at all: with the setting off, git is never consulted.
    let (_plain, plain) = corpus();
    assert!(!plain.commit_setting().enabled);
    seed(&plain, "A", &[]);
    assert!(matches!(
        ops::commit(&plain, "new", &["a"]),
        Ok(CommitOutcome::Disabled)
    ));

    // In a repository, off still means the write is left uncommitted.
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::commit(&corpus, "new", &["a"]),
        Ok(CommitOutcome::Disabled)
    ));
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? nodes/"),
        "the write is there, not staged, not committed"
    );

    // The setting is a `config.yaml` key that is absent until turned on, so
    // a file written before it existed renders back unchanged.
    let config = || std::fs::read_to_string(root.join("config.yaml")).unwrap();
    assert!(!config().contains("commit"), "{}", config());
    assert!(ops::set_commit(&mut corpus, true).unwrap().enabled);
    assert!(config().contains("commit: true"), "{}", config());
    assert!(!ops::set_commit(&mut corpus, false).unwrap().enabled);
    assert!(!config().contains("commit"), "{}", config());
}

#[test]
fn commit_on_records_each_write_as_one_commit_of_corpus_paths_only() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    let start = committed(ops::commit(&corpus, "config", &["commit"]));
    assert_eq!(start.message, "neb config commit");
    assert_eq!(
        committed_paths(&root, "HEAD"),
        [".gitignore", "config.yaml"]
    );

    // A capture creates inbox/, a promote writes a node and settles the
    // capture: one commit each, naming what the verb touched.
    let entry = ops::capture(&corpus, "a thought").unwrap();
    let captured = committed(ops::commit(&corpus, "capture", &[&entry.id]));
    assert_eq!(captured.message, format!("neb capture {}", entry.id));
    assert_eq!(captured.hash, head(&root));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    let id = ops::promote(&corpus, &entry.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let promoted = committed(ops::commit(&corpus, "promote", &[&entry.id, &id]));
    assert_eq!(promoted.message, format!("neb promote {} {id}", entry.id));
    let mut paths = committed_paths(&root, "HEAD");
    paths.sort();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.iter().any(|p| p == &format!("nodes/{id}.md")));
    assert!(paths.iter().any(|p| p.starts_with("inbox/")));

    // A stray file under the root that is not a corpus path is left alone,
    // and a write that changed nothing produces no commit.
    std::fs::write(root.join("scratch.txt"), "not the corpus\n").unwrap();
    let before = head(&root);
    assert!(matches!(
        ops::commit(&corpus, "note", &[&id]),
        Ok(CommitOutcome::NothingToCommit)
    ));
    assert_eq!(head(&root), before);
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? scratch.txt"),
        "scratch.txt is neither staged nor committed"
    );
    assert!(
        git(
            &root,
            &[
                "status",
                "--porcelain",
                "--",
                "nodes",
                "inbox",
                "config.yaml"
            ]
        )
        .is_empty()
    );
    assert!(
        git(&root, &["remote"]).is_empty(),
        "nothing to push to, and nothing pushed"
    );
}

/// A `neb` commit is exactly the corpus, by pathspec: work staged elsewhere
/// in the repository neither blocks it nor rides in it, and stays staged.
#[test]
fn staged_work_outside_the_corpus_stays_staged_and_out_of_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join("README.md"), "theirs\n").unwrap();
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));
    assert_eq!(committed_paths(&outer, "HEAD"), ["corpus/config.yaml"]);

    std::fs::write(outer.join("README.md"), "theirs, edited\n").unwrap();
    std::fs::write(root.join("unrelated.txt"), "also theirs\n").unwrap();
    git(&outer, &["add", "README.md", "corpus/unrelated.txt"]);
    let a = seed(&corpus, "A", &[]);
    let done = committed(ops::commit(&corpus, "new", &[&a]));
    assert_eq!(done.message, format!("neb new {a}"));
    assert_eq!(
        committed_paths(&outer, "HEAD"),
        [format!("corpus/nodes/{a}.md")]
    );
    let staged = git(&outer, &["diff", "--cached", "--name-only"]);
    assert_eq!(
        staged.lines().collect::<Vec<_>>(),
        ["README.md", "corpus/unrelated.txt"],
        "their staging is intact"
    );
}

#[test]
fn staged_corpus_paths_with_unicode_spaces_and_newlines_are_unambiguous() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));

    let names = ["시간.md", "two words.md", "two\nlines.md"];
    for name in names {
        std::fs::write(root.join("inbox").join(name), "fixture\n").unwrap();
    }
    git(&outer, &["add", "--", "corpus/inbox"]);

    let id = seed(&corpus, "A", &[]);
    committed(ops::commit(&corpus, "new", &[&id]));

    let committed = git(&outer, &["show", "--name-only", "--format=", "-z", "HEAD"]);
    let paths: Vec<&str> = committed
        .split('\0')
        .filter(|path| !path.is_empty())
        .collect();
    for name in names {
        assert!(
            paths.contains(&format!("corpus/inbox/{name}").as_str()),
            "missing {name:?} from {paths:?}"
        );
    }
    assert!(paths.contains(&format!("corpus/nodes/{id}.md").as_str()));
}

#[test]
fn commit_on_in_a_corpus_the_containing_repository_ignores_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join(".gitignore"), "corpus/\n").unwrap();
    ops::set_commit(&mut corpus, true).unwrap();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::commit(&corpus, "new", &[&a]),
        Err(Error::CorpusIgnored(r)) if r == root
    ));
    assert!(corpus.node_path(&a).unwrap().exists());
}

/// Commit everything in `dir` dated `stamp`, an ISO 8601 time with its offset.
fn commit_stamped(dir: &std::path::Path, stamp: &str) {
    git(dir, &["add", "-A"]);
    let out = support::output(
        support::git_command(dir, support::home())
            .args(["commit", "-q", "-m", stamp])
            .env("GIT_AUTHOR_DATE", stamp)
            .env("GIT_COMMITTER_DATE", stamp),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A date names each commit by its own day, in the offset it was recorded
/// with: the day `history` prints. Here the two commits' UTC days are the
/// other way round, and no single reader's zone puts both on their own days,
/// so neither UTC nor the reader's local day would pass.
#[test]
fn load_at_a_date_takes_each_commit_on_its_own_day() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    let id = seed(&corpus, "Days travel", &[]);
    // 2020-01-02T11:30Z, late on the first in UTC-12.
    commit_stamped(&root, "2020-01-01T23:30:00-12:00");
    ops::note(&corpus, &id, "written a day ahead", None).unwrap();
    // 2020-01-01T18:00Z, early on the second in UTC+14.
    commit_stamped(&root, "2020-01-02T08:00:00+14:00");

    let days: Vec<String> = corpus
        .history(&id)
        .unwrap()
        .into_iter()
        .map(|entry| entry.date)
        .collect();
    assert_eq!(days, ["2020-01-02", "2020-01-01"]);
    let noted = |date| {
        corpus
            .load_at(&id, date)
            .unwrap()
            .body
            .contains("written a day ahead")
    };
    assert!(!noted("2020-01-01"), "the first day is before the note");
    assert!(noted("2020-01-02"), "the second day has it");
    assert!(matches!(
        corpus.load_at(&id, "2019-12-31"),
        Err(Error::NoNodeAtRevision { .. })
    ));
}

// ------------------------------------------------------ supervised git --
//
// Every git child runs in its own process group with a deadline. A hook is
// the part of `git commit` nebula does not control, so these drive the
// runner through one: it records its process group, then misbehaves.

/// The deadline the timeout tests shorten every git command to.
#[cfg(all(unix, debug_assertions))]
const SHORT: std::time::Duration = std::time::Duration::from_secs(2);

/// How much later than planned a supervised stop may finish on a loaded CI
/// machine.
#[cfg(unix)]
const SLACK: std::time::Duration = std::time::Duration::from_secs(5);

/// A corpus that is its own repository, with commits on and that setting
/// already committed, so the next commit is the one under test.
#[cfg(unix)]
fn committing_corpus() -> (tempfile::TempDir, Corpus, std::path::PathBuf) {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));
    (dir, corpus, root)
}

/// Give the repository at `repo` a `pre-commit` hook that writes its process
/// group to `pgid_file`, atomically, and then runs `body`. The hooks
/// directory is named in the repository's own config, so a developer's
/// global `core.hooksPath` cannot replace it.
#[cfg(unix)]
fn pre_commit_hook(repo: &std::path::Path, pgid_file: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let hook = hooks.join("pre-commit");
    let recorded = pgid_file.display();
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n\
             ps -o pgid= -p $$ | tr -d ' ' > '{recorded}.tmp' && mv '{recorded}.tmp' '{recorded}'\n\
             {body}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The process group a hook recorded.
#[cfg(unix)]
fn recorded_group(pgid_file: &std::path::Path) -> rustix::process::Pid {
    let raw = std::fs::read_to_string(pgid_file).expect("the hook ran and recorded its group");
    rustix::process::Pid::from_raw(raw.trim().parse().expect("a process group id"))
        .expect("a positive process group id")
}

/// Whether `group` is empty, by the group's own answer to signal 0: anything
/// but "no such group" counts as a member left. Members the sweep killed are
/// orphans, reaped by init rather than by nebula, so they may linger as
/// zombies for a moment; that moment is bounded here, and a live process
/// never leaves on its own within it.
#[cfg(unix)]
fn group_gone(group: rustix::process::Pid) -> bool {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if matches!(
            rustix::process::test_kill_process_group(group),
            Err(rustix::io::Errno::SRCH)
        ) {
            return true;
        }
        if std::time::Instant::now() >= give_up {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Whether another thread could take the corpus lock right now, without
/// waiting. Another thread, because the lock is re-entrant on this one.
#[cfg(unix)]
fn lock_is_free(root: &std::path::Path) -> bool {
    let root = root.to_path_buf();
    std::thread::spawn(move || CorpusLock::acquire_within(&root, std::time::Duration::ZERO).is_ok())
        .join()
        .unwrap()
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn a_hung_commit_hook_times_out_and_releases_the_lock() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    pre_commit_hook(&root, &pgid_file, "sleep 30");
    let before = head(&root);

    let _short = nebula_core::GitDeadlineOverride::new(SHORT);
    let entry = ops::capture(&corpus, "a thought the hook holds up").unwrap();
    let started = std::time::Instant::now();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let took = started.elapsed();

    assert!(
        matches!(&err, Error::GitTimedOut { root: r, context, after }
            if r == &root && context == "commit" && *after == SHORT),
        "got {err:?}"
    );
    assert_eq!(err.code(), "git_timed_out");
    assert!(
        took < SHORT + nebula_core::GIT_TERMINATION_GRACE + SLACK,
        "took {took:?}"
    );
    assert!(
        corpus.inbox().unwrap().0.iter().any(|e| e.id == entry.id),
        "the write stays on disk whatever git does"
    );
    assert_eq!(head(&root), before, "nothing was committed");
    assert!(lock_is_free(&root), "the lock went with the verb");
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook's group outlived the timeout"
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn a_hook_that_ignores_sigterm_is_killed_after_the_grace_period() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    // `sleep` inherits the ignored SIGTERM, and holds git's stderr open.
    pre_commit_hook(&root, &pgid_file, "trap '' TERM\nsleep 30");

    let _short = nebula_core::GitDeadlineOverride::new(SHORT);
    let entry = ops::capture(&corpus, "a thought the hook will not let go").unwrap();
    let started = std::time::Instant::now();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let took = started.elapsed();

    assert!(matches!(err, Error::GitTimedOut { .. }), "got {err:?}");
    let grace = nebula_core::GIT_TERMINATION_GRACE;
    assert!(
        took >= SHORT + grace,
        "SIGTERM was ignored, so only SIGKILL after the grace ends it: took {took:?}"
    );
    assert!(took < SHORT + grace + SLACK, "took {took:?}");
    assert!(group_gone(recorded_group(&pgid_file)));
    assert!(lock_is_free(&root));
}

#[cfg(unix)]
#[test]
fn a_hook_descendant_does_not_outlive_a_clean_commit() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    // The background `sleep` keeps git's stderr open for a minute after git
    // itself is done.
    pre_commit_hook(&root, &pgid_file, "sleep 60 &\nexit 0");

    let entry = ops::capture(&corpus, "a thought with a straggler").unwrap();
    let started = std::time::Instant::now();
    let done = committed(ops::commit(&corpus, "capture", &[&entry.id]));
    let took = started.elapsed();

    assert_eq!(done.message, format!("neb capture {}", entry.id));
    assert_eq!(done.hash, head(&root));
    assert!(took < SLACK, "the commit waited on the straggler: {took:?}");
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook's background sleep outlived a clean commit"
    );
}

#[cfg(unix)]
#[test]
fn oversized_git_stderr_is_truncated_and_marked() {
    let (dir, corpus, root) = committing_corpus();
    let pgid_file = dir.path().join("hook.pgid");
    pre_commit_hook(
        &root,
        &pgid_file,
        "head -c 5000000 /dev/zero | tr '\\0' x >&2\nexit 1",
    );

    let entry = ops::capture(&corpus, "a thought the hook shouts at").unwrap();
    let err = ops::commit(&corpus, "capture", &[&entry.id]).unwrap_err();
    let Error::Git {
        context, stderr, ..
    } = &err
    else {
        panic!("got {err:?}");
    };
    assert_eq!(context, "commit");
    let cap = nebula_core::GIT_OUTPUT_CAP;
    let (shown, marker) = stderr
        .rsplit_once(" … ")
        .expect("a truncation marker after the text");
    assert!(shown.len() <= cap, "{} bytes shown", shown.len());
    assert!(
        shown.bytes().all(|b| b == b'x'),
        "the head of what the hook said"
    );
    let hidden: u64 = marker
        .strip_prefix("[truncated ")
        .and_then(|rest| rest.strip_suffix(" bytes]"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("`{marker}` is not the truncation marker"));
    assert!(hidden >= 5_000_000 - cap as u64, "{hidden}");
    assert!(stderr.len() <= cap + marker.len() + " … ".len());
    assert!(lock_is_free(&root));
}

// ------------------------------------------------------- observatory root --

/// Give the corpus at `root` the `observatory_root` key an older `neb` wrote
/// into `config.yaml`: another machine's absolute path. Written by hand
/// because nothing in this build writes it any more. Returns the path.
fn with_legacy_observatory_root(root: &std::path::Path) -> std::path::PathBuf {
    let foreign = std::path::PathBuf::from("/Users/someone-else/workspace/observatory");
    let config = root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{raw}observatory_root: {}\n", foreign.display()),
    )
    .unwrap();
    foreign
}

/// A corpus written before the setting moved out of `config.yaml` still
/// opens, and the key is reported as the legacy setting it is, whichever
/// setting wins on this machine.
#[test]
fn a_legacy_observatory_root_in_config_yaml_still_loads_and_is_reported() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    let corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    let setting = corpus.observatory_root().unwrap();
    assert_eq!(setting.legacy, Some(foreign));
    assert!(setting.root.is_some(), "the legacy key is still a fallback");
}

/// Dropping the key rewrites `config.yaml` without it and keeps the rest;
/// with no key there it writes nothing at all.
#[test]
fn dropping_the_legacy_observatory_root_keeps_every_other_setting() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    let config = root.join("config.yaml");
    let pristine = std::fs::read(&config).unwrap();

    let dropped = ops::drop_legacy_observatory_root(&mut corpus).unwrap();
    assert_eq!((dropped.removed, dropped.setting.legacy), (None, None));
    assert_eq!(pristine, std::fs::read(&config).unwrap(), "a no-op wrote");

    let foreign = with_legacy_observatory_root(&root);
    let mut corpus = Corpus::open(&process_locations(), Some(root)).unwrap();
    let dropped = ops::drop_legacy_observatory_root(&mut corpus).unwrap();
    assert_eq!(dropped.removed, Some(foreign), "it names what it removed");
    assert_eq!(pristine, std::fs::read(&config).unwrap());
}

/// The machine setting is read from any directory, so a relative one is
/// refused before anything is written.
#[test]
fn a_relative_observatory_root_is_refused_before_it_is_written() {
    let (_dir, corpus) = corpus();
    let relative = std::path::Path::new("observatory");
    assert!(matches!(
        ops::set_observatory_root(&corpus, relative),
        Err(Error::RelativeObservatoryRoot { root, setting: None }) if root == relative
    ));
    // The machine setting lives under `HOME`, which here is this process's
    // temporary one, never the developer's.
    assert_eq!(std::env::var_os("HOME"), Some(support::home().into()));
    assert!(
        !support::home()
            .join(".config/nebula/observatory-root")
            .exists()
    );
}

// -------------------------------------------- settings written under a lock --

/// `Corpus::open` reads `config.yaml` without the lock, because a read must
/// never wait on a writer. A writer that then waits its turn is holding a
/// snapshot from before the writer ahead of it finished, and every config
/// write rewrites the file whole — so without a reload under the lock the
/// waiter carries the stale copy of every key it is not setting back to disk
/// and the other writer's setting is gone without a word.
///
/// Two `Corpus` handles on one thread rather than two threads: the interleave
/// under test is a *stale open*, not contention, and sequencing it by hand is
/// what makes the loss deterministic. `crates/neb/tests/cli.rs` runs the same
/// shape across two processes against a really held `flock`.
#[test]
fn a_setting_landed_since_open_survives_the_next_writers_rewrite() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    // The waiter opens — and so snapshots the config — before the writer
    // ahead of it in the queue has written anything.
    let mut waiting = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(!waiting.commit_setting().enabled);

    // The writer ahead finishes its own config write and lets go.
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(ops::set_commit(&mut ahead, true).unwrap().enabled);

    // The waiter gets in and removes a different key.
    let setting = ops::drop_legacy_observatory_root(&mut waiting)
        .unwrap()
        .setting;
    assert_eq!(setting.legacy, None);

    let reopened = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the commit setting written after the waiter opened was erased by its rewrite"
    );
    assert_eq!(
        reopened.observatory_root().unwrap().legacy,
        None,
        "and the waiter's own change must still have landed"
    );
    let raw = std::fs::read_to_string(root.join("config.yaml")).unwrap();
    assert!(!raw.contains(foreign.to_str().unwrap()), "{raw}");
}

/// The same loss in the other direction, so neither setting is merely the one
/// that happens to be written last: a stale snapshot that still holds the
/// legacy key must not write it back.
#[test]
fn a_stale_commit_write_does_not_restore_the_legacy_key_dropped_since_it_opened() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let foreign = with_legacy_observatory_root(&root);

    let mut waiting = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert_eq!(
        waiting.observatory_root().unwrap().legacy,
        Some(foreign.clone())
    );

    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::drop_legacy_observatory_root(&mut ahead).unwrap();

    assert!(ops::set_commit(&mut waiting, true).unwrap().enabled);

    let reopened = Corpus::open(&process_locations(), Some(root)).unwrap();
    assert_eq!(
        reopened.observatory_root().unwrap().legacy,
        None,
        "the legacy key dropped after the waiter opened came back"
    );
    assert!(reopened.commit_setting().enabled);
}

/// Whether a write is recorded is a question about the configuration in
/// force, not the one this corpus happened to open with. A verb that waited
/// out a writer who turned the setting on commits; one that waited out a
/// writer who turned it off does not.
#[test]
fn the_commit_decision_follows_the_setting_on_disk_not_the_one_at_open() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);

    // Opened while the setting was off.
    let stale_off = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(!stale_off.commit_setting().enabled);

    // Another writer turns it on and records that.
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, true).unwrap();
    committed(ops::commit(&ahead, "config", &["commit"]));

    let entry = ops::capture(&stale_off, "a thought the setting now says to record").unwrap();
    let done = committed(ops::commit(&stale_off, "capture", &[&entry.id]));
    assert_eq!(done.message, format!("neb capture {}", entry.id));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    // And the other way: opened while it was on, but off by the time the
    // verb lands.
    let stale_on = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    assert!(stale_on.commit_setting().enabled);
    let mut ahead = Corpus::open(&process_locations(), Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, false).unwrap();

    let before = head(&root);
    let entry = ops::capture(&stale_on, "a thought the setting now says to leave").unwrap();
    assert!(matches!(
        ops::commit(&stale_on, "capture", &[&entry.id]),
        Ok(CommitOutcome::Disabled)
    ));
    assert_eq!(head(&root), before, "nothing was committed");
    assert!(
        git(&root, &["status", "--porcelain"]).contains(" M inbox/"),
        "the capture is on disk, just not recorded"
    );
}

// ------------------------------------------------------ tags and citations --

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

// -------------------------------------------------------------------- lock --

/// Two writers editing one node's tags at the same time. Each op is a load,
/// an edit and a save; without the lock the second load sees the corpus as it
/// was before the first save, and the later rename wins — one tag survives and
/// the other is silently gone.
///
/// Threads rather than processes here: this is what the in-process half of
/// the lock is for, since `flock` is held by the open file description and
/// would let a second thread of one process straight through.
/// `crates/neb/tests/cli.rs` runs the same race across two spawned binaries,
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

// ---------------------------------------------------------------- node ids --

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

/// Whether `result` is the refusal of `path` as `found`.
fn not_regular<T>(
    result: &nebula_core::Result<T>,
    path: &Path,
    found: nebula_core::fs::EntryKind,
) -> bool {
    matches!(result, Err(Error::NotRegularFile { path: p, found: f }) if p == path && *f == found)
}

/// Make a FIFO at `path` with `mkfifo`, through the isolating builder.
#[cfg(unix)]
fn mkfifo(path: &Path) {
    let output = support::output(
        support::command("mkfifo", support::home()).arg(path),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(output.status.success(), "mkfifo {}", path.display());
}

/// Run `f` on a thread and return what it returned, failing the test if it
/// has not returned within two seconds: a read that blocks on a FIFO or
/// never finishes on a device fails here instead of hanging the suite.
fn within_two_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .expect("the call did not return within 2 s")
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

// ------------------------------------------------- resolved locations --

/// A `Locations` naming only `home`: no working directory, no variables.
fn at_home(home: &Path) -> Locations {
    Locations {
        home: Some(home.as_os_str().to_owned()),
        ..Locations::default()
    }
}

/// Every answer `resolve_root` gives comes from the `Locations` it is handed,
/// in order: `--root`, then `NEBULA_ROOT` (empty counts as unset), then the
/// corpus the working directory is in, then the machine file, then
/// `~/.nebula` under the given home. With no home the default is refused
/// rather than read from the process.
#[test]
fn resolve_root_follows_explicit_then_nebula_root_then_cwd_then_machine_file_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");
    let named = dir.path().join("named");
    let explicit = dir.path().join("explicit");
    let found = dir.path().join("found");
    let base = at_home(&home);
    Corpus::init(&base, &found).unwrap();
    Corpus::write_root_config(&base, &configured, false).unwrap();

    let every = Locations {
        cwd: Some(found.join("nodes")),
        nebula_root: Some(named.as_os_str().to_owned()),
        ..base.clone()
    };
    let resolve = |locations: &Locations| Corpus::resolve_root(locations, None).unwrap();

    assert_eq!(
        Corpus::resolve_root(&every, Some(explicit.clone())).unwrap(),
        explicit
    );
    assert_eq!(resolve(&every), named);

    let unset = Locations {
        nebula_root: Some("".into()),
        ..every.clone()
    };
    assert_eq!(resolve(&unset), found, "an empty NEBULA_ROOT is unset");

    let outside = Locations {
        cwd: Some(home.clone()),
        ..unset.clone()
    };
    assert_eq!(resolve(&outside), configured);
    assert_eq!(outside.corpus_root().unwrap(), configured);

    let fresh = dir.path().join("fresh-home");
    let unconfigured = Locations {
        home: Some(fresh.as_os_str().to_owned()),
        ..outside.clone()
    };
    assert_eq!(resolve(&unconfigured), fresh.join(".nebula"));

    let homeless = Locations {
        home: None,
        ..outside
    };
    assert!(matches!(
        Corpus::resolve_root(&homeless, None),
        Err(Error::HomeUnset)
    ));
}

/// The observatory setting is `OBSERVATORY_ROOT` as `Locations` holds it,
/// else the machine file under its home, else the legacy `config.yaml` key,
/// which is reported whichever wins. An empty variable is unset, and a
/// corpus opened with no home still reads the legacy key.
#[test]
fn observatory_root_prefers_env_then_machine_file_then_legacy_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("corpus");
    let locations = at_home(&home);
    Corpus::init(&locations, &root).unwrap();
    let foreign = with_legacy_observatory_root(&root);
    let setting = |locations: &Locations| {
        Corpus::open(locations, Some(root.clone()))
            .unwrap()
            .observatory_root()
            .unwrap()
    };

    let legacy = setting(&locations);
    assert_eq!(legacy.source, nebula_core::ObservatorySource::Config);
    assert_eq!(legacy.root.as_deref(), Some(foreign.as_path()));
    assert_eq!(setting(&Locations::default()).root, legacy.root);

    let machine = dir.path().join("machine-observatory");
    let corpus = Corpus::open(&locations, Some(root.clone())).unwrap();
    ops::set_observatory_root(&corpus, &machine).unwrap();
    let from_file = setting(&locations);
    assert_eq!(from_file.source, nebula_core::ObservatorySource::Machine);
    assert_eq!(from_file.root.as_deref(), Some(machine.as_path()));
    assert_eq!(from_file.legacy.as_deref(), Some(foreign.as_path()));

    let env = dir.path().join("env-observatory");
    let exported = Locations {
        observatory_root: Some(env.as_os_str().to_owned()),
        ..locations.clone()
    };
    let from_env = setting(&exported);
    assert_eq!(from_env.source, nebula_core::ObservatorySource::Env);
    assert_eq!(from_env.root.as_deref(), Some(env.as_path()));
    assert_eq!(from_env.legacy.as_deref(), Some(foreign.as_path()));

    let empty = Locations {
        observatory_root: Some("".into()),
        ..locations
    };
    assert_eq!(
        setting(&empty).source,
        nebula_core::ObservatorySource::Machine,
        "an empty OBSERVATORY_ROOT is unset"
    );
}

/// `~/.nebula`, the machine file and a corpus created with nothing else set
/// all sit under the home `Locations` names, not the one this process has.
#[test]
fn default_root_comes_from_locations_home_not_process_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("elsewhere");
    assert_ne!(home, support::home());
    let locations = at_home(&home);

    assert_eq!(
        Corpus::default_root(&locations).unwrap(),
        home.join(".nebula")
    );
    assert_eq!(
        Corpus::root_config_path(&locations).unwrap(),
        home.join(".config").join("nebula").join("root")
    );

    let captured = verb::capture_at(
        &locations,
        None,
        "a thought with nowhere else to go",
        0,
        &WriteOptions::default(),
    )
    .unwrap()
    .value;
    assert_eq!(captured.root, home.join(".nebula"));
    assert!(captured.created);
    assert!(home.join(".nebula").join("nodes").is_dir());
}

/// `$PWD` names the working directory only when it is absolute, has no `.`
/// or `..`, and is the same directory as the OS's answer; then a corpus
/// reached through a symlink is found under the spelling the shell used.
#[cfg(unix)]
#[test]
fn logical_pwd_is_used_only_when_it_names_the_same_directory_as_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    let other = dir.path().join("other");
    let link = dir.path().join("link");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let working = |pwd: Option<&Path>, cwd: Option<&Path>| {
        Locations {
            cwd: cwd.map(Path::to_path_buf),
            pwd: pwd.map(|p| p.as_os_str().to_owned()),
            ..Locations::default()
        }
        .working_dir()
    };

    assert_eq!(working(Some(&link), Some(&real)), Some(link.clone()));
    assert_eq!(working(Some(&real), Some(&real)), Some(real.clone()));
    assert_eq!(working(Some(&other), Some(&real)), Some(real.clone()));
    assert_eq!(
        working(Some(Path::new("link")), Some(&real)),
        Some(real.clone()),
        "a relative PWD is not trusted"
    );
    assert_eq!(
        working(Some(&other.join("..").join("link")), Some(&real)),
        Some(real.clone()),
        "a PWD with `..` is not trusted, even naming the same directory"
    );
    assert_eq!(working(None, Some(&real)), Some(real.clone()));
    assert_eq!(working(Some(&link), None), None);

    let corpus = dir.path().join("corpus");
    let linked = dir.path().join("linked");
    Corpus::init(&Locations::default(), &corpus).unwrap();
    std::os::unix::fs::symlink(&corpus, &linked).unwrap();
    let inside = Locations {
        cwd: Some(corpus.join("nodes")),
        pwd: Some(linked.join("nodes").into_os_string()),
        ..at_home(&dir.path().join("home"))
    };
    assert_eq!(Corpus::resolve_root(&inside, None).unwrap(), linked);
}

/// `init` names the machine setting that would otherwise send every later
/// command elsewhere: `~/.nebula` created while it names another corpus,
/// and any other corpus created beside it. Setting the root warns of
/// neither, and says where the setting is.
#[test]
fn init_returns_default_and_shadowing_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let locations = at_home(&home);
    let setting = home.join(".config").join("nebula").join("root");
    let configured = dir.path().join("configured");

    let first = verb::init(&locations, None, Some(configured.clone()), true, false).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert_eq!(first.setting, setting);
    assert!(!first.suggest_set_root);

    let default = verb::init(&locations, Some(home.join(".nebula")), None, false, false).unwrap();
    assert_eq!(
        default.warnings,
        [RootWarning::DefaultWhileConfigured {
            setting: setting.clone(),
            configured: configured.clone(),
        }]
    );

    let other = verb::init(
        &locations,
        None,
        Some(dir.path().join("other")),
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        other.warnings,
        [RootWarning::Shadowed {
            setting: setting.clone(),
            configured,
        }]
    );
    assert!(!other.suggest_set_root, "a machine setting already exists");

    let third = dir.path().join("third");
    let moved = verb::init(&locations, None, Some(third.clone()), true, true).unwrap();
    assert!(moved.warnings.is_empty(), "{:?}", moved.warnings);
    assert_eq!(Corpus::configured_root(&locations).unwrap(), Some(third));
}

/// The migration's commit is made inside the migration's lock: a hook run
/// by that commit finds this process's holder record in `.lock`, and the
/// record is gone once `migrate` returns.
#[cfg(unix)]
#[test]
fn migrate_commits_under_its_own_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "first", V1_NODE);
    std::fs::write(
        root.join("config.yaml"),
        "schema_version: 1\ncorpus_id: neb-abc123\ncommit: true\n",
    )
    .unwrap();
    git_init(&root);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "v1 corpus"]);
    let seen = dir.path().join("holder-seen");
    pre_commit_hook(
        &root,
        &dir.path().join("hook.pgid"),
        &format!("cat .lock > '{}'", seen.display()),
    );

    let migrated = verb::migrate(
        &process_locations(),
        Some(root.clone()),
        &WriteOptions::default(),
    )
    .unwrap();
    let done = committed(migrated.commit.expect("the setting is on"));
    assert_eq!(done.message, "neb migrate");
    assert_eq!(migrated.value.root, root);

    let holder = std::fs::read_to_string(&seen).expect("the hook ran");
    assert!(
        holder.contains(&std::process::id().to_string()),
        "the commit ran without this process holding the lock: {holder:?}"
    );
    // Released: the holder empties its record before the guard lets go.
    // Read from the file rather than by taking the lock from another thread,
    // which a sibling test's fork can briefly make look held.
    assert_eq!(
        std::fs::read_to_string(root.join(nebula_core::LOCK_FILE)).unwrap(),
        ""
    );
    assert_eq!(
        git(&root, &["log", "--format=%s"])
            .lines()
            .collect::<Vec<_>>(),
        ["neb migrate", "v1 corpus"]
    );
}

/// What is left uncommitted in a corpus that is its own repository, the
/// lock file aside.
#[cfg(unix)]
fn dirt(root: &Path) -> String {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude).lock",
        ],
    )
}

/// With commits on, each writing verb commits what it wrote, as
/// `neb <verb> <ids>`, and leaves nothing behind for the next one.
#[cfg(unix)]
#[test]
fn every_write_op_commits_what_it_wrote_when_commit_is_on() {
    let (_dir, mut corpus, root) = committing_corpus();
    let options = WriteOptions::default();
    let expect = |commit: Option<Result<CommitOutcome, Error>>, message: &str| {
        let done = committed(commit.expect("the policy commits"));
        assert_eq!(done.message, message);
        assert!(dirt(&root).is_empty(), "{message} left: {}", dirt(&root));
    };

    let captured = verb::capture(&corpus, "a thought to keep", &options).unwrap();
    let kept = captured.value.id.clone();
    expect(captured.commit, &format!("neb capture {kept}"));
    let gone = verb::capture(&corpus, "a thought to drop", &options).unwrap();
    expect(gone.commit, &format!("neb capture {}", gone.value.id));
    let dropped = verb::drop(&corpus, &gone.value.id, &options).unwrap();
    expect(dropped.commit, &format!("neb drop {}", gone.value.id));

    let promotion = Promotion {
        title: Some("Kept".to_string()),
        ..Promotion::default()
    };
    let promoted = verb::promote(&corpus, &kept, &promotion, 0, &options).unwrap();
    let a = promoted.value.doc.node.id.clone();
    expect(promoted.commit, &format!("neb promote {kept} {a}"));

    let created = verb::new_node(
        &corpus,
        &NewNode {
            title: "Other".to_string(),
            ..NewNode::default()
        },
        &options,
    )
    .unwrap();
    let b = created.value.doc.node.id.clone();
    expect(created.commit, &format!("neb new {b}"));

    let before = corpus.load(&a).unwrap().body;
    let edited = verb::edit(
        &corpus,
        &a,
        &before,
        &format!("{before}\nMore prose.\n"),
        None,
        false,
        &options,
    )
    .unwrap();
    assert!(edited.value.changed);
    expect(edited.commit, &format!("neb edit {a}"));

    let noted = verb::note(&corpus, &a, "a dated line", None, false, &options).unwrap();
    expect(noted.commit, &format!("neb note {a}"));

    let sharpened =
        verb::sharpen(&corpus, &a, "it never shows up", Some("claude"), &options).unwrap();
    expect(sharpened.commit, &format!("neb sharpen {a}"));
    let confirmed = verb::confirm_kill(&corpus, &a, &options).unwrap();
    expect(confirmed.commit, &format!("neb sharpen {a}"));

    let status = verb::set_status(&corpus, &b, Status::Hypothesis, None, &options);
    assert!(
        status.is_err(),
        "a seed with no kill cannot be a hypothesis"
    );
    let refuted =
        verb::set_status(&corpus, &a, Status::Refuted, Some("it fired"), &options).unwrap();
    expect(refuted.commit, &format!("neb status {a}"));

    let linked = verb::link(&corpus, &b, EdgeType::DerivesFrom, &a, None, &options).unwrap();
    expect(linked.commit, &format!("neb link {b} {a}"));

    let tagged = verb::retag(&corpus, &b, &["physics".to_string()], &[], &options).unwrap();
    expect(tagged.commit, &format!("neb tag {b}"));

    let cited = verb::cite(&corpus, &b, &citing("https://example.com/paper"), &options).unwrap();
    let reference = cited.value.cited.reference.clone();
    expect(cited.commit, &format!("neb cite {b} {reference}"));

    let handed = verb::handoff(&corpus, &b, &handing("H012"), &options).unwrap();
    expect(handed.commit, &format!("neb handoff {b} H012"));

    let triaged = verb::capture(&corpus, "a thought triage drops", &options).unwrap();
    let entry = triaged.value.id.clone();
    expect(triaged.commit, &format!("neb capture {entry}"));
    let mut session = Triage::start(&corpus, None).unwrap();
    let step = session
        .decide(&corpus, nebula_core::triage::Action::Drop, &options)
        .unwrap();
    step.value.unwrap();
    expect(step.commit, &format!("neb drop {entry}"));

    with_legacy_observatory_root(&root);
    git(&root, &["commit", "-q", "-am", "a legacy key"]);
    let dropped = verb::set_observatory_root(&mut corpus, None, true, &options).unwrap();
    assert!(dropped.value.dropped.is_some_and(|d| d.removed.is_some()));
    expect(dropped.commit, "neb config observatory-root");

    // Off means off: the setting that turns commits off is not committed.
    let off = verb::set_commit(&mut corpus, false, &options).unwrap();
    assert!(matches!(off.commit, Some(Ok(CommitOutcome::Disabled))));
    assert!(dirt(&root).contains("config.yaml"), "{}", dirt(&root));
}

/// `--no-commit` writes and does not commit, whatever the setting says:
/// the verb reports no commit attempted and `HEAD` stays where it was.
#[cfg(unix)]
#[test]
fn write_op_with_skip_policy_does_not_commit() {
    let (_dir, corpus, root) = committing_corpus();
    let skip = WriteOptions {
        commit: CommitPolicy::Skip,
        ..WriteOptions::default()
    };
    let before = head(&root);

    let captured = verb::capture(&corpus, "an uncommitted thought", &skip).unwrap();
    assert!(captured.commit.is_none());
    let created = verb::new_node(
        &corpus,
        &NewNode {
            title: "Loose".to_string(),
            ..NewNode::default()
        },
        &skip,
    )
    .unwrap();
    assert!(created.commit.is_none());
    let id = created.value.doc.node.id;
    let noted = verb::note(&corpus, &id, "still loose", None, false, &skip).unwrap();
    assert!(noted.commit.is_none());

    assert_eq!(head(&root), before);
    assert!(
        dirt(&root).contains(&format!("nodes/{id}.md")),
        "the writes landed on disk: {}",
        dirt(&root)
    );
}

/// The append-only notes rule is enforced where the body is written, not
/// only by the CLI's editor: a body whose `## Notes` section was rewritten
/// or removed is refused and the file is left as it was, while prose
/// outside the notes stays editable.
#[test]
fn set_body_refuses_a_rewritten_notes_section() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Noted", &[]);
    ops::note(&corpus, &id, "the first thought", None).unwrap();
    let path = corpus.node_path(&id).unwrap();
    let file = std::fs::read_to_string(&path).unwrap();
    let body = corpus.load(&id).unwrap().body;
    assert!(body.contains("## Notes"), "{body}");

    let without = body.split("## Notes").next().unwrap().to_string();
    for rewritten in [
        body.replace("the first thought", "a better thought"),
        without,
    ] {
        assert!(
            matches!(
                ops::set_body(&corpus, &id, &rewritten),
                Err(Error::NotesChanged)
            ),
            "{rewritten:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    }

    let prose = format!("Prose added above.\n\n{body}");
    assert!(ops::set_body(&corpus, &id, &prose).unwrap().is_some());
}
