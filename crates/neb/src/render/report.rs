//! The multi-line renderings: one function per report the core returns.

use super::{Notice, bold, count, dim, paint, status_badge};
use crate::output::Role;
use nebula_core::{
    Band, CommitSetting, EdgeType, HUMAN, INBOX_DAYS, Impact, Inbox, MigrationReport, Near,
    Neighbour, NodeView, OBSERVATORY_ROOT_ENV, ObservatoryRoot, ObservatorySource, OpenReport,
    Report, ReviewItem, ReviewReport, ReviewRule, Severity, TagCounts, Via,
};
use std::fmt::Write as _;

/// Who wrote something, as a dimmed ` (label)` suffix, or nothing for the
/// human: the view states `human` outright, and saying so on every line
/// would bury the one line an agent wrote.
fn by(label: Option<&str>) -> String {
    match label.filter(|by| *by != HUMAN) {
        Some(by) => format!(" {}", dim(&format!("({by})"))),
        None => String::new(),
    }
}

/// One node in full.
///
/// The header is the status and id, the title with its author when that is
/// not the human, then a labelled line for the tags and one for the dates.
pub fn node(view: &NodeView) -> String {
    let n = &view.node;
    let mut out = String::new();
    let _ = writeln!(out, "{} {}", status_badge(n.status), bold(&n.id));
    let _ = writeln!(out, "{}{}", n.title, by(n.title_by.as_deref()));
    if !n.tags.is_empty() {
        let _ = writeln!(out, "{} {}", dim("tags:"), n.tags.join(", "));
    }
    let _ = writeln!(
        out,
        "{} {}  {} {}\n",
        dim("created:"),
        n.created,
        dim("updated:"),
        n.updated
    );
    if let Some(k) = &n.kill {
        // Whose falsifier this is decides how much the hypothesis is worth,
        // so an unconfirmed one says so where the human will read it.
        let _ = writeln!(out, "{} {k}{}\n", dim("kill:"), by(n.kill_by.as_deref()));
    }
    if let Some(c) = &n.closed {
        let _ = writeln!(out, "{} {} {}", dim("closed:"), c.why, dim(&c.at));
        // A hand-off names where the idea went, so where that record is on
        // this machine belongs right under it.
        if let Some(record) = &view.handed_off_to {
            let located = view
                .observatory
                .iter()
                .find(|l| &l.record == record)
                .and_then(|l| l.path.as_ref())
                .map_or_else(
                    || dim("(does not resolve; check the observatory root)"),
                    |path| dim(&path.display().to_string()),
                );
            let _ = writeln!(out, "        {located}");
        }
        out.push('\n');
    }
    if !view.body.is_empty() {
        let _ = writeln!(out, "{}\n", view.body);
    }
    if !n.edges.is_empty() {
        let _ = writeln!(out, "{}", dim("edges"));
        for e in &n.edges {
            let _ = writeln!(out, "  {:<14} {}{}", e.kind, e.to, by(e.by.as_deref()));
        }
        out.push('\n');
    }
    if !n.references.is_empty() {
        let _ = writeln!(out, "{}", dim("references"));
        for r in &n.references {
            match &r.uri {
                Some(uri) => {
                    let _ = writeln!(
                        out,
                        "  {} {:<10} {uri}{}",
                        bold(&r.id),
                        r.kind,
                        by(r.by.as_deref())
                    );
                }
                None => {
                    let _ = writeln!(out, "  {} {}{}", bold(&r.id), r.kind, by(r.by.as_deref()));
                }
            }
            // An observatory reference stores a record id, so where that
            // record actually is on this machine is the useful line.
            if let Some(link) = view.observatory.iter().find(|l| l.reference == r.id) {
                let located = match &link.path {
                    Some(path) => dim(&path.display().to_string()),
                    None => dim("(does not resolve; check the observatory root)"),
                };
                let _ = writeln!(out, "     {located}");
            }
            let text = r.note.as_deref().map_or("(no note)", str::trim);
            let _ = writeln!(out, "     {}", dim(text));
        }
        out.push('\n');
    }
    out
}

