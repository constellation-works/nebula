//! The `Corpus:` section of `neb --help`: `init`, `check`, `migrate`, `config` and `completions`.

use crate::cli::args::{Cli, OnOff};
use crate::cli::emit::{notify, out_json, report_commit, warn_root};
use crate::cli::failure::shell_word;
use crate::cli::{Invocation, Outcome};
use crate::output::{self, errln, out, outln};
use crate::render;
use clap::CommandFactory;
use nebula_core::verb;
use nebula_core::{Corpus, Severity};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// `neb init`.
pub(in crate::cli) fn init(
    cx: Invocation<'_>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let done = verb::init(locations, root, path, set_root, force)?;
    let target = &done.initialized.root;
    if json {
        out_json(&done.initialized)?;
    } else {
        outln!("corpus ready at {}", target.display());
        if set_root {
            outln!("wrote {} so every command finds it", done.setting.display());
        } else if done.suggest_set_root {
            errln!(
                "{}",
                render::notice(&format!(
                    "run `neb init {} --set-root` to make this corpus the machine default",
                    shell_word(&locations.absolute(target).to_string_lossy())
                ))
            );
        }
    }
    for warning in &done.warnings {
        warn_root(locations, warning, target);
    }
    let mut writes = Vec::new();
    if done.wrote_corpus {
        writes.push(format!("corpus initialization at {}", target.display()));
    }
    if set_root {
        writes.push(format!(
            "machine root setting at {}",
            done.setting.display()
        ));
    }
    if !writes.is_empty() {
        output::finish_written(writes.join("; "))?;
    }
    Ok(ok)
}

/// `neb check`.
pub(in crate::cli) fn check(cx: Invocation<'_>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        ..
    } = cx;
    let ok = ExitCode::SUCCESS;
    let report = verb::check(&Corpus::open(locations, root)?)?;
    if json {
        out_json(&report)?;
    } else {
        out!("{}", render::check(&report));
    }
    notify(json, Some(render::check_tally(&report)));
    Ok(
        if report.unreadable.is_empty()
            && report.findings.iter().all(|f| f.level != Severity::Error)
        {
            ok
        } else {
            ExitCode::FAILURE
        },
    )
}

/// `neb migrate`.
pub(in crate::cli) fn migrate(cx: Invocation<'_>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let done = verb::migrate(locations, root, &commits.options())?;
    let report = &done.value.report;
    if json {
        out_json(report)?;
    } else {
        out!("{}", render::migration(report));
    }
    notify(json, Some(render::migration_notice(report)));
    report_commit(&done.value.root, commits, done.commit)?;
    if report.config_rewritten || !report.rewritten.is_empty() {
        output::finish_written(format!("migration at {}", done.value.root.display()))?;
    }
    Ok(ok)
}

/// `neb config observatory-root`.
pub(in crate::cli) fn config_observatory_root(
    cx: Invocation<'_>,
    dir: Option<PathBuf>,
    drop_legacy: bool,
) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    // Without a change to make, this is a read and takes no lock.
    let mut corpus = Corpus::open(locations, root)?;
    let done =
        verb::set_observatory_root(&mut corpus, dir.as_deref(), drop_legacy, &commits.options())?;
    let setting = &done.value.setting;
    if json {
        out_json(setting)?;
    } else {
        out!("{}", render::observatory_root(setting));
    }
    for note in render::observatory_root_notes(setting, dir.is_some()) {
        notify(json, Some(note));
    }
    // Said in every mode, like a commit asked for and not made: the
    // file it names was rewritten, or was left alone (STD-01 §R30).
    if let Some(dropped) = &done.value.dropped {
        let config = corpus.root().join("config.yaml");
        notify(json, Some(render::dropped_legacy(dropped, &config)));
    }
    report_commit(corpus.root(), commits, done.commit)?;
    let mut writes = Vec::new();
    if dir.is_some() {
        let setting = Corpus::machine_settings_dir(locations)?.join("observatory-root");
        writes.push(format!("observatory root setting at {}", setting.display()));
    }
    if done
        .value
        .dropped
        .as_ref()
        .is_some_and(|d| d.removed.is_some())
    {
        let config = corpus.root().join("config.yaml");
        writes.push(format!(
            "legacy observatory root removal at {}",
            config.display()
        ));
    }
    if !writes.is_empty() {
        output::finish_written(writes.join("; "))?;
    }
    Ok(ok)
}

/// `neb config commit`.
pub(in crate::cli) fn config_commit(cx: Invocation<'_>, state: Option<OnOff>) -> Outcome {
    let Invocation {
        locations,
        root,
        json,
        commits,
    } = cx;
    let ok = ExitCode::SUCCESS;
    let mut corpus = Corpus::open(locations, root)?;
    let writes = state.is_some();
    // Reading the setting is a read, so it takes no lock.
    let (setting, commit) = match state {
        Some(state) => {
            let done = verb::set_commit(&mut corpus, bool::from(state), &commits.options())?;
            (done.value, done.commit)
        }
        None => (corpus.commit_setting(), None),
    };
    if json {
        out_json(&setting)?;
    } else {
        out!("{}", render::commit_setting(setting));
    }
    notify(json, render::commit_setting_hint(setting));
    // Turning it on records itself; turning it off leaves the file
    // for the next commit you make by hand, because off means off.
    report_commit(corpus.root(), commits, commit)?;
    if writes {
        output::finish_written(format!(
            "commit setting at {}",
            corpus.root().join("config.yaml").display()
        ))?;
    }
    Ok(ok)
}

/// `neb completions`.
pub(in crate::cli) fn completions(cx: Invocation<'_>, shell: clap_complete::Shell) -> Outcome {
    let Invocation { .. } = cx;
    let ok = ExitCode::SUCCESS;
    // Rendered whole first: `generate` panics on a failed write, and
    // the layer, not clap, decides what a closed stdout means.
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "neb", &mut script);
    output::stdout()
        .write_all(&script)
        .map_err(output::StdoutFailed::from)?;
    Ok(ok)
}
