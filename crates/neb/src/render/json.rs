//! The `--json` view of every payload that core's own serialisation would
//! not state in full.
//!
//! Core's types serialise for the corpus files and the desktop, and several
//! of them leave a value out when it is absent: the YAML frontmatter stores
//! the human's authorship and every empty field by omission. A reader of
//! `--json` should not have to know that, so every `--json` arm that returns
//! a node, or something holding one, serialises through a view here instead
//! (STD-01 §R11, §R10):
//!
//! - an absent value is `null` and an empty collection `[]`, never a missing
//!   key, so one record type has one key set whichever verb wrote it;
//! - author labels are stated through [`nebula_core::Node::with_authorship_stated`],
//!   so a write verb says `"human"` exactly where `show` does.
//!
//! Each view is built by destructuring the core type in full, so a field
//! added to core fails to compile here until the view says what it is.
//!
//! A list that a limit cut is a [`Capped`] envelope rather than a bare array
//! (STD-01 §R34): always for `near`, and for `list`, `inbox`, `review` and
//! `trace` whenever `--limit` or `--depth` bounds them. See [`List`].

use nebula_core::{Closed, EdgeType, InboxEntry, Neighbour, Note, Status, TraceHop};
use serde::Serialize;
use std::path::PathBuf;

/// One node, every field present. See [`nebula_core::Node`].
#[derive(Debug, Serialize)]
pub(crate) struct Node {
    id: String,
    title: String,
    title_by: Option<String>,
    status: Status,
    created: String,
    updated: String,
    kill: Option<String>,
    kill_by: Option<String>,
    tags: Vec<String>,
    edges: Vec<Edge>,
    references: Vec<Reference>,
    closed: Option<Closed>,
    origin: Option<Origin>,
}

impl From<&nebula_core::Node> for Node {
    fn from(node: &nebula_core::Node) -> Self {
        let nebula_core::Node {
            id,
            title,
            title_by,
            status,
            created,
            updated,
            kill,
            kill_by,
            tags,
            edges,
            references,
            closed,
            origin,
        } = node.clone().with_authorship_stated();
        Self {
            id,
            title,
            title_by,
            status,
            created,
            updated,
            kill,
            kill_by,
            tags,
            edges: edges.iter().map(Edge::from).collect(),
            references: references.iter().map(Reference::from).collect(),
            closed,
            origin: origin.as_ref().map(Origin::from),
        }
    }
}

/// A typed link, its author stated. See [`nebula_core::Edge`].
#[derive(Debug, Serialize)]
struct Edge {
    #[serde(rename = "type")]
    kind: EdgeType,
    to: String,
    by: Option<String>,
}

impl From<&nebula_core::Edge> for Edge {
    fn from(edge: &nebula_core::Edge) -> Self {
        let nebula_core::Edge { kind, to, by } = edge;
        Self {
            kind: *kind,
            to: to.clone(),
            by: by.clone(),
        }
    }
}

/// A reference, every field present. See [`nebula_core::Reference`].
#[derive(Debug, Serialize)]
struct Reference {
    id: String,
    kind: String,
    uri: Option<String>,
    title: Option<String>,
    note: Option<String>,
    added: String,
    by: Option<String>,
    origin: Option<Origin>,
}

impl From<&nebula_core::Reference> for Reference {
    fn from(reference: &nebula_core::Reference) -> Self {
        let nebula_core::Reference {
            id,
            kind,
            uri,
            title,
            note,
            added,
            by,
            origin,
        } = reference;
        Self {
            id: id.clone(),
            kind: kind.clone(),
            uri: uri.clone(),
            title: title.clone(),
            note: note.clone(),
            added: added.clone(),
            by: by.clone(),
            origin: origin.as_ref().map(Origin::from),
        }
    }
}

/// Provenance, every field present. See [`nebula_core::Origin`].
#[derive(Debug, Serialize)]
struct Origin {
    task: Option<String>,
    workspace: Option<String>,
    run: Option<String>,
    artifact: Option<String>,
    agent: Option<String>,
    at: Option<String>,
}

impl From<&nebula_core::Origin> for Origin {
    fn from(origin: &nebula_core::Origin) -> Self {
        let nebula_core::Origin {
            task,
            workspace,
            run,
            artifact,
            agent,
            at,
        } = origin.clone();
        Self {
            task,
            workspace,
            run,
            artifact,
            agent,
            at,
        }
    }
}

/// A node file: the node and its prose. See [`nebula_core::Doc`].
#[derive(Debug, Serialize)]
pub(crate) struct Doc {
    node: Node,
    body: String,
}

impl From<&nebula_core::Doc> for Doc {
    fn from(doc: &nebula_core::Doc) -> Self {
        let nebula_core::Doc { node, body } = doc;
        Self {
            node: Node::from(node),
            body: body.clone(),
        }
    }
}

/// `new` and `promote`. See [`nebula_core::Created`].
#[derive(Debug, Serialize)]
pub(crate) struct Created {
    doc: Doc,
    path: PathBuf,
    near: Vec<Neighbour>,
}

impl From<&nebula_core::Created> for Created {
    fn from(created: &nebula_core::Created) -> Self {
        let nebula_core::Created { doc, path, near } = created;
        Self {
            doc: Doc::from(doc),
            path: path.clone(),
            near: near.clone(),
        }
    }
}

/// `capture`. See [`nebula_core::Captured`].
#[derive(Debug, Serialize)]
pub(crate) struct Captured {
    entry: InboxEntry,
    near: Vec<Neighbour>,
}

