//! The invariant checker.
//!
//! This is the lock, the same role `check-theory.py` plays in principia. A
//! schema is a suggestion until something refuses to accept a corpus that
//! violates it, and the invariants here are the ones that keep the record
//! honest rather than merely tidy. The rule IDs below are the source for the
//! tables in the v0.2 spec, the lineage spec, and the agent skill.
//!
//! The checker reports; it never fixes and never prints. What an error costs
//! the caller — an exit code, a red badge — is the caller's business.

use crate::error::Result;
use crate::graph::Graph;
use crate::model::{Doc, EdgeType, Status, is_iso_date};
use crate::store::Corpus;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// Stable IDs for corpus invariants. Some rules are enforced before a graph
/// reaches `check`, at parsing or at the point of action.
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum Rule {
    AcyclicGenealogy = 1,
    HypothesisKill = 2,
    EdgeTargets = 3,
    MutualContradicts = 4,
    RefutedReason = 5,
    RefutedReopens = 6,
    NoReferenceVerdict = 7,
    LocalReference = 8,
    ObservatoryReference = 9,
    ReferenceNote = 10,
    TagDrift = 11,
    OpenNodeClosed = 12,
    SeedKill = 13,
    Dates = 14,
    NodeId = 15,
    ReferenceKind = 16,
}

/// How badly a finding breaks the corpus.
///
/// Serialized under the key `level`, which is the name `neb check --json` has
/// always used for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The corpus is inconsistent. `check` exits non-zero.
    Error,
    /// Worth your attention, not worth blocking on.
    Warn,
}

/// One violated invariant.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Finding {
    /// Error or warning.
    pub level: Severity,
    /// Which invariant, by its number in `docs/design/v0.2/1_spec.md`.
    pub rule: u8,
    /// The node it belongs to, where there is one.
    pub node: Option<String>,
    /// What is wrong.
    pub message: String,
}

/// The result of a full check.
#[derive(Debug, Default, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Report {
    /// Everything found, errors first.
    pub findings: Vec<Finding>,
    /// How many nodes were examined.
    pub nodes: usize,
}

impl Report {
    fn push(
        &mut self,
        level: Severity,
        rule: Rule,
        node: Option<&str>,
        message: impl Into<String>,
    ) {
        self.findings.push(Finding {
            level,
            rule: rule as u8,
            node: node.map(String::from),
            message: message.into(),
        });
    }
}

/// Run every invariant over a graph.
///
/// The corpus is needed for rules 8 and 9, which ask the filesystem whether
/// local references and Observatory records resolve.
///
/// Rule 7 never reaches this function: a reference carrying a `verdict` or
/// `strength`, or a node with an unknown field, fails to deserialize, so the
/// corpus load itself errors out rather than producing a finding.
pub fn run(graph: &Graph<'_>, corpus: &Corpus) -> Result<Report> {
    let docs = graph.docs();
    let ids: HashSet<&str> = docs.iter().map(|d| d.node.id.as_str()).collect();
    let mut r = Report {
        nodes: docs.len(),
        ..Report::default()
    };
    let observatory = corpus.observatory_root().root;

    for doc in docs {
        check_node(doc, &ids, corpus, observatory.as_deref(), &mut r);
    }

    // 4. `contradicts` is a claim about both nodes, so a one-sided declaration
    //    is a half-recorded fact that will read as settled later.
    for doc in docs {
        for target in doc.node.edges_of(EdgeType::Contradicts) {
            let mutual = graph.get(target).is_some_and(|d| {
                d.node
                    .edges_of(EdgeType::Contradicts)
                    .any(|b| b == doc.node.id)
            });
            if !mutual {
                r.push(
                    Severity::Error,
                    Rule::MutualContradicts,
                    Some(&doc.node.id),
                    format!("contradicts `{target}`, which does not contradict back"),
                );
            }
        }
    }

    // 1. Genealogy must be acyclic: an idea cannot be its own ancestor.
    //    Diamonds are legal; only a loop is not.
    if let Some(cycle) = graph.cycle() {
        r.push(
            Severity::Error,
            Rule::AcyclicGenealogy,
            None,
            format!("genealogy cycle: {}", cycle.join(" -> ")),
        );
    }

    // 11. Two tags that differ only by case or a trailing `s` are one label
    //     drifting into two. Writes normalise case, so this mostly catches
    //     hand edits and plurals; a warning keeps drift visible without a
    //     declared list to maintain.
    let tags: BTreeSet<&str> = docs
        .iter()
        .flat_map(|d| d.node.tags.iter().map(String::as_str))
        .collect();
    let tags: Vec<&str> = tags.into_iter().collect();
    for (i, a) in tags.iter().enumerate() {
        for b in &tags[i + 1..] {
            if let Some(how) = tag_drift(a, b) {
                r.push(
                    Severity::Warn,
                    Rule::TagDrift,
                    None,
                    format!("tags `{a}` and `{b}` differ only by {how}"),
                );
            }
        }
    }

    r.findings
        .sort_by_key(|f| (f.level != Severity::Error, f.rule));
    Ok(r)
}

