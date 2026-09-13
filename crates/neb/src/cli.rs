//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `nebula_core`. Nothing else in the crate knows about clap.
//!
//! Dispatch is a table. Each arm parses, makes one core call, and either
//! writes the returned value as JSON or hands it to `render`. A command that
//! wants to do anything else belongs in core.
//!
//! [`StatusArg`] and [`EdgeKindArg`] exist because core does not depend on
//! clap: they are the `ValueEnum` wrappers that keep `--help` listing the
//! possible values and the shell completions offering them.
//!
//! The grouped sections in `neb --help` are a render concern, not a structure
//! one. Clap's derive has no per-variant `help_heading` for subcommands
//! (`next_help_heading` is args-only and `subcommand_help_heading` only renames
//! the single `Commands:` block), so [`Cli`] carries a hand-rolled
//! `help_template` with the rows written out by hand. When adding a command,
//! add the variant to [`Command`] in the position its section dictates *and*
//! add its row to the template; a `#[test]` below checks the two stay in sync.

use crate::render;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use nebula_core::{
    Citation, Corpus, Direction, EdgeType, Error, Graph, NewNode, Origin, Promotion, Severity,
    Status, check, graph, migrate, ops,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `neb --help`, with the subcommands grouped by lifecycle stage. Each row's
/// one-liner is the first line of that variant's doc comment, minus the
/// trailing period clap strips; `help_rows_match_the_variants` enforces it.
const HELP_TEMPLATE: &str = "\
{before-help}{about-with-newline}
{usage-heading} {usage}

Corpus:
  init         Create an empty corpus
  check        Run the invariants. Exits non-zero on any error
  migrate      Bring a v1 corpus forward to the v2 schema, in place
  completions  Generate shell completion scripts

Inbox:
  capture      Append a thought to the inbox. One line, no decisions, no parent
  inbox        List captures that have not been promoted or dropped
  promote      Turn an inbox entry into a seed node
  drop         Discard an inbox entry. Struck through, never deleted

Nodes:
  new          Create a node directly, without going through the inbox
  sharpen      Sharpen a seed into a hypothesis by naming what would kill it
  status       Move a node to a new status, with the transition guards applied
  link         Add a typed edge between two nodes
  tag          Edit a node's tags, or list every tag with its node count

References:
  cite         Attach context, with a note saying why it is here

Query:
  show         Show one node in full
  list         List nodes
  trace        Walk ancestry. The feature the whole system exists for
  impact       What descends from this node, and what contradicts it
  graph        The whole corpus as nodes and edges, for a tool that draws it

Maintenance:
  open         Nodes that need attention
  review       The weekly maintenance report: stale hypotheses, untouched seeds,
               nodes with no references, and inbox entries waiting too long

Options:
{options}{after-help}";

#[derive(Parser)]
#[command(
    name = "neb",
    version,
    about = "Idea lineage graph",
    long_about = "Capture a half-formed thought in five seconds. Trace where any idea came \
                  from years later.\n\nIdeas branch, merge and die, so the structure is a \
                  directed acyclic graph. Nothing is ever deleted: refuted and abandoned \
                  ideas are what stop you re-treading ground.",
    after_help = "The corpus lives outside this repository. It is found via --root, else \
                  $NEBULA_ROOT, else ~/.nebula.",
    disable_help_subcommand = true,
    help_template = HELP_TEMPLATE
)]
struct Cli {
    /// Corpus location. Defaults to `$NEBULA_ROOT`, else `~/.nebula`.
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Emit JSON instead of text, for scripts and agents.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

/// [`Status`] as a command-line value.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
enum StatusArg {
    Seed,
    Hypothesis,
    Refuted,
    Abandoned,
}

impl From<StatusArg> for Status {
    fn from(s: StatusArg) -> Self {
        match s {
            StatusArg::Seed => Self::Seed,
            StatusArg::Hypothesis => Self::Hypothesis,
            StatusArg::Refuted => Self::Refuted,
            StatusArg::Abandoned => Self::Abandoned,
        }
    }
}

/// [`EdgeType`] as a command-line value.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum EdgeKindArg {
    DerivesFrom,
    Refines,
    Generalizes,
    Reopens,
    Contradicts,
}

