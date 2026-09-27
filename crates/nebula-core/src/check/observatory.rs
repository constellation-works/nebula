//! Where an Observatory record resolves on this machine, and rule 9, which
//! reports a record that does not.

use crate::config::ObservatoryRoot;
use crate::error::{Error, Result};
use crate::model::Reference;
use crate::store::Corpus;
use std::path::{Path, PathBuf};

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

/// Whether `id` has the shape of an Observatory record id: one of `Q`, `H`,
/// `T`, `R` followed by digits. The shape is the whole contract; how many
/// digits Observatory uses is its business.
pub(crate) fn is_observatory_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| observatory_dir(c).is_some())
        && !chars.as_str().is_empty()
        && chars.all(|c| c.is_ascii_digit())
}

/// The Observatory directory a record id's letter files it under. Research
/// layout v2: questions, hypotheses and theories are files, research
/// records are directories.
fn observatory_dir(letter: char) -> Option<&'static str> {
    match letter {
        'Q' => Some("questions"),
        'H' => Some("hypotheses"),
        'T' => Some("theories"),
        'R' => Some("research"),
        _ => None,
    }
}

/// Where an Observatory record is under `root`: the entry of the id's
/// directory whose name is the id, or the id followed by `-` or `.`, so
/// `Q002` finds `questions/Q002-is-proper-time-a-count.md` without the
/// reference having to know the slug. `Ok(None)` when the id has the wrong
/// shape, the directory is absent, or nothing there starts with it; an
/// unreadable directory or entry is an I/O error naming the directory.
///
/// Matched by prefix in a listing rather than by resolving a path, and used
/// as given: nothing is canonicalized. Ties (two records claiming one id)
/// go to the first in name order, which `check` in Observatory is the place
/// to catch.
pub(crate) fn resolve_observatory(root: &Path, id: &str) -> Result<Option<PathBuf>> {
    if !is_observatory_id(id) {
        return Ok(None);
    }
    let Some(record_dir) = id.chars().next().and_then(observatory_dir) else {
        return Ok(None);
    };
    let dir = root.join(record_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io_at("reading", &dir, error)),
    };
    let mut names: Vec<String> = crate::store::collect_directory_entries(&dir, entries)
        .into_strict()?
        .into_iter()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
        })
        .collect();
    names.sort();
    Ok(names.into_iter().next().map(|name| dir.join(name)))
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
            Err(error) => r.push(
                Severity::Error,
                Rule::ObservatoryReference,
                node,
                format!("reference `{id}` names Observatory record `{record}`, but could not read its directory: {error}"),
            ),
        },
    }
}
