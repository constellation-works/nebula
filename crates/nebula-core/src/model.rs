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
use crate::fs_impl::write_private_atomic;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;
use time::{Date, format_description::well_known::Iso8601};

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
        match self {
            Self::Seed | Self::Abandoned => false,
            Self::Hypothesis | Self::Refuted => true,
        }
    }

    /// Whether the inquiry is still live. Refuted and abandoned nodes stay in
    /// the graph forever but no longer ask anything of you.
    pub fn is_open(self) -> bool {
        match self {
            Self::Seed | Self::Hypothesis => true,
            Self::Refuted | Self::Abandoned => false,
        }
    }

    /// A node that has been ruled out cannot quietly return to active work.
    /// Reviving one takes a new node with a `reopens` edge, which keeps the
    /// fact that it was once dead visible in the graph.
    pub fn is_closed_by_verdict(self) -> bool {
        match self {
            Self::Refuted => true,
            Self::Seed | Self::Hypothesis | Self::Abandoned => false,
        }
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
        // `pad`, not `write_str`: a caller's width and fill (`{s:<10}` for a
        // column) are part of the format, and `write_str` drops them.
        f.pad(s)
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
            other => Err(Error::NotAStatus(other.to_string())),
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
        match self {
            Self::DerivesFrom | Self::Refines | Self::Generalizes | Self::Reopens => true,
            Self::Contradicts => false,
        }
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
        // `pad` for the same reason as [`Status`]'s.
        f.pad(s)
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
            other => Err(Error::NotAnEdgeType(other.to_string())),
        }
    }
}

/// A typed link to another node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// The relation this edge asserts.
    #[serde(rename = "type")]
    pub kind: EdgeType,
    /// The node id on the other end.
    pub to: String,
    /// Who claimed the relation. Absent is [`HUMAN`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// The author of anything nobody attributed: the human whose corpus this is.
///
/// Stored by omission. A field with no author reads as the human's own, which
/// is what every node written before authorship existed is, and is why
/// `neb migrate` has nothing to do about it.
pub const HUMAN: &str = "human";

/// Whether a stored author label means the human.
///
/// `None`, an empty label and `human` are the same answer, so a hand-written
/// `by: human` reads the same as leaving the line out.
pub(crate) fn is_human(by: Option<&str>) -> bool {
    by.is_none_or(|b| {
        let b = b.trim();
        b.is_empty() || b == HUMAN
    })
}

/// A `--by` label as it is stored: [`None`] for the human, the trimmed label
/// otherwise.
///
/// The label is free text on purpose — a session id, a crew name, whatever
/// identifies the writer — and nothing here knows an agent family. What it
/// cannot hold is the punctuation a note line uses to carry its author in the
/// prose, since a note is markdown rather than YAML and has to parse back.
pub(crate) fn author(by: Option<&str>) -> Result<Option<String>> {
    if is_human(by) {
        return Ok(None);
    }
    let label = by.unwrap_or_default().trim();
    if label.contains(['(', ')', '\n']) || label.contains(": ") {
        return Err(Error::InvalidAuthorLabel(label.to_string()));
    }
    Ok(Some(label.to_string()))
}

/// What produced a node or a reference.
///
/// Every field is optional, because plenty of ideas genuinely do arrive in the
/// shower and a provenance block that demanded filling would just go unfilled.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
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
    /// `thread`, `observatory` or `other`, lowercase as `cite` writes it.
    pub kind: String,
    /// A URL, DOI, repo path, or almanac wikilink. Discussions may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Human-readable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Why this is attached. The only field that still matters in a year.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// When it was attached.
    pub added: String,
    /// Who attached it and wrote the note. Absent is [`HUMAN`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
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

/// How `closed.why` begins on a node handed off to an Observatory record;
/// the record id follows. See [`handoff_why`] and [`Node::handed_off_to`].
const HANDED_OFF_TO: &str = "handed off to ";

