//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `nebula_core`. Nothing else in the crate knows about clap.
//!
//! Dispatch is a table. [`run`] hands each command to its handler in
//! [`commands`], one module per section of `neb --help`. Each handler
//! parses, makes one core operation, and either writes the returned value as
//! JSON, through [`render::json`]'s view where core's serialisation would
//! leave a field out, or hands it to `render`. A write is one call into
//! [`nebula_core::verb`], which takes the lock, writes, commits and reads the
//! advice after; a read is [`nebula_core::Corpus::open`] plus one query. A
//! command that wants to do anything else belongs in core.
//!
//! The environment and the working directory are read once, in [`main`],
//! into the [`Locations`] every handler hands down: core never reads either.
//!
//! - `args`      the clap tree and its value parsers
//! - `help`      the hand-rolled `neb --help` template
//! - `commands`  one handler per verb, grouped as `neb --help` groups them
//! - `triage`    the `neb triage` session and its keys
//! - `editor`    the editor and standard-input readers a verb's text comes from
//! - `emit`      the notices, JSON and commit reports handlers write
//! - `failure`   the refusals the CLI exits on

mod args;
mod commands;
mod editor;
mod emit;
mod failure;
mod help;
mod triage;

pub(crate) use failure::shell_word;

use crate::output::{self, errln};
use crate::render;
use args::{Cli, Command, ConfigSetting};
use clap::{CommandFactory, FromArgMatches};
use emit::CommitOpts;
use failure::Failure;
use nebula_core::{CorpusLock, Locations};
use std::path::PathBuf;
use std::process::ExitCode;

/// Parse a command line as [`Cli::parse`] does, except that a verb's
/// `conflicts_with` a global flag holds wherever that flag is written.
///
/// Clap checks a subcommand's conflicts while it parses the subcommand's own
/// arguments, and a global flag written before the verb joins those only
/// afterwards. So `neb graph --mermaid --json` was refused while
/// `neb --json graph --mermaid` ran the Mermaid path under `--json`. A global
/// flag means the same on either side of the verb (STD-01 §R4), so a pair
/// that slipped through that way is refused here, with the error clap gives
/// when the flag follows the verb: same words, same usage line, exit 2.
pub(crate) fn parse_from<I, T>(argv: I) -> std::result::Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString>,
{
    let argv: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
    let mut cmd = Cli::command();
    let matches = cmd.try_get_matches_from_mut(&argv)?;
    cmd.build();
    if let Some(conflict) = global_conflict(&cmd, &matches) {
        return Err(conflict.error(&argv));
    }
    let mut cli = Cli::from_arg_matches(&matches).map_err(|e| e.format(&mut cmd))?;
    cli.verb = std::iter::successors(matches.subcommand(), |(_, sub)| sub.subcommand())
        .map(|(name, _)| name)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(cli)
}

/// A verb's argument and a global flag it declares a conflict with, both
/// given, which clap let through because the global came before the verb.
struct GlobalConflict {
    /// The verb's argument as clap names it in errors, e.g. `--from <ID>`.
    arg: String,
    /// The global flag as clap names it in errors, e.g. `--json`.
    global: String,
    /// The global flag spelled as one token that can follow the verb, as
    /// given; `None` when it has no long name to spell it with.
    token: Option<std::ffi::OsString>,
}

/// The first declared conflict between a verb's argument and a global flag
/// that are both on the command line.
///
/// Clap has already refused every declared conflict among arguments it saw
/// together, so any pair found here straddles the verb. `cmd` must be built,
/// so that each subcommand carries the global flags it inherits.
fn global_conflict(cmd: &clap::Command, matches: &clap::ArgMatches) -> Option<GlobalConflict> {
    use clap::parser::ValueSource;

    let (mut cmd, mut matches) = (cmd, matches);
    while let Some((name, sub)) = matches.subcommand() {
        cmd = cmd.find_subcommand(name)?;
        matches = sub;
    }
    let given = |arg: &clap::Arg| {
        matches.value_source(arg.get_id().as_str()) == Some(ValueSource::CommandLine)
    };
    cmd.get_arguments()
        .filter(|arg| !arg.is_global_set() && given(arg))
        .find_map(|arg| {
            let global = cmd
                .get_arg_conflicts_with(arg)
                .into_iter()
                .find(|other| other.is_global_set() && given(other))?;
            let token = global.get_long().and_then(|long| {
                if !global.get_action().takes_values() {
                    return Some(format!("--{long}").into());
                }
                let value = matches.get_raw(global.get_id().as_str())?.next()?;
                let mut token = std::ffi::OsString::from(format!("--{long}="));
                token.push(value);
                Some(token)
            });
            Some(GlobalConflict {
                arg: arg.to_string(),
                global: global.to_string(),
                token,
            })
        })
}

