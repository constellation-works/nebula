//! The graph and the queries over it: trace, impact, open, review, export,
//! near, plus the two listings the CLI and the desktop both need.
//!
//! A [`Graph`] is an indexed snapshot of loaded nodes. It is built once per
//! command, and every query here is pure over it: nothing reads the disk,
//! nothing prints, and each returns a `Serialize` struct that is both the
//! `--json` output and the desktop's IPC payload. That is what lets the
//! desktop hold a graph in memory and rebuild it on a file-watch event.

use crate::check::{OBSERVATORY, resolve_observatory};
use crate::error::{Error, Result};
use crate::model::{self, Doc, EdgeType, Node, Note, Status};
use crate::store::{self, Inbox};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Days a seed may sit untouched before `open` and `review` raise it.
pub const SEED_DAYS: i64 = 90;
/// Days a hypothesis may sit untouched before `review` calls it stale.
pub const HYPOTHESIS_DAYS: i64 = 30;
/// Days an inbox capture may wait before `open` and `review` raise it.
pub const INBOX_DAYS: i64 = 14;
/// Days a new node may remain without references before `open` and `review` raise it.
pub const NO_REFERENCES_DAYS: i64 = 14;

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

/// Hypotheses at least fourteen days old with no references, seeds untouched
/// for ninety days, and inbox captures waiting fourteen days or more.
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
        if n.status == Status::Hypothesis
            && n.references.is_empty()
            && older_than(&n.created, NO_REFERENCES_DAYS)
        {
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
    /// A hypothesis standing on a kill condition the human never wrote.
    UnconfirmedKill,
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
    let mut unconfirmed_kills = Vec::new();

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
        if n.status == Status::Hypothesis
            && n.references.is_empty()
            && older_than(&n.created, NO_REFERENCES_DAYS)
        {
            no_references.push(ReviewItem {
                rule: ReviewRule::NoReferences,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: "no references attached".into(),
            });
        }
        // A hypothesis is only as honest as the falsifier under it, and a
        // falsifier somebody else proposed is not yet the human's claim.
        if n.status == Status::Hypothesis && !model::is_human(n.kill_by.as_deref()) {
            let by = n.kill_by.as_deref().unwrap_or(model::HUMAN);
            unconfirmed_kills.push(ReviewItem {
                rule: ReviewRule::UnconfirmedKill,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("kill condition written by `{by}`, not confirmed by a human"),
            });
        }
    }

    let waiting = stale_inbox(inbox);
    let mut items = stale_hypotheses;
    items.append(&mut untouched_seeds);
    items.append(&mut no_references);
    items.append(&mut unconfirmed_kills);
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
    /// The argument, trimmed. Includes the `## Notes` section when one exists.
    pub body: String,
    /// Dated reasoning parsed from every `## Notes` section, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Where each `observatory` reference lands on this machine. Filled in
    /// by [`NodeView::with_observatory`], and empty until a caller asks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observatory: Vec<ObservatoryLink>,
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
    #[must_use]
    pub fn with_observatory(mut self, root: Option<&Path>) -> Self {
        self.observatory = self
            .node
            .references
            .iter()
            .filter(|r| r.kind == OBSERVATORY)
            .filter_map(|r| {
                let record = r.uri.clone()?;
                Some(ObservatoryLink {
                    reference: r.id.clone(),
                    path: root.and_then(|root| resolve_observatory(root, &record)),
                    record,
                })
            })
            .collect();
        self
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

/// The nodes closest to a query, best first. Serializes as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Near(pub Vec<Neighbour>);

/// One existing node a query lands near, and how near.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Neighbour {
    /// The node's id.
    pub id: String,
    /// Its one-line title.
    pub title: String,
    /// Where it is in its lifecycle.
    pub status: Status,
    /// Its labels, since a promotion reuses the parent's.
    pub tags: Vec<String>,
    /// Lexical similarity in `0..=1`, rounded to three places. `1` would be a
    /// node that saturates every word of the query; anything over about
    /// `0.3` shares real vocabulary rather than one incidental word.
    pub score: f64,
}

/// How many neighbours `near` returns unless told otherwise.
pub const NEAR_DEFAULT: usize = 3;

/// BM25 term-frequency saturation. The textbook value.
const BM25_K1: f64 = 1.2;
/// BM25 length normalisation. The textbook value.
const BM25_B: f64 = 0.75;
/// A word in the title counts this many times over one in the body.
const TITLE_WEIGHT: f64 = 3.0;
/// A tag counts this many times over a word in the body.
const TAG_WEIGHT: f64 = 2.0;

/// Words that are in every node and say nothing about which one.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "because",
    "been", "before", "being", "but", "by", "can", "could", "did", "do", "does", "doing", "for",
    "from", "had", "has", "have", "having", "how", "i", "if", "in", "into", "is", "it", "its",
    "just", "may", "might", "more", "most", "my", "no", "not", "of", "on", "one", "only", "or",
    "our", "out", "over", "should", "so", "some", "than", "that", "the", "their", "them", "then",
    "there", "these", "they", "this", "those", "to", "too", "up", "very", "was", "we", "were",
    "what", "when", "where", "which", "while", "who", "why", "will", "with", "would", "you",
    "your",
];