/// Where `observatory` references resolve, and what set that: one line,
/// or nothing when no root is set.
pub fn observatory_root(setting: &ObservatoryRoot) -> String {
    let source = match setting.source {
        ObservatorySource::Env => format!("(${OBSERVATORY_ROOT_ENV})"),
        ObservatorySource::Machine => "(this machine: ~/.config/nebula/observatory-root)".into(),
        ObservatorySource::Config => "(config.yaml, legacy)".into(),
        ObservatorySource::Unset => return String::new(),
    };
    setting.root.as_ref().map_or_else(String::new, |root| {
        format!("{} {}\n", bold(&root.display().to_string()), dim(&source))
    })
}

/// The advice around an observatory root, for stderr. `saved` is true right
/// after `neb config observatory-root <DIR>`, so a machine setting the
/// environment outranks says so rather than seeming lost.
pub fn observatory_root_notes(setting: &ObservatoryRoot, saved: bool) -> Vec<Notice> {
    let mut notes = Vec::new();
    match (&setting.root, setting.source) {
        (Some(_), ObservatorySource::Config) => notes.push(Notice::human(format!(
            "config.yaml travels with the corpus, and this path is one machine's. Give \
             each machine its own with `neb config observatory-root <DIR>` or \
             ${OBSERVATORY_ROOT_ENV}, then remove the key with \
             `neb config observatory-root --drop-legacy`."
        ))),
        (Some(_), _) => {}
        (None, _) => notes.push(Notice::human(format!(
            "no observatory root; set one with `neb config observatory-root <DIR>` or \
             ${OBSERVATORY_ROOT_ENV}"
        ))),
    }
    if saved && setting.source == ObservatorySource::Env {
        notes.push(Notice::human(format!(
            "Saved for this machine, but ${OBSERVATORY_ROOT_ENV} outranks it while exported."
        )));
    }
    if let Some(legacy) = setting
        .legacy
        .as_ref()
        .filter(|_| setting.source != ObservatorySource::Config)
    {
        notes.push(Notice::human(format!(
            "config.yaml still carries a legacy observatory_root ({}), ignored here; \
             once every machine has its own setting, remove it with \
             `neb config observatory-root --drop-legacy`.",
            legacy.display()
        )));
    }
    notes
}

/// Whether writes are committed, as `neb config commit` reports it.
pub fn commit_setting(setting: CommitSetting) -> String {
    if setting.enabled {
        format!("{} {}\n", bold("on"), dim("(config.yaml)"))
    } else {
        "off\n".to_string()
    }
}

/// How to turn committing on, when it is off.
pub fn commit_setting_hint(setting: CommitSetting) -> Option<Notice> {
    (!setting.enabled).then(|| {
        Notice::human("turn it on with `neb config commit on` once the corpus is a git repository")
    })
}

/// Captures waiting to be promoted or dropped, one line each.
pub fn inbox(inbox: &Inbox) -> String {
    let mut out = String::new();
    for e in &inbox.0 {
        let _ = writeln!(out, "{} {} {}", bold(&e.id), dim(&e.at), e.text);
    }
    out
}

/// How many captures wait. `waiting` is how many there are in all, which is
/// more than `shown` when `--limit` cut the listing.
pub fn inbox_notice(shown: usize, waiting: usize) -> Notice {
    if waiting == 0 {
        Notice::always("inbox is empty")
    } else if shown < waiting {
        Notice::always(format!(
            "{shown} of {waiting} waiting shown; raise --limit for more. Promote or drop each one."
        ))
    } else {
        Notice::human(format!("{waiting} waiting. Promote or drop each one."))
    }
}

/// One neighbour as a line: band, status, id, title, and any link it
/// already has to the node asked about. The raw score stays in `--json`:
/// printed bare, it reads as a percentage it is not.
pub(super) fn neighbour(n: &Neighbour) -> String {
    let mut line = format!(
        "{} {} {} {}",
        band(n.band),
        status_badge(n.status),
        bold(&n.id),
        dim(&n.title)
    );
    if let Some(linked) = linked(n) {
        let _ = write!(line, "  {linked}");
    }
    line
}

/// A band, padded so the columns after it line up.
fn band(b: Band) -> String {
    paint(Role::of("band", &b.to_string()), &format!("{b:<6}"))
}

