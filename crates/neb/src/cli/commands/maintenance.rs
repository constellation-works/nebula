//! The `Maintenance:` section of `neb --help`: `review`, and the deprecated `open`.

use crate::cli::emit::{cap, notify, out_json, write_report};
use crate::cli::failure::Failure;
use crate::cli::{Invocation, Outcome};
use crate::output::{errln, out};
use crate::render::{self, json};
use nebula_core::{Corpus, Locations, graph};
use std::path::PathBuf;
use std::process::ExitCode;

/// `neb review --short`.
pub(in crate::cli) fn review_short(
    cx: Invocation<'_>,
    tags: Vec<String>,
    limit: Option<usize>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    short_review(locations, root, json, &tags, limit)?;
    Ok(ok)
}

/// `neb review`, the full report.
pub(in crate::cli) fn review(
    cx: Invocation<'_>,
    since: Option<i64>,
    out: Option<PathBuf>,
    limit: Option<usize>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let mut report = corpus.query(|graph| graph::review(graph, &corpus.inbox()?, since))?;
    // Every rule's findings, counted before the cut keeps each rule's first N.
    let found = report.0.len();
    let omitted = limit.map_or_else(Vec::new, |n| report.truncate_per_rule(n));
    let text = if json {
        let listed = json::List::new(report.0, limit.map(|_| (found, !omitted.is_empty())));
        serde_json::to_string_pretty(&listed).map_err(Failure::from)?
    } else {
        render::review(
            &report,
            since.unwrap_or(nebula_core::HYPOTHESIS_DAYS),
            since.unwrap_or(nebula_core::SEED_DAYS),
            &omitted,
        )
    };
    write_report(out.as_deref(), &text)?;
    notify(json, render::review_notice(&omitted));
    Ok(ok)
}

/// `neb open`, deprecated for `review --short`.
pub(in crate::cli) fn open(cx: Invocation<'_>, tags: Vec<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    errln!("warning: `neb open` is deprecated; use `neb review --short`");
    short_review(locations, root, json, &tags, None)?;
    Ok(ok)
}

/// `review --short`, and the deprecated `open` that now forwards to it:
/// what needs attention now, one line per item.
fn short_review(
    locations: &Locations,
    root: Option<PathBuf>,
    json: bool,
    tags: &[String],
    limit: Option<usize>,
) -> std::result::Result<(), Failure> {
    let corpus = Corpus::open(locations, root)?;
    let mut report = corpus.query(|graph| graph::open(graph, &corpus.inbox()?, tags))?;
    let cut = cap(&mut report.0, limit);
    let notice = render::open_notice(report.0.len(), cut.0);
    if json {
        out_json(&json::List::new(report.0, limit.map(|_| cut)))?;
    } else {
        out!("{}", render::open(&report, render::Target::stdout()));
    }
    notify(json, notice);
    Ok(())
}
