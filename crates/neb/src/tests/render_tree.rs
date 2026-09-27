//! Unit tests for `render_tree`.

use crate::render::{Notice, bold, count, dim, status_badge};
use nebula_core::{Direction, EdgeType, Trace, TraceNode};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::render::visible;
use crate::render::tree::*;
use nebula_core::{Corpus, Graph, NewNode, Status, TraceHop, graph, ops};

/// `root <- left, right <- synthesis`, with `synthesis` also reopening
/// `left`: a diamond with a parallel pair in it.
fn diamond() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(
        &nebula_core::Locations::default(),
        &dir.path().join("corpus"),
    )
    .unwrap();
    let new = |title: &str, parents: &[&str]| {
        let args = NewNode {
            title: title.into(),
            parents: parents.iter().map(ToString::to_string).collect(),
            ..NewNode::default()
        };
        ops::new_node(&corpus, &args).unwrap();
    };
    new("Root", &[]);
    new("Left", &["root"]);
    new("Right", &["root"]);
    new("Synthesis", &["left", "right"]);
    ops::link(&corpus, "synthesis", EdgeType::Reopens, "left", None).unwrap();
    (dir, corpus)
}

fn walk(corpus: &Corpus, from: &str, direction: Direction) -> Trace {
    let docs = corpus.load_all().unwrap();
    graph::trace(&Graph::build(&docs).unwrap(), from, direction).unwrap()
}

/// Each node is drawn out once, under the step that first reached it,
/// and every other place it joins points back to it with no edge label.
#[test]
fn a_diamond_is_drawn_once_and_pointed_to_from_its_other_branch() {
    let (_dir, corpus) = diamond();
    let up = visible(&draw(
        &walk(&corpus, "synthesis", Direction::Up),
        Direction::Up,
    ));
    assert_eq!(
        up.lines().collect::<Vec<_>>(),
        [
            "seed       synthesis Synthesis",
            "├─ derives-from, reopens  seed       left Left",
            "│  └─ derives-from  seed       root Root",
            "└─ derives-from  seed       right Right",
            "   └─ seed       root Root  (shown above)",
        ],
        "\n{up}"
    );

    let down = visible(&draw(
        &walk(&corpus, "root", Direction::Down),
        Direction::Down,
    ));
    assert_eq!(
        down.lines().collect::<Vec<_>>(),
        [
            "seed       root Root",
            "├─ derives-from  seed       left Left",
            "│  └─ derives-from, reopens  seed       synthesis Synthesis",
            "└─ derives-from  seed       right Right",
            "   └─ seed       synthesis Synthesis  (shown above)",
        ],
        "\n{down}"
    );
}

/// The piped form holds what the tree does, one node a line, with the
/// steps counted along the first path to each.
#[test]
fn tabbed_lines_are_the_walk_in_order() {
    let (_dir, corpus) = diamond();
    let lines = tabbed(&walk(&corpus, "synthesis", Direction::Up));
    assert_eq!(
        lines,
        "0\tsynthesis\tseed\tSynthesis\t-\n\
         1\tleft\tseed\tLeft\tderives-from,reopens\n\
         2\troot\tseed\tRoot\tderives-from\n\
         1\tright\tseed\tRight\tderives-from\n"
    );
}

fn node(id: &str, parents: &[&str], via: Option<&str>) -> TraceNode {
    TraceNode {
        id: id.into(),
        title: id.to_uppercase(),
        status: Status::Hypothesis,
        parents: parents.iter().map(ToString::to_string).collect(),
        via: via.map(|from| TraceHop {
            from: from.into(),
            kinds: vec![EdgeType::DerivesFrom],
        }),
        handed_off_to: None,
    }
}

/// A walk `--depth` cut can reach a node deep first and near later; the
/// node is drawn under its first step, and the nearer place, drawn
/// before it, points down to it.
#[test]
fn a_node_drawn_later_is_pointed_down_to() {
    // Down from `a`: `a -> b -> c`, and `a -> c` directly, but the walk
    // reached `c` through `b` first.
    let trace = Trace(vec![
        node("a", &[], None),
        node("c", &["b", "a"], Some("b")),
        node("b", &["a"], Some("a")),
    ]);
    let drawn = visible(&draw(&trace, Direction::Down));
    assert_eq!(
        drawn.lines().collect::<Vec<_>>(),
        [
            "hypothesis a A",
            "├─ hypothesis c C  (shown below)",
            "└─ derives-from  hypothesis b B",
            "   └─ derives-from  hypothesis c C",
        ],
        "\n{drawn}"
    );
}

#[test]
fn a_hand_off_is_named_on_the_node_line_and_tabs_stay_fields() {
    let mut start = node("a", &[], None);
    start.handed_off_to = Some("H123".into());
    start.title = "tabs\tin a\ntitle".into();
    let trace = Trace(vec![start]);
    assert_eq!(
        visible(&draw(&trace, Direction::Up)),
        "hypothesis a tabs\tin a\ntitle  handed off to H123\n"
    );
    assert_eq!(tabbed(&trace), "0\ta\thypothesis\ttabs in a title\t-\n");
}

#[test]
fn only_a_walk_the_bound_cut_has_a_notice() {
    assert_eq!(trace_notice(4, 4, Some(1)), None);
    assert_eq!(trace_notice(4, 4, None), None);
    let cut = trace_notice(2, 3, Some(1)).unwrap();
    assert_eq!(
        cut.line(true).map(|l| visible(&l)).as_deref(),
        Some("1 more node beyond --depth 1; raise --depth for more"),
        "said under --json too"
    );
}
