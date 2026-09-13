//! The node schema.
//!
//! One markdown file per node, YAML frontmatter plus prose. Everything about a
//! node lives in that one file: edges, references, provenance and the
//! argument itself. See `docs/design/v0.2/1_spec.md`, "Node".
//!
//! Nothing here knows about clap. [`Status`] and [`EdgeType`] carry `FromStr`
//! and `Display`, which is all a command-line wrapper needs to parse and print
//! them, and all the desktop needs to round-trip them through JSON.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// Lifecycle. `seed → hypothesis → refuted | abandoned`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Vague idea, observation or corollary. Costs one line of text.
    Seed,
    /// Sharpened into something that could be wrong. Requires a kill condition.
    Hypothesis,
    /// The kill condition fired. Requires `closed.why`.
    Refuted,
    /// Lost interest. Deliberately distinct from refuted.
    Abandoned,
}

impl Status {
    /// Statuses that must name what would falsify them. A refuted node is
    /// included because "the kill condition fired" presumes there was one.
    pub fn needs_kill(self) -> bool {
        matches!(self, Self::Hypothesis | Self::Refuted)
    }

    /// Whether the inquiry is still live. Refuted and abandoned nodes stay in
    /// the graph forever but no longer ask anything of you.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Seed | Self::Hypothesis)
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
            Self::Refuted => "refuted",
            Self::Abandoned => "abandoned",
        };
        f.write_str(s)
    }
}

impl FromStr for Status {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "seed" => Ok(Self::Seed),
            "hypothesis" => Ok(Self::Hypothesis),
            "refuted" => Ok(Self::Refuted),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(Error::corpus(format!("`{other}` is not a status"))),
        }
    }
}

/// Edge kinds: the genealogy DAG plus one symmetric relation between ideas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "kebab-case")]
pub enum EdgeType {
    /// Genealogy: this was prompted by that.
    DerivesFrom,
    /// Genealogy: this is a sharpened successor to that.
    Refines,
    /// Genealogy: this is the broader form of that.
    Generalizes,
    /// Genealogy: this revives a refuted node.
    Reopens,
    /// Mutual exclusion. Symmetric, and the checker enforces the symmetry.
    Contradicts,
}

impl EdgeType {
    /// Genealogy edges form the DAG that `trace` walks and `check` proves
    /// acyclic. `contradicts` is a relation between ideas, not a lineage.
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
            Self::Contradicts => "contradicts",
        };
        f.write_str(s)
    }
}

impl FromStr for EdgeType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "derives-from" => Ok(Self::DerivesFrom),
            "refines" => Ok(Self::Refines),
            "generalizes" => Ok(Self::Generalizes),
            "reopens" => Ok(Self::Reopens),
            "contradicts" => Ok(Self::Contradicts),
            other => Err(Error::corpus(format!("`{other}` is not an edge type"))),
        }
    }
}

/// A typed link to another node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Edge {
    /// The relation this edge asserts.
    #[serde(rename = "type")]
    pub kind: EdgeType,
    /// The node id on the other end.
    pub to: String,
}

/// What produced a node or a reference.
///
/// Every field is optional, because plenty of ideas genuinely do arrive in the
/// shower and a provenance block that demanded filling would just go unfilled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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

impl Origin {
    /// The provenance a caller supplied, or nothing at all when it supplied
    /// neither half. An empty `origin:` block is noise in the file.
    pub fn of(task: Option<String>, run: Option<String>) -> Option<Self> {
        (task.is_some() || run.is_some()).then_some(Self {
            task,
            run,
            ..Self::default()
        })
    }
}

/// Context attached to a node. The note is the field that matters.
///
/// `deny_unknown_fields` is what enforces invariant 7: a reference that tries
/// to carry a `verdict` or `strength` fails to parse, so the truth-rating
/// ladder v0.2 removed cannot creep back in through the side door.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
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
    /// What produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Why and when a node left active work. Present only on refuted and
/// abandoned nodes; required, with a non-empty `why`, on refuted ones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
pub struct Closed {
    /// What settled it. For a refuted node, how the kill condition fired.
    pub why: String,
    /// When, as `YYYY-MM-DD`.
    pub at: String,
}

