//! The rules judged one node at a time: its lifecycle, its dates, its edges
//! and its references.

use crate::model::{Doc, Status, is_iso_date};
use crate::store::Corpus;
use std::collections::HashSet;

use super::observatory::check_observatory_reference;
use super::{
    OBSERVATORY, ObservatoryState, REFERENCE_KINDS, Report, Rule, Severity, is_absolute_local,
    is_local_path, is_reference_kind, resolve_local,
};

/// Rules about where a node sits in its lifecycle and what that costs.
pub(super) fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before anything
    //    is read. Without this, the verdict is retroactive rationalisation.
    if n.status == Status::Hypothesis && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Severity::Error,
            Rule::HypothesisKill,
            id,
            "status is hypothesis but no kill condition is named",
        );
    }

    // 5. A refuted node says how its kill condition fired. The reference
    //    that convinced you goes in `references`; the sentence goes here.
    if n.status == Status::Refuted && n.closed.as_ref().is_none_or(|c| c.why.trim().is_empty()) {
        r.push(
            Severity::Error,
            Rule::RefutedReason,
            id,
            "status is refuted but closed.why is empty; say what fired the kill condition",
        );
    }
}

/// Rules about whether the stored lifecycle fields still agree with
/// `status`, the shape every verb leaves them in but a hand edit can pull
/// apart.
pub(super) fn lifecycle_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 12. `set_status` clears `closed` the moment a node leaves refuted or
    //     abandoned, and no verb ever sets it any other way, so a `closed`
    //     block on a seed or hypothesis is not a state any verb produces —
    //     it is what an earlier abandonment or refutation left behind after
    //     a hand edit reopened the node without going through `status`.
    if n.status.is_open() && n.closed.is_some() {
        r.push(
            Severity::Error,
            Rule::OpenNodeClosed,
            id,
            format!(
                "status is `{}` but a `closed` block is still set; leftover from an earlier close?",
                n.status
            ),
        );
    }
    // 13. `new --kill` and `sharpen` only ever write a kill condition
    //     together with a move to `hypothesis`, and `status` refuses to move
    //     a node carrying one back to `seed`, so a `seed` carrying one was
    //     set by hand without the guard that would have moved the status
    //     too. Not wrong by itself — the node has not yet been re-sharpened
    //     — so this is a warning rather than an error.
    if n.status == Status::Seed && n.kill.is_some() {
        r.push(
            Severity::Warn,
            Rule::SeedKill,
            id,
            "status is seed but a kill condition is set; `new --kill`/`sharpen` always move \
             status to hypothesis, so this looks like a hand edit",
        );
    }
}

/// Rules about whether the stored dates parse and, where order matters,
/// agree with each other.
pub(super) fn date_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 14. Every date `ops.rs` writes comes from `stamp::today()`, so
    //     `created`/`updated` are always `YYYY-MM-DD` and never move
    //     backwards. A hand edit is the only way either goes wrong, and
    //     `review`/`open` then silently treat the node as never stale,
    //     since `days_since` returns `None` for a date it cannot parse.
    let created_ok = is_iso_date(&n.created);
    if !created_ok {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!("created `{}` is not a YYYY-MM-DD date", n.created),
        );
    }
    let updated_ok = is_iso_date(&n.updated);
    if !updated_ok {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!("updated `{}` is not a YYYY-MM-DD date", n.updated),
        );
    }
    if created_ok && updated_ok && n.updated < n.created {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!(
                "updated `{}` is earlier than created `{}`",
                n.updated, n.created
            ),
        );
    }
    for f in &n.references {
        if !is_iso_date(&f.added) {
            r.push(
                Severity::Error,
                Rule::Dates,
                id,
                format!(
                    "reference `{}` has an added date `{}` that does not parse",
                    f.id, f.added
                ),
            );
        }
    }
}

/// Rules about the links a node declares.
pub(super) fn edge_rules(doc: &Doc, ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 3. Dangling edges and self-loops. A lineage that points into nothing
    //    is worse than no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Severity::Error,
                Rule::EdgeTargets,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Severity::Error,
                Rule::EdgeTargets,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }
}

/// Rules about the references hanging off a node.
pub(super) fn reference_rules(
    doc: &Doc,
    corpus: &Corpus,
    observatory: ObservatoryState<'_>,
    r: &mut Report,
) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    for f in &n.references {
        if !is_reference_kind(&f.kind) {
            r.push(
                Severity::Warn,
                Rule::ReferenceKind,
                id,
                format!(
                    "reference `{}` has unexpected kind `{}`; accepted kinds: {}",
                    f.id,
                    f.kind,
                    REFERENCE_KINDS.join(", ")
                ),
            );
        }
        if f.uri.as_deref().is_none_or(|uri| uri.trim().is_empty()) && f.kind != "discussion" {
            r.push(
                Severity::Error,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` has kind `{}` but no URI; only discussions may omit it",
                    f.id, f.kind
                ),
            );
        }
        // 10. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Severity::Warn,
                Rule::ReferenceNote,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
        // 9, for an Observatory record: the id is the reference, and where
        //    it is on this machine is a setting. Not finding it here says
        //    the setting is missing or the checkout is behind, not that the
        //    record is gone, so the finding is a warning rather than an
        //    error and the reference stays valid on a machine that has it.
        if f.kind == OBSERVATORY {
            check_observatory_reference(f, id, observatory, r);
            continue;
        }
        // 8, for an absolute path or a `file:` URI: it may well resolve on
        //    this machine, which is exactly why `check` cannot judge it by
        //    resolving — on every other machine the corpus is synced to, it
        //    names nothing. `cite` refuses new ones; one already here, hand
        //    written or carried over from a v1 `evidence` source, is a
        //    warning, so an older corpus still loads and checks.
        if let Some(uri) = f.uri.as_deref().filter(|uri| is_absolute_local(uri)) {
            r.push(
                Severity::Warn,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` uses an absolute local path, which resolves on this machine \
                     only: {uri}; local references are relative to nodes/",
                    f.id
                ),
            );
            continue;
        }
        // 8. A local path that does not resolve is a citation to nothing.
        //    External URLs are not fetched; `check` stays offline and fast.
        if let Some(uri) = f
            .uri
            .as_deref()
            .filter(|uri| !uri.trim().is_empty())
            .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
        {
            r.push(
                Severity::Error,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` points at a path that does not resolve: {}",
                    f.id, uri
                ),
            );
        }
    }
}
