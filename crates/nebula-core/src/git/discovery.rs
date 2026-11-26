//! Filesystem evidence for Git repository discovery.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "the Git adapter resolves discovery paths as Git does and probes external .git pointers"
)]

use super::GitAt;
use crate::fs_impl::{Links, metadata};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Whether git, run at `root`, has a repository to find: a `.git` directory,
/// or a `.git` file naming one (`gitdir: …`), at the root or in a directory
/// above it, stopping below the nearest of the ceilings `at` carries, as git
/// does.
///
/// Decided from the file system and never from git's answer, so git failing
/// inside a repository — a corrupt `HEAD`, a `safe.directory` refusal, git
/// not starting — is never mistaken for there being no repository at all
/// (STD-02 §R29). `GIT_DIR` and the rest of [`super::REPOSITORY_ENV`] are not
/// consulted: [`super::run_git`] removes them, and gives git these same ceilings,
/// so git discovers the repository this same way. Where this finds a `.git`
/// that git does not use — past a mount point, say — a caller that asks git
/// gets git's own reason as an error, which is the safe side to be wrong on.
pub(crate) fn repository_expected(at: GitAt<'_>) -> bool {
    let root = at.root;
    // git walks up from the directory it was moved into, which it knows by
    // its resolved name; the ceilings are resolved the same way below.
    let start = std::fs::canonicalize(root)
        .or_else(|_| std::path::absolute(root))
        .unwrap_or_else(|_| root.to_path_buf());
    let ceilings = ceiling_directories(at.ceilings);
    discovers(&start, &ceilings)
}

/// The directories named in a `GIT_CEILING_DIRECTORIES` value, read as git
/// reads it: relative entries are ignored; entries are resolved, and one that
/// cannot be resolved is dropped, until an empty entry, after which they are
/// taken as written.
pub(crate) fn ceiling_directories(value: Option<&OsStr>) -> Vec<PathBuf> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut resolve = true;
    let mut ceilings = Vec::new();
    for entry in split_ceilings(value) {
        if entry.as_os_str().is_empty() {
            resolve = false;
            continue;
        }
        if !entry.is_absolute() {
            continue;
        }
        if !resolve {
            ceilings.push(entry);
        } else if let Ok(resolved) = std::fs::canonicalize(&entry) {
            ceilings.push(resolved);
        }
    }
    ceilings
}

/// The entries of a `GIT_CEILING_DIRECTORIES` value: separated by `:`, as
/// git separates them on the platforms it is run on here.
#[cfg(unix)]
fn split_ceilings(value: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    use std::os::unix::ffi::OsStrExt;
    value
        .as_bytes()
        .split(|byte| *byte == b':')
        .map(|entry| PathBuf::from(OsStr::from_bytes(entry)))
}

/// Git is never run off Unix (see [`super::run_git`]), so there is nothing to stop.
#[cfg(not(unix))]
fn split_ceilings(_: &OsStr) -> impl Iterator<Item = PathBuf> {
    std::iter::empty()
}

/// Whether a repository is found from `start` upwards, never looking in a
/// ceiling above `start` or anywhere past it.
pub(crate) fn discovers(start: &Path, ceilings: &[PathBuf]) -> bool {
    for dir in start.ancestors() {
        if dir != start && ceilings.iter().any(|ceiling| ceiling == dir) {
            return false;
        }
        if names_a_repository(&dir.join(".git")) {
            return true;
        }
    }
    false
}

/// Whether `dot_git` is a repository directory, or a file pointing at one.
pub(crate) fn names_a_repository(dot_git: &Path) -> bool {
    use std::io::Read;
    match metadata(dot_git, Links::Follow) {
        Ok(meta) if meta.is_dir() => true,
        Ok(meta) if meta.is_file() => {
            let mut head = [0; 7];
            std::fs::File::open(dot_git)
                .and_then(|mut file| file.read_exact(&mut head))
                .is_ok_and(|()| &head == b"gitdir:")
        }
        _ => false,
    }
}