/// The edges a neighbour already shares with the node `near` was asked
/// about, by what the neighbour is to that node: `linked: parent
/// (derives-from)`, `linked: child (refines)`, `linked: contradicts`.
fn linked(n: &Neighbour) -> Option<String> {
    let edges = n.linked.as_deref()?;
    // Kinds grouped by role, each role once, in the order first met.
    let mut roles: Vec<(&str, Vec<EdgeType>)> = Vec::new();
    for e in edges {
        let role = match (e.kind.is_genealogy(), e.from == n.id) {
            (false, _) => "contradicts",
            (true, false) => "parent",
            (true, true) => "child",
        };
        match roles.iter_mut().find(|(r, _)| *r == role) {
            Some((_, kinds)) => kinds.push(e.kind),
            None => roles.push((role, vec![e.kind])),
        }
    }
    let parts: Vec<String> = roles
        .into_iter()
        .map(|(role, kinds)| {
            if role == "contradicts" {
                role.to_string()
            } else {
                let kinds: Vec<String> = kinds.iter().map(ToString::to_string).collect();
                format!("{role} ({})", kinds.join(", "))
            }
        })
        .collect();
    Some(format!("linked: {}", parts.join("; ")))
}

/// The nodes closest to a query, best first, as `near` prints them.
pub fn near(near: &Near) -> String {
    let mut out = String::new();
    for n in &near.0 {
        let _ = writeln!(out, "{}", neighbour(n));
    }
    out
}

/// The notice for `near`: no node shares a word with the query, or `-k`
/// cut the `matched` nodes that do to the `shown` best.
pub fn near_notice(shown: usize, matched: usize) -> Option<Notice> {
    if matched == 0 {
        Some(Notice::always(
            "nothing near: no node shares a word with this",
        ))
    } else if shown < matched {
        Some(Notice::always(format!(
            "{shown} of {matched} shown; raise -k for more"
        )))
    } else {
        None
    }
}

/// The nearest nodes after a `capture` or a `promote`, indented under the
/// id the verb printed, and nothing at all when there are none: the verb's
/// own line stays the whole story for a thought unlike anything here.
pub fn suggestions(near: &[Neighbour]) -> String {
    let mut out = String::new();
    if near.is_empty() {
        return out;
    }
    let _ = writeln!(out, "{}", dim("near:"));
    for n in near {
        let _ = writeln!(out, "  {}", neighbour(n));
    }
    out
}

/// What descends from a node, and what contradicts it.
pub fn impact(report: &Impact, id: &str) -> String {
    let mut out = String::new();
    for (via, heading) in [
        (Via::Descends, format!("descends from `{id}`:")),
        (Via::Contradicts, format!("contradicts `{id}`:")),
    ] {
        let reached: Vec<&str> = report
            .0
            .iter()
            .filter(|t| t.via == via)
            .map(|t| t.id.as_str())
            .collect();
        if reached.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{}", dim(&heading));
        for id in reached {
            let _ = writeln!(out, "  {}", bold(id));
        }
    }
    out
}

/// The notice for a node nothing descends from or contradicts.
pub fn impact_notice(report: &Impact) -> Option<Notice> {
    report
        .0
        .is_empty()
        .then(|| Notice::always("nothing descends from or contradicts this node"))
}

/// Nodes that need attention, one line each.
pub fn open(report: &OpenReport) -> String {
    let mut out = String::new();
    for item in &report.0 {
        let _ = writeln!(out, "{} {}", bold(&item.id), item.why);
    }
    out
}

/// The notice for `review --short`: nothing needs attention, or `--limit`
/// cut the list. `all` is how long it was before the cut.
pub fn open_notice(shown: usize, all: usize) -> Option<Notice> {
    if all == 0 {
        Some(Notice::always("nothing needs attention"))
    } else if shown < all {
        Some(Notice::always(format!(
            "{shown} of {all} shown; raise --limit for more"
        )))
    } else {
        None
    }
}

/// Every tag with the number of nodes carrying it.
pub fn tags(counts: &TagCounts) -> String {
    let mut out = String::new();
    for t in &counts.0 {
        let _ = writeln!(out, "{} {}", bold(&t.tag), dim(&t.count.to_string()));
    }
    out
}

/// The notice for a corpus with no tags.
pub fn tags_notice(counts: &TagCounts) -> Option<Notice> {
    counts.0.is_empty().then(|| Notice::always("no tags"))
}

