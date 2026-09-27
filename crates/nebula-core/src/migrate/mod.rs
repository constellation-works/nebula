//! Bring a corpus forward to this build's schema.
//!
//! The steps are an ordered, append-only registry, `MIGRATIONS`, applied
//! from the schema `config.yaml` declares (a corpus with no config is v1) to
//! [`crate::SCHEMA_VERSION`], all in memory, before anything is written;
//! `config.yaml` is written last, as the ledger. One shot and idempotent.
//! Today there is one step, v1 to v2. The v1 model, in `v1`, is private to this module and
//! deliberately lenient: every field is optional and unknown keys are
//! tolerated, because the point is to read whatever an old corpus holds and
//! re-label it losslessly into the v2 shape. Evidence, task links and the
//! removed edge kinds all become references, since a reference with a good
//! note carries the same information. See `docs/design/v0.2/1_spec.md`,
//! "Migration".
//!
//! That leniency is scoped to old content. A corpus whose `config.yaml`
//! already declares this build's schema is read with the strict current
//! model before anything is written, because there is nothing left to
//! re-label there and the only thing tolerance could do is quietly drop a
//! field the build does not know.

mod v1;

#[cfg(test)]
mod tests;

pub(crate) use v1::v1_node_under_current_schema;
use v1::v1_to_v2;

use crate::config::{self, Config, Declared};
use crate::error::{Error, Result};
use crate::locations::Locations;
use crate::lock::{CorpusLock, LOCK_FILE};
use crate::model;
use crate::store::{self, Corpus};
use serde::Serialize;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------- the report --

/// What a migration did.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MigrationReport {
    /// One entry per node that was rewritten, in corpus order. A node already
    /// in v2 form is left alone and does not appear.
    pub rewritten: Vec<NodeMigration>,
    /// How many nodes were examined.
    pub nodes: usize,
    /// Whether `config.yaml` was rewritten.
    pub config_rewritten: bool,
    /// The `corpus_id` this run gave the corpus, when it had none to carry
    /// over: a corpus from before `config.yaml` existed. `None` when the id
    /// was kept.
    pub minted_corpus_id: Option<String>,
}

/// One node brought forward, and what changed about it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NodeMigration {
    /// The node's id, which migration never changes.
    pub id: String,
    /// A line per thing that was re-labelled.
    pub notes: Vec<String>,
}

// ----------------------------------------------------------- the registry --

/// One schema step: a whole corpus, in memory, from schema `from` to `to`.
pub(crate) struct Migration {
    /// The schema the step reads.
    pub(crate) from: u32,
    /// The schema it leaves the corpus at, always `from + 1`.
    pub(crate) to: u32,
    /// Rewrites every staged node's text and the staged config. Never
    /// touches the disk: [`run`] writes only after every step has succeeded.
    pub(crate) step: fn(&mut Staged) -> Result<()>,
}

/// Every schema step there has been, oldest first (STD-03 §R23).
///
/// Append-only. A shipped entry is never edited, reordered or removed: a
/// corpus written by any past build comes forward by starting at the entry
/// for the schema its `config.yaml` declares and applying every one after
/// it. A new schema adds one entry at the end and bumps
/// [`config::SCHEMA_VERSION`] to its `to`.
///
/// `config.yaml` is the ledger and is written last, so a run that stops part
/// way leaves it declaring the old schema, and the rerun applies every step
/// again, to the nodes that were already written as well. Each step must
/// therefore leave a node already at or past its `to` unchanged.
pub(crate) const MIGRATIONS: &[Migration] = &[
    // Shipped in v0.2. Frozen.
    Migration {
        from: 1,
        to: 2,
        step: v1_to_v2,
    },
];

/// The steps that bring a corpus declared at `version` to this build's
/// schema, or `None` when no entry starts there.
///
/// A corpus already at this schema gets the last step again. It has nothing
/// to convert, and the strict preflight in [`run`] has proved that step has
/// nothing to drop, so all it can do is render a node a hand edit left out
/// of form (a tag's case) the way this build writes it.
fn steps_from(version: u32) -> Option<&'static [Migration]> {
    if version == config::SCHEMA_VERSION {
        return MIGRATIONS
            .len()
            .checked_sub(1)
            .map(|last| &MIGRATIONS[last..]);
    }
    MIGRATIONS
        .iter()
        .position(|migration| migration.from == version)
        .map(|first| &MIGRATIONS[first..])
}

