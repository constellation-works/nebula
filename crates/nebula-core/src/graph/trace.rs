//! Lineage walks: `trace` up or down the genealogy, and `impact`, what a
//! node touches.

use crate::error::Result;
use crate::model::{EdgeType, Status};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::Graph;
#[cfg(doc)]
use crate::model::Node;

/// Which way [`trace`] walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Towards the ancestors: where the idea came from.
    Up,
    /// Towards the descendants: what came of it.
    Down,
}

/// A lineage walk. Serializes as the bare list of nodes, oldest reachable
/// first, each reported once even when a diamond reaches it twice.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Trace(pub Vec<TraceNode>);

/// One node on a lineage walk.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TraceNode {
    /// The node's id.
    pub id: String,
    /// Its one-line title.
    pub title: String,
    /// Where it is in its lifecycle.
    pub status: Status,
    /// Its genealogical parents, each once.
    pub parents: Vec<String>,
    /// The step that first reached it. Null for the node the walk starts at.
    pub via: Option<TraceHop>,
    /// The Observatory record it was handed off to, when it was; see
    /// [`Node::handed_off_to`]. Omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_off_to: Option<String>,
}

/// One step of a lineage walk: the node it was taken from, and every kind of
/// genealogy edge between the two.
///
/// The edges are always the descendant's, so the kinds read the same both
/// ways: walking up, `from` is the child that declares them; walking down, it
/// is the ancestor they name. Parallel edges are one step with several kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TraceHop {
    /// The node the step was taken from.
    pub from: String,
    /// The kinds of edge the step followed, in the order declared.
    pub kinds: Vec<EdgeType>,
}

/// Walk ancestry, or descent. The feature the whole system exists for.
pub fn trace(graph: &Graph<'_>, id: &str, direction: Direction) -> Result<Trace> {
    trace_within(graph, id, direction, None)
}

/// [`trace`], stopping `depth` steps from the start: `Some(0)` is the node
/// alone, `Some(1)` adds its parents (or children), and `None` walks it all.
///
/// A bounded walk holds every node within `depth` steps along *some* path,
/// not just along the first path the walk happened to take. A diamond can
/// reach a node deep first and shallow later, so a node whose walk the bound
/// cut short, met again nearer the start, is walked on from there; it is
/// still reported once.
pub fn trace_within(
    graph: &Graph<'_>,
    id: &str,
    direction: Direction,
    depth: Option<usize>,
) -> Result<Trace> {
    graph.require(id)?;
    let mut walk = Walk {
        graph,
        direction,
        max: depth,
        seen: HashMap::new(),
        acc: Vec::new(),
    };
    walk.collect(id, None, 0);
    Ok(Trace(walk.acc))
}

/// The state of one [`trace_within`].
struct Walk<'g, 'a> {
    graph: &'g Graph<'a>,
    direction: Direction,
    max: Option<usize>,
    /// Each node reached: the fewest steps it has been walked on from, and
    /// whether that walk reached everything below it.
    seen: HashMap<String, (usize, bool)>,
    acc: Vec<TraceNode>,
}

impl Walk<'_, '_> {
    /// Walk on from `id`, `at` steps from the start, and say whether that
    /// reached everything below it: `false` when the bound cut something.
    fn collect(&mut self, id: &str, via: Option<TraceHop>, at: usize) -> bool {
        // Walked before, and either in full or from no further away: this
        // visit could reach nothing new. Unbounded, every walk is in full,
        // which is the walk as it always was.
        if let Some(&(then, complete)) = self.seen.get(id)
            && (complete || then <= at)
        {
            return complete;
        }
        // Complete until shown otherwise, which is also what a cycle back to
        // a node still being walked finds.
        let first = self.seen.insert(id.to_string(), (at, true)).is_none();
        let graph = self.graph;
        let Some(doc) = graph.get(id) else {
            return true;
        };
        if first {
            self.acc.push(TraceNode {
                id: doc.node.id.clone(),
                title: doc.node.title.clone(),
                status: doc.node.status,
                parents: graph
                    .parents_of(id)
                    .iter()
                    .map(|p| (*p).to_string())
                    .collect(),
                via,
                handed_off_to: doc.node.handed_off_to().map(str::to_owned),
            });
        }
        let next: Vec<(String, Vec<EdgeType>)> = match self.direction {
            Direction::Down => graph
                .children_of(id)
                .iter()
                .map(|c| ((*c).to_string(), kinds_between(graph, c, id)))
                .collect(),
            Direction::Up => graph
                .parents_of(id)
                .iter()
                .map(|p| ((*p).to_string(), kinds_between(graph, id, p)))
                .collect(),
        };
        if self.max.is_some_and(|max| at >= max) && !next.is_empty() {
            self.seen.insert(id.to_string(), (at, false));
            return false;
        }
        let mut complete = true;
        for (n, kinds) in next {
            let hop = TraceHop {
                from: id.to_string(),
                kinds,
            };
            complete &= self.collect(&n, Some(hop), at + 1);
        }
        self.seen.insert(id.to_string(), (at, complete));
        complete
    }
}

/// The genealogy edge kinds `child` declares to `parent`.
fn kinds_between(graph: &Graph<'_>, child: &str, parent: &str) -> Vec<EdgeType> {
    graph
        .get(child)
        .and_then(|d| d.node.lineage().into_iter().find(|(p, _)| *p == parent))
        .map(|(_, kinds)| kinds)
        .unwrap_or_default()
}

/// How a node in an [`Impact`] is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// It descends from the node, along any genealogy edge.
    Descends,
    /// It is declared in conflict with the node.
    Contradicts,
}

/// What a node touches. Serializes as the bare list, descendants first.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Impact(pub Vec<Touched>);

/// One node reached from another, and how.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Touched {
    /// The node reached.
    pub id: String,
    /// Which relation reached it.
    pub via: Via,
}

/// Everything that descends from a node, and whatever it contradicts.
pub fn impact(graph: &Graph<'_>, id: &str) -> Result<Impact> {
    graph.require(id)?;
    // Descendants by reverse genealogy, breadth-first so nearer ones come
    // first, each reported once even when reached along two branches.
    let mut out: Vec<Touched> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::from([id]);
    let mut frontier = vec![id];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for at in frontier {
            for c in graph.children_of(at) {
                if seen.insert(c) {
                    out.push(Touched {
                        id: (*c).to_string(),
                        via: Via::Descends,
                    });
                    next.push(*c);
                }
            }
        }
        frontier = next;
    }
    out.extend(graph.contradicting(id).iter().map(|c| Touched {
        id: (*c).to_string(),
        via: Via::Contradicts,
    }));
    Ok(Impact(out))
}
