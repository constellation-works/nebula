//! Terminal output: everything that turns a core type into text.
//!
//! Colour comes from the output layer's one gate, decided per stream, so
//! piping into a file or an agent yields clean text. What a colour means is
//! a role (`output::Role`), never a raw code.
//!
//! stdout carries the payload alone (STD-01 §R12). Counts, cut notices and
//! empty-state lines are [`Notice`]s, which the caller writes to stderr.
//!
//! A list-shaped result is a [`table`]: aligned under a header on a
//! terminal, one tab-separated line per record anywhere else.

pub(crate) mod error;
pub(crate) mod json;
pub(crate) mod report;
pub(crate) mod table;
pub(crate) mod tree;
pub(crate) mod triage;

use crate::output::{self, Role, Stream};
use nebula_core::{EdgeType, Error, GraphExport, HistoryEntry, Node, Status};
use std::collections::HashSet;
use std::fmt::Write as _;
use table::{Cell, Column, Table};

pub(crate) use error::{Refusal, refusal, refusal_about, refusal_for_new};
pub(crate) use report::{
    check, check_tally, commit_setting, commit_setting_hint, dropped_legacy, impact, impact_notice,
    inbox, inbox_notice, migration, migration_notice, near, near_notice, node, observatory_root,
    observatory_root_notes, open, open_notice, retag_notes, review, review_notice, suggestions,
    tags, tags_notice, unchanged,
};
pub(crate) use table::Target;
pub(crate) use tree::{draw as tree, tabbed as trace_lines, trace_notice};
pub(crate) use triage::{step, tally, triage_keys, waiting};

/// Text in `role`'s colour, for stdout.
pub(crate) fn paint(role: Role, s: &str) -> String {
    output::paint(Stream::Stdout, role, s)
}

/// Dim text, for anything secondary.
pub(crate) fn dim(s: &str) -> String {
    paint(Role::Muted, s)
}

/// Bold text, for identifiers.
pub(crate) fn bold(s: &str) -> String {
    output::bold(Stream::Stdout, s)
}

/// Muted text, for a line on stderr about the result rather than in it.
pub(crate) fn notice(s: &str) -> String {
    output::paint(Stream::Stderr, Role::Muted, s)
}

/// The `error:` that starts a refusal, for stderr.
pub(crate) fn error_label() -> String {
    output::paint(Stream::Stderr, Role::Error, "error:")
}

/// A line about a result rather than part of it: a count, a cut, or an
/// empty result. It goes to stderr, never stdout (STD-01 §R12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    text: String,
    role: Role,
    /// Written under `--json` too. An empty result (§R16) and a cut list
    /// (§R34) say so in every mode; a plain count only to a person.
    every_mode: bool,
}

impl Notice {
    /// Said in every mode: nothing matched, or a bound cut the result.
    fn always(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::Muted,
            every_mode: true,
        }
    }

    /// Said in human mode only: a count or a hint.
    fn human(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::Muted,
            every_mode: false,
        }
    }

    /// The same notice, coloured as `role` says.
    fn in_role(self, role: Role) -> Self {
        Self { role, ..self }
    }

    /// The line to write to stderr, or `None` when `--json` leaves it out.
    pub(crate) fn line(&self, json: bool) -> Option<String> {
        (self.every_mode || !json).then(|| output::paint(Stream::Stderr, self.role, &self.text))
    }
}