/// How two distinct tags collide, if they do.
fn tag_drift(a: &str, b: &str) -> Option<&'static str> {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a == b {
        return Some("case");
    }
    let plural = |x: &str, y: &str| x.strip_suffix('s') == Some(y);
    if plural(&a, &b) || plural(&b, &a) {
        return Some("a trailing `s`");
    }
    None
}

// ------------------------------------------------------------- node rules --

/// Whether a URI is a path to resolve against `nodes/`, as opposed to a URL,
/// a wikilink, or a scheme-prefixed handle such as `doi:` or `orbit:`.
pub fn is_local_path(uri: &str) -> bool {
    !uri.contains("://") && !uri.starts_with("[[") && !has_scheme(uri)
}

/// `scheme:` with at least two leading letters, so a Windows drive letter
/// does not count and `doi:`, `orbit:`, `neb:`, `mailto:` all do.
fn has_scheme(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    })
}

/// Whether a URI names a place on one machine's filesystem rather than a
/// path relative to `nodes/`: a `file:` URI, a path from the root (`/etc/x`,
/// `\\server\share\x`), or a drive-letter path (`C:/x`, `C:\x`).
///
/// Judged as written and alike on every platform, never through
/// [`Path::is_absolute`], which answers differently on each: the corpus is
/// synced between machines, so a path that is absolute on any one of them
/// is machine layout on all of them.
pub fn is_absolute_local(uri: &str) -> bool {
    let file_uri = uri
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"));
    let rooted = uri.starts_with(['/', '\\']);
    let drive = matches!(uri.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    file_uri || rooted || drive
}

/// Where a local reference URI lands: relative to the `nodes/` directory,
/// used as given and never canonicalized (macOS temp dirs sit under a
/// symlink, and resolving would pass on one platform and fail on the other).
pub(crate) fn resolve_local(corpus: &Corpus, uri: &str) -> PathBuf {
    corpus.root().join("nodes").join(Path::new(uri))
}

/// The reference kind whose `uri` is a bare Observatory record id rather
/// than a location: `Q002`, `H007`, `T003`, `R012`. Where the record is on
/// this machine is the corpus's `observatory_root` setting, so the reference
/// itself carries nothing machine-specific.
pub const OBSERVATORY: &str = "observatory";

/// The closed vocabulary accepted for new references. The model deliberately
/// keeps `kind` as a string so older, hand-edited corpora still load; `check`
/// reports values outside this list instead of turning them into parse errors.
pub const REFERENCE_KINDS: [&str; 10] = [
    "paper",
    "study",
    "article",
    "note",
    "discussion",
    "book",
    "dataset",
    "thread",
    OBSERVATORY,
    "other",
];

/// Whether a reference kind belongs to the vocabulary accepted for new writes.
pub fn is_reference_kind(kind: &str) -> bool {
    REFERENCE_KINDS.contains(&kind)
}

/// Whether `id` has the shape of an Observatory record id: one of `Q`, `H`,
/// `T`, `R` followed by digits. The shape is the whole contract; how many
/// digits Observatory uses is its business.
pub fn is_observatory_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| observatory_dir(c).is_some())
        && !chars.as_str().is_empty()
        && chars.all(|c| c.is_ascii_digit())
}

