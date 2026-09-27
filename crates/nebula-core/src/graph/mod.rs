//! The graph and the queries over it: trace, impact, open, review, export,
//! near, plus the two listings the CLI and the desktop both need.
//!
//! A [`Graph`] is an indexed snapshot of loaded nodes. It is built once per
//! command, and every query here is pure over it: nothing reads the disk,
//! nothing prints, and each returns a `Serialize` struct that is both the
//! `--json` output and the desktop's IPC payload. That is what lets the
//! desktop hold a graph in memory and rebuild it on a file-watch event.
//!
//! The queries are spread over this module's files by what they answer:
//!
//! - `trace`  lineage walks up or down, and `impact`
//! - `review` `open` and `review`, and the thresholds they apply
//! - `export` the whole corpus as vertices and edges
//! - `near`   lexical similarity, read as a band
//! - `cycles` genealogy cycles, for `link` and for `check`
//!
//! This file holds [`Graph`] itself and the views of single nodes and tags:
//! [`node`], [`list`] and [`tags`].

mod cycles;
mod export;
mod near;
mod review;
mod trace;

pub(crate) use cycles::creates_cycle;
use cycles::find_cycle;
pub use export::{EdgeRecord, GraphExport, NodeSummary, export};
pub use near::{Band, NEAR_DEFAULT, Near, Neighbour, SOME_FROM, STRONG_FROM, near, near_counted};
pub use review::{
    HYPOTHESIS_DAYS, INBOX_DAYS, NO_REFERENCES_DAYS, OpenItem, OpenReport, ReviewItem,
    ReviewReport, ReviewRule, SEED_DAYS, open, review,
};
pub use trace::{
    Direction, Impact, Touched, Trace, TraceHop, TraceNode, Via, impact, trace, trace_within,
};

use crate::check_impl::{OBSERVATORY, resolve_observatory};
use crate::error::{Error, Result};
use crate::model::{self, Doc, EdgeType, Node, Note, Status};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// An indexed snapshot of a loaded corpus.
///
/// Borrowed rather than owning: the docs outlive the graph, and a query that
/// wanted them owned would copy the whole corpus on every command.
#[derive(Debug)]
pub struct Graph<'a> {
    docs: &'a [Doc],
    by_id: HashMap<&'a str, &'a Doc>,
    parents: HashMap<&'a str, Vec<&'a str>>,
    children: HashMap<&'a str, Vec<&'a str>>,
    contradicts: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Graph<'a> {
    /// How many nodes the graph holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Index a set of loaded nodes.
    ///
    /// Ids are the corpus's primary key, so two nodes claiming one id is a
    /// corpus that cannot be reasoned about rather than a finding to report.
    pub fn build(docs: &'a [Doc]) -> Result<Self> {
        let mut by_id: HashMap<&str, &Doc> = HashMap::with_capacity(docs.len());
        let mut parents: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut contradicts: HashMap<&str, Vec<&str>> = HashMap::new();
        for doc in docs {
            let id = doc.node.id.as_str();
            if by_id.insert(id, doc).is_some() {
                return Err(Error::DuplicateId(id.to_string()));
            }
            // Parallel edges collapse here: a node that both derives from
            // and reopens another has one parent, not two of the same.
            let lineage: Vec<&str> = doc.node.lineage().into_iter().map(|(p, _)| p).collect();
            for p in &lineage {
                children.entry(p).or_default().push(id);
            }
            parents.insert(id, lineage);
            contradicts.insert(id, doc.node.edges_of(EdgeType::Contradicts).collect());
        }
        Ok(Self {
            docs,
            by_id,
            parents,
            children,
            contradicts,
        })
    }

    /// Every node, in corpus order.
    pub(crate) fn docs(&self) -> &'a [Doc] {
        self.docs
    }

    /// One node, if it is here.
    pub(crate) fn get(&self, id: &str) -> Option<&'a Doc> {
        self.by_id.get(id).copied()
    }

    /// One node, or [`Error::NoSuchNode`].
    pub(crate) fn require(&self, id: &str) -> Result<&'a Doc> {
        self.get(id)
            .ok_or_else(|| Error::NoSuchNode(id.to_string()))
    }

    /// Genealogical parents of a node, each once.
    pub(crate) fn parents_of(&self, id: &str) -> &[&'a str] {
        self.parents.get(id).map_or(&[], Vec::as_slice)
    }

    /// Nodes that name this one as a parent, each once.
    pub(crate) fn children_of(&self, id: &str) -> &[&'a str] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    /// Nodes this one declares itself in conflict with.
    pub(crate) fn contradicting(&self, id: &str) -> &[&'a str] {
        self.contradicts.get(id).map_or(&[], Vec::as_slice)
    }

    /// The first genealogy cycle, as a readable path. Diamonds are legal and
    /// expected; only a loop is not.
    pub(crate) fn cycle(&self) -> Option<Vec<String>> {
        find_cycle(&self.parents)
    }
}

