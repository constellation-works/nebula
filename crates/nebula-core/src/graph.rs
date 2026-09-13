//! The graph and the queries over it: trace, impact, open, review, export,
//! plus the two listings the CLI and the desktop both need.
//!
//! A [`Graph`] is an indexed snapshot of loaded nodes. It is built once per
//! command, and every query here is pure over it: nothing reads the disk,
//! nothing prints, and each returns a `Serialize` struct that is both the
//! `--json` output and the desktop's IPC payload. That is what lets the
//! desktop hold a graph in memory and rebuild it on a file-watch event.

use crate::error::{Error, Result};
use crate::model::{self, Doc, EdgeType, Node, Status};
use crate::store::{self, Inbox};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Days a seed may sit untouched before `open` and `review` raise it.
pub const SEED_DAYS: i64 = 90;
/// Days a hypothesis may sit untouched before `review` calls it stale.
pub const HYPOTHESIS_DAYS: i64 = 30;
/// Days an inbox capture may wait before `open` and `review` raise it.
pub const INBOX_DAYS: i64 = 14;

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
            parents.insert(id, doc.node.parents().collect());
            for p in doc.node.parents() {
                children.entry(p).or_default().push(id);
            }
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

    /// Genealogical parents of a node.
    pub(crate) fn parents_of(&self, id: &str) -> &[&'a str] {
        self.parents.get(id).map_or(&[], Vec::as_slice)
    }

    /// Nodes that name this one as a parent.
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
    /// Its genealogical parents.
    pub parents: Vec<String>,
}

/// Walk ancestry, or descent. The feature the whole system exists for.
pub fn trace(graph: &Graph<'_>, id: &str, direction: Direction) -> Result<Trace> {
    graph.require(id)?;
    let mut acc = Vec::new();
    let mut seen = HashSet::new();
    collect(graph, id, direction, &mut seen, &mut acc);
    Ok(Trace(acc))
}

fn collect(
    graph: &Graph<'_>,
    id: &str,
    direction: Direction,
    seen: &mut HashSet<String>,
    acc: &mut Vec<TraceNode>,
) {
    if !seen.insert(id.to_string()) {
        return;
    }
    let Some(doc) = graph.get(id) else { return };
    acc.push(TraceNode {
        id: doc.node.id.clone(),
        title: doc.node.title.clone(),
        status: doc.node.status,
        parents: doc.node.parents().map(String::from).collect(),
    });
    let next: Vec<String> = match direction {
        Direction::Down => graph
            .children_of(id)
            .iter()
            .map(|c| (*c).to_string())
            .collect(),
        Direction::Up => graph
            .parents_of(id)
            .iter()
            .map(|p| (*p).to_string())
            .collect(),
    };
    for n in next {
        collect(graph, &n, direction, seen, acc);
    }
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

/// Nodes that need attention. Serializes as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpenReport(pub Vec<OpenItem>);

/// One thing waiting on you.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpenItem {
    /// The node, or `inbox` for the inbox as a whole.
    pub id: String,
    /// What it is waiting for.
    pub why: String,
}

/// Hypotheses with no references, seeds untouched for ninety days, and inbox
/// captures waiting fourteen days or more.
pub fn open(graph: &Graph<'_>, inbox: &Inbox, tags: &[String]) -> Result<OpenReport> {
    let tags = model::normalize_tags(tags);
    let mut items: Vec<OpenItem> = Vec::new();
    let waiting = stale_inbox(inbox);
    if waiting > 0 {
        items.push(OpenItem {
            id: "inbox".into(),
            why: format!("{waiting} captures waiting over fourteen days; promote or drop them"),
        });
    }
    for d in graph.docs().iter().filter(|d| d.node.has_all_tags(&tags)) {
        let n = &d.node;
        // The genuinely actionable gap: a hypothesis that names what would
        // kill it and has nothing attached that bears on the question.
        if n.status == Status::Hypothesis && n.references.is_empty() {
            items.push(OpenItem {
                id: n.id.clone(),
                why: "hypothesis with no references".into(),
            });
        }
        if n.status == Status::Seed && older_than(&n.updated, SEED_DAYS) {
            items.push(OpenItem {
                id: n.id.clone(),
                why: "seed untouched for ninety days; abandon it?".into(),
            });
        }
    }
    Ok(OpenReport(items))
}

/// The weekly maintenance report. Serializes as the bare list of findings.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReviewReport(pub Vec<ReviewItem>);

/// Which review rule raised a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "kebab-case")]
pub enum ReviewRule {
    /// A hypothesis nobody has touched lately.
    StaleHypothesis,
    /// A seed gone cold.
    UntouchedSeed,
    /// A live node with nothing attached to it.
    NoReferences,
    /// Captures rotting in the inbox.
    StaleInbox,
}

/// One finding from `review`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReviewItem {
    /// Which rule raised it.
    pub rule: ReviewRule,
    /// The node, or `inbox`.
    pub id: String,
    /// Its title, or `inbox`.
    pub title: String,
    /// What to look at, and what is proposed.
    pub reason: String,
}