impl GlobalConflict {
    /// The usage error clap gives for this command line with the global flag
    /// repeated after the verb's arguments, where clap does check it.
    ///
    /// The flag goes before a `--`, if there is one, so it stays a flag.
    /// Should the flag have no spelling, or that command line somehow parse,
    /// the error is built by hand from the same words, without a usage line.
    fn error(self, argv: &[std::ffi::OsString]) -> clap::Error {
        if let Some(token) = self.token {
            let mut moved = argv.to_vec();
            let at = moved.iter().position(|t| t == "--").unwrap_or(moved.len());
            moved.insert(at, token);
            if let Err(e) = Cli::command().try_get_matches_from(moved) {
                return e;
            }
        }
        Cli::command().error(
            clap::error::ErrorKind::ArgumentConflict,
            format!(
                "the argument '{}' cannot be used with '{}'",
                self.arg, self.global
            ),
        )
    }
}

/// Parse the command line, run it, and turn the outcome into an exit code.
///
/// A refusal exits 2 when it is a usage error and 1 otherwise, as
/// [`render::Refusal::exit`] says, in either mode. Under `--json` it is one
/// line of JSON on stderr, so stdout still holds nothing but a verb's
/// payload; without, it is the prose it always was.
///
/// A stdout that closed under the verb changes nothing here: the verb ran to
/// its end, and exits as it decided (see [`output`]). A write to stdout that
/// failed some other way is a refusal of its own, reported once the verb is
/// done and only if the verb did not refuse first.
pub(crate) fn main() -> ExitCode {
    let cli = parse_from(std::env::args_os()).unwrap_or_else(|e| e.exit());
    CorpusLock::label_process(&cli.lock_label());
    let locations =
        Locations::from_reader(|name| std::env::var_os(name), std::env::current_dir().ok());
    #[cfg(debug_assertions)]
    let _deadline = test_git_deadline();
    let json = cli.json;
    let outcome = locations
        .validate()
        .map_err(Failure::from)
        .and_then(|()| run(cli, &locations));
    let flushed = output::finish();
    match outcome.and_then(|code| flushed.map(|()| code).map_err(Failure::from)) {
        Ok(code) => code,
        Err(Failure(refused)) => {
            if json {
                errln!("{}", refused.json());
            } else {
                errln!("{} {}", render::error_label(), refused.prose());
            }
            refused.exit.into()
        }
    }
}

/// Debug builds only: every git command this process runs gets
/// `$NEBULA_TEST_GIT_DEADLINE_MS` milliseconds instead of its named deadline,
/// so an end-to-end test of the timeout does not wait two minutes. Read here
/// rather than in core, which never reads the environment; git runs on this
/// thread, which the guard covers. Never read by a release build.
#[cfg(debug_assertions)]
fn test_git_deadline() -> Option<nebula_core::GitDeadlineOverride> {
    let ms = std::env::var("NEBULA_TEST_GIT_DEADLINE_MS")
        .ok()?
        .parse()
        .ok()?;
    Some(nebula_core::GitDeadlineOverride::new(
        std::time::Duration::from_millis(ms),
    ))
}

type Outcome = std::result::Result<ExitCode, Failure>;

/// What [`run`] settles before dispatch, which a handler may need besides
/// its own arguments: where the corpus is, whether `--json` is on, and
/// whether to commit.
struct Invocation<'a> {
    locations: &'a Locations,
    root: Option<PathBuf>,
    json: bool,
    commits: CommitOpts,
}