/// One node in full: the frontmatter and the prose.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NodeView {
    /// The structured fields.
    pub node: Node,
    /// The argument, trimmed. Includes the `## Notes` section when one exists.
    pub body: String,
    /// Dated reasoning parsed from every `## Notes` section, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Where each `observatory` reference lands on this machine. Filled in
    /// by [`NodeView::with_observatory`], and empty until a caller asks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observatory: Vec<ObservatoryLink>,
    /// The Observatory record the node was handed off to, when it was; see
    /// [`Node::handed_off_to`]. Omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_off_to: Option<String>,
}

/// One `observatory` reference, located on this machine.
///
/// The record id is what the corpus stores; the path is what this machine
/// happens to have, and is absent when no root is set or the checkout does
/// not carry the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ObservatoryLink {
    /// The reference id within the node, `r1` and up.
    pub reference: String,
    /// The Observatory record id, as stored: `Q002`, `H007`, `R012`.
    pub record: String,
    /// Where that record is, when it resolves under the configured root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl NodeView {
    /// Locate every `observatory` reference under `root`.
    ///
    /// The one query here that touches the filesystem, which is why it is a
    /// step a caller takes rather than part of [`node`]: a reader that only
    /// wants the node pays nothing, and the rest of this module stays pure
    /// over the graph.
    pub fn with_observatory(mut self, root: Option<&Path>) -> Result<Self> {
        self.observatory = self
            .node
            .references
            .iter()
            .filter(|r| r.kind == OBSERVATORY)
            .filter_map(|r| r.uri.clone().map(|record| (r, record)))
            .map(|(r, record)| {
                Ok(ObservatoryLink {
                    reference: r.id.clone(),
                    path: root
                        .map(|root| resolve_observatory(root, &record))
                        .transpose()?
                        .flatten(),
                    record,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(self)
    }
}

/// Show one node in full.
///
/// Authorship is stated outright here, including the `human` the file leaves
/// implicit, so a reader of `--json` sees who wrote each field without having
/// to know that an absent author means the human.
pub fn node(graph: &Graph<'_>, id: &str) -> Result<NodeView> {
    let doc = graph.require(id)?;
    let body = doc.body.trim().to_string();
    Ok(NodeView {
        node: doc.node.clone().with_authorship_stated(),
        notes: model::notes_from_body(&body),
        body,
        observatory: Vec::new(),
        handed_off_to: doc.node.handed_off_to().map(str::to_owned),
    })
}

/// Nodes matching a filter. Serializes as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Listing(pub Vec<Node>);

/// List nodes. Several tags narrow to nodes carrying all of them.
///
/// Authorship is stated as it is by [`node`], so the two agree.
pub fn list(graph: &Graph<'_>, status: Option<Status>, tags: &[String]) -> Result<Listing> {
    let tags = model::normalize_tags(tags);
    Ok(Listing(
        graph
            .docs()
            .iter()
            .filter(|d| status.is_none_or(|s| d.node.status == s))
            .filter(|d| d.node.has_all_tags(&tags))
            .map(|d| d.node.clone().with_authorship_stated())
            .collect(),
    ))
}

/// Every tag in the corpus with a count. Serializes as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TagCounts(pub Vec<TagCount>);

/// One tag and the number of nodes carrying it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TagCount {
    /// The tag, as it is written on the nodes.
    pub tag: String,
    /// How many nodes carry it.
    pub count: usize,
}

/// Every tag with the number of nodes carrying it, alphabetically.
pub fn tags(graph: &Graph<'_>) -> Result<TagCounts> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for d in graph.docs() {
        for t in &d.node.tags {
            *counts.entry(t.as_str()).or_default() += 1;
        }
    }
    Ok(TagCounts(
        counts
            .into_iter()
            .map(|(tag, count)| TagCount {
                tag: tag.to_string(),
                count,
            })
            .collect(),
    ))
}
