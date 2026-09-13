//! Node lifecycle: create, sharpen, link, move status, edit tags.

use super::{out_json, store_at};
use crate::corpus::{Closed, Doc, Origin, Store};
use crate::corpus::{Edge, EdgeType, Node, Status, model, store};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Create a node directly. Naming a kill condition starts it as a
/// hypothesis; without one it is a seed.
pub fn new_node(
    root: Option<PathBuf>,
    title: &str,
    parents: &[String],
    kill: Option<String>,
    tags: &[String],
    task: Option<String>,
    run: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    if kill.as_ref().is_some_and(|k| k.trim().is_empty()) {
        bail!("a kill condition cannot be empty; that is the whole point of it");
    }
    let status = if kill.is_some() {
        Status::Hypothesis
    } else {
        Status::Seed
    };
    let spec = NodeSpec {
        title,
        parents,
        kill,
        status,
        tags,
        origin: (task.is_some() || run.is_some()).then_some(Origin {
            task,
            run,
            ..Origin::default()
        }),
        body: "",
    };
    let doc = build(&store, &spec)?;
    store.create(&doc)?;
    println!(
        "{} {}",
        bold(&doc.node.id),
        dim(&store.node_path(&doc.node.id).display().to_string())
    );
    Ok(())
}

/// Everything that goes into a fresh node.
pub(super) struct NodeSpec<'a> {
    pub(super) title: &'a str,
    pub(super) parents: &'a [String],
    pub(super) kill: Option<String>,
    pub(super) status: Status,
    pub(super) tags: &'a [String],
    pub(super) origin: Option<Origin>,
    pub(super) body: &'a str,
}

pub(super) fn build(store: &Store, spec: &NodeSpec<'_>) -> Result<Doc> {
    let id = store::slugify(spec.title);
    if id.is_empty() {
        bail!("title `{}` does not reduce to a usable id", spec.title);
    }
    for p in spec.parents {
        if !store.node_path(p).exists() {
            bail!("parent `{p}` does not exist");
        }
    }
    let now = store::today();
    Ok(Doc {
        node: Node {
            id,
            title: spec.title.to_string(),
            status: spec.status,
            created: now.clone(),
            updated: now,
            kill: spec.kill.clone(),
            tags: model::normalize_tags(spec.tags),
            edges: spec
                .parents
                .iter()
                .map(|p| Edge {
                    kind: EdgeType::DerivesFrom,
                    to: p.clone(),
                })
                .collect(),
            references: vec![],
            closed: None,
            origin: spec.origin.clone(),
        },
        body: spec.body.to_string(),
    })
}

/// Seed becomes hypothesis by naming its falsifier.
pub fn sharpen(root: Option<PathBuf>, node_id: &str, kill: &str) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    if kill.trim().is_empty() {
        bail!("a kill condition cannot be empty; that is the whole point of it");
    }
    doc.node.kill = Some(kill.to_string());
    if doc.node.status == Status::Seed {
        doc.node.status = Status::Hypothesis;
    }
    store.save(&mut doc)?;
    println!("{} is now {}", bold(node_id), doc.node.status);
    Ok(())
}

/// Add a typed edge.
pub fn link(root: Option<PathBuf>, from: &str, kind: EdgeType, to: &str) -> Result<()> {
    let store = store_at(root)?;
    if from == to {
        bail!("a node cannot link to itself");
    }
    let mut doc = store.load(from)?;
    store.load(to)?;
    let edge = Edge {
        kind,
        to: to.to_string(),
    };
    if doc.node.edges.contains(&edge) {
        bail!("that edge already exists");
    }
    doc.node.edges.push(edge);

    // Genealogy must stay acyclic, so refuse the edge that would close a
    // loop rather than leaving `check` to find it later.
    if kind.is_genealogy() {
        let mut docs = store.load_all()?;
        docs.retain(|d| d.node.id != doc.node.id);
        docs.push(doc.clone());
        if creates_cycle(&docs, from) {
            bail!("that edge would make `{from}` its own ancestor");
        }
    }
    store.save(&mut doc)?;

    // `contradicts` is a claim about both nodes, so record it on both.
    if kind == EdgeType::Contradicts {
        let mut other = store.load(to)?;
        let back = Edge {
            kind,
            to: from.to_string(),
        };
        if !other.node.edges.contains(&back) {
            other.node.edges.push(back);
            store.save(&mut other)?;
        }
    }
    println!("{} {} {}", bold(from), dim(&kind.to_string()), bold(to));
    Ok(())
}