/// The Observatory directory a record id's letter files it under. Research
/// layout v2: questions, hypotheses and theories are files, research
/// records are directories.
fn observatory_dir(letter: char) -> Option<&'static str> {
    match letter {
        'Q' => Some("questions"),
        'H' => Some("hypotheses"),
        'T' => Some("theories"),
        'R' => Some("research"),
        _ => None,
    }
}

/// Where an Observatory record is under `root`: the entry of the id's
/// directory whose name is the id, or the id followed by `-` or `.`, so
/// `Q002` finds `questions/Q002-is-proper-time-a-count.md` without the
/// reference having to know the slug. `None` when the id has the wrong
/// shape, the directory is unreadable, or nothing there starts with it.
///
/// Matched by prefix in a listing rather than by resolving a path, and used
/// as given: nothing is canonicalized. Ties (two records claiming one id)
/// go to the first in name order, which `check` in Observatory is the place
/// to catch.
pub fn resolve_observatory(root: &Path, id: &str) -> Option<PathBuf> {
    if !is_observatory_id(id) {
        return None;
    }
    let dir = root.join(observatory_dir(id.chars().next()?)?);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.']))
        })
        .collect();
    names.sort();
    names.into_iter().next().map(|name| dir.join(name))
}

/// Every invariant that can be judged from a single node plus the id set.
fn check_node(
    doc: &Doc,
    ids: &HashSet<&str>,
    corpus: &Corpus,
    observatory: Option<&Path>,
    r: &mut Report,
) {
    status_rules(doc, r);
    lifecycle_rules(doc, r);
    date_rules(doc, r);
    edge_rules(doc, ids, r);
    reference_rules(doc, corpus, observatory, r);
}

/// Rules about where a node sits in its lifecycle and what that costs.
fn status_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 2. A hypothesis must say what would kill it, written before anything
    //    is read. Without this, the verdict is retroactive rationalisation.
    if n.status == Status::Hypothesis && n.kill.as_ref().is_none_or(|k| k.trim().is_empty()) {
        r.push(
            Severity::Error,
            Rule::HypothesisKill,
            id,
            "status is hypothesis but no kill condition is named",
        );
    }

    // 5. A refuted node says how its kill condition fired. The reference
    //    that convinced you goes in `references`; the sentence goes here.
    if n.status == Status::Refuted && n.closed.as_ref().is_none_or(|c| c.why.trim().is_empty()) {
        r.push(
            Severity::Error,
            Rule::RefutedReason,
            id,
            "status is refuted but closed.why is empty; say what fired the kill condition",
        );
    }
}

/// Rules about whether the stored lifecycle fields still agree with
/// `status`, the shape every verb leaves them in but a hand edit can pull
/// apart.
fn lifecycle_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 12. `set_status` clears `closed` the moment a node leaves refuted or
    //     abandoned, and no verb ever sets it any other way, so a `closed`
    //     block on a seed or hypothesis is not a state any verb produces —
    //     it is what an earlier abandonment or refutation left behind after
    //     a hand edit reopened the node without going through `status`.
    if n.status.is_open() && n.closed.is_some() {
        r.push(
            Severity::Error,
            Rule::OpenNodeClosed,
            id,
            format!(
                "status is `{}` but a `closed` block is still set; leftover from an earlier close?",
                n.status
            ),
        );
    }
    // 13. `new --kill` and `sharpen` only ever write a kill condition
    //     together with a move to `hypothesis`, and `status` refuses to move
    //     a node carrying one back to `seed`, so a `seed` carrying one was
    //     set by hand without the guard that would have moved the status
    //     too. Not wrong by itself — the node has not yet been re-sharpened
    //     — so this is a warning rather than an error.
    if n.status == Status::Seed && n.kill.is_some() {
        r.push(
            Severity::Warn,
            Rule::SeedKill,
            id,
            "status is seed but a kill condition is set; `new --kill`/`sharpen` always move \
             status to hypothesis, so this looks like a hand edit",
        );
    }
}