/// One unit of inquiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Kebab-case, permanent, never reused.
    pub id: String,
    /// One line naming the idea.
    pub title: String,
    /// Where it is in its lifecycle.
    pub status: Status,
    /// When it entered the graph.
    pub created: String,
    /// When it last changed.
    pub updated: String,
    /// What would falsify this, written when the hypothesis is stated and
    /// before anything is read. This is what keeps the later verdict honest
    /// instead of retroactive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill: Option<String>,
    /// Free-form labels, lowercase kebab-case, normalised on every write.
    /// No declared list: `check` warns on drift instead of walling it off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Typed links to other nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    /// Context, with a note saying why each piece is here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    /// Why and when it was closed. Only on refuted and abandoned nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Closed>,
    /// What produced this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
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

    /// Whether every one of `tags` is on this node. An empty filter matches.
    pub fn has_all_tags(&self, tags: &[String]) -> bool {
        tags.iter().all(|t| self.tags.iter().any(|x| x == t))
    }

    /// Next free reference id.
    pub fn next_reference_id(&self) -> String {
        format!("r{}", next_reference_index(&self.references))
    }
}

/// The `n` of the next free `r<n>`. Ids are never reused, so this counts
/// past the highest ever issued rather than filling gaps left by removals.
pub(crate) fn next_reference_index(refs: &[Reference]) -> usize {
    refs.iter()
        .filter_map(|r| r.id.strip_prefix('r')?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// A tag as it is written: lowercase kebab-case. `Physics` becomes `physics`,
/// `Machine Learning` becomes `machine-learning`. Applied on every write path
/// so the corpus never holds two spellings of one label.
pub fn normalize_tag(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Normalise a list of tags, dropping empties and duplicates, keeping order.
pub fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.iter().map(|t| normalize_tag(t)) {
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// A node file: frontmatter plus the prose you actually wrote.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Doc {
    /// Structured fields.
    pub node: Node,
    /// The argument, the sketch, the thing you actually thought.
    pub body: String,
}

/// Parse a node file.
pub(crate) fn read(path: &Path) -> Result<Doc> {
    let raw = std::fs::read_to_string(path)?;
    parse(&raw).map_err(|e| match e {
        Error::Yaml { context, source } => Error::Yaml {
            context: format!("in {}: {context}", path.display()),
            source,
        },
        other => other,
    })
}

/// Split node text into its YAML frontmatter and its prose.
pub(crate) fn split_frontmatter(raw: &str) -> Result<(&str, &str)> {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return Err(Error::corpus(
            "missing YAML frontmatter (a node file starts with a `---` line)",
        ));
    };
    let Some(end) = rest.find("\n---\n") else {
        return Err(Error::corpus(
            "frontmatter is not terminated by a `---` line",
        ));
    };
    Ok((&rest[..end], rest[end + 5..].trim_start_matches('\n')))
}

/// Parse node text. Split out from [`read`] so it can be tested without a disk.
pub(crate) fn parse(raw: &str) -> Result<Doc> {
    let (front, body) = split_frontmatter(raw)?;
    let node: Node =
        serde_yaml_ng::from_str(front).map_err(|e| Error::yaml("parsing frontmatter", e))?;
    Ok(Doc {
        node,
        body: body.to_string(),
    })
}

/// Serialize a node file.
pub(crate) fn render(doc: &Doc) -> Result<String> {
    let fm = serde_yaml_ng::to_string(&doc.node)
        .map_err(|e| Error::yaml(format!("rendering `{}`", doc.node.id), e))?;
    let body = doc.body.trim();
    Ok(format!("---\n{fm}---\n\n{body}\n"))
}

/// Write a node file atomically.
///
/// Rendered to a sibling temporary file and renamed, so an interrupted write
/// can never leave half a node behind. Losing the tail of a thought to a
/// crashed process is exactly the failure this system exists to prevent.
pub(crate) fn write(path: &Path, doc: &Doc) -> Result<()> {
    let out = render(doc)?;
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_lowercase_kebab_case() {
        assert_eq!(normalize_tag("Physics"), "physics");
        assert_eq!(normalize_tag("Machine Learning"), "machine-learning");
        assert_eq!(normalize_tag("foo_bar--baz "), "foo-bar-baz");
        assert_eq!(normalize_tag("  "), "");
        assert_eq!(
            normalize_tags(&["A".into(), "a".into(), String::new(), "B c".into()]),
            vec!["a", "b-c"]
        );
    }

    #[test]
    fn statuses_and_edge_types_round_trip_through_strings() {
        for s in [
            Status::Seed,
            Status::Hypothesis,
            Status::Refuted,
            Status::Abandoned,
        ] {
            assert_eq!(s.to_string().parse::<Status>().unwrap(), s);
        }
        for e in [
            EdgeType::DerivesFrom,
            EdgeType::Refines,
            EdgeType::Generalizes,
            EdgeType::Reopens,
            EdgeType::Contradicts,
        ] {
            assert_eq!(e.to_string().parse::<EdgeType>().unwrap(), e);
        }
        assert!("graduated".parse::<Status>().is_err());
        assert!("supports".parse::<EdgeType>().is_err());
    }
}
