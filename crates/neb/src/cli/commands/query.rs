//! The `Query:` section of `neb --help`: `show`, `log`, `list`, `near`, `trace`, `impact` and `graph`.

use crate::cli::args::StatusArg;
use crate::cli::emit::{cap, notify, out_json, warn_legacy_observatory_root};
use crate::cli::failure::Failure;
use crate::cli::{Invocation, Outcome};
use crate::output::{self, out};
use crate::render::{self, json};
use nebula_core::verb;
use nebula_core::{Corpus, Direction, Status, graph};
use std::process::ExitCode;

/// `neb show`.
pub(in crate::cli) fn show(cx: Invocation<'_>, node: String, at: Option<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let shown = verb::show(&corpus, &node, at.as_deref())?;
    warn_legacy_observatory_root(&corpus, &shown.setting);
    if json {
        out_json(&json::NodeView::from(&shown.view))?;
    } else {
        out!("{}", render::node(&shown.view));
    }
    Ok(ok)
}

/// `neb log`.
pub(in crate::cli) fn log(cx: Invocation<'_>, node: String) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let history = corpus.history(&node)?;
    if json {
        out_json(&history)?;
    } else {
        out!("{}", render::history(&history, render::Target::stdout()));
    }
    notify(json, render::history_notice(&history));
    Ok(ok)
}

/// `neb list`.
pub(in crate::cli) fn list(
    cx: Invocation<'_>,
    status: Option<StatusArg>,
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
    let (mut listing, nodes) = Corpus::open(locations, root)?.query(|graph| {
        let listing = graph::list(graph, status.map(Status::from), &tags)?;
        Ok((listing, graph.len()))
    })?;
    let cut = cap(&mut listing.0, limit);
    if json {
        let nodes = listing.0.iter().map(json::Node::from).collect();
        out_json(&json::List::new(nodes, limit.map(|_| cut)))?;
    } else {
        out!("{}", render::list(&listing.0, render::Target::stdout()));
    }
    notify(
        json,
        Some(render::list_notice(listing.0.len(), cut.0, nodes)),
    );
    Ok(ok)
}

/// `neb near`.
pub(in crate::cli) fn near(cx: Invocation<'_>, limit: usize, query: Vec<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let query = query.join(" ");
    if query.trim().is_empty() {
        return Err(Failure::say("nothing to look near"));
    }
    let (near, matched) =
        Corpus::open(locations, root)?.query(|graph| graph::near_counted(graph, &query, limit))?;
    let truncated = near.0.len() < matched;
    // On stderr in every mode, so a capped answer never reads as
    // the whole one and the payload stays all stdout holds (STD-01
    // §R34, §R12).
    let notice = render::near_notice(near.0.len(), matched);
    if json {
        out_json(&json::Capped::new(near.0, (matched, truncated)))?;
    } else {
        out!("{}", render::near(&near, render::Target::stdout()));
    }
    notify(json, notice);
    Ok(ok)
}

/// `neb trace`.
pub(in crate::cli) fn trace(
    cx: Invocation<'_>,
    node: String,
    down: bool,
    depth: Option<usize>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let direction = if down { Direction::Down } else { Direction::Up };
    let (walk, cut) = Corpus::open(locations, root)?.query(|graph| {
        let walk = graph::trace_within(graph, &node, direction, depth)?;
        // A bounded walk's `total` is what the whole walk reaches.
        let cut = depth
            .map(|_| graph::trace(graph, &node, direction))
            .transpose()?
            .map(|whole| (whole.0.len(), walk.0.len() < whole.0.len()));
        Ok((walk, cut))
    })?;
    if json {
        let steps = walk.0.iter().map(json::TraceNode::from).collect();
        out_json(&json::List::new(steps, cut))?;
    } else if output::stdout_on_terminal() {
        out!("{}", render::tree(&walk, direction));
    } else {
        out!("{}", render::trace_lines(&walk));
    }
    let whole = cut.map_or(walk.0.len(), |(total, _)| total);
    notify(json, render::trace_notice(walk.0.len(), whole, depth));
    Ok(ok)
}

/// `neb impact`.
pub(in crate::cli) fn impact(cx: Invocation<'_>, node: String) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let report = Corpus::open(locations, root)?.query(|graph| graph::impact(graph, &node))?;
    if json {
        out_json(&report)?;
    } else {
        out!("{}", render::impact(&report, &node));
    }
    notify(json, render::impact_notice(&report));
    Ok(ok)
}

/// `neb graph`.
pub(in crate::cli) fn graph(cx: Invocation<'_>, mermaid: bool, from: Option<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    if !json && !mermaid {
        return Err(Failure::say(
            "neb graph needs an output format; pass --json or --mermaid",
        ));
    }
    let exported = Corpus::open(locations, root)?.query(|graph| {
        if let Some(id) = from.as_deref() {
            graph::node(graph, id)?;
        }
        graph::export(graph)
    })?;
    if mermaid {
        out!("{}", render::mermaid(&exported, from.as_deref())?);
    } else {
        out_json(&exported)?;
    }
    Ok(ok)
}
