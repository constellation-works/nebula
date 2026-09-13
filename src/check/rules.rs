//! The per-node invariants. Each function owns the rules that share a
//! subject, and the numbers match `docs/design/v0.2/1_spec.md`.

use super::{Level, Report};
use crate::corpus::{Doc, Status, Store};
use std::collections::HashSet;
use std::path::Path;

/// Whether a URI is a path to resolve against `nodes/`, as opposed to a URL,
/// a wikilink, or a scheme-prefixed handle such as `doi:` or `orbit:`.
pub fn is_local_path(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("[[") && !has_scheme(uri)
}

/// `scheme:` with at least two leading letters, so a Windows drive letter
/// does not count and `doi:`, `orbit:`, `neb:`, `mailto:` all do.
fn has_scheme(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    })
}

/// Where a local reference URI lands: relative to the `nodes/` directory,
/// used as given and never canonicalized (macOS temp dirs sit under a
/// symlink, and resolving would pass on one platform and fail on the other).
pub fn resolve_local(store: &Store, uri: &str) -> std::path::PathBuf {
    store.root().join("nodes").join(Path::new(uri))
}

/// Every invariant that can be judged from a single node plus the id set.
pub(super) fn check_node(doc: &Doc, ids: &HashSet<&str>, store: &Store, r: &mut Report) {
    status_rules(doc, r);
    edge_rules(doc, ids, r);
    reference_rules(doc, store, r);
}

/// Rules about where a node sits in its lifecycle and what that costs.
fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before anything
    //    is read. Without this, the verdict is retroactive rationalisation.
    if n.status == Status::Hypothesis && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Level::Error,
            2,
            id,
            "status is hypothesis but no kill condition is named",
        );
    }

    // 5. A refuted node says how its kill condition fired. The reference
    //    that convinced you goes in `references`; the sentence goes here.
    if n.status == Status::Refuted && n.closed.as_ref().is_none_or(|c| c.why.trim().is_empty()) {
        r.push(
            Level::Error,
            5,
            id,
            "status is refuted but closed.why is empty; say what fired the kill condition",
        );
    }
}

/// Rules about the links a node declares.
fn edge_rules(doc: &Doc, ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 3. Dangling edges and self-loops. A lineage that points into nothing
    //    is worse than no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Level::Error,
                3,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Level::Error,
                3,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }
}

/// Rules about the references hanging off a node.
fn reference_rules(doc: &Doc, store: &Store, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    for f in &n.references {
        // 9. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Level::Warn,
                9,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
        // 8. A local path that does not resolve is a citation to nothing.
        //    External URLs are not fetched; `check` stays offline and fast.
        if is_local_path(&f.uri) && !resolve_local(store, &f.uri).exists() {
            r.push(
                Level::Error,
                8,
                id,
                format!(
                    "reference `{}` points at a path that does not resolve: {}",
                    f.id, f.uri
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_local_path;

    #[test]
    fn schemes_and_urls_are_not_local_paths() {
        for uri in [
            "https://example.org",
            "doi:10.1000/x",
            "orbit:DANI-10345",
            "neb:some-node",
            "[[almanac/page]]",
        ] {
            assert!(!is_local_path(uri), "{uri}");
        }
        for uri in ["./notes/x.md", "notes/x.md", "C:/x.md", "x"] {
            assert!(is_local_path(uri), "{uri}");
        }
    }
}
