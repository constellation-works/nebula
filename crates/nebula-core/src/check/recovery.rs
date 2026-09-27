//! Rule 17: writes that did not finish, reported with the command that
//! clears each one.

use crate::config;
use crate::error::{Error, Result};
use crate::pending::{self, PENDING_FILE, PendingWrite};
use crate::store::Corpus;
use crate::store::GITIGNORE_FILE;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{Report, Rule, Severity};

/// 17. A write that did not finish is reported, never repaired here: the
///     temporary file a killed write left beside the file it was replacing,
///     and a pending-write record the next writer has not yet settled.
///
/// A temporary file is read by nothing and committed by nothing, but it is
/// not nebula's to delete: its name says a nebula write made it, not that
/// the bytes in it are nothing anybody wants (STD-03 §R29). So each one is
/// named, with the command that removes it, for a person to run. The
/// directories nebula writes into are the ones looked in: `nodes/`,
/// `inbox/`, and at the root the temporaries of the files kept there.
///
/// A pending record is a warning while it can be read, since the next write
/// settles it, and an error when it cannot, since every write refuses until
/// someone deals with it.
pub(super) fn interrupted_writes(corpus: &Corpus, r: &mut Report) -> Result<()> {
    let root = corpus.root();
    // Relative to the root, in the order they are reported.
    let mut debris: Vec<PathBuf> = Vec::new();
    for dir in ["nodes", "inbox"] {
        let path = root.join(dir);
        match temporaries(&path) {
            Ok(found) => debris.extend(found.into_iter().map(|name| Path::new(dir).join(name))),
            Err(error) => r.push(
                Severity::Error,
                Rule::InterruptedWrite,
                None,
                format!(
                    "could not inspect temporary files in {}: {error}",
                    path.display()
                ),
            ),
        }
    }
    let beside_root_files = |name: &OsString| {
        let name = name.to_string_lossy();
        [config::FILE, GITIGNORE_FILE, PENDING_FILE]
            .iter()
            .any(|file| name.starts_with(&format!("{file}.")))
    };
    match temporaries(root) {
        Ok(found) => debris.extend(
            found
                .into_iter()
                .filter(beside_root_files)
                .map(PathBuf::from),
        ),
        Err(error) => r.push(
            Severity::Error,
            Rule::InterruptedWrite,
            None,
            format!(
                "could not inspect temporary files in {}: {error}",
                root.display()
            ),
        ),
    }
    for relative in debris {
        r.push(
            Severity::Warn,
            Rule::InterruptedWrite,
            None,
            format!(
                "`{}` is a temporary file left by a write that did not finish; nothing reads \
                 or commits it, and nebula never deletes it. Once it holds nothing you need, \
                 remove it: rm {}",
                relative.display(),
                shell_quote(&root.join(&relative).display().to_string())
            ),
        );
    }

    let record = pending::pending_path(root);
    match pending::read(root) {
        Ok(None) => {}
        Ok(Some(PendingWrite::Promote { entry, node, .. })) => {
            let outcome = if corpus.node_path(&node)?.exists() {
                format!("`{node}` was written, so the next write strikes the entry `-> {node}`")
            } else {
                format!("`{node}` was never written, so the next write leaves the entry waiting")
            };
            r.push(
                Severity::Warn,
                Rule::InterruptedWrite,
                None,
                format!(
                    "a promotion of inbox entry `{entry}` did not finish, as {} records; \
                     {outcome}",
                    record.display()
                ),
            );
        }
        Err(Error::PendingWriteUnreadable { path, reason }) => r.push(
            Severity::Error,
            Rule::InterruptedWrite,
            None,
            format!(
                "{} records an unfinished write that this build cannot read ({reason}); every \
                 write refuses until it has been read by hand and removed: rm {}",
                path.display(),
                shell_quote(&path.display().to_string())
            ),
        ),
        Err(error) => return Err(error),
    }
    Ok(())
}

/// The names in `dir` that end `.tmp`, sorted. None when `dir` is not there.
fn temporaries(dir: &Path) -> Result<Vec<OsString>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io_at("reading", dir, error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let name = entry
            .map_err(|error| Error::io_at("reading", dir, error))?
            .file_name();
        if name.to_string_lossy().ends_with(".tmp") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// `text` as one POSIX shell word: single-quoted, with each `'` closed,
/// escaped and reopened, so a suggested command runs as shown whatever the
/// path holds.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