/// A corpus as the steps see it: its config and every node file, as text.
pub(crate) struct Staged {
    /// Where the corpus is, for the paths an error names and the id a step
    /// mints.
    pub(crate) root: PathBuf,
    /// `config.yaml`'s text at the schema the steps have reached; `None`
    /// while there is no config, which is where a corpus from before the
    /// file starts.
    pub(crate) config: Option<String>,
    /// The `corpus_id` a step minted, when there was none to carry over.
    pub(crate) minted_corpus_id: Option<String>,
    /// Every node file, in corpus order.
    pub(crate) nodes: Vec<StagedNode>,
}

/// One node file on its way forward.
pub(crate) struct StagedNode {
    /// The file.
    pub(crate) path: PathBuf,
    /// Its text as it was read, to tell whether it needs writing.
    pub(crate) original: String,
    /// Its text at the schema the steps have reached.
    pub(crate) text: String,
    /// A line per thing a step re-labelled.
    pub(crate) notes: Vec<String>,
}

impl Staged {
    /// Read every node file and the config text under `root`, writing
    /// nothing.
    fn read(root: &Path, config: Option<String>) -> Result<Self> {
        store::refuse_nodes_symlink(root)?;
        let dir = root.join("nodes");
        let mut paths = Vec::new();
        for entry in store::list_directory(&dir)?.into_strict()? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                paths.push(path);
            }
        }
        paths.sort();
        let nodes = paths
            .into_iter()
            .map(|path| {
                // Only a regular file: migration rewrites what it reads, and
                // a symlink or a FIFO here is refused as a scan refuses it.
                let text = crate::fs_impl::read_regular_text(&path)?.ok_or_else(|| {
                    Error::io_at("reading", &path, std::io::ErrorKind::NotFound.into())
                })?;
                Ok(StagedNode {
                    path,
                    original: text.clone(),
                    text,
                    notes: Vec::new(),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            root: root.to_path_buf(),
            config,
            minted_corpus_id: None,
            nodes,
        })
    }
}

// --------------------------------------------------------------- the verb --

/// Convert every node and the config in place.
///
/// Takes a root rather than a [`Corpus`], because a corpus at the old schema
/// is exactly what refuses to open.
///
/// Every refusal comes before the first write (STD-02 §R34, STD-03 §R23):
/// the config is read, every node is read and put through every step in
/// memory, and every result is read back with the current model, before
/// anything is written. Then the nodes that changed are written one by one,
/// and `config.yaml` last, as the record that the corpus is at this schema.
///
/// [`crate::verb::migrate`] is this and the commit that records it, under
/// the one lock.
pub fn run(locations: &Locations, root: Option<PathBuf>) -> Result<MigrationReport> {
    run_then(locations, root, |_| ()).map(|(_, report, _)| report)
}

