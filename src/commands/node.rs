//! Node lifecycle: create, sharpen, link, move status, record work, graduate.

use super::store_at;
use crate::corpus::{Doc, Store};
use crate::corpus::{Edge, EdgeType, Node, Status, TaskLink, Verdict, store};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::PathBuf;

/// Create a node directly.
pub fn new_node(
    root: Option<PathBuf>,
    title: &str,
    domain: Option<&str>,
    parents: &[String],
    kill: Option<String>,
    status: Status,
    tags: &[String],
) -> Result<()> {
    let store = store_at(root)?;
    if status.needs_kill() && kill.is_none() {
        bail!("status `{status}` needs --kill: name what would falsify this before you look");
    }
    let spec = NodeSpec {
        title,
        domain,
        parents,
        kill,
        status,
        tags,
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
    /// `None` falls back to the corpus default, which is where the
    /// one-domain and default-domain cases stay decision-free.
    pub(super) domain: Option<&'a str>,
    pub(super) parents: &'a [String],
    pub(super) kill: Option<String>,
    pub(super) status: Status,
    pub(super) tags: &'a [String],
    pub(super) body: &'a str,
}

pub(super) fn build(store: &Store, spec: &NodeSpec<'_>) -> Result<Doc> {
    let id = store::slugify(spec.title);
    if id.is_empty() {
        bail!("title `{}` does not reduce to a usable id", spec.title);
    }
    let domain = store.config().resolve_new(spec.domain)?;
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
            domain,
            status: spec.status,
            created: now.clone(),
            updated: now,
            kill: spec.kill.clone(),
            tags: spec.tags.to_vec(),
            edges: spec
                .parents
                .iter()
                .map(|p| Edge {
                    kind: EdgeType::DerivesFrom,
                    to: p.clone(),
                })
                .collect(),
            evidence: vec![],
            references: vec![],
            tasks: vec![],
            origin: None,
            graduated_to: None,
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

    // Genealogy is the one graph that must stay acyclic, so refuse the edge
    // that would close a loop rather than leaving `check` to find it later.
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
pub fn set_status(root: Option<PathBuf>, node_id: &str, status: Status) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let from = doc.node.status;

    if status == Status::Graduated {
        bail!("use `neb graduate {node_id} --to <uri>` so the lineage does not stop here");
    }
    if status.needs_kill() && doc.node.kill.is_none() {
        bail!("`{status}` needs a kill condition first:\n\n  neb sharpen {node_id} --kill \"...\"");
    }
    if status == Status::Supported && !doc.node.has_verdict(Verdict::Supports) {
        bail!("nothing supports `{node_id}` yet; attach evidence first");
    }
    if status == Status::Refuted && !doc.node.has_verdict(Verdict::Undermines) {
        bail!("nothing undermines `{node_id}` yet; attach the evidence that killed it");
    }
    // A ruled-out idea cannot quietly come back. Reviving one takes a new node
    // with a `reopens` edge, so the fact that it was once dead stays visible.
    if from.is_closed_by_verdict() && status.is_open() {
        bail!(
            "`{node_id}` is refuted and cannot simply reopen.\n\n\
             Create the new idea and link it:\n  \
             neb new \"...\" && neb link <new> reopens {node_id}"
        );
    }
    doc.node.status = status;
    store.save(&mut doc)?;
    println!("{} {} -> {}", bold(node_id), dim(&from.to_string()), status);
    Ok(())
}

/// Record work spawned to settle a node.
pub fn task(
    root: Option<PathBuf>,
    node_id: &str,
    id: &str,
    why: Option<String>,
    state: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    if let Some(existing) = doc.node.tasks.iter_mut().find(|t| t.id == id) {
        if let Some(s) = state {
            existing.state = s;
        }
        if why.is_some() {
            existing.why = why;
        }
    } else {
        doc.node.tasks.push(TaskLink {
            id: id.to_string(),
            state: state.unwrap_or_else(|| "open".into()),
            why,
        });
    }
    store.save(&mut doc)?;
    println!("{} {}", bold(node_id), bold(id));
    Ok(())
}

/// Hand a node downstream.
pub fn graduate(root: Option<PathBuf>, node_id: &str, to: &str) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    if doc.node.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        bail!(
            "`{node_id}` has no kill condition, and downstream will demand one.\n\n  \
             neb sharpen {node_id} --kill \"...\""
        );
    }
    doc.node.graduated_to = Some(to.to_string());
    doc.node.status = Status::Graduated;
    store.save(&mut doc)?;
    // The node stays here forever with a link out, so a trace that starts
    // downstream can still walk back to the observation that began it.
    println!("{} -> {}", bold(node_id), to);
    Ok(())
}
