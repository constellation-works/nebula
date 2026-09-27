//! References: `cite`, and the hand-off that cites an Observatory record and
//! closes the node in one write.

use super::{Citation, Cited, HandedOff, Handoff};
use crate::check_impl::{
    OBSERVATORY, is_absolute_local, is_local_path, is_observatory_id, is_reference_kind,
    normalize_reference_kind, resolve_local, resolve_observatory,
};
use crate::error::{Error, Result};
use crate::graph_impl::ObservatoryLink;
use crate::model::{self, Closed, Node, Reference, Status};
use crate::stamp;
use crate::store::Corpus;
use std::path::Path;

/// Attach context to a node. The note is the field that matters.
///
/// An `observatory` reference stores a bare record id rather than a
/// location, so the citation says the same thing on every machine. Whether
/// that record is on *this* machine is a question for `check`, which warns
/// rather than refuses: a checkout that is not there yet is not a broken
/// citation.
///
/// A local reference is a path relative to `nodes/`, for the same reason:
/// an absolute path or a `file:` URI is refused as [`Error::AbsoluteUri`]
/// even when it exists here, and a relative one that does not resolve as
/// [`Error::UnresolvedUri`].
pub fn cite(corpus: &Corpus, id: &str, args: &Citation) -> Result<Cited> {
    cite_with_observatory(corpus, id, args, None)
}

/// [`cite`] with the effective Observatory checkout. Resolve its directory
/// before saving, so an unreadable checkout cannot turn a landed citation
/// into a reported failure after the write.
pub fn cite_with_observatory(
    corpus: &Corpus,
    id: &str,
    args: &Citation,
    observatory_root: Option<&Path>,
) -> Result<Cited> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(args.by.as_deref()))?;
    let resolved_record = if normalize_reference_kind(&args.kind) == OBSERVATORY {
        match (observatory_root, args.uri.as_deref()) {
            (Some(root), Some(record)) => resolve_observatory(root, &observatory_record(record)?)?,
            _ => None,
        }
    } else {
        None
    };
    let _lock = corpus.lock()?;
    let by = model::author(args.by.as_deref())?;
    let mut doc = corpus.load(id)?;
    let reference = attach(corpus, &mut doc.node, args, by)?;
    corpus.save(&mut doc)?;
    let observatory = doc
        .node
        .references
        .iter()
        .find(|r| r.id == reference && r.kind == OBSERVATORY)
        .and_then(|r| r.uri.clone())
        .map(|record| ObservatoryLink {
            reference: reference.clone(),
            record,
            path: resolved_record,
        });
    Ok(Cited {
        doc,
        reference,
        observatory,
    })
}

/// Validate a citation and add it to `node` as its next reference, without
/// saving. Returns the new reference's id. Shared by [`cite`] and
/// [`handoff`], so a hand-off's reference is held to exactly the rules a
/// citation is.
fn attach(corpus: &Corpus, node: &mut Node, args: &Citation, by: Option<String>) -> Result<String> {
    // Case is normalised the way tags are: `Paper` is a spelling of `paper`,
    // not a new kind. Anything still outside the vocabulary is refused.
    let kind = normalize_reference_kind(&args.kind);
    let uri = args
        .uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .map(str::to_owned);
    if uri.is_none() && kind != "discussion" {
        return Err(Error::UriRequired { kind });
    }
    if !is_reference_kind(&kind) {
        return Err(Error::UnknownReferenceKind(args.kind.clone()));
    }
    // An Observatory record id is checked for its shape at the point of
    // action, because a path or a slug stored here would never resolve and
    // the mistake is obvious now and cryptic later.
    let uri = match (kind.as_str(), uri.as_deref()) {
        (OBSERVATORY, Some(record)) => Some(observatory_record(record)?),
        _ => uri,
    };
    // Rule 8, before resolving: an absolute path or a `file:` URI may well
    // exist on this machine, and that is the trap. The corpus is synced, so
    // on every other machine it names nothing, and it leaks this one's
    // layout into the corpus. Refused whether or not it resolves here.
    if let Some(uri) = uri.as_deref().filter(|uri| is_absolute_local(uri)) {
        return Err(Error::AbsoluteUri(uri.to_string()));
    }
    // Rule 8 at the point of action: a local path that does not resolve is a
    // citation to nothing, and refusing it here is cheaper than finding it
    // in `check` after the context of why it was attached has gone.
    if let Some(uri) = uri
        .as_deref()
        .filter(|_| kind != OBSERVATORY)
        .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
    {
        return Err(Error::UnresolvedUri {
            uri: uri.to_string(),
            from: corpus.root().join("nodes"),
        });
    }
    let reference = node.next_reference_id();
    node.references.push(Reference {
        id: reference.clone(),
        kind,
        uri,
        title: args.title.clone(),
        note: args.note.clone(),
        added: stamp::today(),
        by,
        origin: args.origin.clone(),
    });
    Ok(reference)
}

