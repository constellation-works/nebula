//! The reference vocabulary: what a reference's kind and URI may be, and
//! where an Observatory record resolves on this machine.
//!
//! A leaf below `model`, `graph`, `config`, `ops`, `error` and `check`:
//! every one of those needs this vocabulary, and the checker is only one
//! consumer of it, so none of them reaches up into `check` for it. This
//! module knows no corpus and no graph; `resolve_local` takes a root and
//! `resolve_observatory` an Observatory root, both as plain paths.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// Whether a URI is a path to resolve against `nodes/`, as opposed to a URL,
/// a wikilink, or a scheme-prefixed handle such as `doi:` or `orbit:`.
pub(crate) fn is_local_path(uri: &str) -> bool {
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

/// Whether a URI names a place on one machine's filesystem rather than a
/// path relative to `nodes/`: a `file:` URI, a path from the root (`/etc/x`,
/// `\\server\share\x`), or a drive-letter path (`C:/x`, `C:\x`).
///
/// Judged as written and alike on every platform, never through
/// [`Path::is_absolute`], which answers differently on each: the corpus is
/// synced between machines, so a path that is absolute on any one of them
/// is machine layout on all of them.
pub(crate) fn is_absolute_local(uri: &str) -> bool {
    let file_uri = uri
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"));
    let rooted = uri.starts_with(['/', '\\']);
    let drive = matches!(uri.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    file_uri || rooted || drive
}

/// Where a local reference URI lands: relative to the `nodes/` directory
/// under `root`, used as given and never canonicalized (macOS temp dirs sit
/// under a symlink, and resolving would pass on one platform and fail on
/// the other).
pub(crate) fn resolve_local(root: &Path, uri: &str) -> PathBuf {
    root.join("nodes").join(Path::new(uri))
}

/// The reference kind whose `uri` is a bare Observatory record id rather
/// than a location: `Q002`, `H007`, `T003`, `R012`. Where the record is on
/// this machine is a machine setting ([`crate::Corpus::observatory_root`]),
/// so the reference itself carries nothing machine-specific.
pub const OBSERVATORY: &str = "observatory";

/// The closed vocabulary accepted for new references. The model deliberately
/// keeps `kind` as a string so older, hand-edited corpora still load; `check`
/// reports values outside this list instead of turning them into parse errors.
pub const REFERENCE_KINDS: [&str; 10] = [
    "paper",
    "study",
    "article",
    "note",
    "discussion",
    "book",
    "dataset",
    "thread",
    OBSERVATORY,
    "other",
];

/// Whether a reference kind belongs to the vocabulary accepted for new writes.
/// Exact: a stored kind is judged as written. Writes normalise first, with
/// [`normalize_reference_kind`].
pub(crate) fn is_reference_kind(kind: &str) -> bool {
    REFERENCE_KINDS.contains(&kind)
}

/// A reference kind as a write stores it: trimmed and lowercased, the way
/// tags are, so `Paper` is `paper` rather than a refusal. Whether the result
/// is in the vocabulary is still [`is_reference_kind`]'s question.
pub fn normalize_reference_kind(kind: &str) -> String {
    kind.trim().to_lowercase()
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
/// shape, the directory is absent, or no matching entry is an accessible
/// record of the expected type. An unreadable directory or target is an I/O
/// error naming the path. A dangling symlink is not a record.
///
/// Matched by prefix in a listing, then probed using the same lexical path:
/// nothing is canonicalized. Ties (two records claiming one id) go to the
/// first accessible record in name order, which `check` in Observatory is
/// the place to catch.
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
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| Error::io_at("reading directory entry in", &dir, error))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.retain(|name| {
        name.strip_prefix(id)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
    });
    names.sort();
    for name in names {
        let path = dir.join(name);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io_at("inspecting", &path, error)),
        };
        // A record's existence and accessibility are part of the integrity
        // check used by handoff. `metadata` follows symlinks, but cannot by
        // itself establish that a file or directory can be opened.
        let accessible = if record_dir == "research" && metadata.is_dir() {
            std::fs::read_dir(&path).map(|_| ())
        } else if record_dir != "research" && metadata.is_file() {
            std::fs::File::open(&path).map(|_| ())
        } else {
            continue;
        };
        match accessible {
            Ok(()) => return Ok(Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io_at("opening", &path, error)),
        }
    }
    Ok(None)
}
