//! Whether two names in `nodes/` are one directory entry: a symlink that is
//! an alias of a node, a respelling the filesystem stores differently, and
//! hard links. Asked by identity (device and inode), never by resolving or
//! canonicalizing a path.

use super::list_directory;
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// The node a symlink in `nodes/` is an alias of: the id of the regular node
/// file beside it that it resolves to. `None` for a link that leads anywhere
/// else, or nowhere.
///
/// Asked with `stat`, which follows the link but opens nothing, so a device
/// or a FIFO at its far end answers without being read; the node's own file
/// is found by identity among the node files beside the link. The id is that
/// file's name, which a node that loads at all agrees with.
#[cfg(unix)]
pub(super) fn node_behind(link: &Path) -> Result<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    let Some(target) = std::fs::metadata(link)
        .ok()
        .filter(std::fs::Metadata::is_file)
    else {
        return Ok(None);
    };
    let Some(dir) = link.parent() else {
        return Ok(None);
    };
    let found = list_directory(dir)?
        .into_strict()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path != link && is_node_file_name(path))
        .find(|path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.is_file()
                    && metadata.dev() == target.dev()
                    && metadata.ino() == target.ino()
            })
        })
        .and_then(|node| Some(node.file_stem()?.to_str()?.to_owned()));
    Ok(found)
}

/// Elsewhere a symlink is refused as one, whatever it names.
#[cfg(not(unix))]
pub(super) fn node_behind(_link: &Path) -> Result<Option<String>> {
    Ok(None)
}

/// Whether a name in `nodes/` is one a scan reads as a node.
pub(super) fn is_node_file_name(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "md")
}

/// Whether two paths name the same directory entry.
///
/// Identity, because that is the only reason two different spellings may name
/// one node: a volume may store a file name in a different Unicode
/// normalization than the id it was written from, and the filesystem is the
/// one that knows the two are one entry. A device and inode pair is that
/// answer, read without following the last component, so a symlink is its
/// own entry rather than the file it points at. Both paths are otherwise used
/// as given — nothing is resolved or canonicalized — so a corpus reached
/// through a symlinked root, which is every corpus under a macOS temporary
/// directory, answers this the same way a corpus reached directly does.
///
/// A hard link is the one case the pair cannot tell apart from a respelling,
/// which is why [`hard_links_beside`] is asked first.
///
/// A path that cannot be read is not the same entry as anything, including
/// itself: the caller is deciding whether to trust a mismatched name, and an
/// unanswered question is not a yes.
#[cfg(unix)]
pub(super) fn is_same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Elsewhere the question does not arise. A Windows volume compares file
/// names case-insensitively but not across Unicode normalizations, so a name
/// that differs from the id it stores differs for some other reason, and the
/// byte comparison the caller already made is the whole answer.
#[cfg(not(unix))]
pub(super) fn is_same_entry(_a: &Path, _b: &Path) -> bool {
    false
}

/// The node file names in `path`'s directory that are hard links to it,
/// itself included, sorted.
///
/// Empty for the ordinary file with one link, which is answered from its own
/// metadata without listing the directory. A link count above one sends the
/// question to the directory, because only links a scan would read as nodes
/// are aliases: a backup hard-linked from outside the corpus is not a second
/// name for the node, and one Unicode respelling of a name is one entry
/// however it is typed.
#[cfg(unix)]
pub(super) fn hard_links_beside(path: &Path) -> Result<Vec<PathBuf>> {
    use std::os::unix::fs::MetadataExt;
    let Ok(file) = std::fs::symlink_metadata(path) else {
        return Ok(Vec::new());
    };
    if file.nlink() <= 1 {
        return Ok(Vec::new());
    }
    let Some(dir) = path.parent() else {
        return Ok(Vec::new());
    };
    let entries = std::fs::read_dir(dir).map_err(|error| Error::io_at("reading", dir, error))?;
    let mut links = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io_at("reading", dir, error))?;
        let link = entry.path();
        if !is_node_file_name(&link) {
            continue;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(&link)
            && metadata.dev() == file.dev()
            && metadata.ino() == file.ino()
        {
            links.push(link);
        }
    }
    links.sort();
    Ok(links)
}

/// Elsewhere a hard link is not an alias this module can see, and the byte
/// comparison stands alone.
#[cfg(not(unix))]
pub(super) fn hard_links_beside(_path: &Path) -> Result<Vec<PathBuf>> {
    Ok(Vec::new())
}
