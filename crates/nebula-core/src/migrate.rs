//! Bring a v1 corpus forward to v2.
//!
//! One shot and idempotent. The v1 model below is private to this module and
//! deliberately lenient: every field is optional and unknown keys are
//! tolerated, because the point is to read whatever an old corpus holds and
//! re-label it losslessly into the v2 shape. Evidence, task links and the
//! removed edge kinds all become references, since a reference with a good
//! note carries the same information. See `docs/design/v0.2/1_spec.md`,
//! "Migration".

use crate::config::{self, Config};
use crate::error::{Error, Result};
use crate::model::{self, Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
use crate::store::{self, Corpus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

// --------------------------------------------------------------- v1 model --

#[derive(Debug, Deserialize)]
struct V1Node {
    id: String,
    title: String,
    #[serde(default)]
    domain: String,
    status: String,
    created: String,
    updated: String,
    #[serde(default)]
    kill: Option<String>,
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
    origin: Option<Origin>,
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
    origin: Option<Origin>,
}

#[derive(Debug, Deserialize)]
struct V1Reference {
    id: String,
    kind: String,
    uri: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    note: Option<String>,
    added: String,
    #[serde(default)]
    promoted_to: Option<String>,
    #[serde(default)]
    origin: Option<Origin>,
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
    refuse_dirty_tree(&root)?;

    let mut report = MigrationReport::default();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join("nodes"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    report.nodes = paths.len();
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

    report.config_rewritten = migrate_config(&root)?;
    Ok(report)
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
    let inside = Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "rev-parse",
            "--is-inside-work-tree",
        ])
        .output();
    let Ok(inside) = inside else {
        return Ok(()); // no git on PATH: nothing to protect against
    };
    if !inside.status.success() {
        return Ok(());
    }
    let status = Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "status",
            "--porcelain",
            "--",
            ".",
        ])
        .output()?;
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

/// Rewrite `config.yaml` to hold only `corpus_id` and `schema_version`.
fn migrate_config(root: &Path) -> Result<bool> {
    #[derive(Deserialize)]
    struct Lenient {
        #[serde(default)]
        corpus_id: Option<String>,
    }
    let path = root.join(config::FILE);
    let existing = if path.exists() {
        Some(std::fs::read_to_string(&path)?)
    } else {
        None
    };
    let corpus_id = existing
        .as_deref()
        .map(serde_yaml_ng::from_str::<Lenient>)
        .transpose()
        .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?
        .and_then(|l| l.corpus_id)
        .unwrap_or_else(|| store::corpus_id(root));
    let fresh = Config::fresh(corpus_id);
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
            status,
            created: v1.created,
            updated: v1.updated,
            kill: v1.kill,
            tags,
            edges,
            references,
            closed,
            origin: v1.origin,
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
                origin: r.origin,
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
            uri,
            title,
            note: Some(note),
            added,
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
            let id = self.push(ev.source, title, note, added, ev.origin);
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
            out.push(Edge { kind, to: e.to });
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