impl From<&nebula_core::Captured> for Captured {
    fn from(captured: &nebula_core::Captured) -> Self {
        let nebula_core::Captured { entry, near } = captured;
        Self {
            entry: entry.clone(),
            near: near.clone(),
        }
    }
}

/// `cite`, `observatory` null for any kind but `observatory`. See
/// [`nebula_core::Cited`].
#[derive(Debug, Serialize)]
pub(crate) struct Cited {
    doc: Doc,
    reference: String,
    observatory: Option<ObservatoryLink>,
}

impl From<&nebula_core::Cited> for Cited {
    fn from(cited: &nebula_core::Cited) -> Self {
        let nebula_core::Cited {
            doc,
            reference,
            observatory,
        } = cited;
        Self {
            doc: Doc::from(doc),
            reference: reference.clone(),
            observatory: observatory.as_ref().map(ObservatoryLink::from),
        }
    }
}

/// `status`. See [`nebula_core::StatusChange`].
#[derive(Debug, Serialize)]
pub(crate) struct StatusChange {
    doc: Doc,
    from: Status,
}

impl From<&nebula_core::StatusChange> for StatusChange {
    fn from(changed: &nebula_core::StatusChange) -> Self {
        let nebula_core::StatusChange { doc, from } = changed;
        Self {
            doc: Doc::from(doc),
            from: *from,
        }
    }
}

/// `handoff`. See [`nebula_core::HandedOff`].
#[derive(Debug, Serialize)]
pub(crate) struct HandedOff {
    doc: Doc,
    reference: String,
    record: String,
    from: Status,
    observatory: ObservatoryLink,
}

impl From<&nebula_core::HandedOff> for HandedOff {
    fn from(done: &nebula_core::HandedOff) -> Self {
        let nebula_core::HandedOff {
            doc,
            reference,
            record,
            from,
            observatory,
        } = done;
        Self {
            doc: Doc::from(doc),
            reference: reference.clone(),
            record: record.clone(),
            from: *from,
            observatory: ObservatoryLink::from(observatory),
        }
    }
}

/// `show`, and `edit` and `note`, which answer with the node as `show` would.
/// See [`nebula_core::NodeView`].
#[derive(Debug, Serialize)]
pub(crate) struct NodeView {
    node: Node,
    body: String,
    notes: Vec<Note>,
    observatory: Vec<ObservatoryLink>,
    handed_off_to: Option<String>,
}

impl From<&nebula_core::NodeView> for NodeView {
    fn from(view: &nebula_core::NodeView) -> Self {
        let nebula_core::NodeView {
            node,
            body,
            notes,
            observatory,
            handed_off_to,
        } = view;
        Self {
            node: Node::from(node),
            body: body.clone(),
            notes: notes.clone(),
            observatory: observatory.iter().map(ObservatoryLink::from).collect(),
            handed_off_to: handed_off_to.clone(),
        }
    }
}

/// One `observatory` reference, `path` null where it does not resolve.
/// See [`nebula_core::ObservatoryLink`].
#[derive(Debug, Serialize)]
struct ObservatoryLink {
    reference: String,
    record: String,
    path: Option<PathBuf>,
}

impl From<&nebula_core::ObservatoryLink> for ObservatoryLink {
    fn from(link: &nebula_core::ObservatoryLink) -> Self {
        let nebula_core::ObservatoryLink {
            reference,
            record,
            path,
        } = link;
        Self {
            reference: reference.clone(),
            record: record.clone(),
            path: path.clone(),
        }
    }
}

/// One node on a `trace` walk. See [`nebula_core::TraceNode`].
#[derive(Debug, Serialize)]
pub(crate) struct TraceNode {
    id: String,
    title: String,
    status: Status,
    parents: Vec<String>,
    via: Option<TraceHop>,
    handed_off_to: Option<String>,
}

impl From<&nebula_core::TraceNode> for TraceNode {
    fn from(step: &nebula_core::TraceNode) -> Self {
        let nebula_core::TraceNode {
            id,
            title,
            status,
            parents,
            via,
            handed_off_to,
        } = step;
        Self {
            id: id.clone(),
            title: title.clone(),
            status: *status,
            parents: parents.clone(),
            via: via.clone(),
            handed_off_to: handed_off_to.clone(),
        }
    }
}

/// A list a limit cut, or could have: the records kept, how many matched
/// before the cut, and whether it dropped any (STD-01 §R34).
#[derive(Debug, Serialize)]
pub(crate) struct Capped<T> {
    items: Vec<T>,
    total: usize,
    truncated: bool,
}

impl<T> Capped<T> {
    /// The `items` a cut kept, and the `(total, truncated)` it reported.
    pub(crate) fn new(items: Vec<T>, (total, truncated): (usize, bool)) -> Self {
        Self {
            items,
            total,
            truncated,
        }
    }
}

/// A list as `--json` gives it: the bare array when nothing bounded it, and
/// the [`Capped`] envelope whenever a `--limit` or `--depth` did, cut or not,
/// so the shape follows the flag and never the data.
///
/// A bare array for an unbounded list is what STD-01 §R34 allows, and what
/// `neb list --json | jq '.[]'` has always read.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum List<T> {
    /// No bound was asked for: every match.
    Bare(Vec<T>),
    /// A bound was asked for.
    Capped(Capped<T>),
}

impl<T> List<T> {
    /// `items`, bare when `cut` is `None` (nothing bounded the list), and
    /// otherwise enveloped with the `(total, truncated)` the cut reported.
    pub(crate) fn new(items: Vec<T>, cut: Option<(usize, bool)>) -> Self {
        match cut {
            None => Self::Bare(items),
            Some(cut) => Self::Capped(Capped::new(items, cut)),
        }
    }
}
