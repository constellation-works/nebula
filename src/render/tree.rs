//! Tree drawing for `trace`.

use super::{bold, dim, line};
use crate::corpus::Doc;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

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
