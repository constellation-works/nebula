//! `near`: the nodes lexically closest to a query, by BM25 over their words,
//! read as a coarse band.

use crate::error::Result;
use crate::model::{Doc, Status};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fmt;

use super::{EdgeRecord, Graph};

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
    /// Lexical similarity in `0..=1`, rounded to three places. Not
    /// calibrated: `1` would be a node that saturates every word of the
    /// query, which nothing does, and a word-for-word copy of the query
    /// scores about `0.35`–`0.6`. [`Band`] is the reading of it.
    pub score: f64,
    /// The score read as a coarse band, which is what a human is shown.
    pub band: Band,
    /// The edges already joining this node and the node `near` was asked
    /// about, either way round, each once, in declaration order. Null when
    /// there are none, which is always the case for a free-text query.
    pub linked: Option<Vec<EdgeRecord>>,
}

/// How much of a query a [`Neighbour`] shares, coarsely.
///
/// A band and not the number, because the number invites a precision it
/// does not have: it is word overlap scaled by a ceiling nothing reaches,
/// so a duplicate reads as `0.4` and a reader takes that for "40% alike".
/// The cut-offs were read off a synthetic corpus of sixteen varied nodes:
///
/// - word-for-word duplicates and promoted copies of a node scored
///   `0.44`–`0.59` (one in a real corpus scored `0.36`), and
///   sentences reusing most of a node's key words `0.28`–`0.53`;
/// - a sentence sharing one incidental word with a node scored
///   `0.09`–`0.20`;
/// - a whole node against the others scored `0.07`–`0.13` for its topical
///   neighbours and under `0.07` for nearly everything else.
///
/// So [`Band::Strong`] starts at [`STRONG_FROM`], with margin under every
/// copy and over every one-word match, and [`Band::Weak`] ends at
/// [`SOME_FROM`], where a node's unrelated neighbours stop. Short queries score
/// higher for the same overlap, since one word is a larger share of two
/// than of twenty, so a one-word match on a two-word query can read
/// `strong`. The band ranks nothing; the order is still the score's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Band {
    /// Shares most of the query's distinctive words: a possible duplicate,
    /// or the obvious parent. Scores from [`STRONG_FROM`] up.
    Strong,
    /// Shares a few distinctive words: worth reading before deciding.
    /// Scores from [`SOME_FROM`] up to [`STRONG_FROM`].
    Some,
    /// Shares a word or two in passing. Scores under [`SOME_FROM`].
    Weak,
}

/// The lowest score read as [`Band::Strong`].
pub const STRONG_FROM: f64 = 0.25;
/// The lowest score read as [`Band::Some`].
pub const SOME_FROM: f64 = 0.07;

impl Band {
    /// The band a score falls in.
    pub fn of(score: f64) -> Self {
        if score >= STRONG_FROM {
            Self::Strong
        } else if score >= SOME_FROM {
            Self::Some
        } else {
            Self::Weak
        }
    }
}

impl fmt::Display for Band {
    /// Honours width and alignment, so a renderer can line bands up.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Self::Strong => "strong",
            Self::Some => "some",
            Self::Weak => "weak",
        })
    }
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
/// Each neighbour carries its score's [`Band`]. When `query` is a node,
/// each also carries the edges already joining it to that node, so a parent
/// or a child in the answer reads as a link that exists and not as one to
/// make.
///
/// This *suggests*. It never writes an edge, and a caller that turned the
/// first answer into a `--parent` unread would be doing the automatic
/// linking the spec rules out.
///
/// Nodes that share no word with the query are not returned, so an empty
/// list is the honest answer for a thought unlike anything in the corpus.
/// Ties are broken by id, so the same corpus and query give the same order.
pub fn near(graph: &Graph<'_>, query: &str, k: usize) -> Result<Near> {
    near_counted(graph, query, k).map(|(near, _)| near)
}

/// [`near`], with how many nodes matched before the cut to `k`: every node
/// sharing a word with the query, so a caller can say the answer is capped.
///
/// A count beside [`Near`] rather than inside it, so the list keeps the
/// bare shape the desktop reads.
pub fn near_counted(graph: &Graph<'_>, query: &str, k: usize) -> Result<(Near, usize)> {
    let query = query.trim();
    let (text, exclude) = match graph.get(query) {
        Some(doc) => (node_text(doc), Some(doc.node.id.as_str())),
        None => (query.to_string(), None),
    };
    let terms: BTreeSet<String> = tokens(&text).into_iter().collect();
    if terms.is_empty() {
        return Ok((Near(Vec::new()), 0));
    }

    // Index every candidate: weighted term frequencies and weighted length.
    let candidates: Vec<&Doc> = graph
        .docs()
        .iter()
        .filter(|d| exclude != Some(d.node.id.as_str()))
        .collect();
    if candidates.is_empty() {
        return Ok((Near(Vec::new()), 0));
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
        return Ok((Near(Vec::new()), 0));
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
    let matched = scored.len();
    scored.truncate(k);
    let near = Near(
        scored
            .into_iter()
            .map(|(score, d)| {
                // Banded after rounding, so the band always agrees with the
                // score a reader of `--json` sees beside it.
                let score = (score * 1000.0).round() / 1000.0;
                Neighbour {
                    id: d.node.id.clone(),
                    title: d.node.title.clone(),
                    status: d.node.status,
                    tags: d.node.tags.clone(),
                    score,
                    band: Band::of(score),
                    linked: exclude.and_then(|q| links_between(graph, q, &d.node.id)),
                }
            })
            .collect(),
    );
    Ok((near, matched))
}

/// Every edge between two nodes, either way round, each distinct claim
/// once and in declaration order: `a`'s edges to `b`, then `b`'s to `a`.
/// `None` when there are none.
fn links_between(graph: &Graph<'_>, a: &str, b: &str) -> Option<Vec<EdgeRecord>> {
    let mut out: Vec<EdgeRecord> = Vec::new();
    for (from, to) in [(a, b), (b, a)] {
        let Some(doc) = graph.get(from) else { continue };
        for e in doc.node.edges.iter().filter(|e| e.to == to) {
            // The same claim written twice is one claim, as in `has_edge`.
            if !out.iter().any(|r| r.from == from && r.kind == e.kind) {
                out.push(EdgeRecord {
                    from: from.to_string(),
                    kind: e.kind,
                    to: to.to_string(),
                });
            }
        }
    }
    (!out.is_empty()).then_some(out)
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
