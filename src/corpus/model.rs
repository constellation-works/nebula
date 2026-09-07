//! The node schema.
//!
//! One markdown file per node, YAML frontmatter plus prose. Everything about a
//! node lives in that one file: edges, evidence, references, provenance and the
//! argument itself. See `docs/spec.md`, "One file, and why it holds".

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Lifecycle. The four stages of an inquiry are a status, not sub-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum Status {
    /// Vague idea, observation or corollary. Costs one line of text.
    Seed,
    /// Sharpened into something that could be wrong. Requires a kill condition.
    Hypothesis,
    /// Evidence is being gathered.
    Testing,
    /// Evidence favours it, for now. Never terminal, never certain.
    Supported,
    /// The kill condition fired.
    Refuted,
    /// Lost interest. Deliberately distinct from refuted.
    Abandoned,
    /// Promoted downstream to principia or orbit-research.
    Graduated,
}

impl Status {
    /// Statuses that must name what would falsify them.
    pub fn needs_kill(self) -> bool {
        !matches!(self, Self::Seed | Self::Abandoned)
    }

    /// Whether the inquiry is still live. Refuted, abandoned and graduated
    /// nodes stay in the graph forever but no longer ask anything of you.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Seed | Self::Hypothesis | Self::Testing | Self::Supported
        )
    }

    /// A node that has been ruled out cannot quietly return to active work.
    /// Reviving one takes a new node with a `reopens` edge, which keeps the
    /// fact that it was once dead visible in the graph.
    pub fn is_closed_by_verdict(self) -> bool {
        self == Self::Refuted
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Seed => "seed",
            Self::Hypothesis => "hypothesis",
            Self::Testing => "testing",
            Self::Supported => "supported",
            Self::Refuted => "refuted",
            Self::Abandoned => "abandoned",
            Self::Graduated => "graduated",
        };
        f.write_str(s)
    }
}

/// Edge kinds, spanning the two graphs that share one node set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum EdgeType {
    /// Genealogy: this was prompted by that.
    DerivesFrom,
    /// Genealogy: this is a sharpened successor to that.
    Refines,
    /// Genealogy: this is the broader form of that.
    Generalizes,
    /// Genealogy: this revives a refuted node.
    Reopens,
    /// Evidence: this being true makes that more likely.
    Supports,
    /// Evidence: this being true makes that less likely.
    Undermines,
    /// Dependency: if that dies, this dies with it.
    DependsOn,
    /// Mutual exclusion. Symmetric, and the checker enforces the symmetry.
    Contradicts,
}

impl EdgeType {
    /// Genealogy edges form the DAG that `trace` walks and `check` proves
    /// acyclic. The evidence graph is deliberately not constrained this way.
    pub fn is_genealogy(self) -> bool {
        matches!(
            self,
            Self::DerivesFrom | Self::Refines | Self::Generalizes | Self::Reopens
        )
    }
}

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::DerivesFrom => "derives-from",
            Self::Refines => "refines",
            Self::Generalizes => "generalizes",
            Self::Reopens => "reopens",
            Self::Supports => "supports",
            Self::Undermines => "undermines",
            Self::DependsOn => "depends-on",
            Self::Contradicts => "contradicts",
        };
        f.write_str(s)
    }
}

/// A typed link to another node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edge {
    /// The relation this edge asserts.
    #[serde(rename = "type")]
    pub kind: EdgeType,
    /// The node id on the other end.
    pub to: String,
}

/// Which way a piece of evidence cuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum Verdict {
    /// Makes the claim more likely.
    Supports,
    /// Makes the claim less likely.
    Undermines,
    /// Looked, learned nothing. Worth recording so you do not look twice.
    Inconclusive,
}

/// How much the evidence is worth. Three levels on purpose: rigor belongs
/// downstream, and finer grain here would be false precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum Strength {
    /// One observation, no control, easily fooled.
    Anecdote,
    /// Points somewhere, does not settle it.
    Suggestive,
    /// Would be hard to explain away.
    Strong,
}

/// What produced a node, an evidence entry or a reference.
///
/// Every field is optional, because plenty of ideas genuinely do arrive in the
/// shower and a provenance block that demanded filling would just go unfilled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Origin {
    /// Orbit task id, e.g. `ORB-11440`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Workspace selector, copied verbatim and never constructed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Job run id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// Task artifact key holding the output this came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// Agent family. Never hardcode one: orbit-research pinned `codex` into
    /// its task lookup and the crew names changed underneath it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// When.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Something that bears on whether the idea is true.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Unique within the node, never reused.
    pub id: String,
    /// Which way it cuts.
    pub verdict: Verdict,
    /// How much it is worth.
    pub strength: Strength,
    /// A URL, DOI, sim path, study note, screenshot or memory.
    pub source: String,
    /// When it was attached.
    pub date: String,
    /// What it actually showed, and what is shaky about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Context, never evidence.