/// Rules about whether the stored dates parse and, where order matters,
/// agree with each other.
fn date_rules(doc: &Doc, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 14. Every date `ops.rs` writes comes from `store::today()`, so
    //     `created`/`updated` are always `YYYY-MM-DD` and never move
    //     backwards. A hand edit is the only way either goes wrong, and
    //     `review`/`open` then silently treat the node as never stale,
    //     since `days_since` returns `None` for a date it cannot parse.
    let created_ok = is_iso_date(&n.created);
    if !created_ok {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!("created `{}` is not a YYYY-MM-DD date", n.created),
        );
    }
    let updated_ok = is_iso_date(&n.updated);
    if !updated_ok {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!("updated `{}` is not a YYYY-MM-DD date", n.updated),
        );
    }
    if created_ok && updated_ok && n.updated < n.created {
        r.push(
            Severity::Error,
            Rule::Dates,
            id,
            format!(
                "updated `{}` is earlier than created `{}`",
                n.updated, n.created
            ),
        );
    }
    for f in &n.references {
        if !is_iso_date(&f.added) {
            r.push(
                Severity::Error,
                Rule::Dates,
                id,
                format!(
                    "reference `{}` has an added date `{}` that does not parse",
                    f.id, f.added
                ),
            );
        }
    }
}

/// Rules about the links a node declares.
fn edge_rules(doc: &Doc, ids: &HashSet<&str>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    // 3. Dangling edges and self-loops. A lineage that points into nothing
    //    is worse than no lineage, because it looks like a record.
    for e in &n.edges {
        if !ids.contains(e.to.as_str()) {
            r.push(
                Severity::Error,
                Rule::EdgeTargets,
                id,
                format!("edge `{}` points at missing node `{}`", e.kind, e.to),
            );
        }
        if e.to == n.id {
            r.push(
                Severity::Error,
                Rule::EdgeTargets,
                id,
                format!("edge `{}` points at itself", e.kind),
            );
        }
    }
}

