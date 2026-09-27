//! Node files: where one lives, reading one or all of them, and writing one.
//!
//! A node file's name and the id it stores are one fact, and every read
//! refuses a file that says otherwise, so no later write can land on a
//! different node than the one that was read.

use super::links::{hard_links_beside, is_node_file_name, is_same_entry, node_behind};
use super::{Corpus, Scan, UnreadableNode, list_directory, refuse_nodes_symlink};
use crate::error::{Error, Result};
use crate::fs_impl::{EntryKind, regular_file_at};
use crate::id::is_path_safe_id;
use crate::model::{self, Doc};
use crate::stamp::today;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

impl Corpus {
    /// Path of a node file, whether or not it exists.
    ///
    /// Public so a consumer that hands a node to something outside the
    /// corpus (the desktop's "open in editor") asks for the path rather than
    /// re-deriving the layout. Fallible because an id becomes a path here:
    /// one that is not [`is_path_safe_id`] is refused rather than joined, so
    /// no caller can be handed a path outside `nodes/`.
    pub fn node_path(&self, id: &str) -> Result<PathBuf> {
        Self::node_path_at(&self.root, id)
    }

    fn node_path_at(root: &Path, id: &str) -> Result<PathBuf> {
        // "Ids stay strings, checked where they become paths" (4_decisions.md, STD-02@2 §R14).
        if !is_path_safe_id(id) {
            return Err(Error::UnsafeId(id.to_string()));
        }
        refuse_nodes_symlink(root)?;
        Ok(root.join("nodes").join(format!("{id}.md")))
    }

