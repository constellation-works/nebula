//! Command implementations.

use crate::check::{self, Level};
use crate::model::{
    Doc, Edge, EdgeType, Evidence, Node, Origin, Reference, Status, Strength, TaskLink, Verdict,
};
use crate::render::{self, Tree, bold, dim};
use crate::store::{self, Store};
use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::ExitCode;

fn store_at(root: Option<PathBuf>) -> Result<Store> {
    Store::open(root)
}

fn out_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Create an empty corpus.
pub fn init(root: Option<PathBuf>, path: Option<PathBuf>) -> Result<()> {
    let target = path
        .or(root)
        .or_else(|| std::env::var("NEBULA_ROOT").ok().map(PathBuf::from))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".nebula"))
        })
        .ok_or_else(|| anyhow::anyhow!("nowhere to create a corpus; pass a path"))?;
    let store = Store::init(&target)?;
    println!("corpus ready at {}", store.root().display());
    println!(
        "\nExport it so every command finds it:\n  export NEBULA_ROOT={}",
        target.display()
    );
    Ok(())
}

/// The five-second path.
pub fn capture(root: Option<PathBuf>, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("nothing to capture");
    }
    // Capture must work on a corpus that does not exist yet. Being told to run
    // a setup command is precisely the friction that loses the thought.
    let store = if let Ok(s) = Store::open(root.clone()) {
        s
    } else {
        let target = root
            .or_else(|| std::env::var("NEBULA_ROOT").ok().map(PathBuf::from))
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".nebula"))
            })
            .ok_or_else(|| anyhow::anyhow!("nowhere to store a corpus; set NEBULA_ROOT"))?;
        Store::init(&target)?
    };
    let id = store.capture(text)?;
    println!("{}", bold(&id));
    Ok(())
}

/// Captures not yet promoted or dropped.
pub fn inbox(root: Option<PathBuf>, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let entries = store.inbox()?;
    if json {
        let v: Vec<_> = entries
            .iter()
            .map(|e| serde_json::json!({"id": e.id, "at": e.at, "text": e.text}))
            .collect();
        return out_json(&v);
    }
    if entries.is_empty() {
        println!("{}", dim("inbox is empty"));
        return Ok(());
    }
    for e in &entries {
        println!("{} {} {}", bold(&e.id), dim(&e.at), e.text);
    }
    println!(
        "\n{}",
        dim(&format!(
            "{} waiting. Promote or drop each one.",
            entries.len()
        ))
    );
    Ok(())
}

/// Discard a capture, struck through rather than deleted.
pub fn drop_entry(root: Option<PathBuf>, entry: &str) -> Result<()> {
    let store = store_at(root)?;
    let e = store.inbox_entry(entry)?;
    store.settle_inbox(&e, "dropped")?;
    println!("dropped {}", bold(entry));
    Ok(())
}

/// Inbox entry becomes a seed node.
pub fn promote(
    root: Option<PathBuf>,
    entry: &str,
    title: Option<String>,
    parents: &[String],
    tags: &[String],
) -> Result<()> {
    let store = store_at(root)?;
    let e = store.inbox_entry(entry)?;
    let title = title.unwrap_or_else(|| e.text.clone());
    let doc = build(&store, &title, parents, None, Status::Seed, tags, &e.text)?;
    store.create(&doc)?;
    store.settle_inbox(&e, &format!("-> {}", doc.node.id))?;
    println!(
        "{} {}",
        bold(&doc.node.id),
        dim(&store.node_path(&doc.node.id).display().to_string())
    );
    Ok(())
}

/// Create a node directly.
pub fn new_node(
    root: Option<PathBuf>,
    title: &str,
    parents: &[String],
    kill: Option<String>,
    status: Status,
    tags: &[String],
) -> Result<()> {
    let store = store_at(root)?;
    if status.needs_kill() && kill.is_none() {
        bail!("status `{status}` needs --kill: name what would falsify this before you look");
    }
    let doc = build(&store, title, parents, kill, status, tags, "")?;
    store.create(&doc)?;
    println!(
        "{} {}",
        bold(&doc.node.id),
        dim(&store.node_path(&doc.node.id).display().to_string())
    );
    Ok(())
}