/// [`run`], then `after` on the migrated root while the lock is still held:
/// the root, the report, and what `after` returned. `after` is not run when
/// the migration refuses or changes nothing.
pub(crate) fn run_then<T>(
    locations: &Locations,
    root: Option<PathBuf>,
    after: impl FnOnce(&Path) -> T,
) -> Result<(PathBuf, MigrationReport, Option<T>)> {
    locations.write_gate(crate::locations::WriteIntent::Ordinary)?;
    let root = Corpus::resolve_root(locations, root)?;
    store::refuse_nodes_symlink(&root)?;
    if !root.join("nodes").is_dir() {
        return Err(Error::NoCorpus(root));
    }
    // A migration rewrites every node in the corpus, so it is the last write
    // that should run beside another. Taken after the corpus is known to be
    // there, so a missing one still reports itself as missing.
    let _lock = CorpusLock::acquire(&root)?;
    // Validate the config before touching a node. In particular, a future
    // schema may contain fields this build does not know how to preserve, so
    // treating it as v1 would turn migration into a destructive downgrade.
    let (version, existing) = match config::declared(&root)? {
        // No config at all is a corpus from before the file existed, which
        // is v1: the one verb that can bring such a corpus forward accepts
        // it (STD-02 §R33, STD-03 §R24).
        Declared::Missing => (1, None),
        Declared::Current { raw, .. } => (config::SCHEMA_VERSION, Some(raw)),
        Declared::Older { version, raw } => (version, Some(raw)),
        Declared::Newer { version } => return Err(config::schema_mismatch(&root, version)),
    };
    if version != config::SCHEMA_VERSION {
        refuse_dirty_tree(locations, &root, Some((version, existing.clone())))?;
    }
    let steps = steps_from(version).ok_or_else(|| config::schema_mismatch(&root, version))?;

    let mut staged = Staged::read(&root, existing.clone())?;
    // A corpus already at this build's schema has nothing to convert, so
    // every node in it must already read under the current model. Proving
    // that here, over the whole corpus, is what stops the v1 model's
    // leniency from turning a no-op migration into a silent deletion:
    // unknown keys are tolerated because a v1 file holds retired ones, and
    // the only thing that tolerance can do to current-schema content is drop
    // what this build does not recognise.
    if version == config::SCHEMA_VERSION {
        preflight_nodes(&root, &staged, version)?;
    }
    let mut reached = version;
    for migration in steps {
        (migration.step)(&mut staged)?;
        reached = migration.to;
    }
    // The registry's contiguity is a unit test; this is the same fact
    // checked where a gap would otherwise write a corpus at the wrong schema.
    if reached != config::SCHEMA_VERSION {
        return Err(config::schema_mismatch(&root, reached));
    }
    let (writes, config) = verified(&staged)?;

    let mut report = MigrationReport {
        nodes: staged.nodes.len(),
        minted_corpus_id: staged.minted_corpus_id.clone(),
        ..MigrationReport::default()
    };
    let config_changed = existing.as_deref() != Some(config.render()?.as_str());
    if writes.is_empty() && !config_changed {
        // A verified no-op neither needs a clean tree nor commits any user
        // edits that appeared since migration, even with commits enabled.
        return Ok((root, report, None));
    }
    if version == config::SCHEMA_VERSION {
        // Current-schema normalization still rewrites content, so it keeps
        // the same recoverability requirement as a schema upgrade.
        refuse_dirty_tree(locations, &root, None)?;
    }
    // All content has passed preflight. Install the same runtime exclusions
    // as init before changing nodes, keeping config last as the ledger.
    store::ensure_lock_ignored(&root)?;
    for (node, id) in writes {
        store::refuse_nodes_symlink(&root)?;
        crate::fs_impl::write_private_atomic(&node.path, &node.text)?;
        report.rewritten.push(NodeMigration {
            id,
            notes: node.notes.clone(),
        });
    }
    // Last: the ledger. Idempotence is byte equality here as for the nodes.
    if config_changed {
        config.save(&root)?;
        report.config_rewritten = true;
    }
    let after = after(&root);
    Ok((root, report, Some(after)))
}

/// Read back what the steps produced with the current model, in memory: each
/// node, including its filename identity, and the config. Return the changed
/// nodes with their ids. A step that produced
/// something this build cannot read is refused here, before any write,
/// rather than found by the next verb in a half-written corpus.
fn verified(staged: &Staged) -> Result<(Vec<(&StagedNode, String)>, Config)> {
    let mut writes = Vec::new();
    for node in &staged.nodes {
        let doc = model::parse(&node.text, &node.path).map_err(|source| {
            Error::MigratedNodeUnreadable {
                path: node.path.clone(),
                source: Box::new(source),
            }
        })?;
        // An unchanged node is still part of the corpus the ledger will
        // declare readable. Share the loader's filesystem identity rule,
        // including Unicode respellings and hard-link alias refusal.
        Corpus::require_file_agrees(&staged.root, &node.path, &doc)?;
        // Idempotence is byte equality; validation must not skip unchanged
        // nodes, but neither may it rewrite them or bump `updated`.
        if node.text != node.original {
            writes.push((node, doc.node.id));
        }
    }
    let path = staged.root.join(config::FILE);
    let config: Config = serde_yaml_ng::from_str(staged.config.as_deref().unwrap_or_default())
        .map_err(|e| Error::yaml(format!("the migrated {}", path.display()), e))?;
    if config.schema_version != config::SCHEMA_VERSION {
        return Err(config::schema_mismatch(&staged.root, config.schema_version));
    }
    Ok((writes, config))
}

