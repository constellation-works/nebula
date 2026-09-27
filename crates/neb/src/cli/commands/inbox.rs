//! The `Inbox:` section of `neb --help`: `capture`, `inbox`, `promote`, `drop` and `triage`.

use crate::cli::editor::{body_value, capture_text};
use crate::cli::emit::{cap, note_close_tags, notify, out_json, report_commit, warn_root};
use crate::cli::{Invocation, Outcome};
use crate::output::{self, errln, out, outln};
use crate::render::{self, json};
use nebula_core::verb;
use nebula_core::{Corpus, Error, NEAR_DEFAULT, Promotion, ops};
use std::process::ExitCode;

/// `neb capture`.
pub(in crate::cli) fn capture(cx: Invocation<'_>, quiet: bool, text: Vec<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let text = capture_text(&text)?;
    let k = if quiet { 0 } else { NEAR_DEFAULT };
    let done = verb::capture_at(locations, root, &text, k, &commits.options())?;
    let captured = done.value;
    let written = captured.entry.id.clone();
    if let Some(warning) = &captured.warning {
        warn_root(locations, warning, &captured.root);
    }
    // Never silent, never a question: a mistyped `--root` or
    // `$NEBULA_ROOT` would otherwise start a second corpus unnoticed.
    // Absolute so a relative typo shows where it landed; made so
    // lexically, because paths are used as given, never resolved.
    if captured.created {
        let shown = locations.absolute(&captured.root);
        errln!("note: created a new corpus at {}", shown.display());
    }
    // The id goes out before the advice: the capture never depended
    // on the rest of the corpus parsing, and a node file that will
    // not must not read as a lost thought.
    if !json {
        outln!("{}", render::bold(&captured.entry.id));
    }
    let committed = report_commit(&captured.root, commits, done.commit);
    // The capture has landed either way: the repeat is for the triage
    // to settle, and an inbox that cannot be read back costs the note.
    match &captured.same_as {
        Ok(Some(earlier)) => errln!("note: same as {}, still waiting", earlier.id),
        Ok(None) => {}
        Err(e) => errln!("warning: could not check the inbox for a repeat: {e}"),
    }
    let near = captured.near.unwrap_or_else(|e| {
        errln!("warning: suggestions unavailable: {e}");
        Vec::new()
    });
    if json {
        out_json(&json::Captured::from(&ops::Captured {
            entry: captured.entry,
            near,
        }))?;
    } else {
        out!("{}", render::suggestions(&near));
    }
    committed?;
    output::finish_written(&written)?;
    Ok(ok)
}

/// `neb inbox`.
pub(in crate::cli) fn inbox(cx: Invocation<'_>, limit: Option<usize>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let mut inbox = corpus.inbox()?;
    let cut = cap(&mut inbox.0, limit);
    let notice = render::inbox_notice(inbox.0.len(), cut.0);
    if json {
        out_json(&json::List::new(inbox.0, limit.map(|_| cut)))?;
    } else {
        out!("{}", render::inbox(&inbox, render::Target::stdout()));
    }
    notify(json, Some(notice));
    Ok(ok)
}

/// `neb promote`.
pub(in crate::cli) fn promote(
    cx: Invocation<'_>,
    entry: String,
    quiet: bool,
    title: Option<String>,
    body: Option<String>,
    parents: Vec<String>,
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
    let body_supplied = body.is_some();
    let body = body_value(body)?;
    let promotion = Promotion {
        title,
        body,
        body_supplied,
        parents,
        tags,
        origin: locations.origin(task, run)?,
        id,
        by,
    };
    let corpus = Corpus::open(locations, root)?;
    let k = if quiet { 0 } else { NEAR_DEFAULT };
    let done = verb::promote(&corpus, &entry, &promotion, k, &commits.options())?;
    let created = &done.value;
    if json {
        out_json(&json::Created::from(created))?;
    } else if quiet {
        outln!("{}", render::bold(&created.doc.node.id));
    } else {
        outln!(
            "{} {}",
            render::bold(&created.doc.node.id),
            render::dim(&created.path.display().to_string())
        );
        out!("{}", render::suggestions(&created.near));
    }
    let committed = report_commit(corpus.root(), commits, done.commit);
    note_close_tags(&done.close_tags);
    committed?;
    output::finish_written(&created.doc.node.id)?;
    Ok(ok)
}

/// `neb drop`.
pub(in crate::cli) fn drop(cx: Invocation<'_>, entry: String) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let corpus = Corpus::open(locations, root)?;
    let done = verb::drop(&corpus, &entry, &commits.options())?;
    if json {
        out_json(&done.value)?;
    } else {
        outln!("dropped {}", render::bold(&entry));
    }
    report_commit(corpus.root(), commits, done.commit)?;
    output::finish_written(&entry)?;
    Ok(ok)
}

/// `neb triage`, which runs the [`crate::cli::triage::triage`] session.
pub(in crate::cli) fn triage(cx: Invocation<'_>, by: Option<String>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    locations.write_gate(nebula_core::WriteIntent::Ordinary)?;
    // Refused before anything is read: there is no one payload a
    // session of keyed decisions could honestly be.
    if json {
        return Err(Error::Interactive("triage".into()).into());
    }
    let corpus = Corpus::open(locations, root)?;
    let interactive = output::stdin_on_terminal();
    crate::cli::triage::triage(
        &corpus,
        by,
        &mut std::io::stdin().lock(),
        &mut output::stdout(),
        interactive,
        commits,
    )?;
    Ok(ok)
}
