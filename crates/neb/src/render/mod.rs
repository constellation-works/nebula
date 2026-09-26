//! Terminal output: everything that turns a core type into text.
//!
//! Colour is applied only when stdout is a terminal and `NO_COLOR` is unset, so
//! piping into a file or an agent yields clean text.

mod error;
mod report;
mod tree;
mod triage;

use nebula_core::{EdgeType, GraphExport, HistoryEntry, Node, Status};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::sync::OnceLock;

pub use error::{Refusal, refusal, refusal_about};
pub use report::{
    check, commit_setting, impact, inbox, migration, near, node, observatory_root, open, review,
    suggestions, tags,
};
pub use tree::draw as tree;
pub use triage::{step, tally, triage_keys, waiting};

fn colour() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

/// Wrap text in an ANSI code, or return it untouched when colour is off.
pub fn paint(code: &str, s: &str) -> String {
    if colour() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Dim text, for anything secondary.
pub fn dim(s: &str) -> String {
    paint("2", s)
}

/// Bold text, for identifiers.
pub fn bold(s: &str) -> String {
    paint("1", s)
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
pub fn status_badge(s: Status) -> String {
    let code = match s {
        Status::Seed => "36",       // cyan, unformed
        Status::Hypothesis => "33", // yellow, live and owing a look
        Status::Refuted => "31",    // red, dead
        Status::Abandoned => "2",   // dim, settled
    };
    paint(code, &format!("{s:<10}"))
}

/// One node as a single line.
pub fn line(n: &Node) -> String {
    let tags = if n.tags.is_empty() {
        String::new()
    } else {
        format!(" {}", dim(&format!("[{}]", n.tags.join(", "))))
    };
    format!(
        "{} {}{tags} {}",
        status_badge(n.status),
        bold(&n.id),
        dim(&n.title)
    )
}

/// A listing of nodes, with the count of how many of the corpus it is.
/// `matched` is how many the filter kept, which is more than `nodes` holds
/// when `--limit` cut it.
pub fn list(nodes: &[Node], matched: usize, total: usize) -> String {
    if matched == 0 {
        return format!("{}\n", dim("no nodes match"));
    }
    let mut out = String::new();
    for n in nodes {
        let _ = writeln!(out, "{}", line(n));
    }
    let tally = if nodes.len() < matched && matched < total {
        format!(
            "{} of {} shown, of {total} in all; raise --limit for more",
            nodes.len(),
            count(matched, "matching node")
        )
    } else if nodes.len() < matched {
        format!(
            "{} of {} shown; raise --limit for more",
            nodes.len(),
            count(total, "node")
        )
    } else {
        format!("{matched} of {}", count(total, "node"))
    };
    let _ = writeln!(out, "\n{}", dim(&tally));
    out
}

/// Commits that changed a node, newest first.
pub fn history(entries: &[HistoryEntry]) -> String {
    if entries.is_empty() {
        return format!("{}\n", dim("no commits touched this node"));
    }
    let mut out = String::new();
    for entry in entries {
        let short = entry.hash.get(..7).unwrap_or(&entry.hash);
        let _ = writeln!(
            out,
            "{} {} {}",
            bold(short),
            dim(&entry.date),
            entry.message
        );
    }
    out
}

/// A Mermaid flowchart for the whole export, or one node's ancestry and
/// descendants. Genealogy points from a child to its parent, hence `BT`.
pub fn mermaid(graph: &GraphExport, from: Option<&str>) -> Result<String, String> {
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
fn lineage(graph: &GraphExport, from: Option<&str>) -> Result<HashSet<String>, String> {
    let Some(from) = from else {
        return Ok(graph.nodes.iter().map(|node| node.id.clone()).collect());
    };
    if !graph.nodes.iter().any(|node| node.id == from) {
        return Err(format!("no node `{from}`"));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The text with every ANSI escape removed, so a test reads what a
    /// terminal shows whether or not colour happens to be on.
    fn visible(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(|c| *c == 'm');
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The badge is a column: every status comes out the same width, so the
    /// id after it starts at the same place on every line.
    #[test]
    fn status_badges_are_padded_to_one_width() {
        for s in [
            Status::Seed,
            Status::Hypothesis,
            Status::Refuted,
            Status::Abandoned,
        ] {
            let badge = visible(&status_badge(s));
            assert_eq!(badge.chars().count(), 10, "{badge:?}");
            assert!(badge.starts_with(&s.to_string()), "{badge:?}");
        }
        assert_eq!(visible(&status_badge(Status::Seed)), "seed      ");
    }

    #[test]
    fn counts_agree_with_their_nouns() {
        assert_eq!(count(0_usize, "warning"), "0 warnings");
        assert_eq!(count(1_usize, "warning"), "1 warning");
        assert_eq!(count(2_usize, "warning"), "2 warnings");
        assert_eq!(count(1_i64, "day"), "1 day");
    }
}