/// Refuse a corpus that declares the current schema but holds a node this
/// build cannot read, naming the first such file in corpus order.
///
/// Reads with the same model every other verb reads with, so `migrate`
/// accepts exactly what `check` accepts and neither one's idea of a node can
/// drift from the other's.
fn preflight_nodes(root: &Path, staged: &Staged, version: u32) -> Result<()> {
    for node in &staged.nodes {
        if let Err(source) = model::read(&node.path) {
            return Err(
                v1_node_under_current_schema(root, &node.path, version).unwrap_or_else(|| {
                    Error::CurrentSchemaUnreadable {
                        path: node.path.clone(),
                        version,
                        source: Box::new(source),
                    }
                }),
            );
        }
    }
    Ok(())
}

/// A corpus under git with unrelated uncommitted changes is refused, so the
/// migration lands as its own commit and the pre-migration state stays
/// recoverable. Old schemas may resume only proven migration output.
/// A corpus with no repository at or above it is migrated as is.
///
/// Fails closed (integrity, STD-02 §R31): inside a repository, git that
/// cannot say whether the tree is clean refuses the migration, because
/// unknown is not clean and the rewrite that follows touches every node.
fn refuse_dirty_tree(
    locations: &Locations,
    root: &Path,
    recovery: Option<(u32, Option<String>)>,
) -> Result<()> {
    let at = locations.git_at(root);
    if !store::inside_work_tree(at)? {
        return Ok(());
    }
    // The write lock is a fact about which process is writing, not corpus
    // content, and it is untracked in a corpus that is its own repository.
    // Without excluding it here the first `neb` write of the day would leave
    // `migrate` refusing for good.
    let exclude = format!(":(exclude){LOCK_FILE}");
    let status = store::git(
        at,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
            &exclude,
        ],
    )?;
    if !status.status.success() {
        return Err(Error::Git {
            root: root.to_path_buf(),
            context: "status".to_string(),
            stderr: status.stderr.text(),
        });
    }
    let dirty = status.stdout_whole(root, "status")?;
    if status.stdout.is_empty() {
        return Ok(());
    }
    if let Some((version, config)) = recovery {
        return verify_interrupted(at, dirty, version, config);
    }
    Err(Error::DirtyTree(root.to_path_buf()))
}

