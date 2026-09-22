//! Bring a v1 corpus forward to v2.
//!
//! One shot and idempotent. The v1 model below is private to this module and
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

use crate::config::{self, Config};
use crate::error::{Error, Result};
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

// --------------------------------------------------------------- the verb --

/// Convert every node and the config in place.
///
/// Takes a root rather than a [`Corpus`], because a corpus at the old schema
/// is exactly what refuses to open.
pub fn run(root: Option<PathBuf>) -> Result<MigrationReport> {
    let root = Corpus::resolve_root(root)?;
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
    let config = preflight_config(&root)?;

    let mut report = MigrationReport::default();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join("nodes"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    report.nodes = paths.len();
    // A corpus already at this build's schema has nothing to convert, so
    // every node in it must already read under the current model. Proving
    // that here, over the whole corpus and before the loop below writes
    // anything, is what stops the v1 model's leniency from turning a no-op
    // migration into a silent deletion: unknown keys are tolerated because a
    // v1 file holds retired ones, and the only thing that tolerance can do to
    // current-schema content is drop what this build does not recognise. Up
    // front rather than per node, so an unreadable node late in the corpus
    // cannot leave the earlier ones rewritten.
    if config.version == config::SCHEMA_VERSION {
        preflight_nodes(&paths, config.version)?;
    }
    for path in &paths {
        let raw = std::fs::read_to_string(path)?;
        let (front, body) = model::split_frontmatter(&raw).map_err(|e| in_file(e, path))?;
        let v1: V1Node = serde_yaml_ng::from_str(front)
            .map_err(|e| Error::yaml(format!("parsing {} with the v1 model", path.display()), e))?;
        let (node, notes) = convert(v1).map_err(|e| in_file(e, path))?;
        let doc = Doc {
            node,
            body: body.to_string(),
        };
        // Idempotence is byte equality: a node already in v2 form renders
        // back to exactly what is on disk and is left alone, so a second run
        // rewrites nothing and `updated` is never bumped by a migration.
        if model::render(&doc)? == raw {
            continue;
        }
        model::write(path, &doc)?;
        report.rewritten.push(NodeMigration {
            id: doc.node.id,
            notes,
        });
    }

    report.config_rewritten = migrate_config(&root, config)?;
    Ok(report)
}

/// Refuse a corpus that declares the current schema but holds a node this
/// build cannot read, naming the first such file in corpus order.
///
/// Reads with the same model every other verb reads with, so `migrate`
/// accepts exactly what `check` accepts and neither one's idea of a node can
/// drift from the other's.
fn preflight_nodes(paths: &[PathBuf], version: u32) -> Result<()> {
    for path in paths {
        if let Err(source) = model::read(path) {
            return Err(Error::CurrentSchemaUnreadable {
                path: path.clone(),
                version,
                source: Box::new(source),
            });
        }
    }
    Ok(())
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
    // No git on PATH, or no repository: nothing to protect against.
    if !store::inside_work_tree(root).unwrap_or(false) {
        return Ok(());
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
            String::from_utf8_lossy(&status.stderr).trim()
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

/// Rewrite `config.yaml` to hold `corpus_id`, `schema_version`, the
/// observatory root when one was set, and `commit` when it is on. The v1
/// keys it drops are named in `docs/runbooks/migrate-v1-to-v2.md`.
#[derive(Deserialize)]
struct ConfigVersion {
    #[serde(default)]
    schema_version: Option<u32>,
}

#[derive(Default, Deserialize)]
struct V1Config {
    #[serde(default)]
    corpus_id: Option<String>,
    #[serde(default)]
    observatory_root: Option<PathBuf>,
    #[serde(default)]
    commit: bool,
}

struct MigrationConfig {
    /// The schema the corpus declares, which decides how its nodes are read.
    /// Absent `schema_version` means v1, as it always has.
    version: u32,
    existing: Option<String>,
    corpus_id: Option<String>,
    observatory_root: Option<PathBuf>,
    commit: bool,
}

/// Parse and validate the config before migration can rewrite any corpus
/// content. V1 remains lenient because its retired keys are intentionally
/// discarded; v2 uses the current strict model, and every other version is
/// refused rather than guessed at.
fn preflight_config(root: &Path) -> Result<MigrationConfig> {
    let path = root.join(config::FILE);
    let existing = if path.exists() {
        Some(std::fs::read_to_string(&path)?)
    } else {
        None
    };
    let version = existing
        .as_deref()
        .map(serde_yaml_ng::from_str::<ConfigVersion>)
        .transpose()
        .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?
        .and_then(|probe| probe.schema_version)
        .unwrap_or(1);
    let (corpus_id, observatory_root, commit) = match version {
        1 => {
            let legacy = existing
                .as_deref()
                .map(serde_yaml_ng::from_str::<V1Config>)
                .transpose()
                .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?
                .unwrap_or_default();
            (legacy.corpus_id, legacy.observatory_root, legacy.commit)
        }
        config::SCHEMA_VERSION => {
            let current: Config = serde_yaml_ng::from_str(existing.as_deref().unwrap_or_default())
                .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?;
            (
                Some(current.corpus_id),
                current.observatory_root,
                current.commit,
            )
        }
        found => {
            return Err(Error::SchemaMismatch {
                path,
                found,
                expected: config::SCHEMA_VERSION,
            });
        }
    };
    Ok(MigrationConfig {
        version,
        existing,
        corpus_id,
        observatory_root,
        commit,
    })
}

fn migrate_config(root: &Path, config: MigrationConfig) -> Result<bool> {
    let MigrationConfig {
        version: _,
        existing,
        corpus_id,
        observatory_root,
        commit,
    } = config;
    let mut fresh = Config::fresh(corpus_id.unwrap_or_else(|| store::corpus_id(root)));
    fresh.observatory_root = observatory_root;
    fresh.commit = commit;
    if existing.as_deref() == Some(fresh.render()?.as_str()) {
        return Ok(false);
    }
    fresh.save(root)?;
    Ok(true)
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
