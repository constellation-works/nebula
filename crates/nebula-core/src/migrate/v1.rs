//! The v1 schema, read leniently, and the step that brings it to v2.
//!
//! Private to `migrate`: every field is optional and unknown keys are
//! tolerated, because the point is to read whatever an old corpus holds.

use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::Staged;

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
    let raw = crate::fs_impl::read_regular_text(path).ok()??;
    let (front, _) = model::split_frontmatter(&raw, path).ok()?;
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

/// The first entry in [`MIGRATIONS`](super::MIGRATIONS): `config.yaml` keeps `corpus_id`, the
/// observatory root when one was set, and `commit` when it is on, gaining
/// an id when it had none; every node is read with the lenient v1 model and
/// rendered in v2 form.
pub(super) fn v1_to_v2(corpus: &mut Staged) -> Result<()> {
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
        let id = crate::id::corpus_id(&corpus.root);
        corpus.minted_corpus_id = Some(id.clone());
        id
    };
    let mut migrated = Config::fresh(corpus_id);
    migrated.observatory_root = legacy.observatory_root;
    migrated.commit = legacy.commit;
    corpus.config = Some(migrated.render()?);

    for node in &mut corpus.nodes {
        let (front, body) = model::split_frontmatter(&node.text, &node.path)?;
        let v1: V1Node = serde_yaml_ng::from_str(front).map_err(|e| {
            Error::yaml(
                format!("parsing {} with the v1 model", node.path.display()),
                e,
            )
        })?;
        let (converted, notes) = convert(v1, &node.path)?;
        node.text = model::render(&Doc {
            node: converted,
            body: body.to_string(),
        })?;
        node.notes.extend(notes);
    }
    Ok(())
}

// ------------------------------------------------------------ conversion --

/// One v1 node in v2 form, plus a line per thing that changed. `path` is the
/// file it was read from, which a refusal names.
fn convert(v1: V1Node, path: &Path) -> Result<(Node, Vec<String>)> {
    let mut notes = Vec::new();

    // The v1 model is lenient by design, so this is the one place a v1 id is
    // looked at. A corpus whose id could not name a file would migrate
    // cleanly and then refuse to open, which reads as the migration having
    // broken it.
    if !crate::id::is_path_safe_id(&v1.id) {
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
        next: None,
        updated: &v1.updated,
        notes: &mut notes,
    };
    refs.references(v1.references);
    refs.evidence(v1.evidence)?;
    refs.tasks(v1.tasks)?;
    let edges = refs.edges(v1.edges, path)?;
    let references = refs.out;

    let (status, closed) = convert_status(
        &v1.status,
        v1.closed,
        v1.graduated_to.as_deref(),
        &v1.updated,
        &mut notes,
        path,
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
    next: Option<u64>,
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
    ) -> Result<String> {
        let next = match self.next {
            Some(next) => next,
            None => model::next_reference_index(&self.out)?,
        };
        let id = format!("r{next}");
        self.next = next.checked_add(1);
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
        Ok(id)
    }

    fn evidence(&mut self, evidence: Vec<V1Evidence>) -> Result<()> {
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
            let id = self.push(ev.source, title, note, added, ev.origin.map(Into::into))?;
            self.notes
                .push(format!("evidence {} -> reference {id}", ev.id));
        }
        Ok(())
    }

    fn tasks(&mut self, tasks: Vec<V1Task>) -> Result<()> {
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
            )?;
            self.notes.push(format!("task {} -> reference {id}", t.id));
        }
        Ok(())
    }

    /// The five surviving edge kinds pass through. The evidence graph in
    /// edge form becomes a reference to the other node, so the claim
    /// survives even though the edge kind does not.
    fn edges(&mut self, edges: Vec<V1Edge>, path: &Path) -> Result<Vec<Edge>> {
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
                    )?;
                    self.notes
                        .push(format!("edge {kind} {} -> reference {id}", e.to));
                    continue;
                }
                other => {
                    return Err(Error::NotAV1EdgeType {
                        path: path.to_path_buf(),
                        edge_type: other.to_string(),
                    });
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
    path: &Path,
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
            return Err(Error::NotAV1Status {
                path: path.to_path_buf(),
                status: other.to_string(),
            });
        }
    })
}