fn build(
    store: &Store,
    title: &str,
    parents: &[String],
    kill: Option<String>,
    status: Status,
    tags: &[String],
    body: &str,
) -> Result<Doc> {
    let id = store::slugify(title);
    if id.is_empty() {
        bail!("title `{title}` does not reduce to a usable id");
    }
    for p in parents {
        if !store.node_path(p).exists() {
            bail!("parent `{p}` does not exist");
        }
    }
    let now = store::today();
    Ok(Doc {
        node: Node {
            id,
            title: title.to_string(),
            status,
            created: now.clone(),
            updated: now,
            kill,
            tags: tags.to_vec(),
            edges: parents
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
        body: body.to_string(),
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

/// Attach something that bears on truth.
pub fn evidence(
    root: Option<PathBuf>,
    node_id: &str,
    verdict: Verdict,
    strength: Strength,
    source: &str,
    note: Option<String>,
    task: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let id = doc.node.next_evidence_id();
    doc.node.evidence.push(Evidence {
        id: id.clone(),
        verdict,
        strength,
        source: source.to_string(),
        date: store::today(),
        note,
        origin: task.map(|t| Origin {
            task: Some(t),
            ..Origin::default()
        }),
    });
    // Evidence arriving is what moves a hypothesis into testing. The stronger
    // transitions stay manual, because deciding a claim is supported is a
    // judgement and should not be a side effect of filing a note.
    if doc.node.status == Status::Hypothesis {
        doc.node.status = Status::Testing;
    }
    store.save(&mut doc)?;
    println!(
        "{} {} {}",
        bold(node_id),
        bold(&id),
        dim(&format!("{verdict:?}").to_lowercase())
    );
    if let Some(kill) = &doc.node.kill {
        if verdict == Verdict::Undermines {
            println!(
                "\n{}\n  {kill}",
                dim("Check this against the kill condition:")
            );
        }
    }
    Ok(())
}

/// Attach context that does not bear on truth.
pub fn cite(
    root: Option<PathBuf>,
    node_id: &str,
    uri: &str,
    kind: &str,
    title: Option<String>,
    note: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let id = doc.node.next_reference_id();
    let bare = note.is_none();
    doc.node.references.push(Reference {
        id: id.clone(),
        kind: kind.to_string(),
        uri: uri.to_string(),
        title,
        note,
        added: store::today(),
        promoted_to: None,
        origin: None,
    });
    store.save(&mut doc)?;
    println!("{} {}", bold(node_id), bold(&id));
    if bare {
        println!(
            "\n{}",
            dim("No note. Add one saying why it is here, or this is a link that rots.")
        );
    }
    Ok(())
}

/// A reference becomes evidence, once you know which way it cuts.
pub fn weigh(
    root: Option<PathBuf>,
    node_id: &str,
    reference: &str,
    verdict: Verdict,
    strength: Strength,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    let Some(idx) = doc.node.references.iter().position(|r| r.id == reference) else {
        bail!("no reference `{reference}` on `{node_id}`");
    };
    if let Some(to) = &doc.node.references[idx].promoted_to {
        bail!("reference `{reference}` was already weighed as `{to}`");
    }
    let ev_id = doc.node.next_evidence_id();
    let r = &doc.node.references[idx];
    let ev = Evidence {
        id: ev_id.clone(),
        verdict,
        strength,
        source: r.uri.clone(),
        date: store::today(),
        note: r.note.clone(),
        origin: r.origin.clone(),
    };
    // The reference stays where it is, marked. Nothing is moved or deleted, so
    // the reading history survives alongside the finding it produced.
    doc.node.references[idx].promoted_to = Some(ev_id.clone());
    doc.node.evidence.push(ev);
    if doc.node.status == Status::Hypothesis {
        doc.node.status = Status::Testing;
    }
    store.save(&mut doc)?;
    println!("{} {} -> {}", bold(node_id), dim(reference), bold(&ev_id));
    Ok(())
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

/// Walk ancestry, or descent.
pub fn trace(root: Option<PathBuf>, node_id: &str, down: bool, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let by_id = store::by_id(&docs);
    if !by_id.contains_key(node_id) {
        bail!("no node `{node_id}`");
    }
    if json {
        let mut acc = Vec::new();
        collect(
            node_id,
            &by_id,
            down,
            &mut std::collections::HashSet::new(),
            &mut acc,
        );
        return out_json(&acc);
    }
    let tree = Tree::new(&by_id);
    let text = if down {
        let children: std::collections::HashMap<&str, Vec<String>> = by_id
            .keys()
            .map(|k| {
                (
                    *k,
                    docs.iter()
                        .filter(|d| d.node.parents().any(|p| p == *k))
                        .map(|d| d.node.id.clone())
                        .collect(),
                )
            })
            .collect();
        tree.draw(node_id, &|d: &Doc| {
            children
                .get(d.node.id.as_str())
                .cloned()
                .unwrap_or_default()
        })
    } else {
        tree.draw(node_id, &|d: &Doc| {
            d.node.parents().map(String::from).collect()
        })
    };
    print!("{text}");
    Ok(())
}

fn collect(
    id: &str,
    by_id: &std::collections::HashMap<&str, &Doc>,
    down: bool,
    seen: &mut std::collections::HashSet<String>,
    acc: &mut Vec<serde_json::Value>,
) {
    if !seen.insert(id.to_string()) {
        return;
    }
    let Some(doc) = by_id.get(id) else { return };
    acc.push(serde_json::json!({
        "id": doc.node.id,
        "title": doc.node.title,
        "status": doc.node.status.to_string(),
        "parents": doc.node.parents().collect::<Vec<_>>(),
    }));
    let next: Vec<String> = if down {
        by_id
            .values()
            .filter(|d| d.node.parents().any(|p| p == id))
            .map(|d| d.node.id.clone())
            .collect()
    } else {
        doc.node.parents().map(String::from).collect()
    };
    for n in next {
        collect(&n, by_id, down, seen, acc);
    }
}

/// What collapses if this node dies.
pub fn impact(root: Option<PathBuf>, node_id: &str, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    store.load(node_id)?;
    let mut hit: Vec<(&str, EdgeType)> = Vec::new();
    for d in &docs {
        for e in &d.node.edges {
            if e.to == node_id && matches!(e.kind, EdgeType::DependsOn | EdgeType::Supports) {
                hit.push((d.node.id.as_str(), e.kind));
            }
        }
    }
    if json {
        let v: Vec<_> = hit
            .iter()
            .map(|(id, k)| serde_json::json!({"id": id, "via": k.to_string()}))
            .collect();
        return out_json(&v);
    }
    if hit.is_empty() {
        println!("{}", dim("nothing leans on this node"));
        return Ok(());
    }
    println!("{}", dim(&format!("if `{node_id}` dies:")));
    for (id, kind) in hit {
        let severity = if kind == EdgeType::DependsOn {
            "dies with it"
        } else {
            "loses support"
        };
        println!("  {} {} {}", bold(id), dim(&kind.to_string()), severity);
    }
    Ok(())
}

/// Nodes that need attention.
pub fn open(root: Option<PathBuf>, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let mut items: Vec<(String, String)> = Vec::new();
    for d in &docs {
        let n = &d.node;
        // The genuinely actionable gap: a hypothesis that names what would kill
        // it, has found nothing either way, and has nothing running to find out.
        if matches!(n.status, Status::Hypothesis)
            && n.evidence.is_empty()
            && n.open_tasks().next().is_none()
        {
            items.push((
                n.id.clone(),
                "hypothesis with no evidence and no task running".into(),
            ));
        }
        if n.status == Status::Seed && store::days_since(&n.updated).is_some_and(|d| d > 90) {
            items.push((
                n.id.clone(),
                "seed untouched for over ninety days; abandon it?".into(),
            ));
        }
        if n.status == Status::Testing && store::days_since(&n.updated).is_some_and(|d| d > 30) {
            items.push((
                n.id.clone(),
                "testing, but nothing has moved in a month".into(),
            ));
        }
        for r in &n.references {
            if r.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
                items.push((n.id.clone(), format!("reference `{}` has no note", r.id)));
            }
        }
    }
    if json {
        let v: Vec<_> = items
            .iter()
            .map(|(id, w)| serde_json::json!({"id": id, "why": w}))
            .collect();
        return out_json(&v);
    }
    if items.is_empty() {
        println!("{}", dim("nothing needs attention"));
        return Ok(());
    }
    for (id, why) in &items {
        println!("{} {}", bold(id), why);
    }
    Ok(())
}

/// Show one node in full.
pub fn show(root: Option<PathBuf>, node_id: &str, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let doc = store.load(node_id)?;
    if json {
        return out_json(&doc.node);
    }
    let n = &doc.node;
    println!("{} {}", render::status_badge(n.status), bold(&n.id));
    println!("{}\n", n.title);
    if let Some(k) = &n.kill {
        println!("{} {k}\n", dim("kill:"));
    }
    if !doc.body.trim().is_empty() {
        println!("{}\n", doc.body.trim());
    }
    if !n.edges.is_empty() {
        println!("{}", dim("edges"));
        for e in &n.edges {
            println!("  {:<14} {}", e.kind.to_string(), e.to);
        }
        println!();
    }
    if !n.evidence.is_empty() {
        println!("{}", dim("evidence"));
        for e in &n.evidence {
            let v = format!("{:?}", e.verdict).to_lowercase();
            let s = format!("{:?}", e.strength).to_lowercase();
            println!("  {} {:<12} {:<10} {}", bold(&e.id), v, s, e.source);
            if let Some(note) = &e.note {
                println!("     {}", dim(note.trim()));
            }
        }
        println!();
    }
    if !n.references.is_empty() {
        println!("{}", dim("references"));
        for r in &n.references {
            println!("  {} {:<10} {}", bold(&r.id), r.kind, r.uri);
            let text = r.note.as_deref().map_or("(no note)", str::trim);
            println!("     {}", dim(text));
        }
        println!();
    }
    if !n.tasks.is_empty() {
        println!("{}", dim("tasks"));
        for t in &n.tasks {
            println!(
                "  {} {:<8} {}",
                bold(&t.id),
                t.state,
                t.why.as_deref().unwrap_or("")
            );
        }
        println!();
    }
    if let Some(g) = &n.graduated_to {
        println!("{} {g}", dim("graduated to:"));
    }
    Ok(())
}

/// List nodes.
pub fn list(
    root: Option<PathBuf>,
    status: Option<Status>,
    tag: Option<&str>,
    json: bool,
) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let picked: Vec<&Doc> = docs
        .iter()
        .filter(|d| status.is_none_or(|s| d.node.status == s))
        .filter(|d| tag.is_none_or(|t| d.node.tags.iter().any(|x| x == t)))
        .collect();
    if json {
        let v: Vec<_> = picked.iter().map(|d| &d.node).collect();
        return out_json(&v);
    }
    if picked.is_empty() {
        println!("{}", dim("no nodes match"));
        return Ok(());
    }
    for d in &picked {
        println!("{}", render::line(d));
    }
    println!(
        "\n{}",
        dim(&format!("{} of {} nodes", picked.len(), docs.len()))
    );
    Ok(())
}

/// Run the invariants.
pub fn check(root: Option<PathBuf>, json: bool) -> Result<ExitCode> {
    let store = store_at(root)?;
    let report = check::run(&store)?;
    if json {
        out_json(&report)?;
    } else {
        for f in &report.findings {
            let (tag, code) = match f.level {
                Level::Error => ("ERROR", "31;1"),
                Level::Warn => ("warn ", "33"),
            };
            let where_ = f.node.as_deref().unwrap_or("corpus");
            println!(
                "{} {} {} {}",
                render::paint(code, tag),
                dim(&format!("[{}]", f.rule)),
                bold(where_),
                f.message
            );
        }
        println!(
            "\n{} nodes, {} errors, {} warnings",
            report.nodes,
            report.errors(),
            report.warnings()
        );
    }
    Ok(if report.errors() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
