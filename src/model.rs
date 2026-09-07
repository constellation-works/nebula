//! The node schema. One markdown file with YAML frontmatter per node.
//!
//! Everything about a node lives in its file: edges, evidence, references,
//! provenance and prose. See `docs/spec.md`, "One file, and why it holds".

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Lifecycle. The four stages of an inquiry are a status, not sub-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Vague idea, observation or corollary. Costs one line of text.
    Seed,
    /// Sharpened into something that could be wrong. Requires `kill`.
    Hypothesis,
    /// Evidence is being gathered.
    Testing,
    /// Evidence favours it, for now. Never terminal, never certain.
    Supported,
    /// The kill condition fired.
    Refuted,
    /// Lost interest. Distinct from refuted on purpose.
    Abandoned,
    /// Promoted downstream to principia or orbit-research.
    Graduated,
}

impl Status {
    /// Statuses at or past `hypothesis`, which must name what would kill them.
    pub fn needs_kill(self) -> bool {
        !matches!(self, Status::Seed | Status::Abandoned)
    }
}

/// Edge kinds, across the two graphs that share one node set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeType {
    // Genealogy: "where did this come from". Enforced acyclic.
    DerivesFrom,
    Refines,
    Generalizes,
    Reopens,
    // Dependency and evidence: "what breaks if this dies". Cycles allowed.
    Supports,
    Undermines,
    DependsOn,
    Contradicts,
}

impl EdgeType {
    /// Genealogy edges form the DAG that `trace` walks and `check` proves acyclic.
    pub fn is_genealogy(self) -> bool {
        matches!(
            self,
            EdgeType::DerivesFrom | EdgeType::Refines | EdgeType::Generalizes | EdgeType::Reopens
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    #[serde(rename = "type")]
    pub kind: EdgeType,
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Supports,
    Undermines,
    Inconclusive,
}

/// Deliberately coarse. Rigor is downstream's job; finer grain here would be
/// false precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Anecdote,
    Suggestive,
    Strong,
}

/// What produced a node, an evidence entry or a reference. All fields
/// optional, because plenty of ideas genuinely do arrive in the shower.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Origin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// Never hardcode a family here. orbit-research pinned `codex` into its
    /// task lookup and the crew names changed underneath it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub verdict: Verdict,
    pub strength: Strength,
    pub source: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Context, never evidence. `deny_unknown_fields` is what enforces invariant
/// 10: a reference that tried to carry a verdict fails to parse, so the
/// discipline cannot be dodged by routing findings through references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub id: String,
    pub kind: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Why this is attached. The only field that still matters in a year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub added: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Work this node has spawned. The forward half of provenance, and what lets
/// `open` find a hypothesis with no evidence and no task running against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLink {
    pub id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub created: String,
    pub updated: String,
    /// What would falsify this. Written when the hypothesis is stated, before
    /// any evidence arrives, which is what keeps the verdict honest later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graduated_to: Option<String>,
}

impl Node {
    pub fn parents(&self) -> impl Iterator<Item = &str> {
        self.edges
            .iter()
            .filter(|e| e.kind.is_genealogy())
            .map(|e| e.to.as_str())
    }
}

/// A node file: frontmatter plus the prose you actually wrote.
pub struct Doc {
    pub node: Node,
    pub body: String,
}

pub fn read(path: &Path) -> Result<Doc> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let rest = raw
        .strip_prefix("---\n")
        .ok_or_else(|| anyhow::anyhow!("{}: missing YAML frontmatter", path.display()))?;
    let Some(end) = rest.find("\n---\n") else {
        bail!("{}: frontmatter is not terminated", path.display());
    };
    let node: Node = serde_yaml_ng::from_str(&rest[..end])
        .with_context(|| format!("parsing frontmatter of {}", path.display()))?;
    Ok(Doc {
        node,
        body: rest[end + 5..].to_string(),
    })
}

pub fn write(path: &Path, doc: &Doc) -> Result<()> {
    let fm = serde_yaml_ng::to_string(&doc.node)?;
    let out = format!("---\n{fm}---\n\n{}", doc.body.trim_start());
    // Write beside the target then rename, so an interrupted write can never
    // leave a half-serialized node behind.
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
