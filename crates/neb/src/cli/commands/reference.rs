//! The `References:` section of `neb --help`: `cite` and `handoff`.

use crate::cli::emit::{
    out_json, print_bare_note, print_record_location, report_commit, warn_legacy_observatory_root,
};
use crate::cli::failure::Failure;
use crate::cli::{Invocation, Outcome};
use crate::output::outln;
use crate::render::{self, json};
use nebula_core::verb::{self, CiteReport};
use nebula_core::{Citation, Corpus, Handoff};
use std::process::ExitCode;

/// `neb cite`.
pub(in crate::cli) fn cite(
    cx: Invocation<'_>,
    node: String,
    uri: Option<String>,
    kind: String,
    title: Option<String>,
    note: Option<String>,
    by: Option<String>,
    task: Option<String>,
    run: Option<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
    // Core reads the observatory setting before the write, under
    // `--json` too, whose payload says where the record is.
    let done = verb::cite(
        &corpus,
        &node,
        &Citation {
            uri,
            kind,
            title,
            note,
            by,
            origin: locations.origin(task, run)?,
        },
        &commits.options(),
    )
    .map_err(|e| Failure::about(&e, &node))?;
    let CiteReport { cited, setting } = &done.value;
    if let Some(setting) = setting {
        warn_legacy_observatory_root(&corpus, setting);
    }
    if json {
        out_json(&json::Cited::from(cited))?;
    } else {
        outln!("{} {}", render::bold(&node), render::bold(&cited.reference));
        if let (Some(setting), Some(link)) = (setting, &cited.observatory) {
            print_record_location(setting, link);
        }
        if bare {
            print_bare_note();
        }
    }
    report_commit(corpus.root(), commits, done.commit)?;
    Ok(ok)
}

/// `neb handoff`.
pub(in crate::cli) fn handoff(
    cx: Invocation<'_>,
    node: String,
    record: String,
    note: Option<String>,
    by: Option<String>,
    task: Option<String>,
    run: Option<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
    // Core reads the setting before the write, under `--json` too:
    // the record has to resolve under this root.
    let done = verb::handoff(
        &corpus,
        &node,
        &Handoff {
            record,
            note,
            by,
            origin: locations.origin(task, run)?,
        },
        &commits.options(),
    )
    .map_err(|e| Failure::about(&e, &node))?;
    let handed = &done.value.done;
    let setting = &done.value.setting;
    warn_legacy_observatory_root(&corpus, setting);
    if json {
        out_json(&json::HandedOff::from(handed))?;
    } else {
        outln!(
            "{} {} -> {}, handed off to {} {}",
            render::bold(&node),
            render::dim(&handed.from.to_string()),
            handed.doc.node.status,
            render::bold(&handed.record),
            render::dim(&format!("({})", handed.reference))
        );
        print_record_location(setting, &handed.observatory);
        if bare {
            print_bare_note();
        }
    }
    report_commit(corpus.root(), commits, done.commit)?;
    Ok(ok)
}
