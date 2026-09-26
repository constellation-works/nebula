//! The multi-line renderings: one function per report the core returns.

use super::{bold, count, dim, paint, status_badge};
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
        let _ = writeln!(out, "{} {} {}\n", dim("closed:"), c.why, dim(&c.at));
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

/// Where `observatory` references resolve, and what set that. `saved` is
/// true right after `neb config observatory-root <DIR>`, so a machine
/// setting the environment outranks says so rather than seeming lost.
pub fn observatory_root(setting: &ObservatoryRoot, saved: bool) -> String {
    let mut out = match (&setting.root, setting.source) {
        (Some(root), ObservatorySource::Env) => format!(
            "{} {}\n",
            bold(&root.display().to_string()),
            dim(&format!("(${OBSERVATORY_ROOT_ENV})"))
        ),
        (Some(root), ObservatorySource::Machine) => format!(
            "{} {}\n",
            bold(&root.display().to_string()),
            dim("(this machine: ~/.config/nebula/observatory-root)")
        ),
        (Some(root), ObservatorySource::Config) => format!(
            "{} {}\n{}\n",
            bold(&root.display().to_string()),
            dim("(config.yaml, legacy)"),
            dim(&format!(
                "config.yaml travels with the corpus, and this path is one machine's. Give \
                 each machine its own with `neb config observatory-root <DIR>` or \
                 ${OBSERVATORY_ROOT_ENV}, then remove the key with \
                 `neb config observatory-root --drop-legacy`."
            ))
        ),
        _ => format!(
            "{}\n",
            dim(&format!(
                "no observatory root; set one with `neb config observatory-root <DIR>` or \
                 ${OBSERVATORY_ROOT_ENV}"
            ))
        ),
    };
    if saved && setting.source == ObservatorySource::Env {
        let _ = writeln!(
            out,
            "{}",
            dim(&format!(
                "Saved for this machine, but ${OBSERVATORY_ROOT_ENV} outranks it while exported."
            ))
        );
    }
    if let Some(legacy) = setting
        .legacy
        .as_ref()
        .filter(|_| setting.source != ObservatorySource::Config)
    {
        let _ = writeln!(
            out,
            "{}",
            dim(&format!(
                "config.yaml still carries a legacy observatory_root ({}), ignored here; \
                 once every machine has its own setting, remove it with \
                 `neb config observatory-root --drop-legacy`.",
                legacy.display()
            ))
        );
    }
    out
}

/// Whether writes are committed, as `neb config commit` reports it.
pub fn commit_setting(setting: CommitSetting) -> String {
    if setting.enabled {
        format!("{} {}\n", bold("on"), dim("(config.yaml)"))
    } else {
        format!(
            "{}\n",
            dim("off; turn it on with `neb config commit on` once the corpus is a git repository")
        )
    }
}

/// Captures waiting to be promoted or dropped. `waiting` is how many there
/// are in all, which is more than `inbox` holds when `--limit` cut it.
pub fn inbox(inbox: &Inbox, waiting: usize) -> String {
    if waiting == 0 {
        return format!("{}\n", dim("inbox is empty"));
    }
    let mut out = String::new();
    for e in &inbox.0 {
        let _ = writeln!(out, "{} {} {}", bold(&e.id), dim(&e.at), e.text);
    }
    let shown = inbox.0.len();
    let tally = if shown < waiting {
        format!("{shown} of {waiting} waiting shown; raise --limit for more.")
    } else {
        format!("{waiting} waiting.")
    };
    let _ = writeln!(
        out,
        "\n{}",
        dim(&format!("{tally} Promote or drop each one."))
    );
    out
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
    let text = format!("{b:<6}");
    match b {
        Band::Strong => bold(&text),
        Band::Some => text,
        Band::Weak => dim(&text),
    }
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
    if near.0.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            dim("nothing near: no node shares a word with this")
        );
        return out;
    }
    for n in &near.0 {
        let _ = writeln!(out, "{}", neighbour(n));
    }
    out
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
    if report.0.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            dim("nothing descends from or contradicts this node")
        );
        return out;
    }
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

/// Nodes that need attention. `omitted` is how many lines `--limit` cut.
pub fn open(report: &OpenReport, omitted: usize) -> String {
    let mut out = String::new();
    if report.0.is_empty() && omitted == 0 {
        let _ = writeln!(out, "{}", dim("nothing needs attention"));
    }
    for item in &report.0 {
        let _ = writeln!(out, "{} {}", bold(&item.id), item.why);
    }
    if omitted > 0 {
        let _ = writeln!(
            out,
            "{}",
            dim(&format!("… and {omitted} more; raise --limit for more"))
        );
    }
    out
}

/// Every tag with the number of nodes carrying it.
pub fn tags(counts: &TagCounts) -> String {
    let mut out = String::new();
    if counts.0.is_empty() {
        let _ = writeln!(out, "{}", dim("no tags"));
    }
    for t in &counts.0 {
        let _ = writeln!(out, "{} {}", bold(&t.tag), dim(&t.count.to_string()));
    }
    out
}

/// The invariant report, findings first and a tally last.
pub fn check(report: &Report) -> String {
    let mut out = String::new();
    for f in &report.findings {
        let (tag, code) = match f.level {
            Severity::Error => ("ERROR", "31;1"),
            Severity::Warn => ("warn ", "33"),
        };
        let where_ = f.node.as_deref().unwrap_or("corpus");
        let _ = writeln!(
            out,
            "{} {} {} {}",
            paint(code, tag),
            dim(&format!("[{}]", f.rule)),
            bold(where_),
            f.message
        );
    }
    let errors = report
        .findings
        .iter()
        .filter(|f| f.level == Severity::Error)
        .count();
    let _ = writeln!(
        out,
        "\n{}, {}, {}",
        count(report.nodes, "node"),
        count(errors, "error"),
        count(report.findings.len() - errors, "warning")
    );
    out
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
    if report.rewritten.is_empty() && !report.config_rewritten {
        let _ = writeln!(out, "{}", dim("already at schema 2; nothing changed"));
    } else {
        let _ = writeln!(
            out,
            "\n{}",
            dim(&format!(
                "{} of {} rewritten; run `neb check` to confirm",
                report.rewritten.len(),
                count(report.nodes, "node")
            ))
        );
    }
    out
}