/// Rules about the references hanging off a node.
fn reference_rules(doc: &Doc, corpus: &Corpus, observatory: Option<&Path>, r: &mut Report) {
    let n = &doc.node;
    let id = Some(n.id.as_str());
    for f in &n.references {
        if !is_reference_kind(&f.kind) {
            r.push(
                Severity::Warn,
                Rule::ReferenceKind,
                id,
                format!(
                    "reference `{}` has unexpected kind `{}`; accepted kinds: {}",
                    f.id,
                    f.kind,
                    REFERENCE_KINDS.join(", ")
                ),
            );
        }
        if f.uri.as_deref().is_none_or(|uri| uri.trim().is_empty()) && f.kind != "discussion" {
            r.push(
                Severity::Error,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` has kind `{}` but no URI; only discussions may omit it",
                    f.id, f.kind
                ),
            );
        }
        // 10. A bare link is how a collection like this rots.
        if f.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
            r.push(
                Severity::Warn,
                Rule::ReferenceNote,
                id,
                format!("reference `{}` has no note saying why it is here", f.id),
            );
        }
        // 9, for an Observatory record: the id is the reference, and where
        //    it is on this machine is a setting. Not finding it here says
        //    the setting is missing or the checkout is behind, not that the
        //    record is gone, so the finding is a warning rather than an
        //    error and the reference stays valid on a machine that has it.
        if f.kind == OBSERVATORY {
            if let Some(record) = f.uri.as_deref().filter(|record| !record.trim().is_empty()) {
                match observatory {
                    None => r.push(
                        Severity::Warn,
                        Rule::ObservatoryReference,
                        id,
                        format!(
                            "reference `{}` names Observatory record `{record}` but no \
                             observatory root is set (config observatory_root or \
                             $OBSERVATORY_ROOT)",
                            f.id
                        ),
                    ),
                    Some(root) if resolve_observatory(root, record).is_none() => r.push(
                        Severity::Warn,
                        Rule::ObservatoryReference,
                        id,
                        format!(
                            "reference `{}` names Observatory record `{record}`, which does \
                             not resolve under {}",
                            f.id,
                            root.display()
                        ),
                    ),
                    Some(_) => {}
                }
            }
            continue;
        }
        // 8, for an absolute path or a `file:` URI: it may well resolve on
        //    this machine, which is exactly why `check` cannot judge it by
        //    resolving — on every other machine the corpus is synced to, it
        //    names nothing. `cite` refuses new ones; one already here, hand
        //    written or carried over from a v1 `evidence` source, is a
        //    warning, so an older corpus still loads and checks.
        if let Some(uri) = f.uri.as_deref().filter(|uri| is_absolute_local(uri)) {
            r.push(
                Severity::Warn,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` uses an absolute local path, which resolves on this machine \
                     only: {uri}; local references are relative to nodes/",
                    f.id
                ),
            );
            continue;
        }
        // 8. A local path that does not resolve is a citation to nothing.
        //    External URLs are not fetched; `check` stays offline and fast.
        if let Some(uri) = f
            .uri
            .as_deref()
            .filter(|uri| !uri.trim().is_empty())
            .filter(|uri| is_local_path(uri) && !resolve_local(corpus, uri).exists())
        {
            r.push(
                Severity::Error,
                Rule::LocalReference,
                id,
                format!(
                    "reference `{}` points at a path that does not resolve: {}",
                    f.id, uri
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Rule, is_absolute_local, is_local_path, is_observatory_id, resolve_observatory, tag_drift,
    };

    // The canonical labels and IDs for the published invariant tables. Keep
    // this beside the code that emits findings, so documentation drift fails
    // a core test rather than silently changing the meaning of `[N]`.
    const RULES: &[(Rule, &str)] = &[
        (Rule::AcyclicGenealogy, "Genealogy is acyclic"),
        (
            Rule::HypothesisKill,
            "`hypothesis` names a non-empty `kill`",
        ),
        (Rule::EdgeTargets, "Every edge target exists; no self-loop"),
        (Rule::MutualContradicts, "`contradicts` is mutual"),
        (Rule::RefutedReason, "`refuted` carries `closed.why`"),
        (
            Rule::RefutedReopens,
            "`refuted` leaves only via a new node's `reopens` edge",
        ),
        (
            Rule::NoReferenceVerdict,
            "A reference carries no `verdict`/`strength`",
        ),
        (
            Rule::LocalReference,
            "Non-discussion references have a URI; local URIs resolve relative to `nodes/` and are never absolute",
        ),
        (
            Rule::ObservatoryReference,
            "An `observatory` reference's record resolves under the configured root",
        ),
        (Rule::ReferenceNote, "Every reference has a note"),
        (
            Rule::TagDrift,
            "No two tags differ only by case or a trailing `s`",
        ),
        (
            Rule::OpenNodeClosed,
            "`closed` is set only on a `refuted`/`abandoned` node, never an open one",
        ),
        (Rule::SeedKill, "A `seed` does not carry a `kill` condition"),
        (
            Rule::Dates,
            "`created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created`",
        ),
        (
            Rule::NodeId,
            "A node's `id` names one file under `nodes/`, and is the id its file name names",
        ),
        (
            Rule::ReferenceKind,
            "Every reference kind belongs to the documented vocabulary",
        ),
    ];

    fn table_rules(markdown: &str) -> Vec<(u8, &str)> {
        markdown
            .lines()
            .skip_while(|line| !line.starts_with("| # |"))
            .skip(2)
            .take_while(|line| line.starts_with('|'))
            .map(|line| {
                let mut cells = line.split('|').map(str::trim);
                assert_eq!(cells.next(), Some(""), "table row begins with `|`");
                let number = cells.next().unwrap().parse::<u8>().unwrap();
                (number, cells.next().unwrap())
            })
            .collect()
    }

    #[test]
    fn published_invariant_tables_match_checker_rules() {
        let expected: Vec<_> = RULES
            .iter()
            .map(|(rule, label)| (*rule as u8, *label))
            .collect();
        assert!(
            expected.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "checker rule IDs must be distinct and ordered"
        );
        for (name, markdown) in [
            (
                "v0.2 spec",
                include_str!("../../../docs/design/v0.2/1_spec.md"),
            ),
            (
                "skill",
                include_str!("../../../skills/nebula/references/invariants.md"),
            ),
            (
                "lineage spec",
                include_str!("../../../docs/design/lineage-graph/specs/invariants.md"),
            ),
        ] {
            assert_eq!(table_rules(markdown), expected, "{name} invariant table");
        }
    }

    #[test]
    fn tag_drift_catches_case_and_plurals_only() {
        assert_eq!(tag_drift("Physics", "physics"), Some("case"));
        assert_eq!(tag_drift("sim", "sims"), Some("a trailing `s`"));
        assert_eq!(tag_drift("Sims", "sim"), Some("a trailing `s`"));
        assert_eq!(tag_drift("physics", "orrery"), None);
        assert_eq!(tag_drift("sim", "simulation"), None);
    }

    #[test]
    fn schemes_and_urls_are_not_local_paths() {
        for uri in [
            "https://example.org",
            "doi:10.1000/x",
            "orbit:DANI-10345",
            "neb:some-node",
            "[[almanac/page]]",
        ] {
            assert!(!is_local_path(uri), "{uri}");
        }
        for uri in ["./notes/x.md", "notes/x.md", "C:/x.md", "x"] {
            assert!(is_local_path(uri), "{uri}");
        }
    }

    /// Absolute on any platform is absolute on all of them, and nothing a
    /// scheme, a URL or a path under `nodes/` looks like is caught with it.
    #[test]
    fn absolute_paths_and_file_uris_are_absolute_on_every_platform() {
        for uri in [
            "/etc/hostname",
            "\\\\server\\share\\x.md",
            "C:/x.md",
            "c:\\x.md",
            "C:x.md",
            "file:///etc/hostname",
            "FILE://host/x.md",
            "file:/etc/hostname",
        ] {
            assert!(is_absolute_local(uri), "{uri}");
        }
        for uri in [
            "./notes/x.md",
            "notes/x.md",
            "../../studies/x.md",
            "x",
            "https://example.org",
            "http://example.org/file:///x",
            "mailto:someone@example.org",
            "orbit:DANI-10345",
            "neb:some-node",
            "doi:10.1000/x",
            "[[almanac/page]]",
            "files/x.md",
        ] {
            assert!(!is_absolute_local(uri), "{uri}");
        }
    }

    #[test]
    fn an_observatory_id_is_one_letter_of_four_then_digits() {
        for id in ["Q002", "H7", "T003", "R012"] {
            assert!(is_observatory_id(id), "{id}");
        }
        for id in ["", "Q", "q002", "X002", "Q002-slug", "Q 2", "/abs/Q002.md"] {
            assert!(!is_observatory_id(id), "{id}");
        }
    }

    /// The record is found by its id alone, whatever slug follows it, and a
    /// longer id that merely starts with the same digits is not a match.
    #[test]
    fn a_record_resolves_by_id_prefix_in_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("questions")).unwrap();
        std::fs::create_dir_all(root.join("research").join("R012-arc")).unwrap();
        std::fs::write(root.join("questions").join("Q002-a-question.md"), "").unwrap();
        std::fs::write(root.join("questions").join("Q0021-not-it.md"), "").unwrap();
        std::fs::write(root.join("questions").join("README.md"), "").unwrap();

        assert_eq!(
            resolve_observatory(root, "Q002"),
            Some(root.join("questions").join("Q002-a-question.md"))
        );
        assert_eq!(
            resolve_observatory(root, "R012"),
            Some(root.join("research").join("R012-arc"))
        );
        assert_eq!(resolve_observatory(root, "Q003"), None);
        assert_eq!(
            resolve_observatory(root, "H001"),
            None,
            "no hypotheses/ at all"
        );
        assert_eq!(resolve_observatory(root, "Q002-a-question"), None);
    }
}
