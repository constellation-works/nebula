//! Rule 9, which reports an Observatory record that does not resolve.
//!
//! Where a record resolves is the `reference` leaf's business
//! ([`crate::reference::resolve_observatory`]); this file only judges the
//! outcome for a reference, against the machine's Observatory setting.

use crate::config::ObservatoryRoot;
use crate::error::Error;
use crate::model::Reference;
use crate::reference::resolve_observatory;
use crate::store::Corpus;

use super::{ObservatoryState, Report, Rule, Severity};

pub(super) fn checked_observatory_setting(
    corpus: &Corpus,
    r: &mut Report,
) -> Option<ObservatoryRoot> {
    match corpus.observatory_root() {
        Ok(setting) => Some(setting),
        Err(error) => {
            r.push(
                Severity::Error,
                Rule::ObservatoryReference,
                None,
                format!("could not read the observatory root setting: {error}"),
            );
            None
        }
    }
}

pub(super) fn check_observatory_reference(
    reference: &Reference,
    node: Option<&str>,
    observatory: ObservatoryState<'_>,
    r: &mut Report,
) {
    let Some(record) = reference
        .uri
        .as_deref()
        .filter(|record| !record.trim().is_empty())
    else {
        return;
    };
    let id = &reference.id;
    match observatory {
        ObservatoryState::Broken => r.push(
            Severity::Warn,
            Rule::ObservatoryReference,
            node,
            format!("reference `{id}` names Observatory record `{record}`, but its location could not be checked because the observatory root setting is broken"),
        ),
        ObservatoryState::Unset => r.push(
            Severity::Warn,
            Rule::ObservatoryReference,
            node,
            format!("reference `{id}` names Observatory record `{record}` but no observatory root is set (`neb config observatory-root <DIR>` or $OBSERVATORY_ROOT)"),
        ),
        ObservatoryState::Root(root) => match resolve_observatory(root, record) {
            Ok(None) => r.push(
                Severity::Warn,
                Rule::ObservatoryReference,
                node,
                format!("reference `{id}` names Observatory record `{record}`, which does not resolve under {}", root.display()),
            ),
            Ok(Some(_)) => {},
            Err(error) => {
                let directory_error =
                    matches!(&error, Error::IoAt { path, .. } if path.parent() == Some(root));
                let cause = if directory_error {
                    "could not read its directory"
                } else {
                    "could not inspect its record target"
                };
                r.push(
                    Severity::Error,
                    Rule::ObservatoryReference,
                    node,
                    format!("reference `{id}` names Observatory record `{record}`, but {cause}: {error}"),
                );
            }
        },
    }
}
