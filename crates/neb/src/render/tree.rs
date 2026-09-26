//! Tree drawing for `trace`.

use super::{bold, dim, line};
use nebula_core::{Direction, Doc, EdgeType};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// One branch of the tree: the node it leads to, and the edge kinds joining
/// that node to the one above it.
type Branch = (String, Vec<EdgeType>);

/// Draw an ancestry or descent tree from `start`.
///
/// Every line below the start names the kind of edge that joins it to the
/// line above. The edges are always the descendant's, so walking down a line
/// reads "this derives from the one above", and walking up "the one above
/// derives from this".
///
/// A node reachable by more than one path is a diamond, which is legal and
/// expected here. It is printed at every place it appears so the shape of the
/// graph is visible, but expanded only once so the output stays finite. Two
/// edges between the same pair are not a diamond: they are one line naming
/// both kinds.
pub fn draw(docs: &[Doc], start: &str, direction: Direction) -> String {
    let by_id: HashMap<&str, &Doc> = docs.iter().map(|d| (d.node.id.as_str(), d)).collect();
    let mut children: HashMap<&str, Vec<Branch>> = HashMap::new();
    for d in docs {
        for (p, kinds) in d.node.lineage() {
            children
                .entry(p)
                .or_default()
                .push((d.node.id.clone(), kinds));
        }
    }
    let mut tree = Tree {
        by_id: &by_id,
        children: &children,
        direction,
        seen: HashSet::new(),
        out: String::new(),
    };
    tree.node(start, &[], "", 0, true);
    tree.out
}

struct Tree<'a> {
    by_id: &'a HashMap<&'a str, &'a Doc>,
    children: &'a HashMap<&'a str, Vec<Branch>>,
    direction: Direction,
    seen: HashSet<String>,
    out: String,
}

impl Tree<'_> {
    fn next(&self, doc: &Doc) -> Vec<Branch> {
        match self.direction {
            Direction::Down => self
                .children
                .get(doc.node.id.as_str())
                .cloned()
                .unwrap_or_default(),
            Direction::Up => doc
                .node
                .lineage()
                .into_iter()
                .map(|(p, kinds)| (p.to_string(), kinds))
                .collect(),
        }
    }

    fn node(&mut self, id: &str, kinds: &[EdgeType], prefix: &str, depth: usize, last: bool) {
        let branch = match (depth, last) {
            (0, _) => String::new(),
            (_, true) => format!("{prefix}└─ {}  ", joined(kinds)),
            (_, false) => format!("{prefix}├─ {}  ", joined(kinds)),
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
        for (i, (c, kinds)) in children.iter().enumerate() {
            self.node(c, kinds, &child_prefix, depth + 1, i + 1 == children.len());
        }
    }
}

/// Edge kinds as one label: `derives-from, reopens`.
fn joined(kinds: &[EdgeType]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
