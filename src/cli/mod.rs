//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `commands`. Nothing else in the crate depends on clap.
//!
//! The grouped sections in `neb --help` are a render concern, not a structure
//! one. Clap's derive has no per-variant `help_heading` for subcommands
//! (`next_help_heading` is args-only and `subcommand_help_heading` only renames
//! the single `Commands:` block), so [`Cli`] carries a hand-rolled
//! `help_template` with the rows written out by hand. When adding a command,
//! add the variant to [`Command`] in the position its section dictates *and*
//! add its row to the template; a `#[test]` below checks the two stay in sync.

use crate::corpus::{EdgeType, Status};
use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;
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
        status: Status,
        /// Why it closed. Required for refuted, optional for abandoned.
        #[arg(long)]
        why: Option<String>,
    },

    /// Add a typed edge between two nodes.
    Link {
        /// Node the edge starts at.
        from: String,
        /// The relation.
        kind: EdgeType,
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
        status: Option<Status>,
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

/// Parse the command line, run it, and turn the outcome into an exit code.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e:#}", crate::render::paint("31;1", "error:"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    use crate::commands as c;
    let ok = ExitCode::SUCCESS;
    let root = cli.root.clone();
    let json = cli.json;

    match cli.command {
        Command::Init { path } => c::init(root, path).map(|()| ok),
        Command::Capture { text } => c::capture(root, &text.join(" ")).map(|()| ok),
        Command::Inbox => c::inbox(root, json).map(|()| ok),
        Command::Drop { entry } => c::drop_entry(root, &entry).map(|()| ok),
        Command::Promote {
            entry,
            title,
            parents,
            tags,
            task,
            run,
        } => c::promote(root, &entry, title, &parents, &tags, task, run).map(|()| ok),
        Command::New {
            title,
            parents,
            kill,
            tags,
            task,
            run,
        } => c::new_node(root, &title, &parents, kill, &tags, task, run).map(|()| ok),
        Command::Sharpen { node, kill } => c::sharpen(root, &node, &kill).map(|()| ok),
        Command::Link { from, kind, to } => c::link(root, &from, kind, &to).map(|()| ok),
        Command::Cite {
            node,
            uri,
            kind,
            title,
            note,
            task,
            run,
        } => c::cite(root, &node, &uri, &kind, title, note, task, run).map(|()| ok),
        Command::Status { node, status, why } => {
            c::set_status(root, &node, status, why).map(|()| ok)
        }
        Command::Tag {
            target,
            add,
            remove,
        } if target == "list" && add.is_empty() && remove.is_empty() => {
            c::tag_list(root, json).map(|()| ok)
        }
        Command::Tag {
            target,
            add,
            remove,
        } => c::tag(root, &target, &add, &remove).map(|()| ok),
        Command::Trace { node, down } => c::trace(root, &node, down, json).map(|()| ok),
        Command::Impact { node } => c::impact(root, &node, json).map(|()| ok),
        Command::Open { tags } => c::open(root, &tags, json).map(|()| ok),
        Command::Show { node } => c::show(root, &node, json).map(|()| ok),
        Command::Review { since, out } => c::review(root, since, out.as_deref(), json).map(|()| ok),
        Command::List { status, tags } => c::list(root, status, &tags, json).map(|()| ok),
        Command::Check => c::check(root, json),
        Command::Migrate => c::migrate(root).map(|()| ok),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "neb", &mut std::io::stdout());
            Ok(ok)
        }
    }
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
        assert_eq!(seen, 20, "template rows need updating for a new subcommand");
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
            &["show", "list", "trace", "impact"],
            &["open", "review"],
        ]
        .concat();
        assert_eq!(names, expected);
    }
}
