//! Where a verb's text comes from when it is not on the command line: the
//! editor `neb edit` opens, and standard input for the `-` spelling.

use super::failure::{EditorError, Failure};
use nebula_core::{Corpus, Error, ops};
use std::process::Command as ProcessCommand;

/// Resolve a `--body` value, using stdin only for the explicit `-` spelling.
pub(super) fn body_value(value: Option<String>) -> std::result::Result<String, Failure> {
    match value.as_deref() {
        None => Ok(String::new()),
        Some("-") => read_stdin("a body", ops::BODY_INPUT_LIMIT),
        Some(_) => Ok(value.unwrap_or_default()),
    }
}

/// The words of a `capture`, or standard input when they are a lone `-`.
///
/// Only that exact spelling reads stdin, as with `--body -`: a dash among
/// other words is part of the thought. The text is passed on as it came;
/// the core joins its lines, so a pipe and a paste store the same line.
/// Arguments and stdin share the same byte limit before any corpus write.
pub(super) fn capture_text(words: &[String]) -> std::result::Result<String, Failure> {
    match words {
        [dash] if dash == "-" => read_stdin("a capture", ops::CAPTURE_INPUT_LIMIT),
        _ => Ok(ops::read_bounded(
            words.join(" ").as_bytes(),
            "a capture",
            ops::CAPTURE_INPUT_LIMIT,
        )?),
    }
}

/// All of standard input, as text, refused past `limit` bytes.
///
/// Every caller reads it before opening the corpus for writing, so a pipe
/// that is slow or enormous holds no lock while it drains, and one over the
/// ceiling is refused with nothing written (STD-03 §R22).
fn read_stdin(what: &'static str, limit: usize) -> std::result::Result<String, Failure> {
    Ok(ops::read_bounded(std::io::stdin().lock(), what, limit)?)
}

/// The body as the editor left it, and the temporary file it is still in.
///
/// The file is deleted when this drops, so a refusal keeps the text first
/// with [`keep_refused`].
pub(super) struct Edited {
    pub(super) text: String,
    file: tempfile::NamedTempFile,
}

/// Let the configured editor rewrite body prose in a temporary file.
///
/// No lock is held here: a person may type for as long as they like, and
/// every other writer carries on meanwhile (STD-03 §R1). The caller saves
/// the result with [`verb::edit`], which checks it.
pub(super) fn edit_body(body: &str) -> std::result::Result<Edited, Failure> {
    use std::io::Write;

    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .ok_or(EditorError::NotConfigured)?;
    let editor_name = editor.to_string_lossy().into_owned();
    let mut words = shlex::split(&editor_name)
        .filter(|words| !words.is_empty())
        .ok_or_else(|| EditorError::InvalidCommand(editor_name.clone()))?;
    let program = words.remove(0);
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|e| Error::io_at("creating a temporary file in", std::env::temp_dir(), e))?;
    file.write_all(body.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| Error::io_at("writing", file.path(), e))?;
    // The one child not run like git: it stays in the terminal's foreground
    // group with no deadline, because a person drives it (STD-03@2 §R11,
    // recorded in docs/design/lineage-graph/4_decisions.md).
    let status = ProcessCommand::new(program)
        .args(words)
        .arg(file.path())
        .status()
        .map_err(|source| EditorError::Start {
            editor: editor_name.clone(),
            source,
        })?;
    if !status.success() {
        return Err(keep_editor_file(
            EditorError::Unsuccessful(editor_name).into(),
            file,
        ));
    }
    let text = match std::fs::read_to_string(file.path()) {
        Ok(text) => text,
        Err(error) => {
            let refused = Error::io_at("reading", file.path(), error).into();
            return Err(keep_editor_file(refused, file));
        }
    };
    Ok(Edited { text, file })
}

/// Preserve an early refusal without requiring readable UTF-8. Disable
/// deletion before attempting recovery, including when recovery itself fails.
/// Reopen by path: editors may replace the inode the original handle names.
fn keep_editor_file(refused: Failure, file: tempfile::NamedTempFile) -> Failure {
    let Failure(refusal) = refused;
    let mut path = file.into_temp_path();
    path.disable_cleanup(true);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Failure(refusal.not_kept(&Error::io_at("reading", &path, error).to_string()));
        }
        Err(error) => {
            return Failure(refusal.kept_at(&path).hinted(Some(format!(
                "The editor file was left in place; {}",
                Error::io_at("reading", &path, error)
            ))));
        }
    };
    // Even an editor that replaced the file with a more permissive one
    // leaves an owner-only, durable recovery file. An atomic write failure
    // leaves the existing file in place; the original refusal stays primary.
    let hint = nebula_core::fs::write_private_atomic(&path, bytes)
        .err()
        .map(|error| format!("Could not make the editor file durable and owner-only: {error}"));
    Failure(refusal.kept_at(&path).hinted(hint))
}

/// `refused`, after keeping the text the person typed (STD-03 §R30).
///
/// Everything after the editor exits can refuse — the notes check, the wait
/// for the lock, a body another writer changed, the save itself — and the
/// text is then in memory and in a temporary file about to be deleted, and
/// nowhere else. So before the refusal is returned it is kept as a new
/// owner-only file under [`Corpus::kept_edits_dir`], outside the corpus, and
/// the refusal names it. Should that fail, the temporary file is kept where it
/// is instead. Only when both fail is the text lost, and the refusal says so
/// and why.
pub(super) fn keep_refused(
    refused: Failure,
    corpus: &Corpus,
    node: &str,
    edited: Edited,
) -> Failure {
    let Failure(refusal) = refused;
    Failure(
        match Corpus::keep_edit(corpus.locations(), node, &edited.text) {
            Ok(path) => refusal.kept_at(&path),
            Err(not_kept) => match edited.file.keep() {
                Ok((_, path)) => refusal.kept_at(&path),
                Err(also) => refusal.not_kept(&format!("{not_kept}; {}", also.error)),
            },
        },
    )
}