/// The `closed.why` of a node handed off to Observatory record `record`:
/// `handed off to H012`.
pub(crate) fn handoff_why(record: &str) -> String {
    format!("{HANDED_OFF_TO}{record}")
}

/// One unit of inquiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Kebab-case, permanent, never reused. Also the name of the file the
    /// node lives in, which is why an id that could not name one is refused
    /// when the file is parsed.
    pub id: String,
    /// One line naming the idea.
    pub title: String,
    /// Who wrote that line. Absent is [`HUMAN`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_by: Option<String>,
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
    /// Who wrote the kill condition. Absent is [`HUMAN`]. A kill somebody
    /// else proposed is a claim the human has not yet made: `review` lists
    /// it until one is confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_by: Option<String>,
    /// Free-form labels, lowercase kebab-case, normalised on every write.
    /// No declared list: a write notes drift and `check` warns on it instead
    /// of walling it off.
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

    /// Genealogical parents, each once, with every kind of edge naming it.
    ///
    /// Two edges to one parent (a `derives-from` and a later `reopens`) are
    /// parallel, not a diamond: one relation between the pair, stated twice.
    /// Parents keep the order of their first edge, and kinds the order they
    /// were declared in.
    pub fn lineage(&self) -> Vec<(&str, Vec<EdgeType>)> {
        let mut out: Vec<(&str, Vec<EdgeType>)> = Vec::new();
        for e in self.edges.iter().filter(|e| e.kind.is_genealogy()) {
            match out.iter_mut().find(|(to, _)| *to == e.to) {
                Some((_, kinds)) if kinds.contains(&e.kind) => {}
                Some((_, kinds)) => kinds.push(e.kind),
                None => out.push((e.to.as_str(), vec![e.kind])),
            }
        }
        out
    }

    /// Edges of one kind.
    pub fn edges_of(&self, kind: EdgeType) -> impl Iterator<Item = &str> {
        self.edges
            .iter()
            .filter(move |e| e.kind == kind)
            .map(|e| e.to.as_str())
    }

    /// Whether this node already declares an edge of `kind` to `to`.
    ///
    /// Authorship is not part of a link's identity: the same claim written
    /// twice is one claim, whoever wrote it the second time.
    pub fn has_edge(&self, kind: EdgeType, to: &str) -> bool {
        self.edges.iter().any(|e| e.kind == kind && e.to == to)
    }

    /// This node with every authorship field stated outright.
    ///
    /// The corpus stores the human by omission, because a node written before
    /// authorship existed has to read the same as one written after. A reader
    /// of `--json` should not have to know that rule, so the queries fill the
    /// default in before serializing.
    #[must_use]
    pub fn with_authorship_stated(mut self) -> Self {
        fn state(by: &mut Option<String>) {
            if is_human(by.as_deref()) {
                *by = Some(HUMAN.to_string());
            }
        }
        state(&mut self.title_by);
        if self.kill.is_some() {
            state(&mut self.kill_by);
        }
        for e in &mut self.edges {
            state(&mut e.by);
        }
        for r in &mut self.references {
            state(&mut r.by);
        }
        self
    }

    /// Whether every one of `tags` is on this node. An empty filter matches.
    pub fn has_all_tags(&self, tags: &[String]) -> bool {
        tags.iter().all(|t| self.tags.iter().any(|x| x == t))
    }

    /// The Observatory record this node was handed off to, if it was.
    ///
    /// Read from what the hand-off wrote rather than from a field of its
    /// own, since the schema has none: an `abandoned` node whose `closed.why`
    /// is exactly [`handoff_why`] of a record, and which carries an
    /// `observatory` reference to that same record. Both halves must agree,
    /// so a reason that merely mentions a record is not a hand-off.
    pub fn handed_off_to(&self) -> Option<&str> {
        if self.status != Status::Abandoned {
            return None;
        }
        let record = self.closed.as_ref()?.why.strip_prefix(HANDED_OFF_TO)?;
        self.references
            .iter()
            .any(|r| r.kind == crate::check_impl::OBSERVATORY && r.uri.as_deref() == Some(record))
            .then_some(record)
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
pub(crate) fn normalize_tag(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase().filter(|lower| lower.is_alphanumeric()));
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Normalise a list of tags, dropping empties and duplicates, keeping order.
pub(crate) fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw.iter().map(|t| normalize_tag(t)) {
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// A dated paragraph of reasoning appended to a node's body.
///
/// Notes live in the prose, under a `## Notes` section, not in the
/// frontmatter. This type is the parsed projection that `show --json`
/// exposes; the file itself is still markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Note {
    /// When it was written, as `YYYY-MM-DD`.
    pub at: String,
    /// The reasoning, in the author's words.
    pub text: String,
    /// Who wrote it. A projection rather than a stored field, so this is
    /// always stated: the human's line simply does not name an author.
    pub by: String,
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
///
/// Only a regular file is read: a symlink, a FIFO, a device or a directory at
/// `path` is [`Error::NotRegularFile`] before a byte is read from it.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "all other parsing errors pass through unchanged"
)]
pub(crate) fn read(path: &Path) -> Result<Doc> {
    let raw = crate::fs_impl::read_regular_text(path)?
        .ok_or_else(|| Error::io_at("reading", path, std::io::ErrorKind::NotFound.into()))?;
    parse(&raw, path).map_err(|e| match e {
        Error::Yaml { context, source } => Error::Yaml {
            context: format!("in {}: {context}", path.display()),
            source,
        },
        // An id out of a *file* is reported against that file: a name that
        // could not be one is a disagreement with the file it was found in,
        // and the path is what the human needs to go and look at.
        Error::UnsafeId(id) => Error::IdMismatch {
            path: path.to_path_buf(),
            id,
        },
        other => other,
    })
}

