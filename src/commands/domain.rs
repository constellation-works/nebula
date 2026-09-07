//! Declared domains and moving nodes between them.

use super::{out_json, store_at};

use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::PathBuf;

/// Declared domains, with the default marked.
pub fn domain_list(root: Option<PathBuf>, json: bool) -> Result<()> {
    let store = store_at(root)?;
    let cfg = store.config();
    if json {
        return out_json(&serde_json::json!({
            "corpus_id": cfg.corpus_id,
            "domains": cfg.domains,
            "default": cfg.default_domain,
        }));
    }
    let docs = store.load_all()?;
    for d in &cfg.domains {
        let count = docs.iter().filter(|x| &x.node.domain == d).count();
        let mark = if cfg.default_domain.as_deref() == Some(d) {
            " (default)"
        } else {
            ""
        };
        println!(
            "{}{} {}",
            bold(d),
            dim(mark),
            dim(&format!("{count} nodes"))
        );
    }
    let homeless = docs.iter().filter(|x| !cfg.has(&x.node.domain)).count();
    if homeless > 0 {
        println!(
            "\n{}",
            dim(&format!(
                "{homeless} nodes name no declared domain; fix with:  neb domain set <node> <domain>"
            ))
        );
    }
    Ok(())
}

/// Declare a domain.
pub fn domain_add(root: Option<PathBuf>, name: &str) -> Result<()> {
    crate::corpus::config::validate_name(name)?;
    let mut store = store_at(root)?;
    let mut cfg = store.config().clone();
    if cfg.has(name) {
        bail!("domain `{name}` is already declared");
    }
    cfg.domains.push(name.to_string());
    let several = cfg.domains.len() > 1;
    store.save_config(cfg)?;
    println!("declared {}", bold(name));
    if several && store.config().default_domain.is_none() {
        println!(
            "{}",
            dim("no default domain; bare list/open now show everything until you set one")
        );
    }
    Ok(())
}

/// Choose where bare commands look.
pub fn domain_default(root: Option<PathBuf>, name: &str) -> Result<()> {
    let mut store = store_at(root)?;
    let mut cfg = store.config().clone();
    cfg.require(name)?;
    cfg.default_domain = Some(name.to_string());
    store.save_config(cfg)?;
    println!("default domain is {}", bold(name));
    Ok(())
}

/// Move a node between domains. Also how a corpus that predates domains
/// gets its nodes placed.
pub fn domain_set(root: Option<PathBuf>, node_id: &str, name: &str) -> Result<()> {
    let store = store_at(root)?;
    store.config().require(name)?;
    let mut doc = store.load(node_id)?;
    if doc.node.domain == name {
        println!("{} is already in {}", bold(node_id), name);
        return Ok(());
    }
    let from = if doc.node.domain.is_empty() {
        "(none)".to_string()
    } else {
        doc.node.domain.clone()
    };
    doc.node.domain = name.to_string();
    store.save(&mut doc)?;
    println!("{} {} -> {}", bold(node_id), dim(&from), name);
    Ok(())
}
