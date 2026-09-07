//! The invariant checker. This is the lock, the same role `check-theory.py`
//! plays in principia: the schema is a suggestion until something enforces it.

use crate::model::{self, Doc, EdgeType, Status, Verdict};
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Default)]
struct Report {
    errors: Vec<String>,
    warnings: Vec<String>,
}

impl Report {
    fn err(&mut self, id: &str, msg: impl Into<String>) {
        self.errors.push(format!("{id}: {}", msg.into()));
    }
    fn warn(&mut self, id: &str, msg: impl Into<String>) {
        self.warnings.push(format!("{id}: {}", msg.into()));
    }
}

pub fn run(root: &Path) -> Result<()> {
    let dir = root.join("nodes");
    let mut docs: Vec<Doc> = vec![];
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let p = entry?.path();
            if p.extension().is_some_and(|e| e == "md") {
                // A reference carrying a verdict fails to deserialize, so
                // invariant 10 is enforced here by simply failing to load.
                docs.push(model::read(&p)?);
            }
        }
    }
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report::default();

    for doc in &docs {
        let n = &doc.node;
        let id = n.id.as_str();

        // 2. A hypothesis must name what would kill it, written before the
        //    evidence arrives. This is what keeps step four honest.
        if n.status.needs_kill() && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
            r.err(id, format!("status is {:?} but no kill condition is named", n.status));
        }

        // 3. A verdict has to rest on something.
        let has = |v: Verdict| n.evidence.iter().any(|e| e.verdict == v);
        match n.status {
            Status::Supported if !has(Verdict::Supports) => {
                r.err(id, "status supported with no supporting evidence")
            }
            Status::Refuted if !has(Verdict::Undermines) => {
                r.err(id, "status refuted with no undermining evidence")
            }
            Status::Testing if n.evidence.is_empty() => {
                r.err(id, "status testing with no evidence at all")
            }
            _ => {}
        }

        // 4. Dangling edges.
        for e in &n.edges {
            if !ids.contains(e.to.as_str()) {
                r.err(id, format!("edge {:?} points at missing node {}", e.kind, e.to));
            }
        }

        // 8. A graduated node has to say where it went, or the lineage stops
        //    dead at the boundary it was supposed to cross.
        if n.status == Status::Graduated && n.graduated_to.is_none() {
            r.err(id, "graduated but graduated_to is empty");
        }

        // 11, 12. Reference hygiene and id uniqueness.
        let mut seen_ref = HashSet::new();
        for f in &n.references {
            if !seen_ref.insert(&f.id) {
                r.err(id, format!("duplicate reference id {}", f.id));
            }
            if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
                r.warn(id, format!("reference {} has no note; a bare link rots", f.id));
            }
        }
        let mut seen_ev = HashSet::new();
        for e in &n.evidence {
            if !seen_ev.insert(&e.id) {
                r.err(id, format!("duplicate evidence id {}", e.id));
            }
        }

        // 14. Task ids must look like task ids, so a typo does not quietly
        //     become an unresolvable provenance record.
        let task_ids = n.tasks.iter().map(|t| t.id.as_str()).chain(
            n.origin.as_ref().and_then(|o| o.task.as_deref()),
        );
        for t in task_ids {
            if !t.starts_with("ORB-") || !t[4..].chars().all(|c| c.is_ascii_digit()) {
                r.err(id, format!("malformed Orbit task id: {t}"));
            }
        }
    }

    // 5. `contradicts` is a claim about both nodes, so a one-sided
    //    declaration is a half-recorded fact.
    for doc in &docs {
        for e in doc.node.edges.iter().filter(|e| e.kind == EdgeType::Contradicts) {
            let mutual = docs
                .iter()
                .find(|d| d.node.id == e.to)
                .is_some_and(|d| {
                    d.node.edges.iter().any(|b| {
                        b.kind == EdgeType::Contradicts && b.to == doc.node.id
                    })
                });
            if !mutual {
                r.err(&doc.node.id, format!("contradicts {} is not mutual", e.to));
            }
        }
    }

    // 1. Genealogy must be acyclic. An idea cannot be its own ancestor.
    //    Note this constrains genealogy only: diamonds are legal, and the
    //    evidence graph may cycle (see below).
    let parents: HashMap<&str, Vec<&str>> = docs
        .iter()
        .map(|d| (d.node.id.as_str(), d.node.parents().collect()))
        .collect();
    if let Some(cycle) = find_cycle(&parents) {
        r.errors.push(format!("genealogy cycle: {}", cycle.join(" -> ")));
    }

    // 9. A cycle in `supports` is circular reasoning. Worth surfacing, not
    //    worth forbidding, because sometimes it is how you notice.
    let supports: HashMap<&str, Vec<&str>> = docs
        .iter()
        .map(|d| {
            (
                d.node.id.as_str(),
                d.node
                    .edges
                    .iter()
                    .filter(|e| e.kind == EdgeType::Supports)
                    .map(|e| e.to.as_str())
                    .collect(),
            )
        })
        .collect();
    if let Some(cycle) = find_cycle(&supports) {
        r.warnings
            .push(format!("circular reasoning in supports: {}", cycle.join(" -> ")));
    }

    for w in &r.warnings {
        println!("warn  {w}");
    }
    for e in &r.errors {
        println!("ERROR {e}");
    }
    println!(
        "\n{} nodes, {} errors, {} warnings",
        docs.len(),
        r.errors.len(),
        r.warnings.len()
    );
    if !r.errors.is_empty() {
        bail!("check failed");
    }
    Ok(())
}

/// Depth-first search returning the first cycle found, as a readable path.
fn find_cycle(graph: &HashMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    let mut state: HashMap<&str, u8> = HashMap::new(); // 0 unseen, 1 on stack, 2 done
    let mut stack: Vec<&str> = vec![];
    for start in graph.keys() {
        if state.get(start).copied().unwrap_or(0) == 0 {
            if let Some(c) = dfs(start, graph, &mut state, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}

fn dfs<'a>(
    at: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    state: &mut HashMap<&'a str, u8>,
    stack: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    state.insert(at, 1);
    stack.push(at);
    for next in graph.get(at).map(|v| v.as_slice()).unwrap_or(&[]) {
        match state.get(next).copied().unwrap_or(0) {
            1 => {
                let from = stack.iter().position(|s| s == next).unwrap_or(0);
                let mut c: Vec<String> = stack[from..].iter().map(|s| s.to_string()).collect();
                c.push(next.to_string());
                return Some(c);
            }
            0 => {
                if let Some(c) = dfs(next, graph, state, stack) {
                    return Some(c);
                }
            }
            _ => {}
        }
    }
    stack.pop();
    state.insert(at, 2);
    None
}
