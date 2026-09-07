//! Terminal output.
//!
//! Colour is applied only when stdout is a terminal and `NO_COLOR` is unset, so
//! piping into a file or an agent yields clean text.

use crate::model::{Doc, Status};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::IsTerminal;
use std::sync::OnceLock;

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

/// A status badge, coloured by whether the node still asks anything of you.
pub fn status_badge(s: Status) -> String {
    let code = match s {
        Status::Seed => "36",                         // cyan, unformed
        Status::Hypothesis => "33",                   // yellow, live and owing evidence
        Status::Testing => "35",                      // magenta, in flight
        Status::Supported => "32",                    // green, standing
        Status::Refuted => "31",                      // red, dead
        Status::Abandoned | Status::Graduated => "2", // dim, settled elsewhere
    };
    paint(code, &format!("{s:<10}"))
}

/// One node as a single line.
pub fn line(doc: &Doc) -> String {
    let n = &doc.node;
    format!(
        "{} {} {}",
        status_badge(n.status),
        bold(&n.id),
        dim(&n.title)
    )
}

/// Draw an ancestry or descent tree.
///
/// A node reachable by more than one path is a diamond, which is legal and
/// expected here. It is printed at every place it appears so the shape of the
/// graph is visible, but expanded only once so the output stays finite.
pub struct Tree<'a> {
    by_id: &'a HashMap<&'a str, &'a Doc>,
    seen: HashSet<String>,
    out: String,
}

impl<'a> Tree<'a> {
    /// Start a tree over an indexed corpus.
    pub fn new(by_id: &'a HashMap<&'a str, &'a Doc>) -> Self {
        Self {
            by_id,
            seen: HashSet::new(),
            out: String::new(),
        }
    }

    /// Render from `id`, following `next` to get each node's children.
    pub fn draw<F>(mut self, id: &str, next: &F) -> String
    where
        F: Fn(&Doc) -> Vec<String>,
    {
        self.node(id, "", 0, true, next);
        self.out
    }

    fn node<F>(&mut self, id: &str, prefix: &str, depth: usize, last: bool, next: &F)
    where
        F: Fn(&Doc) -> Vec<String>,
    {
        let branch = match (depth, last) {
            (0, _) => String::new(),
            (_, true) => format!("{prefix}└─ "),
            (_, false) => format!("{prefix}├─ "),
        };
        let Some(doc) = self.by_id.get(id) else {
            let _ = writeln!(self.out, "{branch}{} {}", bold(id), dim("[missing]"));
            return;
        };
        let repeat = !self.seen.insert(id.to_string());
        let suffix = if repeat {
            dim("  (shown above)")
        } else {
            String::new()
        };
        let _ = writeln!(self.out, "{branch}{}{suffix}", line(doc));
        if repeat {
            return;
        }
        let children = next(doc);
        let child_prefix = match depth {
            0 => String::new(),
            _ => format!("{prefix}{}", if last { "   " } else { "│  " }),
        };
        for (i, c) in children.iter().enumerate() {
            self.node(c, &child_prefix, depth + 1, i + 1 == children.len(), next);
        }
    }
}
