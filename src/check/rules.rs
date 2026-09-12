//! The per-node invariants. Each function owns the rules that share a
//! subject, and the numbers match `docs/design/lineage-graph/specs/invariants.md`.

use super::{Level, Report};
use crate::corpus::{Doc, EdgeType, Status, Store, Verdict};
use std::collections::HashSet;

pub(in crate::check) fn is_task_id(s: &str) -> bool {
    let Some((prefix, digits)) = s.split_once('-') else {
        return false;
    };
    (2..=5).contains(&prefix.len())
        && prefix.bytes().all(|b| b.is_ascii_uppercase())
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
}

fn is_local_path(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("[[") && !uri.starts_with("doi:")
}

/// Every invariant that can be judged from a single node plus the id set.
pub(super) fn check_node(
    doc: &Doc,
    docs: &[Doc],
    ids: &HashSet<&str>,
    store: &Store,
    r: &mut Report,
) {
    domain_rule(doc, store, r);
    status_rules(doc, r);
    edge_rules(doc, docs, ids, r);
    attachment_rules(doc, r);
    provenance_rules(doc, store, r);
}

/// 15. Every node names a declared domain. The set is closed so that a
///     domain cannot drift into three spellings, and a node with none is
///     usually one that predates domains and needs `neb domain set`.
fn domain_rule(doc: &Doc, store: &Store, r: &mut Report) {
    let n = &doc.node;
    if n.domain.is_empty() {
        r.push(
            Level::Error,
            15,
            Some(&n.id),
            "no domain; place it with:  neb domain set <node> <domain>",
        );
    } else if !store.config().has(&n.domain) {
        r.push(
            Level::Error,
            15,
            Some(&n.id),
            format!(
                "domain `{}` is not declared (declared: {})",
                n.domain,
                store.config().domains.join(", ")
            ),
        );
    }
}

/// Rules about where a node sits in its lifecycle and what that costs.
fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before the
    //    evidence arrives. Without this, step four is retroactive
    //    rationalisation rather than a verdict.
    if n.status.needs_kill() && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Level::Error,
            2,
            id,
            format!("status is {} but no kill condition is named", n.status),
        );
    }

    // 3. A verdict has to rest on something.
    match n.status {
        Status::Supported if !n.has_verdict(Verdict::Supports) => {
            r.push(
                Level::Error,
                3,
                id,
                "status supported with no supporting evidence",
            );
        }
        Status::Refuted if !n.has_verdict(Verdict::Undermines) => {
            r.push(
                Level::Error,
                3,
                id,
                "status refuted with no undermining evidence",
            );
        }
        Status::Testing if n.evidence.is_empty() => {
            r.push(
                Level::Error,
                3,
                id,
                "status testing with no evidence at all",
            );
        }
        _ => {}
    }
}

/// Rules about the links a node declares.
fn edge_rules(doc: &Doc, docs: &[Doc], ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 4. Dangling edges. A lineage that points into nothing is worse than
    //    no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Level::Error,
                4,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Level::Error,
                4,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }

    // 7. A refuted node stays refuted unless something explicitly reopens
    //    it, which keeps the fact that it once died visible.
    if n.status.is_open() {
        let revived = docs
            .iter()
            .any(|d| d.node.edges_of(EdgeType::Reopens).any(|t| t == n.id));
        let was_refuted = n.evidence.iter().any(|e| e.verdict == Verdict::Undermines)
            && n.status == Status::Supported;
        if was_refuted && !revived {
            r.push(
                Level::Warn,
                7,
                id,
                "supported despite undermining evidence; say why, or reopen properly",
            );
        }
    }

    // 8. Graduating without saying where breaks the lineage at exactly the
    //    boundary it was supposed to cross.
    if n.status == Status::Graduated && n.graduated_to.is_none() {
        r.push(Level::Error, 8, id, "graduated but graduated_to is empty");
    }
}

/// Rules about evidence and references hanging off a node.
fn attachment_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 12. Ids are unique within a node and never reused.
    let mut seen = HashSet::new();
    for f in &n.references {
        if !seen.insert(f.id.as_str()) {
            r.push(
                Level::Error,
                12,
                id,
                format!("duplicate reference id `{}`", f.id),
            );
        }
        // 11. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Level::Warn,
                11,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
    }
    let mut seen = HashSet::new();
    for e in &n.evidence {
        if !seen.insert(e.id.as_str()) {
            r.push(
                Level::Error,
                12,
                id,
                format!("duplicate evidence id `{}`", e.id),
            );
        }
    }
}

/// Rules about where a node and its attachments came from.
fn provenance_rules(doc: &Doc, store: &Store, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 13. A malformed task id is provenance that cannot be resolved, which
    //     is indistinguishable from no provenance at the moment you need it.
    let tasks = n.tasks.iter().map(|t| t.id.as_str());
    let origins = n.origin.as_ref().and_then(|o| o.task.as_deref());
    for t in tasks.chain(origins) {
        if !is_task_id(t) {
            r.push(
                Level::Error,
                13,
                id,
                format!(
                    "malformed Orbit task id `{t}` (expected PREFIX-nnnnn, e.g. ORB-11440 or DANI-10293)"
                ),
            );
        }
    }

    // 14. A local path that does not resolve is a citation to nothing.
    //     External URLs are not fetched; `check` stays offline and fast.
    let local = n
        .evidence
        .iter()
        .map(|e| (e.id.as_str(), e.source.as_str()))
        .chain(n.references.iter().map(|f| (f.id.as_str(), f.uri.as_str())));
    for (eid, uri) in local {
        if is_local_path(uri) {
            let base = store.node_path(&n.id);
            let resolved = base.parent().map(|p| p.join(uri));
            if resolved.is_some_and(|p| !p.exists()) {
                r.push(
                    Level::Warn,
                    14,
                    id,
                    format!("`{eid}` points at a path that does not resolve: {uri}"),
                );
            }
        }
    }
}
