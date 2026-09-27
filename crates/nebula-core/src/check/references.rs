//! What a reference's kind and URI may be: the kind vocabulary, and whether a
//! URI is a path under `nodes/`, a path on one machine, or neither.

use crate::store::Corpus;
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

/// Where a local reference URI lands: relative to the `nodes/` directory,
/// used as given and never canonicalized (macOS temp dirs sit under a
/// symlink, and resolving would pass on one platform and fail on the other).
pub(crate) fn resolve_local(corpus: &Corpus, uri: &str) -> PathBuf {
    corpus.root().join("nodes").join(Path::new(uri))
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
