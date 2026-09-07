//! One function per verb, grouped by what they act on. Every function takes
//! the resolved `--root` and returns an error the CLI prints; none of them
//! know about clap.

mod attach;
mod check;
mod domain;
mod inbox;
mod node;
mod read;

pub use attach::{cite, evidence, weigh};
pub use check::check;
pub use domain::{domain_add, domain_default, domain_list, domain_set};
pub use inbox::{capture, drop_entry, inbox, init, promote};
pub use node::{graduate, link, new_node, set_status, sharpen, task};
pub use read::{impact, list, open, show, trace};

use crate::corpus::{Doc, Store};
use crate::render::dim;
use anyhow::Result;
use std::path::PathBuf;

fn store_at(root: Option<PathBuf>) -> Result<Store> {
    Store::open(root)
}

fn out_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Whether a node falls inside a domain scope; `None` is the whole corpus.
fn in_scope(doc: &Doc, scope: Option<&str>) -> bool {
    scope.is_none_or(|s| doc.node.domain == s)
}

/// Say when a read was narrowed, so a missing node is not mistaken for a
/// missing idea.
fn scope_footer(scope: Option<&str>) {
    if let Some(s) = scope {
        println!(
            "{}",
            dim(&format!("\ndomain: {s}  (--all crosses domains)"))
        );
    }
}
