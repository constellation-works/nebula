//! Cycle detection, shared by the genealogy and evidence graphs.

use std::collections::HashMap;

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
pub(super) fn find_cycle(graph: &HashMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    let mut state: HashMap<&str, Mark> = HashMap::new();
    let mut stack: Vec<&str> = Vec::new();
    // Sorted so the reported cycle does not change between runs on the same
    // corpus, which matters when the output goes into a review file.
    let mut roots: Vec<&&str> = graph.keys().collect();
    roots.sort_unstable();
    for start in roots {
        if !state.contains_key(*start) {
            if let Some(c) = dfs(start, graph, &mut state, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}
