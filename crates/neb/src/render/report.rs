//! The multi-line renderings: one function per report the core returns.

use super::{bold, dim, paint, status_badge};
use nebula_core::{
    INBOX_DAYS, Impact, Inbox, MigrationReport, NodeView, OpenReport, Report, ReviewItem,
    ReviewReport, ReviewRule, Severity, TagCounts, Via,
};
use std::fmt::Write as _;

/// One node in full.
pub fn node(view: &NodeView) -> String {
    let n = &view.node;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {} {}",
        status_badge(n.status),
        bold(&n.id),
        dim(&n.tags.join(", "))
    );
    let _ = writeln!(out, "{}\n", n.title);
    if let Some(k) = &n.kill {
        let _ = writeln!(out, "{} {k}\n", dim("kill:"));
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
            let _ = writeln!(out, "  {:<14} {}", e.kind.to_string(), e.to);
        }
        out.push('\n');
    }
    if !n.references.is_empty() {
        let _ = writeln!(out, "{}", dim("references"));
        for r in &n.references {
            let _ = writeln!(out, "  {} {:<10} {}", bold(&r.id), r.kind, r.uri);
            let text = r.note.as_deref().map_or("(no note)", str::trim);
            let _ = writeln!(out, "     {}", dim(text));
        }
        out.push('\n');
    }
    out
}

/// Captures waiting to be promoted or dropped.
pub fn inbox(inbox: &Inbox) -> String {
    if inbox.0.is_empty() {
        return format!("{}\n", dim("inbox is empty"));
    }
    let mut out = String::new();
    for e in &inbox.0 {
        let _ = writeln!(out, "{} {} {}", bold(&e.id), dim(&e.at), e.text);
    }
    let _ = writeln!(
        out,
        "\n{}",
        dim(&format!(
            "{} waiting. Promote or drop each one.",
            inbox.0.len()
        ))
    );
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

/// Nodes that need attention.
pub fn open(report: &OpenReport) -> String {
    let mut out = String::new();
    if report.0.is_empty() {
        let _ = writeln!(out, "{}", dim("nothing needs attention"));
    }
    for item in &report.0 {
        let _ = writeln!(out, "{} {}", bold(&item.id), item.why);
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
        "\n{} nodes, {errors} errors, {} warnings",
        report.nodes,
        report.findings.len() - errors
    );
    out
}

/// The weekly maintenance report, as the markdown that goes into `review.md`.
///
/// Sectioned by rule, with the thresholds in the headings, so the file says
/// what it was asking when it was written.
pub fn review(report: &ReviewReport, hypothesis_days: i64, seed_days: i64) -> String {
    let sections = [
        (
            ReviewRule::StaleHypothesis,
            format!("Hypotheses untouched for {hypothesis_days} days"),
        ),
        (
            ReviewRule::UntouchedSeed,
            format!("Seeds untouched for {seed_days} days"),
        ),
        (ReviewRule::NoReferences, "Nodes with no references".into()),
        (
            ReviewRule::StaleInbox,
            format!("Inbox entries waiting {INBOX_DAYS} days or more"),
        ),
    ];
    let mut text = String::new();
    for (rule, heading) in sections {
        let _ = writeln!(text, "## {heading}\n");
        let items: Vec<&ReviewItem> = report.0.iter().filter(|i| i.rule == rule).collect();
        if items.is_empty() {
            text.push_str("_none_\n\n");
        } else {
            for item in items {
                let _ = writeln!(text, "- `{}` {} — {}", item.id, item.title, item.reason);
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
                "{} of {} nodes rewritten; run `neb check` to confirm",
                report.rewritten.len(),
                report.nodes
            ))
        );
    }
    out
}
