//! Typed edges between two existing nodes.

use crate::error::{Error, Result};
use crate::graph_impl as graph;
use crate::model::{self, Doc, Edge, EdgeType};
use crate::store::Corpus;

/// Add a typed edge between two nodes.
///
/// Returns every node that changed: a `contradicts` edge is a claim about both
/// ends, so it is recorded on both.
///
/// The two ends are two saves, so a crash between them leaves a
/// `contradicts` recorded on `from` alone, which `check` reports under rule
/// 4. Running the same link again finishes it: with the edge already on
/// `from` and missing from `to`, only the reverse is written, credited to
/// whoever made the half that is there, since it is the other half of that
/// same claim (STD-03 §R9). With both halves present it is refused as
/// [`Error::DuplicateEdge`], like any other edge written twice.
///
/// `by` is whoever claims the relation; `None` is the human.
pub fn link(
    corpus: &Corpus,
    from: &str,
    kind: EdgeType,
    to: &str,
    by: Option<&str>,
) -> Result<Vec<Doc>> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(by))?;
    crate::id::require_safe_id(from)?;
    crate::id::require_safe_id(to)?;
    if from == to {
        return Err(Error::SelfLoop);
    }
    // Held across both ends. A `contradicts` edge is saved on `from` and then
    // on `to`, and the cycle check reads every node before saving one; either
    // gap is where a second writer leaves a one-sided edge or closes a loop
    // this check did not see.
    let _lock = corpus.lock()?;
    let by = model::author(by)?;
    let mut doc = corpus.load(from)?;
    let target = corpus.load(to)?;
    if doc.node.has_edge(kind, to) {
        return finish_contradiction(corpus, &doc, kind, target);
    }
    doc.node.edges.push(Edge {
        kind,
        to: to.to_string(),
        by: by.clone(),
    });

    // Genealogy must stay acyclic, so refuse the edge that would close a
    // loop rather than leaving `check` to find it later.
    if kind.is_genealogy() {
        // "The cycle check runs under the write lock" (4_decisions.md, STD-03@2 §R1).
        let mut docs = corpus.load_all()?;
        docs.retain(|d| d.node.id != doc.node.id);
        docs.push(doc.clone());
        if graph::creates_cycle(&docs, from) {
            return Err(Error::Cycle {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
    }
    corpus.save(&mut doc)?;
    let mut changed = vec![doc];

    // `contradicts` is a claim about both nodes, so record it on both.
    if kind == EdgeType::Contradicts {
        let mut other = corpus.load(to)?;
        if !other.node.has_edge(kind, from) {
            other.node.edges.push(Edge {
                kind,
                to: from.to_string(),
                by,
            });
            corpus.save(&mut other)?;
            changed.push(other);
        }
    }
    Ok(changed)
}

/// The half of a `contradicts` edge that a crash kept `to` from recording,
/// written onto `to` from the half `from` already has. Anything else asked
/// for a second time is [`Error::DuplicateEdge`].
fn finish_contradiction(
    corpus: &Corpus,
    from: &Doc,
    kind: EdgeType,
    mut to: Doc,
) -> Result<Vec<Doc>> {
    let recorded = from
        .node
        .edges
        .iter()
        .find(|edge| edge.kind == kind && edge.to == to.node.id);
    let duplicate = || Error::DuplicateEdge {
        from: from.node.id.clone(),
        kind,
        to: to.node.id.clone(),
    };
    let Some(recorded) = recorded.filter(|_| kind == EdgeType::Contradicts) else {
        return Err(duplicate());
    };
    if to.node.has_edge(kind, &from.node.id) {
        return Err(duplicate());
    }
    to.node.edges.push(Edge {
        kind,
        to: from.node.id.clone(),
        by: recorded.by.clone(),
    });
    corpus.save(&mut to)?;
    Ok(vec![to])
}
