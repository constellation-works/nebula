//! `trace`, drawn from the walk core returns and nothing else (STD-01 §R6):
//! a tree for a terminal, and one tab-separated line per node for anything
//! else (§R9).

use super::{Notice, bold, count, dim, status_badge};
use nebula_core::{Direction, EdgeType, Trace, TraceNode};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Draw the walk as a tree from its start, for a terminal.
///
/// Every line below the start names the kind of edge that joins it to the
/// line above. The edges are always the descendant's, so walking down a line
/// reads "this derives from the one above", and walking up "the one above
/// derives from this". Two edges between the same pair are one line naming
/// both kinds.
///
/// Each node is drawn out once, under the node the walk first reached it
/// from. A node joined to more than one node on the walk is a diamond, which
/// is legal and expected here, so each other place it joins points to it,
/// `(shown above)` or `(shown below)`, and the shape of the graph stays
/// visible. The walk records the kinds of the first step only, so a pointer
/// names none.
pub(crate) fn draw(trace: &Trace, direction: Direction) -> String {
    let Some(start) = trace.0.first() else {
        return String::new();
    };
    let mut tree = Tree {
        walk: Walk::new(trace, direction),
        drawn: HashSet::new(),
        out: String::new(),
    };
    tree.node(start, "", 0, true);
    tree.out
}

/// The walk as one tab-separated line per node, in the order it reached
/// them, for anything but a terminal: the steps from the start, the id, the
/// status, the title, and the kinds of the step that reached it, or `-` for
/// the start. No glyphs, no colour, and one line per node, so `cut -f` and
/// `wc -l` work.
pub(crate) fn tabbed(trace: &Trace) -> String {
    // Only `depth` is asked of it, which no direction changes.
    let walk = Walk::new(trace, Direction::Up);
    let mut out = String::new();
    for n in &trace.0 {
        let kinds = n.via.as_ref().map_or_else(
            || "-".to_string(),
            |hop| {
                hop.kinds
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            },
        );
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{kinds}",
            walk.depth(n),
            n.id,
            n.status,
            n.title.replace(['\t', '\n'], " ")
        );
    }
    out
}

/// The notice for a walk `--depth` stopped short: how many nodes the whole
/// walk reaches beyond the ones shown.
pub(crate) fn trace_notice(shown: usize, whole: usize, depth: Option<usize>) -> Option<Notice> {
    let depth = depth?;
    (shown < whole).then(|| {
        Notice::always(format!(
            "{} beyond --depth {depth}; raise --depth for more",
            count(whole - shown, "more node")
        ))
    })
}

/// A walk indexed for drawing.
struct Walk<'a> {
    by_id: HashMap<&'a str, (usize, &'a TraceNode)>,
    nodes: &'a [TraceNode],
    direction: Direction,
}

impl<'a> Walk<'a> {
    fn new(trace: &'a Trace, direction: Direction) -> Self {
        let by_id = trace
            .0
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), (i, n)))
            .collect();
        Self {
            by_id,
            nodes: &trace.0,
            direction,
        }
    }

    /// The nodes on the walk joined to `n` one step further out, in the
    /// order the walk takes them.
    fn next(&self, n: &TraceNode) -> Vec<&'a TraceNode> {
        match self.direction {
            Direction::Up => n
                .parents
                .iter()
                .filter_map(|p| self.by_id.get(p.as_str()).map(|(_, n)| *n))
                .collect(),
            Direction::Down => self
                .nodes
                .iter()
                .filter(|c| c.parents.contains(&n.id))
                .collect(),
        }
    }

    /// Steps from the start to `n` along the steps that first reached each
    /// node. Every step leads from a node reached earlier, so this ends.
    fn depth(&self, n: &TraceNode) -> usize {
        let mut depth = 0;
        let mut at = n;
        while let Some(hop) = &at.via {
            match self.by_id.get(hop.from.as_str()) {
                Some((i, from)) if *i < self.index(at) => {
                    depth += 1;
                    at = from;
                }
                _ => break,
            }
        }
        depth
    }

    fn index(&self, n: &TraceNode) -> usize {
        self.by_id.get(n.id.as_str()).map_or(0, |(i, _)| *i)
    }
}

struct Tree<'a> {
    walk: Walk<'a>,
    /// The nodes drawn out so far, which a pointer to one points up to.
    drawn: HashSet<&'a str>,
    out: String,
}

impl<'a> Tree<'a> {
    fn node(&mut self, n: &'a TraceNode, prefix: &str, depth: usize, last: bool) {
        let kinds = n.via.as_ref().map_or(&[][..], |hop| hop.kinds.as_slice());
        let _ = writeln!(
            self.out,
            "{}{}",
            branch(prefix, depth, last, Some(kinds)),
            line(n)
        );
        self.drawn.insert(&n.id);
        let child_prefix = match depth {
            0 => String::new(),
            _ => format!("{prefix}{}", if last { "   " } else { "│  " }),
        };
        let next = self.walk.next(n);
        for (i, c) in next.iter().enumerate() {
            let last = i + 1 == next.len();
            let reached_here = c.via.as_ref().is_some_and(|hop| hop.from == n.id);
            if reached_here && !self.drawn.contains(c.id.as_str()) {
                self.node(c, &child_prefix, depth + 1, last);
            } else {
                let place = if self.drawn.contains(c.id.as_str()) {
                    "  (shown above)"
                } else {
                    "  (shown below)"
                };
                let _ = writeln!(
                    self.out,
                    "{}{}{}",
                    branch(&child_prefix, depth + 1, last, None),
                    line(c),
                    dim(place)
                );
            }
        }
    }
}

/// The glyphs and edge label that lead into a line `depth` steps out.
fn branch(prefix: &str, depth: usize, last: bool, kinds: Option<&[EdgeType]>) -> String {
    if depth == 0 {
        return String::new();
    }
    let glyph = if last { "└─" } else { "├─" };
    match kinds {
        Some(kinds) => format!("{prefix}{glyph} {}  ", joined(kinds)),
        None => format!("{prefix}{glyph} "),
    }
}

/// One node on the walk: status, id and title, and where it was handed off
/// to when it was.
fn line(n: &TraceNode) -> String {
    let handoff = n
        .handed_off_to
        .as_deref()
        .map(|record| dim(&format!("  handed off to {record}")))
        .unwrap_or_default();
    format!(
        "{} {} {}{handoff}",
        status_badge(n.status),
        bold(&n.id),
        dim(&n.title)
    )
}

/// Edge kinds as one label: `derives-from, reopens`.
fn joined(kinds: &[EdgeType]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