/// `1 node`, `2 nodes`: a count with its noun agreeing. Every noun counted
/// here takes a plain `s`.
pub(crate) fn count<N>(n: N, noun: &str) -> String
where
    N: std::fmt::Display + PartialEq + From<u8> + Copy,
{
    if n == N::from(1) {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A status badge, coloured by whether the node still asks anything of you.
///
/// Padded to the longest status, so the ids after it line up in a column.
pub(crate) fn status_badge(s: Status) -> String {
    paint(Role::of("status", &s.to_string()), &format!("{s:<10}"))
}

/// A status as a table cell, coloured by whether the node still asks
/// anything of you.
pub(crate) fn status_cell(s: Status) -> Cell {
    Cell::new(Role::of("status", &s.to_string()), s.to_string())
}

/// A listing of nodes as a table, one record each, and nothing when there
/// are none. The tags are one cell, comma-joined, and `-` for none.
pub(crate) fn list(nodes: &[Node], to: Target) -> String {
    let mut table = Table::new([
        Column::left("STATUS"),
        Column::left("ID"),
        Column::left("TAGS"),
        Column::left("TITLE"),
    ]);
    for n in nodes {
        table.row([
            status_cell(n.status),
            Cell::bold(&n.id),
            Cell::muted(n.tags.join(",")),
            Cell::muted(&n.title),
        ]);
    }
    table.render(to)
}

/// What a listing is of the corpus. `matched` is how many the filter kept,
/// which is more than `shown` when `--limit` cut it.
pub(crate) fn list_notice(shown: usize, matched: usize, total: usize) -> Notice {
    if matched == 0 {
        Notice::always("no nodes match")
    } else if shown < matched && matched < total {
        Notice::always(format!(
            "{shown} of {} shown, of {total} in all; raise --limit for more",
            count(matched, "matching node")
        ))
    } else if shown < matched {
        Notice::always(format!(
            "{shown} of {} shown; raise --limit for more",
            count(total, "node")
        ))
    } else {
        Notice::human(format!("{matched} of {}", count(total, "node")))
    }
}

/// Commits that changed a node, newest first, as a table.
pub(crate) fn history(entries: &[HistoryEntry], to: Target) -> String {
    let mut table = Table::new([
        Column::left("HASH"),
        Column::left("DATE"),
        Column::left("MESSAGE"),
    ]);
    for entry in entries {
        let short = entry.hash.get(..7).unwrap_or(&entry.hash);
        table.row([
            Cell::bold(short),
            Cell::muted(&entry.date),
            Cell::plain(&entry.message),
        ]);
    }
    table.render(to)
}

/// The notice for a node no commit touched.
pub(crate) fn history_notice(entries: &[HistoryEntry]) -> Option<Notice> {
    entries
        .is_empty()
        .then(|| Notice::always("no commits touched this node"))
}

/// A Mermaid flowchart for the whole export, or one node's ancestry and
/// descendants. Genealogy points from a child to its parent, hence `BT`.
pub(crate) fn mermaid(graph: &GraphExport, from: Option<&str>) -> Result<String, Error> {
    let included = lineage(graph, from)?;
    let mut out = String::from("graph BT\n");

    for node in graph
        .nodes
        .iter()
        .filter(|node| included.contains(&node.id))
    {
        let _ = writeln!(
            out,
            "  {}[\"{}\"]:::{}",
            node.id,
            mermaid_label(&node.title),
            node.status
        );
    }

    let mut contradictions = HashSet::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| included.contains(&edge.from) && included.contains(&edge.to))
    {
        if edge.kind == EdgeType::Contradicts {
            let pair = if edge.from < edge.to {
                (edge.from.as_str(), edge.to.as_str())
            } else {
                (edge.to.as_str(), edge.from.as_str())
            };
            if contradictions.insert(pair) {
                let _ = writeln!(out, "  {} -.-|contradicts| {}", pair.0, pair.1);
            }
        } else {
            let _ = writeln!(out, "  {} -->|{}| {}", edge.from, edge.kind, edge.to);
        }
    }

    out.push_str("  classDef seed fill:#dbeafe,stroke:#2563eb,color:#172554\n");
    out.push_str("  classDef hypothesis fill:#fef3c7,stroke:#d97706,color:#451a03\n");
    out.push_str("  classDef refuted fill:#fee2e2,stroke:#dc2626,color:#450a0a\n");
    out.push_str("  classDef abandoned fill:#f3f4f6,stroke:#6b7280,color:#374151\n");
    Ok(out)
}

/// The union of a node's upward and downward genealogy walks. Keeping the two
/// walks separate matters: walking the union as an undirected graph would pull
/// in siblings through their shared parent.
fn lineage(graph: &GraphExport, from: Option<&str>) -> Result<HashSet<String>, Error> {
    let Some(from) = from else {
        return Ok(graph.nodes.iter().map(|node| node.id.clone()).collect());
    };
    if !graph.nodes.iter().any(|node| node.id == from) {
        return Err(Error::NoSuchNode(from.to_string()));
    }

    let mut included = HashSet::from([from.to_string()]);
    walk_lineage(graph, from, true, &mut included);
    walk_lineage(graph, from, false, &mut included);
    Ok(included)
}

fn walk_lineage(graph: &GraphExport, at: &str, up: bool, included: &mut HashSet<String>) {
    let next: Vec<&str> = graph
        .edges
        .iter()
        .filter(|edge| edge.kind.is_genealogy())
        .filter_map(|edge| {
            if up && edge.from == at {
                Some(edge.to.as_str())
            } else if !up && edge.to == at {
                Some(edge.from.as_str())
            } else {
                None
            }
        })
        .collect();
    for id in next {
        if included.insert(id.to_string()) {
            walk_lineage(graph, id, up, included);
        }
    }
}

fn mermaid_label(title: &str) -> String {
    title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
}