///
/// `deny_unknown_fields` is what enforces the separation: a reference that
/// tried to carry a verdict fails to parse, so findings cannot be routed
/// through references to dodge the discipline evidence demands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// Unique within the node, never reused.
    pub id: String,
    /// `paper`, `study`, `article`, `note`, `discussion`, `book`, `dataset`,
    /// `thread` or `other`.
    pub kind: String,
    /// A URL, DOI, repo path, or almanac wikilink.
    pub uri: String,
    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Why this is attached. The only field that still matters in a year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When it was attached.
    pub added: String,
    /// Set when this reference was later promoted into evidence. The original
    /// stays put, so the reading history survives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_to: Option<String>,
    /// What produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Work this node has spawned.
///
/// The forward half of provenance, and what lets `open` find the genuinely
/// actionable gap: a hypothesis with no evidence and nothing running to get any.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLink {
    /// Orbit task id.
    pub id: String,
    /// `open`, `done` or `dropped`.
    pub state: String,
    /// What it is meant to settle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl TaskLink {
    /// Whether this task is still expected to produce something.
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
}

/// One unit of inquiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Kebab-case, permanent, never reused.
    pub id: String,
    /// One line naming the idea.
    pub title: String,
    /// Which declared domain this belongs to: `principia`, `ranking`, and so
    /// on. A view within the corpus, not a wall: edges cross domains freely.
    /// Defaults to empty on load so corpora that predate domains still open,
    /// and `check` reports the gap.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub domain: String,
    /// Where it is in its lifecycle.
    pub status: Status,
    /// When it entered the graph.
    pub created: String,
    /// When it last changed.
    pub updated: String,
    /// What would falsify this, written when the hypothesis is stated and
    /// before any evidence arrives. This is what keeps the later verdict
    /// honest instead of retroactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill: Option<String>,
    /// Free-form labels, open and multi-valued. Finer than `domain` and
    /// deliberately unvalidated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Typed links to other nodes, across both graphs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Things that bear on the truth of this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Context that does not bear on truth.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    /// Work spawned to settle this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskLink>,
    /// What produced this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// Where it went when it outgrew this system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graduated_to: Option<String>,
}

impl Node {
    /// Genealogical parents: where this idea came from.
    pub fn parents(&self) -> impl Iterator<Item = &str> {
        self.edges
            .iter()
            .filter(|e| e.kind.is_genealogy())
            .map(|e| e.to.as_str())
    }

    /// Edges of one kind.
    pub fn edges_of(&self, kind: EdgeType) -> impl Iterator<Item = &str> {
        self.edges
            .iter()
            .filter(move |e| e.kind == kind)
            .map(|e| e.to.as_str())
    }

    /// Whether any evidence carries this verdict.
    pub fn has_verdict(&self, v: Verdict) -> bool {
        self.evidence.iter().any(|e| e.verdict == v)
    }

    /// Tasks still expected to produce something.
    pub fn open_tasks(&self) -> impl Iterator<Item = &TaskLink> {
        self.tasks.iter().filter(|t| t.is_open())
    }

    /// Next free evidence id. Ids are never reused, so this counts past the
    /// highest ever issued rather than filling gaps left by removals.
    pub fn next_evidence_id(&self) -> String {
        format!(
            "ev{}",
            next_index(self.evidence.iter().map(|e| e.id.as_str()), "ev")
        )
    }

    /// Next free reference id, under the same never-reuse rule.
    pub fn next_reference_id(&self) -> String {
        format!(
            "r{}",
            next_index(self.references.iter().map(|r| r.id.as_str()), "r")
        )
    }
}

fn next_index<'a>(ids: impl Iterator<Item = &'a str>, prefix: &str) -> usize {
    ids.filter_map(|id| id.strip_prefix(prefix)?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// A node file: frontmatter plus the prose you actually wrote.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Structured fields.
    pub node: Node,
    /// The argument, the sketch, the thing you actually thought.
    pub body: String,
}

/// Parse a node file.
pub fn read(path: &Path) -> Result<Doc> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&raw).with_context(|| format!("in {}", path.display()))
}

/// Parse node text. Split out from [`read`] so it can be tested without a disk.
pub fn parse(raw: &str) -> Result<Doc> {
    let Some(rest) = raw.strip_prefix("---\n") else {
        bail!("missing YAML frontmatter (a node file starts with a `---` line)");
    };
    let Some(end) = rest.find("\n---\n") else {
        bail!("frontmatter is not terminated by a `---` line");
    };
    let node: Node = serde_yaml_ng::from_str(&rest[..end]).context("parsing frontmatter")?;
    Ok(Doc {
        node,
        body: rest[end + 5..].trim_start_matches('\n').to_string(),
    })
}

/// Serialize a node file.
pub fn render(doc: &Doc) -> Result<String> {
    let fm = serde_yaml_ng::to_string(&doc.node)?;
    let body = doc.body.trim();
    Ok(format!("---\n{fm}---\n\n{body}\n"))
}

/// Write a node file atomically.
///
/// Rendered to a sibling temporary file and renamed, so an interrupted write
/// can never leave half a node behind. Losing the tail of a thought to a
/// crashed process is exactly the failure this system exists to prevent.
pub fn write(path: &Path, doc: &Doc) -> Result<()> {
    let out = render(doc)?;
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, out).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}
