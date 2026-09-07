//! Things attached to a node: evidence, references, and weighing one into the other.

use super::store_at;
use crate::corpus::{Evidence, Origin, Reference, Status, Strength, Verdict, store};
use crate::render::{bold, dim};
use anyhow::{Result, bail};
use std::path::PathBuf;

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