/// Which `---` delimiter a node file's frontmatter is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterProblem {
    /// The file does not start with a `---` line.
    Missing,
    /// No `---` line closes the frontmatter.
    Unterminated,
}

impl fmt::Display for FrontmatterProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "missing YAML frontmatter (a node file starts with a `---` line)",
            Self::Unterminated => "frontmatter is not terminated by a `---` line",
        })
    }
}

/// Split node text into its YAML frontmatter and its prose. `path` is the
/// file the text came from, which a refusal names.
pub(crate) fn split_frontmatter<'a>(raw: &'a str, path: &Path) -> Result<(&'a str, &'a str)> {
    let malformed = |problem| Error::MalformedFrontmatter {
        path: path.to_path_buf(),
        problem,
    };
    let Some(rest) = raw.strip_prefix("---\n") else {
        return Err(malformed(FrontmatterProblem::Missing));
    };
    let Some(end) = rest.find("\n---\n") else {
        return Err(malformed(FrontmatterProblem::Unterminated));
    };
    Ok((&rest[..end], rest[end + 5..].trim_start_matches('\n')))
}

/// Parse node text. Split out from [`read`] so it can be tested without a
/// disk; `path` is where the text came from, which a refusal names.
///
/// The id is checked here, at the door, the same placement rule 7 gets and
/// for the same reason: an id decides which file a later write lands in, so
/// text carrying `../../escaped` has to fail to parse rather than produce a
/// `Doc` that every verb downstream would treat as a node. A check at the
/// point of action alone would have to be repeated in every verb, and the one
/// that was forgotten would be the one that wrote outside the corpus.
pub(crate) fn parse(raw: &str, path: &Path) -> Result<Doc> {
    let (front, body) = split_frontmatter(raw, path)?;
    let node: Node =
        serde_yaml_ng::from_str(front).map_err(|e| Error::yaml("parsing frontmatter", e))?;
    // "Ids stay strings, checked where they become paths" (4_decisions.md, STD-02@2 §R14).
    if !crate::store::is_path_safe_id(&node.id) {
        return Err(Error::UnsafeId(node.id));
    }
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

const NOTES_HEADING: &str = "## Notes";

/// Append `- YYYY-MM-DD: text` under a `## Notes` section at the end of
/// `body`. Existing body text is never rewritten: if a notes section already
/// closes the file, the new line is added after the last entry; otherwise a
/// section is created at the end.
///
/// A note nobody but the human wrote carries its author inline,
/// `- YYYY-MM-DD (label): text`. The human's own line keeps the shape it has
/// always had, so the prose does not fill with attributions of the obvious.
pub(crate) fn append_note(body: &str, date: &str, text: &str, by: Option<&str>) -> String {
    let entry = match by {
        Some(by) => format!("- {date} ({by}): {text}"),
        None => format!("- {date}: {text}"),
    };
    let body = body.trim_end();
    if has_terminal_notes_section(body) {
        format!("{body}\n{entry}")
    } else if body.is_empty() {
        format!("{NOTES_HEADING}\n\n{entry}")
    } else {
        format!("{body}\n\n{NOTES_HEADING}\n\n{entry}")
    }
}

fn has_terminal_notes_section(body: &str) -> bool {
    notes_section_ranges(body)
        .last()
        .is_some_and(|section| section.end == body.len())
}

/// Byte ranges of every `## Notes` section, in the order they appear. A
/// section runs from its heading to the next `## ` heading, or to the end of
/// the body when nothing closes it.
///
/// There can be more than one: [`append_note`] never rewrites earlier body
/// text, so a note landing on a body whose notes are followed by another
/// section opens a fresh section at the end rather than reaching back. Every
/// one of them holds reasoning somebody wrote, so nothing here may look at
/// the last section alone.
fn notes_section_ranges(body: &str) -> Vec<Range<usize>> {
    let mut sections: Vec<Range<usize>> = Vec::new();
    let mut open: Option<usize> = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let opens = content == NOTES_HEADING;
        // A `## ` heading closes the section it follows, its own included:
        // two notes sections in a row are two sections. A deeper heading
        // does not, so `### ` subheadings stay inside the notes they head.
        let closes = opens || content.starts_with("## ");
        if closes && let Some(start) = open.take() {
            sections.push(start..offset);
        }
        if opens {
            open = Some(offset);
        }
        offset += line.len();
    }
    if let Some(start) = open {
        sections.push(start..body.len());
    }
    sections
}