/// The invariant report's findings, one line each.
pub fn check(report: &Report) -> String {
    let mut out = String::new();
    for f in &report.findings {
        let (tag, token) = match f.level {
            Severity::Error => ("ERROR", "error"),
            Severity::Warn => ("warn ", "warn"),
        };
        let where_ = f.node.as_deref().unwrap_or("corpus");
        let _ = writeln!(
            out,
            "{} {} {} {}",
            paint(Role::of("severity", token), tag),
            dim(&format!("[{}]", f.rule)),
            bold(where_),
            f.message
        );
    }
    out
}

/// The invariant report's tally: how many nodes, errors and warnings.
pub fn check_tally(report: &Report) -> Notice {
    let errors = report
        .findings
        .iter()
        .filter(|f| f.level == Severity::Error)
        .count();
    let warnings = report.findings.len() - errors;
    let outcome = match (errors, warnings) {
        (0, 0) => "clean",
        (0, _) => "warnings",
        _ => "errors",
    };
    Notice::human(format!(
        "{}, {}, {}",
        count(report.nodes, "node"),
        count(errors, "error"),
        count(warnings, "warning")
    ))
    .in_role(Role::of("check", outcome))
}

/// The notice for a report `--limit` cut: how many findings it left out.
/// The markdown says so under each cut section too, since it is the file.
pub fn review_notice(omitted: &[(ReviewRule, usize)]) -> Option<Notice> {
    let cut: usize = omitted.iter().map(|(_, n)| n).sum();
    (cut > 0).then(|| {
        Notice::always(format!(
            "{} not shown; raise --limit for more",
            count(cut, "finding")
        ))
    })
}

/// The weekly maintenance report, as the markdown that goes into `review.md`.
///
/// Sectioned by rule, with the thresholds in the headings, so the file says
/// what it was asking when it was written. `omitted` is how many findings
/// `--limit` cut from each rule, and each cut section ends saying so.
pub fn review(
    report: &ReviewReport,
    hypothesis_days: i64,
    seed_days: i64,
    omitted: &[(ReviewRule, usize)],
) -> String {
    let sections = [
        (
            ReviewRule::StaleHypothesis,
            format!("Hypotheses untouched for {}", count(hypothesis_days, "day")),
        ),
        (
            ReviewRule::UntouchedSeed,
            format!("Seeds untouched for {}", count(seed_days, "day")),
        ),
        (ReviewRule::NoReferences, "Nodes with no references".into()),
        (
            ReviewRule::UnconfirmedKill,
            "Agent-authored kills not yet confirmed by a human".into(),
        ),
        (
            ReviewRule::StaleInbox,
            format!("Inbox entries waiting {INBOX_DAYS} days or more"),
        ),
    ];
    let mut text = String::new();
    for (rule, heading) in sections {
        let _ = writeln!(text, "## {heading}\n");
        let items: Vec<&ReviewItem> = report.0.iter().filter(|i| i.rule == rule).collect();
        let cut = omitted
            .iter()
            .find(|(r, _)| *r == rule)
            .map_or(0, |(_, n)| *n);
        if items.is_empty() && cut == 0 {
            text.push_str("_none_\n\n");
        } else {
            for item in items {
                let _ = writeln!(text, "- `{}` {} — {}", item.id, item.title, item.reason);
            }
            if cut > 0 {
                let _ = writeln!(text, "- _… and {cut} more; raise --limit for more_");
            }
            text.push('\n');
        }
    }
    text.trim_end().to_string()
}

/// What a migration did, node by node.
pub fn migration(report: &MigrationReport) -> String {
    let mut out = String::new();
    for n in &report.rewritten {
        let _ = writeln!(out, "{}", bold(&n.id));
        for note in &n.notes {
            let _ = writeln!(out, "  {note}");
        }
    }
    if report.config_rewritten {
        let _ = writeln!(
            out,
            "{} schema_version -> {}",
            bold("config.yaml"),
            nebula_core::SCHEMA_VERSION
        );
    }
    if let Some(id) = &report.minted_corpus_id {
        let _ = writeln!(
            out,
            "{} minted corpus_id {id} (there was none to keep)",
            bold("config.yaml")
        );
    }
    out
}

/// What a migration came to: nothing, or how much, and what to run next.
pub fn migration_notice(report: &MigrationReport) -> Notice {
    if report.rewritten.is_empty() && !report.config_rewritten {
        Notice::human("already at schema 2; nothing changed")
    } else {
        Notice::human(format!(
            "{} of {} rewritten; run `neb check` to confirm",
            report.rewritten.len(),
            count(report.nodes, "node")
        ))
    }
}
