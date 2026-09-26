//! Tree drawing for `trace`.

use super::{bold, dim, line};
use nebula_core::{Direction, Doc, EdgeType};
use std::collections::HashMap;
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
///
/// `depth` stops the tree that many steps from `start`, as
/// `graph::trace_within` does, and a line whose branches it cut says how
/// many. A node whose drawing was cut short and is met again nearer the
/// start is drawn out again there, so the tree holds the same nodes as the
/// walk; one drawn out in full is not, so a bound the corpus never reaches
/// draws exactly what no bound does.
pub fn draw(docs: &[Doc], start: &str, direction: Direction, depth: Option<usize>) -> String {
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
        max: depth,
        seen: HashMap::new(),
        out: String::new(),
    };
    tree.node(start, &[], "", 0, true);
    tree.out
}

struct Tree<'a> {
    by_id: &'a HashMap<&'a str, &'a Doc>,
    children: &'a HashMap<&'a str, Vec<Branch>>,
    direction: Direction,
    max: Option<usize>,
    /// Each node drawn out: the fewest steps from the start it was drawn
    /// out at, and whether everything below it made it into the drawing.
    seen: HashMap<String, (usize, bool)>,
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

    /// Draw one line and everything under it, and say whether that was
    /// everything: `false` when `--depth` cut something below.
    fn node(
        &mut self,
        id: &str,
        kinds: &[EdgeType],
        prefix: &str,
        depth: usize,
        last: bool,
    ) -> bool {
        let branch = match (depth, last) {
            (0, _) => String::new(),
            (_, true) => format!("{prefix}└─ {}  ", joined(kinds)),
            (_, false) => format!("{prefix}├─ {}  ", joined(kinds)),
        };
        let Some(doc) = self.by_id.get(id) else {
            let _ = writeln!(self.out, "{branch}{} {}", bold(id), dim("[missing]"));
            return true;
        };
        // Drawn before, and either in full or from no further away: this
        // place could add nothing, so it only points back.
        if let Some(&(then, complete)) = self.seen.get(id)
            && (complete || then <= depth)
        {
            let _ = writeln!(
                self.out,
                "{branch}{}{}",
                line(&doc.node),
                dim("  (shown above)")
            );
            return complete;
        }
        // Complete until shown otherwise, which is also what a cycle back
        // to a node still being drawn finds.
        self.seen.insert(id.to_string(), (depth, true));
        let children = self.next(doc);
        if self.max.is_some_and(|max| depth >= max) && !children.is_empty() {
            let cut = format!("  ({} more beyond --depth)", children.len());
            let _ = writeln!(self.out, "{branch}{}{}", line(&doc.node), dim(&cut));
            self.seen.insert(id.to_string(), (depth, false));
            return false;
        }
        let _ = writeln!(self.out, "{branch}{}", line(&doc.node));
        let child_prefix = match depth {
            0 => String::new(),
            _ => format!("{prefix}{}", if last { "   " } else { "│  " }),
        };
        let mut complete = true;
        for (i, (c, kinds)) in children.iter().enumerate() {
            complete &= self.node(c, kinds, &child_prefix, depth + 1, i + 1 == children.len());
        }
        self.seen.insert(id.to_string(), (depth, complete));
        complete
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
