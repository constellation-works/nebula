//! Things attached to a node: references.

use super::store_at;
use crate::check::{is_local_path, resolve_local};
use crate::corpus::{Origin, Reference, store};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::PathBuf;

/// Attach context. The note is the field that matters.
#[allow(clippy::too_many_arguments)]
pub fn cite(
    root: Option<PathBuf>,
    node_id: &str,
    uri: &str,
    kind: &str,
    title: Option<String>,
    note: Option<String>,
    task: Option<String>,
    run: Option<String>,
) -> Result<()> {
    let store = store_at(root)?;
    let mut doc = store.load(node_id)?;
    // Rule 8 at the point of action: a local path that does not resolve is a
    // citation to nothing, and refusing it here is cheaper than finding it
    // in `check` after the context of why it was attached has gone.
    if is_local_path(uri) && !resolve_local(&store, uri).exists() {
        bail!(
            "`{uri}` does not resolve from {}; local references are relative to nodes/",
            store.root().join("nodes").display()
        );
    }
    let id = doc.node.next_reference_id();
    let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
    doc.node.references.push(Reference {
        id: id.clone(),
        kind: kind.to_string(),
        uri: uri.to_string(),
        title,
        note,
        added: store::today(),
        origin: (task.is_some() || run.is_some()).then_some(Origin {
            task,
            run,
            ..Origin::default()
        }),
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
