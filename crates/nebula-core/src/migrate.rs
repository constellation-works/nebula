//! Bring a corpus forward to this build's schema.
//!
//! The steps are an ordered, append-only registry, `MIGRATIONS`, applied
//! from the schema `config.yaml` declares (a corpus with no config is v1) to
//! [`crate::SCHEMA_VERSION`], all in memory, before anything is written;
//! `config.yaml` is written last, as the ledger. One shot and idempotent.
//! Today there is one step, v1 to v2. The v1 model below is private to this module and
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

use crate::config::{self, Config, Declared};
use crate::error::{Error, Result};
use crate::git::RunError;
use crate::lock::{CorpusLock, LOCK_FILE};
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use crate::store::{self, Corpus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// --------------------------------------------------------------- v1 model --

#[derive(Debug, Deserialize)]
struct V1Node {
    id: String,
    title: String,
    // v1 had no authorship, but a v2 node does, and this model is what every
    // node is read through: dropping the fields here would strip authorship
    // off a migrated corpus and stop `neb migrate` being a no-op on v2.
    #[serde(default)]
    title_by: Option<String>,
    #[serde(default)]
    domain: String,
    status: String,
    created: String,
    updated: String,
    #[serde(default)]
    kill: Option<String>,
    #[serde(default)]
    kill_by: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    edges: Vec<V1Edge>,
    #[serde(default)]
    evidence: Vec<V1Evidence>,
    #[serde(default)]
    references: Vec<V1Reference>,
    #[serde(default)]
    tasks: Vec<V1Task>,
    #[serde(default)]
    origin: Option<V1Origin>,
    #[serde(default)]
    graduated_to: Option<String>,
    #[serde(default)]
    closed: Option<Closed>,
}

#[derive(Debug, Deserialize)]
struct V1Edge {
    #[serde(rename = "type")]
    kind: String,
    to: String,
    #[serde(default)]
    by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct V1Evidence {
    id: String,
    verdict: String,
    strength: String,
    source: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    origin: Option<V1Origin>,
}

#[derive(Debug, Deserialize)]
struct V1Reference {
    id: String,
    kind: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
    added: String,
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    promoted_to: Option<String>,
    #[serde(default)]
    origin: Option<V1Origin>,
}

// Only the legacy reader is lenient. Reusing the current Origin here would
// reject retired v1 provenance keys after current-schema reads become strict.
#[derive(Debug, Deserialize)]
struct V1Origin {
    task: Option<String>,
    workspace: Option<String>,
    run: Option<String>,
    artifact: Option<String>,
    agent: Option<String>,
    at: Option<String>,
}

impl From<V1Origin> for Origin {
    fn from(old: V1Origin) -> Self {
        Self {
            task: old.task,
            workspace: old.workspace,
            run: old.run,
            artifact: old.artifact,
            agent: old.agent,
            at: old.at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct V1Task {
    id: String,
    #[serde(default = "open")]
    state: String,
    #[serde(default)]
    why: Option<String>,
}

fn open() -> String {
    "open".into()
}

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
        for entry in std::fs::read_dir(&dir).map_err(|e| Error::io_at("reading", &dir, e))? {
            let path = entry.map_err(|e| Error::io_at("reading", &dir, e))?.path();
            if path.extension().is_some_and(|e| e == "md") {
                paths.push(path);
            }
        }
        paths.sort();
        let nodes = paths
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| Error::io_at("reading", &path, e))?;
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
pub fn run(root: Option<PathBuf>) -> Result<MigrationReport> {
    let root = Corpus::resolve_root(root)?;
    store::refuse_nodes_symlink(&root)?;
    if !root.join("nodes").is_dir() {
        return Err(Error::NoCorpus(root));
    }
    // A migration rewrites every node in the corpus, so it is the last write
    // that should run beside another. Taken after the corpus is known to be
    // there, so a missing one still reports itself as missing.
    let _lock = CorpusLock::acquire(&root)?;
    refuse_dirty_tree(&root)?;
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

    // Nothing has been written yet; everything that could refuse has.
    let mut report = MigrationReport {
        nodes: staged.nodes.len(),
        minted_corpus_id: staged.minted_corpus_id.clone(),
        ..MigrationReport::default()
    };
    for (node, id) in writes {
        store::refuse_nodes_symlink(&root)?;
        crate::fs::write_private_atomic(&node.path, &node.text)?;
        report.rewritten.push(NodeMigration {
            id,
            notes: node.notes.clone(),
        });
    }
    // Last: the ledger. Idempotence is byte equality here as for the nodes.
    if existing.as_deref() != Some(config.render()?.as_str()) {
        config.save(&root)?;
        report.config_rewritten = true;
    }
    Ok(report)
}

/// Read back what the steps produced with the current model, in memory: each
/// node that changed, with its id, and the config. A step that produced
/// something this build cannot read is refused here, before any write,
/// rather than found by the next verb in a half-written corpus.
fn verified(staged: &Staged) -> Result<(Vec<(&StagedNode, String)>, Config)> {
    let mut writes = Vec::new();
    for node in &staged.nodes {
        // Idempotence is byte equality: a node already in v2 form renders
        // back to exactly what is on disk and is left alone, so a second run
        // rewrites nothing and `updated` is never bumped by a migration.
        if node.text == node.original {
            continue;
        }
        let doc = model::parse(&node.text).map_err(|e| {
            Error::corpus(format!(
                "in {}: the migration produced a node this build cannot read: {e}",
                node.path.display()
            ))
        })?;
        writes.push((node, doc.node.id));
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

/// The keys only a v1 node carries. A node holding one of them is old
/// content, not a typo or a newer build's field.
const V1_ONLY_KEYS: [&str; 4] = ["domain", "evidence", "tasks", "graduated_to"];

/// [`Error::V1NodeUnderCurrentSchema`] for the node at `path`, when it reads
/// under the v1 model and carries a key only v1 had; `None` otherwise, and
/// when it cannot be read at all.
///
/// Asked only about a node the current model has already refused, in a
/// corpus whose config declares `version`, this build's schema.
pub(crate) fn v1_node_under_current_schema(
    root: &Path,
    path: &Path,
    version: u32,
) -> Option<Error> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (front, _) = model::split_frontmatter(&raw).ok()?;
    serde_yaml_ng::from_str::<V1Node>(front).ok()?;
    let fields: serde_yaml_ng::Mapping = serde_yaml_ng::from_str(front).ok()?;
    let keys: Vec<String> = V1_ONLY_KEYS
        .iter()
        .filter(|key| fields.contains_key(**key))
        .map(ToString::to_string)
        .collect();
    (!keys.is_empty()).then(|| Error::V1NodeUnderCurrentSchema {
        path: path.to_path_buf(),
        config: root.join(config::FILE),
        version,
        keys,
    })
}

/// Name the file a complaint about the corpus came from.
fn in_file(e: Error, path: &Path) -> Error {
    match e {
        Error::Corpus(message) => Error::corpus(format!("in {}: {message}", path.display())),
        other => other,
    }
}

/// A corpus under git with uncommitted changes is refused, so the migration
/// lands as its own commit and the pre-migration state stays recoverable.
/// A corpus that is not a git repository is migrated as is.
fn refuse_dirty_tree(root: &Path) -> Result<()> {
    match store::inside_work_tree(root) {
        Ok(true) => {}
        // No git on PATH, or no repository: nothing to protect against.
        Ok(false) | Err(RunError::Start(_)) => return Ok(()),
        // git ran and gave no answer: whether the tree is dirty is unknown,
        // which is not the same as clean.
        Err(error) => return Err(error.into_error(root, "rev-parse")),
    }
    // The write lock is a fact about which process is writing, not corpus
    // content, and it is untracked in a corpus that is its own repository.
    // Without excluding it here the first `neb` write of the day would leave
    // `migrate` refusing for good.
    let exclude = format!(":(exclude){LOCK_FILE}");
    let status = store::git(root, &["status", "--porcelain", "--", ".", &exclude])?;
    if !status.status.success() {
        return Err(Error::corpus(format!(
            "git status failed in {}:\n{}",
            root.display(),
            status.stderr.text()
        )));
    }
    if !status.stdout.is_empty() {
        return Err(Error::corpus(format!(
            "{} has uncommitted changes; commit or stash them so the migration is its own commit",
            root.display()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- v1 -> v2 --

/// The keys of a v1 `config.yaml` that v2 keeps. The v1 keys it drops are
/// named in `docs/runbooks/migrate-v1-to-v2.md`.
#[derive(Default, Deserialize)]
struct V1Config {
    #[serde(default)]
    corpus_id: Option<String>,
    #[serde(default)]
    observatory_root: Option<PathBuf>,
    #[serde(default)]
    commit: bool,
}

/// The first entry in [`MIGRATIONS`]: `config.yaml` keeps `corpus_id`, the
/// observatory root when one was set, and `commit` when it is on, gaining
/// an id when it had none; every node is read with the lenient v1 model and
/// rendered in v2 form.
fn v1_to_v2(corpus: &mut Staged) -> Result<()> {
    // The config first, so a malformed one is what a run reports.
    let path = corpus.root.join(config::FILE);
    let legacy: V1Config = corpus
        .config
        .as_deref()
        .map(serde_yaml_ng::from_str)
        .transpose()
        .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?
        .unwrap_or_default();
    let corpus_id = if let Some(id) = legacy.corpus_id.filter(|id| !id.is_empty()) {
        id
    } else {
        let id = store::corpus_id(&corpus.root);
        corpus.minted_corpus_id = Some(id.clone());
        id
    };
    let mut migrated = Config::fresh(corpus_id);
    migrated.observatory_root = legacy.observatory_root;
    migrated.commit = legacy.commit;
    corpus.config = Some(migrated.render()?);

    for node in &mut corpus.nodes {
        let (front, body) =
            model::split_frontmatter(&node.text).map_err(|e| in_file(e, &node.path))?;
        let v1: V1Node = serde_yaml_ng::from_str(front).map_err(|e| {
            Error::yaml(
                format!("parsing {} with the v1 model", node.path.display()),
                e,
            )
        })?;
        let (converted, notes) = convert(v1).map_err(|e| in_file(e, &node.path))?;
        node.text = model::render(&Doc {
            node: converted,
            body: body.to_string(),
        })?;
        node.notes.extend(notes);
    }
    Ok(())
}

// ------------------------------------------------------------ conversion --

/// One v1 node in v2 form, plus a line per thing that changed.
fn convert(v1: V1Node) -> Result<(Node, Vec<String>)> {
    let mut notes = Vec::new();

    // The v1 model is lenient by design, so this is the one place a v1 id is
    // looked at. A corpus whose id could not name a file would migrate
    // cleanly and then refuse to open, which reads as the migration having
    // broken it.
    if !store::is_path_safe_id(&v1.id) {
        return Err(Error::UnsafeId(v1.id));
    }

    let mut tags = model::normalize_tags(&v1.tags);
    if tags != v1.tags {
        notes.push(format!("tags normalised: {}", tags.join(", ")));
    }
    if !v1.domain.is_empty() {
        let tag = model::normalize_tag(&v1.domain);
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag.clone());
        }
        notes.push(format!("domain `{}` -> tag `{tag}`", v1.domain));
    }

    let mut refs = Relabel {
        out: Vec::new(),
        next: 0,
        updated: &v1.updated,
        notes: &mut notes,
    };
    refs.references(v1.references);
    refs.evidence(v1.evidence);
    refs.tasks(v1.tasks);
    let edges = refs.edges(v1.edges)?;
    let references = refs.out;

    let (status, closed) = convert_status(
        &v1.status,
        v1.closed,
        v1.graduated_to.as_deref(),
        &v1.updated,
        &mut notes,
    )?;

    Ok((
        Node {
            id: v1.id,
            title: v1.title,
            title_by: v1.title_by,
            status,
            created: v1.created,
            updated: v1.updated,
            kill: v1.kill,
            kill_by: v1.kill_by,
            tags,
            edges,
            references,
            closed,
            origin: v1.origin.map(Into::into),
        },
        notes,
    ))
}

/// Builds the v2 reference list: the v1 references first, then everything
/// re-labelled into one, with ids continuing the `r<n>` sequence.
struct Relabel<'a> {
    out: Vec<Reference>,
    next: usize,
    updated: &'a str,
    notes: &'a mut Vec<String>,
}

impl Relabel<'_> {
    fn references(&mut self, refs: Vec<V1Reference>) {
        for r in refs {
            if let Some(ev) = &r.promoted_to {
                self.notes.push(format!(
                    "{} was weighed as `{ev}`; that entry follows as its own reference",
                    r.id
                ));
            }
            self.out.push(Reference {
                id: r.id,
                kind: r.kind,
                uri: r.uri,
                title: r.title,
                note: r.note,
                added: r.added,
                by: r.by,
                origin: r.origin.map(Into::into),
            });
        }
        self.next = model::next_reference_index(&self.out);
    }

    /// A `kind: other` reference with a note, which is what every
    /// re-labelled v1 entry becomes.
    fn push(
        &mut self,
        uri: String,
        title: Option<String>,
        note: String,
        added: String,
        origin: Option<Origin>,
    ) -> String {
        let id = format!("r{}", self.next);
        self.next += 1;
        self.out.push(Reference {
            id: id.clone(),
            kind: "other".into(),
            uri: Some(uri),
            title,
            note: Some(note),
            added,
            // A re-labelled v1 entry is the human's own record, restated.
            by: None,
            origin,
        });
        id
    }

    fn evidence(&mut self, evidence: Vec<V1Evidence>) {
        for ev in evidence {
            let note = ev.note.as_deref().map(str::trim).unwrap_or_default();
            let title = note
                .lines()
                .next()
                .filter(|l| !l.is_empty())
                .map(String::from);
            let note = if note.is_empty() {
                format!("[{}/{}]", ev.verdict, ev.strength)
            } else {
                format!("[{}/{}] {note}", ev.verdict, ev.strength)
            };
            let added = ev.date.unwrap_or_else(|| self.updated.to_string());
            let id = self.push(ev.source, title, note, added, ev.origin.map(Into::into));
            self.notes
                .push(format!("evidence {} -> reference {id}", ev.id));
        }
    }

    fn tasks(&mut self, tasks: Vec<V1Task>) {
        for t in tasks {
            let why = t.why.as_deref().map(str::trim).unwrap_or_default();
            let note = if why.is_empty() {
                format!("[{}]", t.state)
            } else {
                format!("[{}] {why}", t.state)
            };
            let id = self.push(
                format!("orbit:{}", t.id),
                Some(t.id.clone()),
                note,
                self.updated.to_string(),
                None,
            );
            self.notes.push(format!("task {} -> reference {id}", t.id));
        }
    }

    /// The five surviving edge kinds pass through. The evidence graph in
    /// edge form becomes a reference to the other node, so the claim
    /// survives even though the edge kind does not.
    fn edges(&mut self, edges: Vec<V1Edge>) -> Result<Vec<Edge>> {
        let mut out = Vec::new();
        for e in edges {
            let kind = match e.kind.as_str() {
                "derives-from" => EdgeType::DerivesFrom,
                "refines" => EdgeType::Refines,
                "generalizes" => EdgeType::Generalizes,
                "reopens" => EdgeType::Reopens,
                "contradicts" => EdgeType::Contradicts,
                kind @ ("supports" | "undermines" | "depends-on") => {
                    let id = self.push(
                        format!("neb:{}", e.to),
                        Some(e.to.clone()),
                        format!("[{kind}] {}", e.to),
                        self.updated.to_string(),
                        None,
                    );
                    self.notes
                        .push(format!("edge {kind} {} -> reference {id}", e.to));
                    continue;
                }
                other => {
                    return Err(Error::corpus(format!(
                        "edge type `{other}` is not a v1 edge type"
                    )));
                }
            };
            out.push(Edge {
                kind,
                to: e.to,
                by: e.by,
            });
        }
        Ok(out)
    }
}

/// The v2 status and `closed` block for a v1 status.
fn convert_status(
    status: &str,
    closed: Option<Closed>,
    graduated_to: Option<&str>,
    updated: &str,
    notes: &mut Vec<String>,
) -> Result<(Status, Option<Closed>)> {
    Ok(match status {
        "seed" => (Status::Seed, closed),
        "hypothesis" => (Status::Hypothesis, closed),
        "testing" | "supported" => {
            notes.push(format!("status {status} -> hypothesis"));
            (Status::Hypothesis, closed)
        }
        "abandoned" => (Status::Abandoned, closed),
        "refuted" => {
            // Rule 5 did not exist in v1. Nothing in the old record says how
            // the kill condition fired, and inventing a reason would be a
            // lie, so the block says exactly that.
            let closed = closed.or_else(|| {
                notes.push("closed.why filled from the v1 record".into());
                Some(Closed {
                    why: "refuted under v1, before a reason was recorded; see references".into(),
                    at: updated.to_string(),
                })
            });
            (Status::Refuted, closed)
        }
        "graduated" => {
            let why = match graduated_to {
                Some(to) => format!("graduated to {to}"),
                None => "graduated".into(),
            };
            notes.push(format!("status graduated -> abandoned ({why})"));
            (
                Status::Abandoned,
                Some(Closed {
                    why,
                    at: updated.to_string(),
                }),
            )
        }
        other => {
            return Err(Error::corpus(format!(
                "status `{other}` is not a v1 status"
            )));
        }
    })
}
