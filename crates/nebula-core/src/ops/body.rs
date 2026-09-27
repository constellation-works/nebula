//! A node's prose: dated notes, and the body replaced whole under the
//! append-only rule for notes.

use crate::error::{Error, Result};
use crate::model::{self, Doc};
use crate::stamp;
use crate::store::Corpus;

/// Append a dated paragraph of reasoning to a node body.
///
/// Creates a `## Notes` section at the end of the body if needed, then
/// appends `- YYYY-MM-DD: <text>`. Earlier body text, status, edges and tags
/// are left as they are. `updated` is stamped by [`Corpus::save`].
///
/// `by` is whoever wrote the paragraph; `None` is the human, whose line
/// carries no attribution.
pub fn note(corpus: &Corpus, id: &str, text: &str, by: Option<&str>) -> Result<Doc> {
    corpus
        .locations()
        .write_gate(crate::locations::WriteIntent::Authored(by))?;
    let text = text
        .trim()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Err(Error::EmptyNote);
    }
    let _lock = corpus.lock()?;
    let by = model::author(by)?;
    let mut doc = corpus.load(id)?;
    doc.body = model::append_note(&doc.body, &stamp::today(), &text, by.as_deref());
    corpus.save(&mut doc)?;
    Ok(doc)
}

/// Replace a node's prose body, leaving its structured fields alone.
///
/// `updated` is stamped by [`Corpus::save`]. The `## Notes` sections are
/// append-only, so a body that removes, reorders or rewrites one is refused
/// with [`Error::NotesChanged`] before anything is written; [`note`] is
/// their append path.
///
/// Returns `None`, having written nothing and left `updated` alone, when the
/// body is already `body` as [`body_unchanged`] compares them: a write that
/// changes nothing is not a write (STD-01 §R30).
pub fn set_body(corpus: &Corpus, id: &str, body: &str) -> Result<Option<Doc>> {
    replace_body(corpus, id, None, body)
}

/// [`set_body`], only if the node's body is still `expected`: the
/// compare-and-set for a body edited without the lock.
///
/// `neb edit` loads a node, hands its body to `$EDITOR` with no lock held
/// (STD-03 §R1: a person typing is the slowest work there is), and saves
/// here. Under the lock the node is loaded again, and a body that is no
/// longer `expected` was changed by another writer meanwhile — a `note`,
/// say — which saving would erase, so it refuses with
/// [`Error::EditConflict`] before writing anything. Only the body is
/// compared: a tag, status or edge changed meanwhile is kept, and the new
/// body lands on top of it.
///
/// A `body` that is still `expected` asks for no change, so it returns
/// `None` and writes nothing, whatever another writer did meanwhile.
pub fn set_body_if(corpus: &Corpus, id: &str, expected: &str, body: &str) -> Result<Option<Doc>> {
    replace_body(corpus, id, Some(expected), body)
}

/// Whether `edited` would store as the body `before` already is: bodies are
/// stored trimmed, so only the text between the outer whitespace counts.
#[must_use]
pub fn body_unchanged(before: &str, edited: &str) -> bool {
    before.trim() == edited.trim()
}

/// The append-only rule for notes: refuse `after` as [`Error::NotesChanged`]
/// unless every `## Notes` section of `before` is still there, in order and
/// unchanged.
///
/// A body can hold more than one `## Notes` section, because [`note`] opens a
/// fresh one rather than reach back into a section some other prose already
/// closed. Every section is checked, not just the last: an edit that rewrote
/// an earlier dated note while leaving the final section alone would
/// otherwise overwrite reasoning that was supposed to be append-only. Prose
/// outside those sections stays editable, which is what a body edit is for.
pub fn refuse_rewritten_notes(before: &str, after: &str) -> Result<()> {
    let before_sections = model::notes_sections(before);
    if before_sections.is_empty() {
        return Ok(());
    }
    let after_sections = model::notes_sections(after);
    if before_sections.len() != after_sections.len()
        || before_sections
            .iter()
            .zip(&after_sections)
            .any(|(before, after)| before.trim_end() != after.trim_end())
    {
        return Err(Error::NotesChanged);
    }
    Ok(())
}

fn replace_body(
    corpus: &Corpus,
    id: &str,
    expected: Option<&str>,
    body: &str,
) -> Result<Option<Doc>> {
    let _lock = corpus.lock()?;
    let mut doc = corpus.load(id)?;
    if body_unchanged(expected.unwrap_or(&doc.body), body) {
        return Ok(None);
    }
    if expected.is_some_and(|expected| doc.body != expected) {
        return Err(Error::EditConflict(id.to_string()));
    }
    refuse_rewritten_notes(&doc.body, body)?;
    doc.body = body.trim().to_string();
    corpus.save(&mut doc)?;
    Ok(Some(doc))
}
