//! The whole corpus as a drawable graph, for the desktop and any other tool
//! that lays it out.

use crate::error::Result;
use crate::model::{EdgeType, Status};
use serde::Serialize;

use super::Graph;

/// The whole corpus as a drawable graph.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct GraphExport {
    /// Every node, in corpus order.
    pub nodes: Vec<NodeSummary>,
    /// Every edge, as declared, in node order.
    pub edges: Vec<EdgeRecord>,
}

/// A node as a graph vertex: what a layout and a label need, nothing else.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NodeSummary {
    /// The node's id.
    pub id: String,
    /// Its one-line title.
    pub title: String,
    /// Where it is in its lifecycle.
    pub status: Status,
    /// Its labels.
    pub tags: Vec<String>,
    /// When it entered the graph.
    pub created: String,
    /// When it last changed.
    pub updated: String,
}

/// One edge, with both ends named, so the drawing does not have to look a
/// node up to know where an edge starts.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct EdgeRecord {
    /// The node the edge starts at.
    pub from: String,
    /// The relation it asserts.
    #[serde(rename = "type")]
    pub kind: EdgeType,
    /// The node it points at.
    pub to: String,
}

/// The whole corpus, for a tool that draws it.
pub fn export(graph: &Graph<'_>) -> Result<GraphExport> {
    let mut nodes = Vec::with_capacity(graph.docs().len());
    let mut edges = Vec::new();
    for d in graph.docs() {
        let n = &d.node;
        nodes.push(NodeSummary {
            id: n.id.clone(),
            title: n.title.clone(),
            status: n.status,
            tags: n.tags.clone(),
            created: n.created.clone(),
            updated: n.updated.clone(),
        });
        for e in &n.edges {
            edges.push(EdgeRecord {
                from: n.id.clone(),
                kind: e.kind,
                to: e.to.clone(),
            });
        }
    }
    Ok(GraphExport { nodes, edges })
}
