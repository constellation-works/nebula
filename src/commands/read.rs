//! Reading the graph: trace, impact, open, show, list.

use super::{in_scope, scope_footer};
use super::{out_json, store_at};
use crate::corpus::Doc;
use crate::corpus::{EdgeType, Status, store};
use crate::render::{self, Tree};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

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
pub fn open(root: Option<PathBuf>, domain: Option<&str>, all: bool, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let scope = store.config().resolve_view(domain, all)?;
    let docs = store.load_all()?;
    let mut items: Vec<(String, String)> = Vec::new();
    let inbox = store.inbox()?;
    let stale_inbox_count = inbox
        .iter()
        .filter_map(|entry| store::days_since_stamp(&entry.at))
        .filter(|days| *days > 14)
        .count();
    if stale_inbox_count > 0 {
        items.push((
            "inbox".into(),
            format!(
                "{stale_inbox_count} captures waiting over fourteen days; promote or drop them"
            ),
        ));
    }
    for d in docs.iter().filter(|d| in_scope(d, scope.as_deref())) {
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
    }
    for (id, why) in &items {
        println!("{} {}", bold(id), why);
    }
    scope_footer(scope.as_deref());
    Ok(())
}

/// Show one node in full.
pub fn show(root: Option<PathBuf>, node_id: &str, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let doc = store.load(node_id)?;
    if json {
        return out_json(&serde_json::json!({
            "node": &doc.node,
            "body": doc.body.trim(),
        }));
    }
    let n = &doc.node;
    println!(
        "{} {} {}",
        render::status_badge(n.status),
        bold(&n.id),
        dim(&n.domain)
    );
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
    domain: Option<&str>,
    all: bool,
    json: bool,
) -> Result<()> {
    let store = store_at(root)?;
    let scope = store.config().resolve_view(domain, all)?;
    let docs = store.load_all()?;
    let picked: Vec<&Doc> = docs
        .iter()
        .filter(|d| in_scope(d, scope.as_deref()))
        .filter(|d| status.is_none_or(|s| d.node.status == s))
        .filter(|d| tag.is_none_or(|t| d.node.tags.iter().any(|x| x == t)))
        .collect();
    if json {
        let v: Vec<_> = picked.iter().map(|d| &d.node).collect();
        return out_json(&v);
    }
    if picked.is_empty() {
        println!("{}", dim("no nodes match"));
    }
    for d in &picked {
        println!("{}", render::line(d));
    }
    if !picked.is_empty() {
        println!(
            "\n{}",
            dim(&format!("{} of {} nodes", picked.len(), docs.len()))
        );
    }
    scope_footer(scope.as_deref());
    Ok(())
}

/// One finding from `review`.
#[derive(serde::Serialize)]
struct ReviewItem {
    rule: &'static str,
    id: String,
    title: String,
    reason: String,
}

/// The weekly maintenance report: hypotheses starved of evidence, seeds gone
/// cold, nodes nobody has situated, and inbox captures rotting unprocessed.
///
/// Read-only by design — see `docs/spec.md`, "The maintenance loop". This
/// walks the corpus and writes a report; it never edits a node, an inbox
/// entry, or the manifest. The judgement calls (kill conditions that look
/// satisfied, likely `contradicts` pairs) stay out of scope for the same
/// reason: they belong to the agent that reads this report, not to this
/// query.
pub fn review(
    root: Option<PathBuf>,
    since: Option<i64>,
    out: Option<&Path>,
    json: bool,
) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let inbox = store.inbox()?;

    let hypothesis_days = since.unwrap_or(30);
    let seed_days = since.unwrap_or(90);

    let mut stale_hypotheses = Vec::new();
    let mut untouched_seeds = Vec::new();
    let mut no_references = Vec::new();

    for d in &docs {
        let n = &d.node;
        if n.status == Status::Hypothesis
            && n.evidence.is_empty()
            && store::days_since(&n.updated).is_some_and(|days| days > hypothesis_days)
        {
            stale_hypotheses.push(ReviewItem {
                rule: "stale-hypothesis",
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("hypothesis with no evidence for over {hypothesis_days} days"),
            });
        }
        if n.status == Status::Seed
            && store::days_since(&n.updated).is_some_and(|days| days > seed_days)
        {
            untouched_seeds.push(ReviewItem {
                rule: "untouched-seed",
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!(
                    "seed untouched for over {seed_days} days; propose: status abandoned"
                ),
            });
        }
        if n.status.is_open() && n.references.is_empty() {
            no_references.push(ReviewItem {
                rule: "no-references",
                id: n.id.clone(),
                title: n.title.clone(),
                reason: "no references attached".into(),
            });
        }
    }

    let stale_inbox_count = inbox
        .iter()
        .filter_map(|entry| store::days_since_stamp(&entry.at))
        .filter(|days| *days > 14)
        .count();
    let stale_inbox = if stale_inbox_count > 0 {
        vec![ReviewItem {
            rule: "stale-inbox",
            id: "inbox".into(),
            title: "inbox".into(),
            reason: format!(
                "{stale_inbox_count} captures waiting over fourteen days; promote or drop them"
            ),
        }]
    } else {
        Vec::new()
    };

    let sections = [
        (
            format!("Hypotheses with no evidence for {hypothesis_days} days"),
            stale_hypotheses,
        ),
        (
            format!("Seeds untouched for {seed_days} days"),
            untouched_seeds,
        ),
        ("Nodes with no references".to_string(), no_references),
        (
            "Inbox entries waiting more than 14 days".to_string(),
            stale_inbox,
        ),
    ];

    if json {
        let items: Vec<&ReviewItem> = sections.iter().flat_map(|(_, v)| v.iter()).collect();
        write_report(out, &serde_json::to_string_pretty(&items)?)
    } else {
        use std::fmt::Write as _;
        let mut text = String::new();
        for (heading, items) in &sections {
            let _ = writeln!(text, "## {heading}\n");
            if items.is_empty() {
                text.push_str("_none_\n\n");
            } else {
                for item in items {
                    let _ = writeln!(text, "- `{}` {} — {}", item.id, item.title, item.reason);
                }
                text.push('\n');
            }
        }
        write_report(out, text.trim_end())
    }
}

/// Send the rendered report to a file, or print it, per `--out`.
fn write_report(out: Option<&Path>, text: &str) -> Result<()> {
    match out {
        Some(path) => std::fs::write(path, format!("{text}\n"))?,
        None => println!("{text}"),
    }
    Ok(())
}

// ------------------------------------------------------------------ domains --
