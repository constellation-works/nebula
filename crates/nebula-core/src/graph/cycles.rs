//! Genealogy cycles: whether a new edge would close one, and the first one a
//! loaded graph holds.

use crate::model::Doc;
use std::collections::{HashMap, HashSet};

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
pub(super) fn find_cycle<'a>(graph: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<String>> {
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
