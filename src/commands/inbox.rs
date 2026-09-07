//! The five-second path and its triage: capture, inbox, drop, promote.

use super::node::{NodeSpec, build};
use super::{out_json, store_at};
use crate::corpus::Status;
use crate::corpus::Store;
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::PathBuf;

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
    domain: Option<&str>,
    parents: &[String],
    tags: &[String],
) -> Result<()> {
    let store = store_at(root)?;
    let e = store.inbox_entry(entry)?;
    let title = title.unwrap_or_else(|| e.text.clone());
    let spec = NodeSpec {
        title: &title,
        domain,
        parents,
        kill: None,
        status: Status::Seed,
        tags,
        body: &e.text,
    };
    let doc = build(&store, &spec)?;
    store.create(&doc)?;
    store.settle_inbox(&e, &format!("-> {}", doc.node.id))?;
    println!(
        "{} {}",
        bold(&doc.node.id),
        dim(&store.node_path(&doc.node.id).display().to_string())
    );
    Ok(())
}