/// Settle the pre-verb `--no-commit`, then hand the command to its handler
/// in [`commands`]: one arm per [`Command`] variant, each a single call.
#[allow(clippy::too_many_lines)] // A dispatch table is one arm per verb.
fn run(cli: Cli, locations: &Locations) -> Outcome {
    let root = cli.root.clone();
    let json = cli.json;
    let offered = cli.command.commit_arg();
    if cli.no_commit {
        // The spelling from when the flag was global (STD-01 §R35): still a
        // skip before a verb that commits, and refused before any work
        // elsewhere, where it would be accepted and do nothing (§R28).
        let verb = &cli.verb;
        if offered.is_none() {
            return Err(Failure::say(format!(
                "`{verb}` never commits, so `--no-commit` before it has nothing to skip; drop it"
            )));
        }
        errln!(
            "warning: `--no-commit` before the verb is deprecated; write `neb {verb} … --no-commit`"
        );
    }
    let commits = CommitOpts {
        skip: cli.no_commit || offered.is_some_and(|c| c.no_commit),
        json,
    };
    let cx = Invocation {
        locations,
        root,
        json,
        commits,
    };

    match cli.command {
        Command::Init {
            path,
            set_root,
            force,
        } => commands::corpus::init(cx, path, set_root, force),
        Command::Check => commands::corpus::check(cx),
        Command::Migrate { .. } => commands::corpus::migrate(cx),
        Command::Config {
            setting:
                ConfigSetting::ObservatoryRoot {
                    dir, drop_legacy, ..
                },
        } => commands::corpus::config_observatory_root(cx, dir, drop_legacy),
        Command::Config {
            setting: ConfigSetting::Commit { state, .. },
        } => commands::corpus::config_commit(cx, state),
        Command::Completions { shell } => commands::corpus::completions(cx, shell),
        Command::Capture { quiet, text, .. } => commands::inbox::capture(cx, quiet, text),
        Command::Inbox { limit } => commands::inbox::inbox(cx, limit),
        Command::Promote {
            entry,
            quiet,
            title,
            body,
            parents,
            tags,
            id,
            by,
            task,
            run,
            ..
        } => commands::inbox::promote(
            cx, entry, quiet, title, body, parents, tags, id, by, task, run,
        ),
        Command::Drop { entry, .. } => commands::inbox::drop(cx, entry),
        Command::Triage { by, .. } => commands::inbox::triage(cx, by),
        Command::New {
            title,
            body,
            parents,
            reopens,
            contradicts,
            kill,
            tags,
            id,
            by,
            task,
            run,
            ..
        } => commands::node::new(
            cx,
            title,
            body,
            parents,
            reopens,
            contradicts,
            kill,
            tags,
            id,
            by,
            task,
            run,
        ),
        Command::Edit { node, by, .. } => commands::node::edit(cx, node, by),
        Command::Sharpen {
            node,
            confirm: true,
            ..
        } => commands::node::confirm_kill(cx, node),
        Command::Sharpen {
            node,
            kill,
            by,
            confirm: false,
            ..
        } => commands::node::sharpen(cx, node, kill, by),
        Command::Status {
            node, status, why, ..
        } => commands::node::status(cx, node, status, why),
        Command::Link {
            from, kind, to, by, ..
        } => commands::node::link(cx, from, kind, to, by),
        Command::Tag {
            target,
            add,
            remove,
            ..
        } if target == "list" && add.is_empty() && remove.is_empty() => {
            commands::node::tag_list(cx)
        }
        Command::Tag {
            target,
            add,
            remove,
            ..
        } => commands::node::tag(cx, target, add, remove),
        Command::Note { node, by, text, .. } => commands::node::note(cx, node, by, text),
        Command::Cite {
            node,
            uri,
            kind,
            title,
            note,
            by,
            task,
            run,
            ..
        } => commands::reference::cite(cx, node, uri, kind, title, note, by, task, run),
        Command::Handoff {
            node,
            record,
            note,
            by,
            task,
            run,
            ..
        } => commands::reference::handoff(cx, node, record, note, by, task, run),
        Command::Show { node, at } => commands::query::show(cx, node, at),
        Command::Log { node } => commands::query::log(cx, node),
        Command::List {
            status,
            tags,
            limit,
        } => commands::query::list(cx, status, tags, limit),
        Command::Near { limit, query } => commands::query::near(cx, limit, query),
        Command::Trace { node, down, depth } => commands::query::trace(cx, node, down, depth),
        Command::Impact { node } => commands::query::impact(cx, node),
        Command::Graph { mermaid, from } => commands::query::graph(cx, mermaid, from),
        Command::Review {
            short: true,
            tags,
            limit,
            ..
        } => commands::maintenance::review_short(cx, tags, limit),
        Command::Review {
            short: false,
            since,
            out,
            limit,
            ..
        } => commands::maintenance::review(cx, since, out, limit),
        Command::Open { tags } => commands::maintenance::open(cx, tags),
    }
}

#[cfg(test)]
mod tests;