impl From<EdgeKindArg> for EdgeType {
    fn from(k: EdgeKindArg) -> Self {
        match k {
            EdgeKindArg::DerivesFrom => Self::DerivesFrom,
            EdgeKindArg::Refines => Self::Refines,
            EdgeKindArg::Generalizes => Self::Generalizes,
            EdgeKindArg::Reopens => Self::Reopens,
            EdgeKindArg::Contradicts => Self::Contradicts,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty corpus.
    Init {
        /// Where to create it. Defaults to the resolved corpus location.
        path: Option<PathBuf>,
    },

    /// Run the invariants. Exits non-zero on any error.
    Check,

    /// Bring a v1 corpus forward to the v2 schema, in place.
    ///
    /// Idempotent, and refused while the corpus has uncommitted git changes
    /// so the migration lands as its own commit. Evidence, task links and
    /// the removed edge kinds become references; nothing is dropped.
    Migrate,

    /// Generate shell completion scripts.
    Completions {
        /// The shell to generate completions for.
        shell: clap_complete::Shell,
    },

    /// Append a thought to the inbox. One line, no decisions, no parent.
    ///
    /// This is the five-second path. It deliberately asks nothing of you,
    /// because a capture step that requires decisions is a capture step you
    /// will skip at the exact moment the idea arrives.
    Capture {
        /// The thought, as you would say it out loud.
        #[arg(required = true, trailing_var_arg = true)]
        text: Vec<String>,
    },

    /// List captures that have not been promoted or dropped.
    Inbox,

    /// Turn an inbox entry into a seed node.
    ///
    /// Deliberately separate from capture. Most captures should never be
    /// promoted, and dropping one is a normal outcome rather than a failure.
    Promote {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
        /// Node title. Defaults to the captured text.
        #[arg(long)]
        title: Option<String>,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
        /// Labels.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
    },

    /// Discard an inbox entry. Struck through, never deleted.
    Drop {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
    },

    /// Create a node directly, without going through the inbox.
    New {
        /// Node title.
        title: String,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
        /// What would falsify this. Naming one starts the node as a hypothesis.
        #[arg(long)]
        kill: Option<String>,
        /// Labels.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
    },

    /// Sharpen a seed into a hypothesis by naming what would kill it.
    Sharpen {
        /// Node id.
        node: String,
        /// The falsifier, written before anything is read.
        #[arg(long)]
        kill: String,
    },

    /// Move a node to a new status, with the transition guards applied.
    ///
    /// `refuted` needs `--why`, saying how the kill condition fired, and is
    /// final: reviving the idea takes a new node with a `reopens` edge.
    /// `abandoned` takes `--why` optionally.
    Status {
        /// Node id.
        node: String,
        /// Target status.
        status: StatusArg,
        /// Why it closed. Required for refuted, optional for abandoned.
        #[arg(long)]
        why: Option<String>,
    },

    /// Add a typed edge between two nodes.
    Link {
        /// Node the edge starts at.
        from: String,
        /// The relation.
        kind: EdgeKindArg,
        /// Node the edge points at.
        to: String,
    },

    /// Edit a node's tags, or list every tag with its node count.
    ///
    /// Tags are normalised to lowercase kebab-case on the way in.
    Tag {
        /// Node id, or `list` to show every tag in the corpus with a count.
        target: String,
        /// A tag to add. Repeatable.
        #[arg(long = "add", value_name = "TAG")]
        add: Vec<String>,
        /// A tag to remove. Repeatable.
        #[arg(long = "remove", value_name = "TAG")]
        remove: Vec<String>,
    },

    /// Attach context, with a note saying why it is here.
    Cite {
        /// Node id.
        node: String,
        /// Where it lives. A URL, DOI, path, or almanac wikilink.
        #[arg(long)]
        uri: String,
        /// paper, study, article, note, discussion, book, dataset, thread, other.
        #[arg(long, default_value = "other")]
        kind: String,
        /// Human-readable name.
        #[arg(long)]
        title: Option<String>,
        /// Why this is attached. The only field that matters in a year.
        #[arg(long)]
        note: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
    },

    /// Show one node in full.
    Show {
        /// Node id.
        node: String,
    },

    /// List nodes.
    List {
        /// Only this status.
        #[arg(long)]
        status: Option<StatusArg>,
        /// Only nodes carrying this tag. Repeat to require every one.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },

    /// Walk ancestry. The feature the whole system exists for.
    Trace {
        /// Node id.
        node: String,
        /// Walk descendants instead of ancestors.
        #[arg(long)]
        down: bool,
    },

    /// What descends from this node, and what contradicts it.
    Impact {
        /// Node id.
        node: String,
    },

    /// The whole corpus as nodes and edges, for a tool that draws it.
    ///
    /// JSON only: a whole-corpus DAG has no useful text form, and `trace`
    /// already draws the part of it you can read in a terminal.
    Graph,

    /// Nodes that need attention.
    ///
    /// Hypotheses with no references, seeds untouched for ninety days, and
    /// inbox captures waiting fourteen days or more.
    Open {
        /// Only nodes carrying this tag. Repeat to require every one.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },

    /// The weekly maintenance report: stale hypotheses, untouched seeds,
    /// nodes with no references, and inbox entries waiting too long.
    ///
    /// Read-only, by the spec's hard rule: this proposes and never mutates a
    /// node, an inbox entry, or the manifest.
    Review {
        /// Override the day thresholds for stale hypotheses (default 30) and
        /// untouched seeds (default 90). The inbox's fourteen-day rule is
        /// unaffected; it is `open`'s rule, reused rather than duplicated.
        #[arg(long)]
        since: Option<i64>,
        /// Write the report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

/// A message the CLI exits on. Core errors become one through [`render`], so
/// the advice that names commands stays in the crate that has commands.
struct Failure(String);

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self(render::message(&e))
    }
}

impl From<serde_json::Error> for Failure {
    fn from(e: serde_json::Error) -> Self {
        Self(format!("could not write JSON: {e}"))
    }
}

impl Failure {
    /// An error raised about one node, so the hint can name it.
    fn about(e: &Error, node: &str) -> Self {
        Self(render::message_about(e, node))
    }

    fn say(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Parse the command line, run it, and turn the outcome into an exit code.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {}", render::paint("31;1", "error:"), e.0);
            ExitCode::FAILURE
        }
    }
}

type Outcome = std::result::Result<ExitCode, Failure>;

#[allow(clippy::too_many_lines)] // A dispatch table is one arm per verb.
fn run(cli: Cli) -> Outcome {
    let ok = ExitCode::SUCCESS;
    let root = cli.root.clone();
    let json = cli.json;

    match cli.command {
        Command::Init { path } => {
            let done = ops::init(root, path)?;
            println!("corpus ready at {}", done.root.display());
            println!(
                "\nExport it so every command finds it:\n  export NEBULA_ROOT={}",
                done.root.display()
            );
            Ok(ok)
        }

        Command::Check => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let g = Graph::build(&docs)?;
            let report = check::run(&g, &corpus)?;
            if json {
                out_json(&report)?;
            } else {
                print!("{}", render::check(&report));
            }
            Ok(
                if report.findings.iter().all(|f| f.level != Severity::Error) {
                    ok
                } else {
                    ExitCode::FAILURE
                },
            )
        }

        Command::Migrate => {
            let report = migrate::run(root)?;
            if json {
                out_json(&report)?;
            } else {
                print!("{}", render::migration(&report));
            }
            Ok(ok)
        }

        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "neb", &mut std::io::stdout());
            Ok(ok)
        }

        Command::Capture { text } => {
            let text = text.join(" ");
            if text.trim().is_empty() {
                return Err(Failure::say("nothing to capture"));
            }
            // Capture must work on a corpus that does not exist yet. Being
            // told to run a setup command is precisely the friction that
            // loses the thought.
            let corpus = Corpus::open_or_init(root)?;
            let entry = ops::capture(&corpus, &text)?;
            println!("{}", render::bold(&entry.id));
            Ok(ok)
        }

        Command::Inbox => {
            let corpus = Corpus::open(root)?;
            let inbox = corpus.inbox()?;
            if json {
                out_json(&inbox)?;
            } else {
                print!("{}", render::inbox(&inbox));
            }
            Ok(ok)
        }

        Command::Promote {
            entry,
            title,
            parents,
            tags,
            task,
            run,
        } => {
            let corpus = Corpus::open(root)?;
            let created = ops::promote(
                &corpus,
                &entry,
                &Promotion {
                    title,
                    parents,
                    tags,
                    origin: Origin::of(task, run),
                },
            )?;
            println!(
                "{} {}",
                render::bold(&created.doc.node.id),
                render::dim(&created.path.display().to_string())
            );
            Ok(ok)
        }

        Command::Drop { entry } => {
            let corpus = Corpus::open(root)?;
            ops::drop(&corpus, &entry)?;
            println!("dropped {}", render::bold(&entry));
            Ok(ok)
        }

        Command::New {
            title,
            parents,
            kill,
            tags,
            task,
            run,
        } => {
            let corpus = Corpus::open(root)?;
            let created = ops::new_node(
                &corpus,
                &NewNode {
                    title,
                    parents,
                    kill,
                    tags,
                    origin: Origin::of(task, run),
                },
            )?;
            println!(
                "{} {}",
                render::bold(&created.doc.node.id),
                render::dim(&created.path.display().to_string())
            );
            Ok(ok)
        }

        Command::Sharpen { node, kill } => {
            let corpus = Corpus::open(root)?;
            let doc = ops::sharpen(&corpus, &node, &kill).map_err(|e| Failure::about(&e, &node))?;
            println!("{} is now {}", render::bold(&node), doc.node.status);
            Ok(ok)
        }

        Command::Status { node, status, why } => {
            let status = Status::from(status);
            // `--why` is this command's flag, so what it does and does not
            // apply to is this command's rule to state.
            if status.is_open() && why.as_ref().is_some_and(|w| !w.trim().is_empty()) {
                return Err(Failure::say("--why only applies to refuted or abandoned"));
            }
            let corpus = Corpus::open(root)?;
            let changed = ops::set_status(&corpus, &node, status, why.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            println!(
                "{} {} -> {status}",
                render::bold(&node),
                render::dim(&changed.from.to_string())
            );
            Ok(ok)
        }

        Command::Link { from, kind, to } => {
            let corpus = Corpus::open(root)?;
            let kind = EdgeType::from(kind);
            ops::link(&corpus, &from, kind, &to)?;
            println!(
                "{} {} {}",
                render::bold(&from),
                render::dim(&kind.to_string()),
                render::bold(&to)
            );
            Ok(ok)
        }

        Command::Tag {
            target,
            add,
            remove,
        } if target == "list" && add.is_empty() && remove.is_empty() => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let counts = graph::tags(&Graph::build(&docs)?)?;
            if json {
                out_json(&counts)?;
            } else {
                print!("{}", render::tags(&counts));
            }
            Ok(ok)
        }

        Command::Tag {
            target,
            add,
            remove,
        } => {
            if add.is_empty() && remove.is_empty() {
                return Err(Failure::say(
                    "nothing to do; pass --add <tag> or --remove <tag>",
                ));
            }
            let corpus = Corpus::open(root)?;
            let mut doc = ops::tag_remove(&corpus, &target, &remove)?;
            if !add.is_empty() {
                doc = ops::tag_add(&corpus, &target, &add)?;
            }
            let shown = if doc.node.tags.is_empty() {
                render::dim("(no tags)")
            } else {
                doc.node.tags.join(", ")
            };
            println!("{} {shown}", render::bold(&target));
            Ok(ok)
        }

        Command::Cite {
            node,
            uri,
            kind,
            title,
            note,
            task,
            run,
        } => {
            let corpus = Corpus::open(root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            let cited = ops::cite(
                &corpus,
                &node,
                &Citation {
                    uri,
                    kind,
                    title,
                    note,
                    origin: Origin::of(task, run),
                },
            )?;
            println!("{} {}", render::bold(&node), render::bold(&cited.reference));
            if bare {
                println!(
                    "\n{}",
                    render::dim(
                        "No note. Add one saying why it is here, or this is a link that rots."
                    )
                );
            }
            Ok(ok)
        }

        Command::Show { node } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let view = graph::node(&Graph::build(&docs)?, &node)?;
            if json {
                out_json(&view)?;
            } else {
                print!("{}", render::node(&view));
            }
            Ok(ok)
        }

        Command::List { status, tags } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let listing = graph::list(&Graph::build(&docs)?, status.map(Status::from), &tags)?;
            if json {
                out_json(&listing)?;
            } else {
                print!("{}", render::list(&listing.0, docs.len()));
            }
            Ok(ok)
        }