fn creates_cycle(docs: &[Doc], start: &str) -> bool {
    let by_id = store::by_id(docs);
    let mut stack = vec![start.to_string()];
    let mut seen = std::collections::HashSet::new();
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

/// Move a node to a new status, with the transition guards applied.
pub fn set_status(
    root: Option<PathBuf>,
    node_id: &str,
    status: Status,
    why: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let from = doc.node.status;

    let why = why.filter(|w| !w.trim().is_empty());
    if status.is_open() && why.is_some() {
        bail!("--why only applies to refuted or abandoned");
    }
    // Rule 5 at the point of action: refuting is asserting the kill
    // condition fired, and that assertion has to be written down.
    if status == Status::Refuted && why.is_none() {
        bail!(
            "refuted needs --why: say how the kill condition fired\n\n  \
             neb status {node_id} refuted --why \"...\""
        );
    }
    if status.needs_kill() && doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        bail!("`{status}` needs a kill condition first:\n\n  neb sharpen {node_id} --kill \"...\"");
    }
    // Rule 6: a ruled-out idea cannot quietly come back. Reviving one takes
    // a new node with a `reopens` edge, so the fact that it was once dead
    // stays visible.
    if from.is_closed_by_verdict() && status != from {
        bail!(
            "`{node_id}` is refuted and cannot simply reopen.\n\n\
             Create the new idea and link it:\n  \
             neb new \"...\" && neb link <new> reopens {node_id}"
        );
    }
    doc.node.status = status;
    doc.node.closed = match status {
        Status::Refuted => why.map(|why| Closed {
            why,
            at: store::today(),
        }),
        Status::Abandoned => why
            .map(|why| Closed {
                why,
                at: store::today(),
            })
            .or_else(|| doc.node.closed.take()),
        Status::Seed | Status::Hypothesis => None,
    };
    store.save(&mut doc)?;
    println!("{} {} -> {}", bold(node_id), dim(&from.to_string()), status);
    Ok(())
}

/// Edit a node's tags. Every tag is normalised to lowercase kebab-case on
/// the way in, so `--add Physics` and `--remove physics` name the same label.
pub fn tag(root: Option<PathBuf>, node_id: &str, add: &[String], remove: &[String]) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let add = model::normalize_tags(add);
    let remove = model::normalize_tags(remove);
    if add.is_empty() && remove.is_empty() {
        bail!("nothing to do; pass --add <tag> or --remove <tag>");
    }
    let mut tags = model::normalize_tags(&doc.node.tags);
    tags.retain(|t| !remove.contains(t));
    for t in add {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    doc.node.tags = tags;
    store.save(&mut doc)?;
    let shown = if doc.node.tags.is_empty() {
        dim("(no tags)")
    } else {
        doc.node.tags.join(", ")
    };
    println!("{} {shown}", bold(node_id));
    Ok(())
}

/// Every tag in the corpus with the number of nodes carrying it.
pub fn tag_list(root: Option<PathBuf>, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let docs = store.load_all()?;
    for d in &docs {
        for t in &d.node.tags {
            *counts.entry(t.as_str()).or_default() += 1;
        }
    }
    if json {
        let v: Vec<_> = counts
            .iter()
            .map(|(tag, count)| serde_json::json!({"tag": tag, "count": count}))
            .collect();
        return out_json(&v);
    }
    if counts.is_empty() {
        println!("{}", dim("no tags"));
    }
    for (tag, count) in &counts {
        println!("{} {}", bold(tag), dim(&count.to_string()));
    }
    Ok(())
}
