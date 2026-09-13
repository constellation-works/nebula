//! Reading the graph: trace, impact, open, show, list, review.

use super::{out_json, store_at};
use crate::corpus::Doc;
use crate::corpus::{EdgeType, Status, model, store};
use crate::render::{self, Tree};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Days a seed may sit untouched before `open` and `review` raise it.
const SEED_DAYS: i64 = 90;
/// Days a hypothesis may sit untouched before `review` calls it stale.
const HYPOTHESIS_DAYS: i64 = 30;
/// Days an inbox capture may wait before `open` and `review` raise it.
const INBOX_DAYS: i64 = 14;

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
        collect(node_id, &by_id, down, &mut HashSet::new(), &mut acc);
        return out_json(&acc);
    }
    let tree = Tree::new(&by_id);
    let text = if down {
        let children = children_index(&docs);
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

/// Every node's genealogical children, so descent can be walked without
/// rescanning the corpus at each step.
fn children_index(docs: &[Doc]) -> HashMap<&str, Vec<String>> {
    let mut out: HashMap<&str, Vec<String>> = HashMap::new();
    for d in docs {
        for p in d.node.parents() {
            out.entry(p).or_default().push(d.node.id.clone());
        }
    }
    out
}

fn collect(
    id: &str,
    by_id: &HashMap<&str, &Doc>,
    down: bool,
    seen: &mut HashSet<String>,
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

/// What this node touches: everything that descends from it, and whatever
/// it contradicts.
pub fn impact(root: Option<PathBuf>, node_id: &str, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let doc = store.load(node_id)?;
    let children = children_index(&docs);

    // Descendants by reverse genealogy, breadth-first so nearer ones print
    // first, each reported once even when reached along two branches.
    let mut descendants: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::from([node_id]);
    let mut frontier = vec![node_id];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for at in frontier {
            for c in children.get(at).map(Vec::as_slice).unwrap_or_default() {
                if seen.insert(c.as_str()) {
                    descendants.push(c.clone());
                    next.push(c.as_str());
                }
            }
        }
        frontier = next;
    }
    let contradicts: Vec<&str> = doc.node.edges_of(EdgeType::Contradicts).collect();

    if json {
        let v: Vec<_> = descendants
            .iter()
            .map(|id| serde_json::json!({"id": id, "via": "descends"}))
            .chain(
                contradicts
                    .iter()
                    .map(|id| serde_json::json!({"id": id, "via": "contradicts"})),
            )
            .collect();
        return out_json(&v);
    }
    if descendants.is_empty() && contradicts.is_empty() {
        println!("{}", dim("nothing descends from or contradicts this node"));
        return Ok(());
    }
    if !descendants.is_empty() {
        println!("{}", dim(&format!("descends from `{node_id}`:")));
        for id in &descendants {
            println!("  {}", bold(id));
        }
    }
    if !contradicts.is_empty() {
        println!("{}", dim(&format!("contradicts `{node_id}`:")));
        for id in &contradicts {
            println!("  {}", bold(id));
        }
    }
    Ok(())
}

/// Nodes that need attention.
pub fn open(root: Option<PathBuf>, tags: &[String], json: bool) -> Result<()> {
    let store = store_at(root)?;
    let tags = model::normalize_tags(tags);
    let docs = store.load_all()?;
    let mut items: Vec<(String, String)> = Vec::new();
    let stale_inbox_count = stale_inbox(&store.inbox()?);
    if stale_inbox_count > 0 {
        items.push((
            "inbox".into(),
            format!(
                "{stale_inbox_count} captures waiting over fourteen days; promote or drop them"
            ),
        ));
    }
    for d in docs.iter().filter(|d| d.node.has_all_tags(&tags)) {
        let n = &d.node;
        // The genuinely actionable gap: a hypothesis that names what would
        // kill it and has nothing attached that bears on the question.
        if n.status == Status::Hypothesis && n.references.is_empty() {
            items.push((n.id.clone(), "hypothesis with no references".into()));
        }
        if n.status == Status::Seed && older_than(&n.updated, SEED_DAYS) {
            items.push((
                n.id.clone(),
                "seed untouched for ninety days; abandon it?".into(),
            ));
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
    Ok(())
}

/// Whether a `YYYY-MM-DD` date is at least `days` old.
fn older_than(date: &str, days: i64) -> bool {
    store::days_since(date).is_some_and(|d| d >= days)
}

/// Captures waiting at least fourteen days.
fn stale_inbox(inbox: &[store::InboxEntry]) -> usize {
    inbox
        .iter()
        .filter_map(|entry| store::days_since_stamp(&entry.at))
        .filter(|days| *days >= INBOX_DAYS)
        .count()
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
        dim(&n.tags.join(", "))
    );
    println!("{}\n", n.title);
    if let Some(k) = &n.kill {
        println!("{} {k}\n", dim("kill:"));
    }
    if let Some(c) = &n.closed {
        println!("{} {} {}\n", dim("closed:"), c.why, dim(&c.at));
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
    if !n.references.is_empty() {
        println!("{}", dim("references"));
        for r in &n.references {
            println!("  {} {:<10} {}", bold(&r.id), r.kind, r.uri);
            let text = r.note.as_deref().map_or("(no note)", str::trim);
            println!("     {}", dim(text));
        }
        println!();
    }
    Ok(())
}

/// List nodes. Several `--tag`s narrow to nodes carrying all of them.
pub fn list(
    root: Option<PathBuf>,
    status: Option<Status>,
    tags: &[String],
    json: bool,
) -> Result<()> {
    let store = store_at(root)?;
    let tags = model::normalize_tags(tags);
    let docs = store.load_all()?;
    let picked: Vec<&Doc> = docs
        .iter()
        .filter(|d| status.is_none_or(|s| d.node.status == s))
        .filter(|d| d.node.has_all_tags(&tags))
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

/// The weekly maintenance report: hypotheses gone quiet, seeds gone cold,
/// nodes nobody has situated, and inbox captures rotting unprocessed.
///
/// Read-only by design. This walks the corpus and writes a report; it never
/// edits a node, an inbox entry, or the config. The judgement calls (kill
/// conditions that look satisfied, likely `contradicts` pairs) stay out of
/// scope for the same reason: they belong to the agent that reads this
/// report, not to this query.
pub fn review(
    root: Option<PathBuf>,
    since: Option<i64>,
    out: Option<&Path>,
    json: bool,
) -> Result<()> {
    let store = store_at(root)?;
    let docs = store.load_all()?;
    let inbox = store.inbox()?;

    let hypothesis_days = since.unwrap_or(HYPOTHESIS_DAYS);
    let seed_days = since.unwrap_or(SEED_DAYS);

    let mut stale_hypotheses = Vec::new();
    let mut untouched_seeds = Vec::new();
    let mut no_references = Vec::new();

    for d in &docs {
        let n = &d.node;
        if n.status == Status::Hypothesis && older_than(&n.updated, hypothesis_days) {
            stale_hypotheses.push(ReviewItem {
                rule: "stale-hypothesis",
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("hypothesis untouched for {hypothesis_days} days"),
            });
        }
        if n.status == Status::Seed && older_than(&n.updated, seed_days) {
            untouched_seeds.push(ReviewItem {
                rule: "untouched-seed",
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("seed untouched for {seed_days} days; propose: status abandoned"),
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

    let stale_inbox_count = stale_inbox(&inbox);
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
            format!("Hypotheses untouched for {hypothesis_days} days"),
            stale_hypotheses,
        ),
        (
            format!("Seeds untouched for {seed_days} days"),
            untouched_seeds,
        ),
        ("Nodes with no references".to_string(), no_references),
        (
            format!("Inbox entries waiting {INBOX_DAYS} days or more"),
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