    /// Read one node.
    ///
    /// The file's name and the id it stores are one fact, so a file that
    /// stores a different id is refused rather than read: [`Self::save`]
    /// derives its destination from the stored id, and a node loaded from
    /// one file that claims to be another would be written to that other
    /// one. Refusing here is what keeps a hand edit from turning a later
    /// verb into an overwrite.
    ///
    /// Only a regular file is a node. Whether one is there is asked of the
    /// entry itself, so a symlink to nowhere is refused as a symlink rather
    /// than reported as no node, and a symlink to anything is refused before
    /// a byte is read through it (see [`Self::read_node_file`]).
    pub fn load(&self, id: &str) -> Result<Doc> {
        let path = self.node_path(id)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NoSuchNode(id.to_string()));
            }
            Err(error) => return Err(Error::io_at("inspecting", &path, error)),
        }
        self.read_node_file(&path)
    }

    /// Write one node, stamping `updated`.
    ///
    /// The destination comes from the stored id, so the id is checked before
    /// anything is written: a `Doc` reaching here holds an id that parsed
    /// ([`crate::model`] refuses an unsafe one) and that agreed with its file
    /// ([`Self::load`]), and this is the last of the three places that has to
    /// hold for a write to land where the node already lives.
    pub(crate) fn save(&self, doc: &mut Doc) -> Result<()> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        let path = self.node_path(&doc.node.id)?;
        doc.node.updated = today();
        model::write(&path, doc)
    }

    /// Write a node that must not already exist.
    ///
    /// Judged on the entry itself: a symlink or anything else that is not a
    /// regular file is [`Error::NotRegularFile`], never replaced.
    pub(crate) fn create(&self, doc: &Doc) -> Result<()> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        let path = self.node_path(&doc.node.id)?;
        if regular_file_at(&path)? {
            return Err(Error::NodeExists(doc.node.id.clone()));
        }
        model::write(&path, doc)
    }

    /// Every node in the corpus, sorted by id.
    ///
    /// A full scan, deliberately. The corpus is small and writes are rare, so
    /// an index would be a second source of truth that could drift for no gain.
    pub fn load_all(&self) -> Result<Vec<Doc>> {
        let scan = self.scan()?;
        if scan.unreadable.is_empty() {
            Ok(scan.docs)
        } else {
            let count = scan.unreadable.len();
            let details = scan
                .unreadable
                .iter()
                .map(|entry| {
                    format!(
                        "{} [{}]: {}",
                        entry.path.display(),
                        entry.code,
                        entry.message
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(Error::UnreadableNodes { count, details })
        }
    }

    /// Read every loadable node and collect a separate entry for each bad
    /// file. Directory-entry failures retain the directory path because the
    /// OS did not provide an entry name.
    pub fn scan(&self) -> Result<Scan> {
        let dir = self.root.join("nodes");
        refuse_nodes_symlink(&self.root)?;
        let listing = match list_directory(&dir) {
            Ok(listing) => listing,
            Err(Error::IoAt { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Scan {
                    docs: Vec::new(),
                    unreadable: Vec::new(),
                });
            }
            Err(error) => {
                return Ok(Scan {
                    docs: Vec::new(),
                    unreadable: vec![UnreadableNode::from_error(dir, &error)],
                });
            }
        };
        let mut unreadable: Vec<UnreadableNode> = listing
            .errors
            .into_iter()
            .map(|error| UnreadableNode::from_error(dir.clone(), &error))
            .collect();
        let mut paths: Vec<PathBuf> = listing
            .entries
            .into_iter()
            .map(|entry| entry.path())
            .filter(|p| is_node_file_name(p))
            .collect();
        paths.sort();
        let mut docs = Vec::new();
        for p in paths {
            match self.read_node_file(&p) {
                Ok(doc) => docs.push(doc),
                Err(error) => unreadable.push(UnreadableNode::from_error(p, &error)),
            }
        }
        // The extension changes prefix ordering: `idea-two.md` sorts before
        // `idea.md`, but callers need `idea` before `idea-two`.
        docs.sort_by(|a, b| a.node.id.cmp(&b.node.id));
        Ok(Scan { docs, unreadable })
    }

    /// Read the node file at `path` and refuse it unless it is the node its
    /// name names. Both doors, [`Self::load`] and [`Self::load_all`], read
    /// through here.
    ///
    /// Only a regular file is read (STD-05 §R7). A symlink is judged before
    /// anything is opened: one that opens another node's file beside it is
    /// an alias of that node, refused as [`Error::IdMismatch`] the way a hard
    /// link is, and any other is [`Error::NotRegularFile`]. A FIFO, a device
    /// or a directory is refused by the read itself, before it can block or
    /// feed it without end.
    fn read_node_file(&self, path: &Path) -> Result<Doc> {
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(match node_behind(path)? {
                Some(id) => Error::IdMismatch {
                    path: path.to_path_buf(),
                    id,
                },
                None => Error::NotRegularFile {
                    path: path.to_path_buf(),
                    found: EntryKind::Symlink,
                },
            });
        }
        let doc = self.read_node(path)?;
        Self::require_file_agrees(&self.root, path, &doc)?;
        Ok(doc)
    }

    /// Read one node file with the current model.
    ///
    /// A node that will not parse and reads as a v1 node is named as one
    /// ([`Error::V1NodeUnderCurrentSchema`]): most likely an older `neb`
    /// stamped this corpus's config over files from before it, and the
    /// parser's unknown-field complaint alone does not lead anyone to the
    /// repair.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "all other read errors pass through unchanged"
    )]
    fn read_node(&self, path: &Path) -> Result<Doc> {
        model::read(path).map_err(|error| match error {
            Error::Yaml { .. } => crate::migrate_impl::v1_node_under_current_schema(
                &self.root,
                path,
                self.config.schema_version,
            )
            .unwrap_or(error),
            other => other,
        })
    }

    /// Refuse a node file whose name is not the id it stores.
    ///
    /// Reads and migration preflight come through here. [`Self::load`]
    /// arrives with the path the caller's id names, and a scan arrives with
    /// a path it found on disk;
    /// either way the question is the same, and answering it in one place is
    /// what keeps a file called one thing and claiming to be another from
    /// reading as the node it claims — or, through [`Self::save`], from
    /// becoming a write over that node.
    ///
    /// A node has one directory entry, and the names are compared by asking
    /// the filesystem whether they open that one entry. A volume may store a
    /// name in a different Unicode normalization than the id it was written
    /// from — `título` is two spellings of the same word — and a byte
    /// comparison would call a perfectly ordinary node a mismatch. What the
    /// fallback asks is [`is_same_entry`]: whether the path the id names *is*
    /// this entry. Nothing is canonicalized; both paths are used as given,
    /// which is what keeps a symlinked root answering the same way on either
    /// platform.
    ///
    /// Equal *text* is not that question and never was. Two distinct files
    /// hold equal text the moment one is copied over the other, so a
    /// comparison of contents let `nodes/safe.md` — a byte-for-byte copy of
    /// `nodes/victim.md` — authorize `victim` as the id `safe` had asked
    /// for, and the write that followed landed on the other node.
    ///
    /// Nor is one *file* under two entries. `nodes/safe.md` hard-linked to
    /// `nodes/victim.md` opens the same bytes, but it is a second name for
    /// the node, and neither door can keep it coherent: a scan reads the node
    /// twice, and [`write_private_atomic`] replaces the entry the id names
    /// with a new file, so a hard link keeps the old bytes under the other
    /// name and the next load refuses. So an alias is refused wherever it is
    /// met, and it is the alias that is named, since it is what has to go. A
    /// hard link is met even when the node is loaded through its own name,
    /// because that is the load whose write would split the pair.
    ///
    /// A symlink never reaches here. A write would not follow it but replace
    /// it with a regular file, leaving whatever it pointed at behind, so
    /// every door refuses it before reading through it
    /// ([`Self::read_node_file`]).
    pub(crate) fn require_file_agrees(root: &Path, path: &Path, doc: &Doc) -> Result<()> {
        let id = OsStr::new(doc.node.id.as_str());
        let refuse = |path: &Path| {
            Err(Error::IdMismatch {
                path: path.to_path_buf(),
                id: doc.node.id.clone(),
            })
        };
        let links = hard_links_beside(path)?;
        if links.len() > 1 {
            let alias = links.iter().find(|link| link.file_stem() != Some(id));
            return refuse(alias.map_or(path, PathBuf::as_path));
        }
        if path.file_stem() == Some(id) {
            return Ok(());
        }
        if is_same_entry(path, &Self::node_path_at(root, &doc.node.id)?) {
            return Ok(());
        }
        refuse(path)
    }
}