/// Hypotheses gone quiet, seeds gone cold, nodes nobody has situated, and
/// inbox captures rotting unprocessed.
///
/// Read-only by design. This walks a graph and returns findings; it never
/// edits a node, an inbox entry, or the config. The judgement calls (kill
/// conditions that look satisfied, likely `contradicts` pairs) stay out of
/// scope for the same reason: they belong to the agent that reads this
/// report, not to this query.
///
/// `since` overrides the hypothesis and seed thresholds together. The inbox's
/// fourteen-day rule is unaffected; it is `open`'s rule, reused rather than
/// duplicated.
pub fn review(graph: &Graph<'_>, inbox: &Inbox, since: Option<i64>) -> Result<ReviewReport> {
    let hypothesis_days = since.unwrap_or(HYPOTHESIS_DAYS);
    let seed_days = since.unwrap_or(SEED_DAYS);

    let mut stale_hypotheses = Vec::new();
    let mut untouched_seeds = Vec::new();
    let mut no_references = Vec::new();

    for d in graph.docs() {
        let n = &d.node;
        if n.status == Status::Hypothesis && older_than(&n.updated, hypothesis_days) {
            stale_hypotheses.push(ReviewItem {
                rule: ReviewRule::StaleHypothesis,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("hypothesis untouched for {hypothesis_days} days"),
            });
        }
        if n.status == Status::Seed && older_than(&n.updated, seed_days) {
            untouched_seeds.push(ReviewItem {
                rule: ReviewRule::UntouchedSeed,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("seed untouched for {seed_days} days; propose: status abandoned"),
            });
        }
        if n.status.is_open() && n.references.is_empty() {
            no_references.push(ReviewItem {
                rule: ReviewRule::NoReferences,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: "no references attached".into(),
            });
        }
    }

    let waiting = stale_inbox(inbox);
    let mut items = stale_hypotheses;
    items.append(&mut untouched_seeds);
    items.append(&mut no_references);
    if waiting > 0 {
        items.push(ReviewItem {
            rule: ReviewRule::StaleInbox,
            id: "inbox".into(),
            title: "inbox".into(),
            reason: format!("{waiting} captures waiting over fourteen days; promote or drop them"),
        });
    }
    Ok(ReviewReport(items))
}

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

/// One node in full: the frontmatter and the prose.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NodeView {
    /// The structured fields.
    pub node: Node,
    /// The argument, trimmed.
    pub body: String,
}

/// Show one node in full.
pub fn node(graph: &Graph<'_>, id: &str) -> Result<NodeView> {
    let doc = graph.require(id)?;
    Ok(NodeView {
        node: doc.node.clone(),
        body: doc.body.trim().to_string(),
    })
}

/// Nodes matching a filter. Serializes as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Listing(pub Vec<Node>);

/// List nodes. Several tags narrow to nodes carrying all of them.
pub fn list(graph: &Graph<'_>, status: Option<Status>, tags: &[String]) -> Result<Listing> {
    let tags = model::normalize_tags(tags);
    Ok(Listing(
        graph
            .docs()
            .iter()
            .filter(|d| status.is_none_or(|s| d.node.status == s))
            .filter(|d| d.node.has_all_tags(&tags))
            .map(|d| d.node.clone())
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

/// Whether a `YYYY-MM-DD` date is at least `days` old.
fn older_than(date: &str, days: i64) -> bool {
    store::days_since(date).is_some_and(|d| d >= days)
}

/// Captures waiting at least fourteen days.
fn stale_inbox(inbox: &Inbox) -> usize {
    inbox
        .0
        .iter()
        .filter_map(|entry| store::days_since_stamp(&entry.at))
        .filter(|days| *days >= INBOX_DAYS)
        .count()
}

/// Whether adding an edge has made `start` its own ancestor.
///
/// Takes the docs as they would be on disk, so `link` can refuse the edge
/// rather than leaving `check` to find the loop later.
pub(crate) fn creates_cycle(docs: &[Doc], start: &str) -> bool {
    let by_id: HashMap<&str, &Doc> = docs.iter().map(|d| (d.node.id.as_str(), d)).collect();
    let mut stack = vec![start.to_string()];
    let mut seen = HashSet::new();
    while let Some(at) = stack.pop() {
        let Some(doc) = by_id.get(at.as_str()) else {
            continue;
        };
        for p in doc.node.parents() {
            if p == start {
                return true;
            }
            if seen.insert(p.to_string()) {
                stack.push(p.to_string());
            }
        }
    }
    false
}

// ------------------------------------------------------------ cycle search --

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    OnStack,
    Done,
}

fn dfs<'a>(
    at: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    state: &mut HashMap<&'a str, Mark>,
    stack: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    state.insert(at, Mark::OnStack);
    stack.push(at);
    for next in graph.get(at).map(Vec::as_slice).unwrap_or_default() {
        match state.get(next) {
            Some(Mark::OnStack) => {
                let from = stack.iter().position(|s| s == next).unwrap_or(0);
                let mut c: Vec<String> = stack[from..].iter().map(ToString::to_string).collect();
                c.push((*next).to_string());
                return Some(c);
            }
            None => {
                if let Some(c) = dfs(next, graph, state, stack) {
                    return Some(c);
                }
            }
            Some(Mark::Done) => {}
        }
    }
    stack.pop();
    state.insert(at, Mark::Done);
    None
}

/// First cycle in a directed graph, as a readable path.
fn find_cycle<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<String>> {
    let mut state: HashMap<&str, Mark> = HashMap::new();
    let mut stack: Vec<&str> = Vec::new();
    // Sorted so the reported cycle does not change between runs on the same
    // corpus, which matters when the output goes into a review file.
    let mut roots: Vec<&&str> = graph.keys().collect();
    roots.sort_unstable();
    for start in roots {
        if !state.contains_key(*start) {
            if let Some(c) = dfs(start, graph, &mut state, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}
