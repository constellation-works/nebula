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
pub fn is_human(by: Option<&str>) -> bool {
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
pub fn author(by: Option<&str>) -> Result<Option<String>> {
    if is_human(by) {
        return Ok(None);
    }
    let label = by.unwrap_or_default().trim();
    if label.contains(['(', ')', '\n']) || label.contains(": ") {
        return Err(Error::corpus(format!(
            "`{label}` cannot be an author label: no parentheses, newlines or `: `"
        )));
    }
    Ok(Some(label.to_string()))
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

/// One unit of inquiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Kebab-case, permanent, never reused.
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
    let Some(idx) = last_notes_heading(body) else {
        return false;
    };
    let after = &body[idx + NOTES_HEADING.len()..];
    !after.lines().skip(1).any(|line| line.starts_with("## "))
}

fn last_notes_heading(body: &str) -> Option<usize> {
    let mut found = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content == NOTES_HEADING {
            found = Some(offset);
        }
        offset += line.len();
    }
    found
}

/// Notes from the last `## Notes` section, oldest first. Lines that are not
/// `- YYYY-MM-DD: text` are ignored, so hand-written asides stay asides.
pub(crate) fn notes_from_body(body: &str) -> Vec<Note> {
    let Some(idx) = last_notes_heading(body) else {
        return Vec::new();
    };
    let after = &body[idx + NOTES_HEADING.len()..];
    after.lines().filter_map(parse_note_line).collect()
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

fn is_iso_date(s: &str) -> bool {
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

    #[test]
    fn notes_accumulate_in_order_and_leave_earlier_body_alone() {
        let first = append_note("the original capture", "2026-09-21", "first thought", None);
        assert_eq!(
            first,
            "the original capture\n\n## Notes\n\n- 2026-09-21: first thought"
        );
        let second = append_note(&first, "2026-09-21", "second thought", None);
        assert_eq!(
            second,
            "the original capture\n\n## Notes\n\n- 2026-09-21: first thought\n- 2026-09-21: second thought"
        );
        assert!(
            second.starts_with("the original capture"),
            "the capture itself is not rewritten"
        );
        assert_eq!(
            notes_from_body(&second),
            vec![
                Note {
                    at: "2026-09-21".into(),
                    text: "first thought".into(),
                    by: HUMAN.into(),
                },
                Note {
                    at: "2026-09-21".into(),
                    text: "second thought".into(),
                    by: HUMAN.into(),
                },
            ]
        );
    }

    #[test]
    fn an_empty_body_still_gets_a_notes_section() {
        let body = append_note("", "2026-09-21", "alone", None);
        assert_eq!(body, "## Notes\n\n- 2026-09-21: alone");
        assert_eq!(
            notes_from_body(&body),
            vec![Note {
                at: "2026-09-21".into(),
                text: "alone".into(),
                by: HUMAN.into(),
            }]
        );
    }

    /// A note somebody else wrote names them in the line, and reads back as
    /// theirs; the human's line is untouched and reads back as the human's.
    #[test]
    fn a_note_carries_its_author_in_the_line_only_when_it_is_not_the_human() {
        let mine = append_note("", "2026-09-21", "my reasoning", None);
        let ours = append_note(&mine, "2026-09-21", "its reasoning", Some("crew-alpha"));
        assert_eq!(
            ours,
            "## Notes\n\n- 2026-09-21: my reasoning\n- 2026-09-21 (crew-alpha): its reasoning"
        );
        let notes = notes_from_body(&ours);
        assert_eq!(notes[0].by, HUMAN);
        assert_eq!(notes[1].by, "crew-alpha");
        assert_eq!(notes[1].text, "its reasoning");
    }

    #[test]
    fn an_author_label_is_free_text_but_cannot_break_a_note_line() {
        assert_eq!(author(None).unwrap(), None);
        assert_eq!(author(Some(" human ")).unwrap(), None);
        assert_eq!(author(Some("  ")).unwrap(), None);
        assert_eq!(
            author(Some("agent:session-7d2")).unwrap(),
            Some("agent:session-7d2".into())
        );
        assert!(author(Some("crew (alpha)")).is_err());
        assert!(author(Some("crew: alpha")).is_err());
        assert!(is_human(None) && is_human(Some("human")));
        assert!(!is_human(Some("agent:session-7d2")));
    }

    #[test]
    fn a_later_heading_gets_a_fresh_notes_section_at_the_end() {
        let body = append_note("intro\n\n## Next\n\ndo x", "2026-09-21", "why", None);
        assert_eq!(
            body,
            "intro\n\n## Next\n\ndo x\n\n## Notes\n\n- 2026-09-21: why"
        );
        assert_eq!(notes_from_body(&body)[0].text, "why");
    }
}
