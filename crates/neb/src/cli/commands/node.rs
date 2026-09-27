//! The `Nodes:` section of `neb --help`: `new`, `edit`, `sharpen`, `status`, `link`, `tag` and `note`.

use crate::cli::args::{EdgeKindArg, StatusArg};
use crate::cli::editor::{body_value, edit_body, keep_refused};
use crate::cli::emit::{note_close_tags, notify, out_json, out_node_view, report_commit};
use crate::cli::failure::Failure;
use crate::cli::{Invocation, Outcome};
use crate::output::{self, out, outln};
use crate::render::{self, json};
use nebula_core::verb;
use nebula_core::{Corpus, EdgeType, NewNode, Status, graph};
use std::process::ExitCode;

/// `neb new`.
pub(in crate::cli) fn new(
    cx: Invocation<'_>,
    title: String,
    body: Option<String>,
    parents: Vec<String>,
    reopens: Option<String>,
    contradicts: Vec<String>,
    kill: Option<String>,
    tags: Vec<String>,
    id: Option<String>,
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
    let body = body_value(body)?;
    let corpus = Corpus::open(locations, root)?;
    let done = verb::new_node(
        &corpus,
        &NewNode {
            title,
            body,
            parents,
            reopens,
            contradicts,
            kill,
            tags,
            origin: locations.origin(task, run)?,
            id,
            by,
        },
        &commits.options(),
    )
    .map_err(|e| Failure(render::refusal_for_new(&e)))?;
    let created = &done.value;
    if json {
        out_json(&json::Created::from(created))?;
    } else {
        outln!(
            "{} {}",
            render::bold(&created.doc.node.id),
            render::dim(&created.path.display().to_string())
        );
    }
    let committed = report_commit(corpus.root(), commits, done.commit);
    note_close_tags(&done.close_tags);
    committed?;
    output::finish_written(&created.doc.node.id)?;
    Ok(ok)
}

/// `neb edit`.
pub(in crate::cli) fn edit(cx: Invocation<'_>, node: String, by: Option<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    locations.write_gate(nebula_core::WriteIntent::Authored(by.as_deref()))?;
    // No lock while the person types (STD-03 §R1). The body is
    // loaded now, edited for as long as it takes, and saved under
    // the lock only if nobody changed it meanwhile.
    let corpus = Corpus::open(locations, root)?;
    let before = corpus.load(&node).map_err(|e| Failure::about(&e, &node))?;
    let edited = edit_body(&before.body)?;
    let done = match verb::edit(
        &corpus,
        &node,
        &before.body,
        &edited.text,
        by.as_deref(),
        json,
        &commits.options(),
    ) {
        Ok(done) => done,
        Err(e) => {
            return Err(keep_refused(
                Failure::about(&e, &node),
                &corpus,
                &node,
                edited,
            ));
        }
    };
    drop(edited);
    // Nothing typed was nothing to write, and is said so in every
    // mode (STD-01 §R30).
    if !done.value.changed {
        notify(json, Some(render::unchanged(&node)));
    } else if !json {
        outln!("{}", render::bold(&node));
    }
    let committed = report_commit(corpus.root(), commits, done.commit);
    let shown = done.value.view.map_or(Ok(()), out_node_view);
    committed?;
    shown?;
    if done.value.changed {
        output::finish_written(&node)?;
    }
    Ok(ok)
}

/// `neb sharpen --confirm`.
pub(in crate::cli) fn confirm_kill(cx: Invocation<'_>, node: String) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    // `--kill` and `--by` conflict with `--confirm` in the clap tree,
    // so there is no text here to reconcile: confirming changes none.
    let corpus = Corpus::open(locations, root)?;
    let done = verb::confirm_kill(&corpus, &node, &commits.options())
        .map_err(|e| Failure::about(&e, &node))?;
    if json {
        out_json(&json::Doc::from(&done.value))?;
    } else {
        outln!("{} kill condition confirmed as yours", render::bold(&node));
    }
    report_commit(corpus.root(), commits, done.commit)?;
    output::finish_written(&done.value.node.id)?;
    Ok(ok)
}