/// An Observatory record id as it is stored: trimmed and upper-cased, or
/// [`Error::InvalidObservatoryId`] when it is not one.
fn observatory_record(raw: &str) -> Result<String> {
    let record = raw.trim().to_ascii_uppercase();
    if !is_observatory_id(&record) {
        return Err(Error::InvalidObservatoryId(record));
    }
    Ok(record)
}

/// Hand a node off to an Observatory record: cite the record and close the
/// node as [`Status::Abandoned`] with `closed.why` reading
/// `handed off to <record>`, in one save.
///
/// What was two verbs (`cite --kind observatory`, then `status abandoned`)
/// is one write here, so no reader, and no failure between the two, ever
/// sees a node that is cited but open or closed but uncited.
///
/// `observatory` is the root this machine resolves records under, as
/// [`Corpus::observatory_root`] reports it; the caller reads it, as it does
/// for [`crate::graph::NodeView::with_observatory`], and so can tell the
/// human where the record is. With a root, the record must resolve under
/// it, or the hand-off is refused as [`Error::UnresolvedObservatoryRecord`]:
/// closing a node for a record that is not there would point its lineage at
/// nothing. With none, the id is accepted on its shape alone, as `cite`
/// accepts it, and `check` keeps warning until this machine has a root.
///
/// Refuses, before anything is written, an unknown node, one that is
/// already closed ([`Error::AlreadyClosed`]: refuted is a verdict, and an
/// abandoned node's reason would be replaced), and a record id of the wrong
/// shape.
pub fn handoff(
    corpus: &Corpus,
    id: &str,
    args: &Handoff,
    observatory: Option<&Path>,
) -> Result<HandedOff> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(args.by.as_deref()))?;
    let _lock = corpus.lock()?;
    let by = model::author(args.by.as_deref())?;
    let mut doc = corpus.load(id)?;
    let from = doc.node.status;
    if !from.is_open() {
        return Err(Error::AlreadyClosed {
            id: id.to_string(),
            status: from,
        });
    }
    let record = observatory_record(&args.record)?;
    let resolved_record = observatory
        .map(|root| resolve_observatory(root, &record))
        .transpose()?
        .flatten();
    if let Some(root) = observatory
        && resolved_record.is_none()
    {
        return Err(Error::UnresolvedObservatoryRecord {
            record,
            root: root.to_path_buf(),
        });
    }
    let reference = attach(
        corpus,
        &mut doc.node,
        &Citation {
            uri: Some(record.clone()),
            kind: OBSERVATORY.to_string(),
            title: None,
            note: args.note.clone(),
            by: None,
            origin: args.origin.clone(),
        },
        by,
    )?;
    // Abandoned asks for no kill condition, and an open node has no verdict
    // to protect, so this is the move `set_status` would allow.
    doc.node.status = Status::Abandoned;
    doc.node.closed = Some(Closed {
        why: model::handoff_why(&record),
        at: stamp::today(),
    });
    corpus.save(&mut doc)?;
    let observatory = ObservatoryLink {
        reference: reference.clone(),
        record: record.clone(),
        path: resolved_record,
    };
    Ok(HandedOff {
        doc,
        reference,
        record,
        from,
        observatory,
    })
}