        Command::Trace { node, down } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let direction = if down { Direction::Down } else { Direction::Up };
            let walk = graph::trace(&Graph::build(&docs)?, &node, direction)?;
            if json {
                out_json(&walk)?;
            } else {
                print!("{}", render::tree(&docs, &node, direction));
            }
            Ok(ok)
        }

        Command::Impact { node } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let report = graph::impact(&Graph::build(&docs)?, &node)?;
            if json {
                out_json(&report)?;
            } else {
                print!("{}", render::impact(&report, &node));
            }
            Ok(ok)
        }

        Command::Graph => {
            if !json {
                println!("neb graph is JSON only; run:  neb graph --json");
                return Ok(ExitCode::from(2));
            }
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            out_json(&graph::export(&Graph::build(&docs)?)?)?;
            Ok(ok)
        }

        Command::Open { tags } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let report = graph::open(&Graph::build(&docs)?, &corpus.inbox()?, &tags)?;
            if json {
                out_json(&report)?;
            } else {
                print!("{}", render::open(&report));
            }
            Ok(ok)
        }

        Command::Review { since, out } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let report = graph::review(&Graph::build(&docs)?, &corpus.inbox()?, since)?;
            let text = if json {
                serde_json::to_string_pretty(&report).map_err(Failure::from)?
            } else {
                render::review(
                    &report,
                    since.unwrap_or(nebula_core::HYPOTHESIS_DAYS),
                    since.unwrap_or(nebula_core::SEED_DAYS),
                )
            };
            write_report(out.as_deref(), &text)?;
            Ok(ok)
        }
    }
}