/// Every `## Notes` section in a body, heading included, in the order they
/// appear.
///
/// Public because the CLI's editor flow has to refuse an edit that touches
/// any of them, and the rule for where a section starts and ends belongs
/// beside the code that writes one.
#[must_use]
pub(crate) fn notes_sections(body: &str) -> Vec<&str> {
    notes_section_ranges(body)
        .into_iter()
        .map(|section| &body[section])
        .collect()
}

/// Notes from every `## Notes` section, oldest first. Lines that are not
/// `- YYYY-MM-DD: text` are ignored, so hand-written asides stay asides.
pub(crate) fn notes_from_body(body: &str) -> Vec<Note> {
    notes_sections(body)
        .into_iter()
        .flat_map(|section| section.lines().filter_map(parse_note_line))
        .collect()
}

fn parse_note_line(line: &str) -> Option<Note> {
    let rest = line.strip_prefix("- ")?;
    let (head, text) = rest.split_once(": ")?;
    let (at, by) = match head.split_once(" (") {
        Some((at, by)) => (at, by.strip_suffix(')')?),
        None => (head, HUMAN),
    };
    if !is_iso_date(at) {
        return None;
    }
    Some(Note {
        at: at.to_string(),
        text: text.to_string(),
        by: by.to_string(),
    })
}

pub(crate) fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                true
            } else {
                c.is_ascii_digit()
            }
        })
        && Date::parse(s, &Iso8601::DATE).is_ok()
}

/// Write a node file atomically.
///
/// Rendered to a sibling temporary file and renamed, so an interrupted write
/// can never leave half a node behind. Losing the tail of a thought to a
/// crashed process is exactly the failure this system exists to prevent.
pub(crate) fn write(path: &Path, doc: &Doc) -> Result<()> {
    let out = render(doc)?;
    write_private_atomic(path, out)
}