/// Prove recovery from committed input, not from the plausibility of dirty
/// v2 data. No journal or extra corpus format is needed: HEAD is the durable
/// pre-migration snapshot, and every accepted change must be its exact output.
/// Staged changes, deleted/renamed files, and edits outside those outputs are
/// refused. This is an accident guard, not proof of which process wrote bytes.
fn verify_interrupted(
    at: crate::git::GitAt<'_>,
    dirty: &[u8],
    version: u32,
    config: Option<String>,
) -> Result<()> {
    let refuse = || Error::DirtyTree(at.root.to_path_buf());
    // Porcelain paths are repository-relative even when the corpus is nested.
    let prefix = migration_git(at, &["rev-parse", "--show-prefix"])?;
    let prefix = prefix.strip_suffix(b"\n").unwrap_or(&prefix);
    let mut paths = Vec::new();
    for entry in dirty.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(refuse());
        }
        let path = entry[3..].strip_prefix(prefix).ok_or_else(refuse)?;
        let path = std::str::from_utf8(path).map_err(|_| refuse())?;
        let node = Path::new(path).parent() == Some(Path::new("nodes"))
            && Path::new(path).extension().is_some_and(|ext| ext == "md");
        if !((entry.starts_with(b" M ") && (node || path == store::GITIGNORE_FILE))
            || (entry.starts_with(b"?? ") && path == store::GITIGNORE_FILE))
        {
            return Err(refuse());
        }
        paths.push(path);
    }
    // Resolve the moving ref once; every historical byte below comes from it.
    let head = migration_git(at, &["rev-parse", "--verify", "HEAD"])?;
    let head = std::str::from_utf8(&head).map_err(|_| refuse())?.trim();
    if committed_file(at, head, config::FILE)?.as_deref() != config.as_deref().map(str::as_bytes) {
        return Err(refuse());
    }
    let mut baseline = Staged::read(at.root, config)?;
    for path in &paths {
        // Atomic migration writes never create executable output. A chmod
        // alongside otherwise matching bytes is still unrelated user work.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = at.root.join(path);
            let metadata = std::fs::symlink_metadata(&file)
                .map_err(|error| Error::io_at("inspecting", &file, error))?;
            if metadata.permissions().mode() & 0o111 != 0 {
                return Err(refuse());
            }
        }
        let committed = committed_file(at, head, path)?;
        if *path == store::GITIGNORE_FILE {
            let expected = store::with_runtime_ignores(committed.clone().unwrap_or_default());
            if committed.as_deref() == Some(expected.as_slice()) {
                return Err(refuse());
            }
            let actual = crate::fs_impl::read_regular_bytes(
                &at.root.join(path),
                crate::fs_impl::Links::Refuse,
            )?;
            if actual.as_deref() != Some(expected.as_slice()) {
                return Err(refuse());
            }
        } else {
            let original =
                String::from_utf8(committed.ok_or_else(refuse)?).map_err(|_| refuse())?;
            let node = baseline
                .nodes
                .iter_mut()
                .find(|node| node.path == at.root.join(path))
                .ok_or_else(refuse)?;
            node.original.clone_from(&original);
            node.text = original;
        }
    }
    for migration in steps_from(version).ok_or_else(refuse)? {
        (migration.step)(&mut baseline)?;
    }
    verified(&baseline)?;
    for path in paths {
        if path == store::GITIGNORE_FILE {
            continue;
        }
        let node = baseline
            .nodes
            .iter()
            .find(|node| node.path == at.root.join(path))
            .ok_or_else(refuse)?;
        let actual = crate::fs_impl::read_regular_text(&node.path)?;
        if node.text == node.original || actual.as_deref() != Some(node.text.as_str()) {
            return Err(refuse());
        }
    }
    Ok(())
}

/// Read a regular blob from the pinned snapshot; absence is distinct from a
/// failed git call. Literal pathspecs and NUL framing handle unusual filenames.
fn committed_file(at: crate::git::GitAt<'_>, head: &str, path: &str) -> Result<Option<Vec<u8>>> {
    let literal = format!(":(literal){path}");
    let tree = migration_git(at, &["ls-tree", "-z", head, "--", &literal])?;
    if tree.is_empty() {
        return Ok(None);
    }
    let refuse = || Error::DirtyTree(at.root.to_path_buf());
    let end = tree.iter().position(|b| *b == b'\t').ok_or_else(refuse)?;
    let metadata = std::str::from_utf8(&tree[..end]).map_err(|_| refuse())?;
    let mut fields = metadata.split_whitespace();
    if !matches!(fields.next(), Some("100644" | "100755")) || fields.next() != Some("blob") {
        return Err(refuse());
    }
    let oid = fields.next().ok_or_else(refuse)?;
    migration_git(at, &["cat-file", "blob", oid]).map(Some)
}

/// Recovery never interprets failed or truncated git output as clean state.
fn migration_git(at: crate::git::GitAt<'_>, args: &[&str]) -> Result<Vec<u8>> {
    let out = store::git(at, args)?;
    if !out.status.success() {
        return Err(Error::Git {
            root: at.root.to_path_buf(),
            context: args[0].to_string(),
            stderr: out.stderr.text(),
        });
    }
    Ok(out.stdout_whole(at.root, args[0])?.to_vec())
}