/// Pretty JSON on stdout, which is what `--json` means everywhere.
fn out_json<T: serde::Serialize>(v: &T) -> std::result::Result<(), Failure> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Send a rendered report to a file, or print it, per `--out`.
fn write_report(out: Option<&Path>, text: &str) -> std::result::Result<(), Failure> {
    match out {
        Some(path) => std::fs::write(path, format!("{text}\n"))
            .map_err(|e| Failure::say(format!("writing {}: {e}", path.display())))?,
        None => println!("{text}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help() -> String {
        Cli::command().render_long_help().to_string()
    }

    fn squash(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Every subcommand clap knows about has a row in the template, and the
    /// row's text is the variant's own one-liner, so the two cannot drift.
    #[test]
    fn help_rows_match_the_variants() {
        let flat = squash(&help());
        let mut seen = 0;
        for sub in Cli::command().get_subcommands() {
            let about = sub.get_about().map(ToString::to_string).unwrap_or_default();
            let row = squash(&format!("{} {about}", sub.get_name()));
            assert!(flat.contains(&row), "help is missing the row {row:?}");
            seen += 1;
        }
        assert_eq!(seen, 21, "template rows need updating for a new subcommand");
        assert!(
            Cli::command().find_subcommand("help").is_none(),
            "clap's `help` subcommand should be disabled"
        );
    }

    /// The sections appear in lifecycle order, and clap's own usage line and
    /// global options are still rendered around them.
    #[test]
    fn help_sections_are_in_lifecycle_order() {
        let text = help();
        let headings = [
            "Corpus:",
            "Inbox:",
            "Nodes:",
            "References:",
            "Query:",
            "Maintenance:",
            "Options:",
        ];
        let mut last = 0;
        for h in headings {
            let at = text
                .find(&format!("\n{h}\n"))
                .unwrap_or_else(|| panic!("no {h} section"));
            assert!(at > last, "{h} is out of order");
            last = at;
        }
        assert!(text.contains("Usage: neb [OPTIONS] <COMMAND>"));
        assert!(text.contains("--root <DIR>"));
        assert!(text.contains("The corpus lives outside this repository"));
    }

    /// The variant order mirrors the template's section order, so a command
    /// left out of the template would still surface next to its group.
    #[test]
    fn variant_order_matches_the_template() {
        let names: Vec<_> = Cli::command()
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        let expected = [
            ["init", "check", "migrate", "completions"].as_slice(),
            &["capture", "inbox", "promote", "drop"],
            &["new", "sharpen", "status", "link", "tag"],
            &["cite"],
            &["show", "list", "trace", "impact", "graph"],
            &["open", "review"],
        ]
        .concat();
        assert_eq!(names, expected);
    }

    /// The wrappers are the only place a status or an edge kind is spelled
    /// for the command line, so they have to agree with the core types they
    /// stand in for.
    #[test]
    fn value_enums_match_the_core_types() {
        for arg in [
            StatusArg::Seed,
            StatusArg::Hypothesis,
            StatusArg::Refuted,
            StatusArg::Abandoned,
        ] {
            let spelled = arg.to_possible_value().unwrap();
            assert_eq!(spelled.get_name(), Status::from(arg).to_string());
        }
        for arg in [
            EdgeKindArg::DerivesFrom,
            EdgeKindArg::Refines,
            EdgeKindArg::Generalizes,
            EdgeKindArg::Reopens,
            EdgeKindArg::Contradicts,
        ] {
            let spelled = arg.to_possible_value().unwrap();
            assert_eq!(spelled.get_name(), EdgeType::from(arg).to_string());
        }
    }
}
