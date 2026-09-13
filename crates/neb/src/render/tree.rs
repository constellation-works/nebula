//! Tree drawing for `trace`.

use super::{bold, dim, line};
use nebula_core::{Direction, Doc};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Draw an ancestry or descent tree from `start`.
///
/// A node reachable by more than one path is a diamond, which is legal and
/// expected here. It is printed at every place it appears so the shape of the
/// graph is visible, but expanded only once so the output stays finite.
pub fn draw(docs: &[Doc], start: &str, direction: Direction) -> String {
    let by_id: HashMap<&str, &Doc> = docs.iter().map(|d| (d.node.id.as_str(), d)).collect();
    let mut children: HashMap<&str, Vec<String>> = HashMap::new();
    for d in docs {
        for p in d.node.parents() {
            children.entry(p).or_default().push(d.node.id.clone());
        }
    }
    let mut tree = Tree {
        by_id: &by_id,
        children: &children,
        direction,
        seen: HashSet::new(),
        out: String::new(),
    };
    tree.node(start, "", 0, true);
    tree.out
}

struct Tree<'a> {
    by_id: &'a HashMap<&'a str, &'a Doc>,
    children: &'a HashMap<&'a str, Vec<String>>,
    direction: Direction,
    seen: HashSet<String>,
    out: String,
}

impl Tree<'_> {
    fn next(&self, doc: &Doc) -> Vec<String> {
        match self.direction {
            Direction::Down => self
                .children
                .get(doc.node.id.as_str())
                .cloned()
                .unwrap_or_default(),
            Direction::Up => doc.node.parents().map(String::from).collect(),
        }
    }

    fn node(&mut self, id: &str, prefix: &str, depth: usize, last: bool) {
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
        let _ = writeln!(self.out, "{branch}{}{suffix}", line(&doc.node));
        if repeat {
            return;
        }
        let children = self.next(doc);
        let child_prefix = match depth {
            0 => String::new(),
            _ => format!("{prefix}{}", if last { "   " } else { "│  " }),
        };
        for (i, c) in children.iter().enumerate() {
            self.node(c, &child_prefix, depth + 1, i + 1 == children.len());
        }
    }
}