/// `neb sharpen --kill`.
pub(in crate::cli) fn sharpen(
    cx: Invocation<'_>,
    node: String,
    kill: Option<String>,
    by: Option<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    // Clap's `required_unless_present` guarantees the kill is here
    // without `--confirm`; saying so beats an unwrap.
    let Some(kill) = kill else {
        return Err(Failure::say("pass --kill <KILL>, or --confirm"));
    };
    let corpus = Corpus::open(locations, root)?;
    let done = verb::sharpen(&corpus, &node, &kill, by.as_deref(), &commits.options())
        .map_err(|e| Failure::about(&e, &node))?;
    let sharpened = &done.value;
    let status = sharpened.doc.node.status;
    if json {
        out_json(&json::Doc::from(&sharpened.doc))?;
    } else if sharpened.from != status {
        outln!("{} is now {}", render::bold(&node), status);
    } else if sharpened.replaced {
        outln!(
            "{} kill condition replaced; status remains {}",
            render::bold(&node),
            status
        );
    } else {
        outln!(
            "{} kill condition set; status remains {}",
            render::bold(&node),
            status
        );
    }
    report_commit(corpus.root(), commits, done.commit)?;
    output::finish_written(&sharpened.doc.node.id)?;
    Ok(ok)
}

/// `neb status`.
pub(in crate::cli) fn status(
    cx: Invocation<'_>,
    node: String,
    status: StatusArg,
    why: Option<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let status = Status::from(status);
    let corpus = Corpus::open(locations, root)?;
    let done = verb::set_status(&corpus, &node, status, why.as_deref(), &commits.options())
        .map_err(|e| Failure::about(&e, &node))?;
    let changed = &done.value;
    if json {
        out_json(&json::StatusChange::from(changed))?;
    } else {
        outln!(
            "{} {} -> {status}",
            render::bold(&node),
            render::dim(&changed.from.to_string())
        );
    }
    report_commit(corpus.root(), commits, done.commit)?;
    output::finish_written(&changed.doc.node.id)?;
    Ok(ok)
}

/// `neb link`.
pub(in crate::cli) fn link(
    cx: Invocation<'_>,
    from: String,
    kind: EdgeKindArg,
    to: String,
    by: Option<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let kind = EdgeType::from(kind);
    let done = verb::link(&corpus, &from, kind, &to, by.as_deref(), &commits.options())?;
    if json {
        out_json(&done.value.iter().map(json::Doc::from).collect::<Vec<_>>())?;
    } else {
        outln!(
            "{} {} {}",
            render::bold(&from),
            render::dim(&kind.to_string()),
            render::bold(&to)
        );
    }
    report_commit(corpus.root(), commits, done.commit)?;
    output::finish_written(
        done.value
            .iter()
            .map(|doc| doc.node.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )?;
    Ok(ok)
}

/// `neb tag list`: every tag with its node count.
pub(in crate::cli) fn tag_list(cx: Invocation<'_>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let counts = Corpus::open(locations, root)?.query(graph::tags)?;
    if json {
        out_json(&counts)?;
    } else {
        out!("{}", render::tags(&counts, render::Target::stdout()));
    }
    notify(json, render::tags_notice(&counts));
    Ok(ok)
}

/// `neb tag <node>`.
pub(in crate::cli) fn tag(
    cx: Invocation<'_>,
    target: String,
    add: Vec<String>,
    remove: Vec<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    if add.is_empty() && remove.is_empty() {
        return Err(Failure::say(
            "nothing to do; pass --add <tag> or --remove <tag>",
        ));
    }
    // One write under one lock: `--remove x --add y` is one change
    // to the node's tags, not two a second writer may split.
    let corpus = Corpus::open(locations, root)?;
    let done = verb::retag(&corpus, &target, &add, &remove, &commits.options())?;
    let doc = &done.value.doc;
    if json {
        out_json(&json::Doc::from(doc))?;
    } else {
        let shown = if doc.node.tags.is_empty() {
            render::dim("(no tags)")
        } else {
            doc.node.tags.join(", ")
        };
        outln!("{} {shown}", render::bold(&target));
    }
    // What changed nothing is said in every mode, and a node whose
    // tags came out as they were is neither written nor committed
    // (STD-01 §R30).
    for note in render::retag_notes(&target, &done.value) {
        notify(json, Some(note));
    }
    let committed = report_commit(corpus.root(), commits, done.commit);
    note_close_tags(&done.close_tags);
    committed?;
    if done.value.written {
        output::finish_written(&doc.node.id)?;
    }
    Ok(ok)
}

/// `neb note`.
pub(in crate::cli) fn note(
    cx: Invocation<'_>,
    node: String,
    by: Option<String>,
    text: Vec<String>,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let text = text.join(" ");
    let corpus = Corpus::open(locations, root)?;
    let done = verb::note(
        &corpus,
        &node,
        &text,
        by.as_deref(),
        json,
        &commits.options(),
    )
    .map_err(|e| Failure::about(&e, &node))?;
    if !json {
        outln!("{}", render::bold(&node));
    }
    let committed = report_commit(corpus.root(), commits, done.commit);
    let shown = done.value.view.map_or(Ok(()), out_node_view);
    committed?;
    shown?;
    output::finish_written(&node)?;
    Ok(ok)
}