/// The nodes lexically closest to `query`, at most `k` of them, best first.
///
/// `query` is free text, or the id of an existing node, in which case that
/// node's own title, body and tags are the query and the node itself is left
/// out of the answer. Similarity is BM25 over the words of title, tags and
/// body (title and tags weighted up), with the sum normalised by what a
/// document saturating every query word would score, so the result sits in
/// `0..=1` and is comparable across queries. No embeddings, no network, no
/// dependency: at the corpus sizes this tool is for, word overlap is enough
/// to put the right candidates in front of whoever is choosing a parent.
///
/// This *suggests*. It never writes an edge, and a caller that turned the
/// first answer into a `--parent` unread would be doing the automatic
/// linking the spec rules out.
///
/// Nodes that share no word with the query are not returned, so an empty
/// list is the honest answer for a thought unlike anything in the corpus.
/// Ties are broken by id, so the same corpus and query give the same order.
pub fn near(graph: &Graph<'_>, query: &str, k: usize) -> Result<Near> {
    let query = query.trim();
    let (text, exclude) = match graph.get(query) {
        Some(doc) => (node_text(doc), Some(doc.node.id.as_str())),
        None => (query.to_string(), None),
    };
    let terms: BTreeSet<String> = tokens(&text).into_iter().collect();
    if terms.is_empty() || k == 0 {
        return Ok(Near(Vec::new()));
    }

    // Index every candidate: weighted term frequencies and weighted length.
    let candidates: Vec<&Doc> = graph
        .docs()
        .iter()
        .filter(|d| exclude != Some(d.node.id.as_str()))
        .collect();
    if candidates.is_empty() {
        return Ok(Near(Vec::new()));
    }
    let indexed: Vec<(&Doc, HashMap<String, f64>, f64)> = candidates
        .iter()
        .map(|d| {
            let (tf, len) = term_weights(d);
            (*d, tf, len)
        })
        .collect();
    let avg_len = indexed.iter().map(|(_, _, len)| len).sum::<f64>() / to_f64(indexed.len());
    let n = to_f64(indexed.len());

    // Inverse document frequency per query term, and the ceiling the
    // normalisation divides by.
    let idf: Vec<(&str, f64)> = terms
        .iter()
        .map(|t| {
            let df = to_f64(
                indexed
                    .iter()
                    .filter(|(_, tf, _)| tf.contains_key(t))
                    .count(),
            );
            (t.as_str(), (1.0 + (n - df + 0.5) / (df + 0.5)).ln())
        })
        .collect();
    let ceiling = (BM25_K1 + 1.0) * idf.iter().map(|(_, w)| w).sum::<f64>();
    if ceiling <= 0.0 {
        return Ok(Near(Vec::new()));
    }

    let mut scored: Vec<(f64, &Doc)> = indexed
        .iter()
        .filter_map(|(doc, tf, len)| {
            let norm = BM25_K1 * (1.0 - BM25_B + BM25_B * len / avg_len);
            let score: f64 = idf
                .iter()
                .filter_map(|(t, w)| tf.get(*t).map(|f| w * f * (BM25_K1 + 1.0) / (f + norm)))
                .sum();
            (score > 0.0).then_some((score / ceiling, *doc))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.node.id.cmp(&b.1.node.id))
    });
    scored.truncate(k);
    Ok(Near(
        scored
            .into_iter()
            .map(|(score, d)| Neighbour {
                id: d.node.id.clone(),
                title: d.node.title.clone(),
                status: d.node.status,
                tags: d.node.tags.clone(),
                score: (score * 1000.0).round() / 1000.0,
            })
            .collect(),
    ))
}

/// Everything about a node that a similarity should read, as one text.
fn node_text(doc: &Doc) -> String {
    format!(
        "{}\n{}\n{}",
        doc.node.title,
        doc.node.tags.join(" "),
        doc.body
    )
}

/// Weighted term frequencies of one node, and its weighted length.
fn term_weights(doc: &Doc) -> (HashMap<String, f64>, f64) {
    let mut tf: HashMap<String, f64> = HashMap::new();
    let mut len = 0.0;
    for (text, weight) in [
        (doc.node.title.as_str(), TITLE_WEIGHT),
        (doc.body.as_str(), 1.0),
    ] {
        for t in tokens(text) {
            *tf.entry(t).or_default() += weight;
            len += weight;
        }
    }
    // Tags are already single labels; `tokens` splits a kebab-case one into
    // its words, so `ranking-decay` meets a body that says "ranking decay".
    for tag in &doc.node.tags {
        for t in tokens(tag) {
            *tf.entry(t).or_default() += TAG_WEIGHT;
            len += TAG_WEIGHT;
        }
    }
    (tf, len)
}

/// Words of a text: lowercased, split on anything that is not a letter or a
/// digit, stopwords dropped, one-letter fragments dropped, and lightly
/// stemmed so `tags` meets `tag` and `linking` meets `link`.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !STOPWORDS.contains(&w.as_str()))
        .map(stem)
        .collect()
}

/// A suffix strip, not a stemmer: enough for plurals and `-ing`, and wrong
/// often enough that it is a similarity heuristic and not a search index.
fn stem(w: String) -> String {
    if let Some(base) = w.strip_suffix("ies").filter(|b| b.len() >= 3) {
        return format!("{base}y");
    }
    if let Some(base) = w.strip_suffix("ing").filter(|b| b.len() >= 4) {
        return base.to_string();
    }
    if let Some(base) = w
        .strip_suffix('s')
        .filter(|b| b.len() >= 3 && !b.ends_with('s'))
    {
        return base.to_string();
    }
    w
}

/// A count as a float, for the BM25 arithmetic. A corpus will not reach the
/// size where this loses precision.
#[allow(clippy::cast_precision_loss)]
fn to_f64(n: usize) -> f64 {
    n as f64
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
        if !state.contains_key(*start)
            && let Some(c) = dfs(start, graph, &mut state, &mut stack)
        {
            return Some(c);
        }
    }
    None
}
