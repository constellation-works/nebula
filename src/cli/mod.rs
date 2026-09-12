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

use crate::corpus::{EdgeType, Status, Strength, Verdict};
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
  domain       Declare domains and move nodes between them
  check        Run the invariants. Exits non-zero on any error
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
  graduate     Hand a node downstream to principia or orbit-research
  link         Add a typed edge between two nodes

References:
  cite         Attach context. A reference explains; it never bears on truth
  evidence     Attach something that bears on whether the idea is true
  weigh        Promote a reference into evidence, once you know which way it cuts
  task         Record work spawned to settle this node

Query:
  show         Show one node in full
  list         List nodes
  trace        Walk ancestry. The feature the whole system exists for
  impact       What collapses if this node dies

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

    /// Declare domains and move nodes between them.
    ///
    /// A domain is a view inside one corpus, so `trace` and `impact` cross
    /// them freely. The boundary that needs separate storage, work against
    /// personal, is a separate corpus.
    #[command(subcommand)]
    Domain(DomainCommand),

    /// Run the invariants. Exits non-zero on any error.
    Check {
        /// Resolve cited Orbit task ids and confirm open `tasks` entries are still open.
        #[arg(long)]
        online: bool,
    },

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
        /// Domain it belongs to. Defaults to the corpus default.
        #[arg(long)]
        domain: Option<String>,
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
        /// Domain it belongs to. Defaults to the corpus default.
        #[arg(long)]
        domain: Option<String>,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
        /// What would falsify this. Required to start above seed.
        #[arg(long)]
        kill: Option<String>,
        /// Starting status.
        #[arg(long, default_value = "seed")]
        status: Status,
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
        /// The falsifier, written before any evidence arrives.
        #[arg(long)]
        kill: String,
    },

    /// Move a node to a new status, with the transition guards applied.
    Status {
        /// Node id.
        node: String,
        /// Target status.
        status: Status,
    },

    /// Hand a node downstream to principia or orbit-research.
    Graduate {
        /// Node id.
        node: String,
        /// Where it went.
        #[arg(long)]
        to: String,
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

    /// Attach context. A reference explains; it never bears on truth.
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

    /// Attach something that bears on whether the idea is true.
    Evidence {
        /// Node id.
        node: String,
        /// Which way it cuts.
        #[arg(long)]
        verdict: Verdict,
        /// How much it is worth.
        #[arg(long, default_value = "suggestive")]
        strength: Strength,
        /// Where it came from.
        #[arg(long)]
        source: String,
        /// What it showed, and what is shaky about it.
        #[arg(long)]
        note: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
    },

    /// Promote a reference into evidence, once you know which way it cuts.
    Weigh {
        /// Node id.
        node: String,
        /// Reference id, from `neb show`.
        reference: String,
        /// Which way it cuts.
        #[arg(long)]
        verdict: Verdict,
        /// How much it is worth.
        #[arg(long, default_value = "suggestive")]
        strength: Strength,
    },

    /// Record work spawned to settle this node.
    Task {
        /// Node id.
        node: String,
        /// Orbit task id.
        id: String,
        /// What it is meant to settle.
        #[arg(long)]
        why: Option<String>,
        /// Mark an existing link done or dropped.
        #[arg(long)]
        state: Option<String>,
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
        /// Only nodes carrying this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Only this domain. Defaults to the corpus default when several exist.
        #[arg(long)]
        domain: Option<String>,
        /// Every domain.
        #[arg(long, conflicts_with = "domain")]
        all: bool,
    },

    /// Walk ancestry. The feature the whole system exists for.
    Trace {
        /// Node id.
        node: String,
        /// Walk descendants instead of ancestors.
        #[arg(long)]
        down: bool,
    },

    /// What collapses if this node dies.
    Impact {
        /// Node id.
        node: String,
    },

    /// Nodes that need attention.
    Open {
        /// Only this domain. Defaults to the corpus default when several exist.
        #[arg(long)]
        domain: Option<String>,
        /// Every domain.
        #[arg(long, conflicts_with = "domain")]
        all: bool,
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

#[derive(Subcommand)]
enum DomainCommand {
    /// Declared domains, with node counts.
    List,
    /// Declare a domain.
    Add {
        /// Lowercase letters, digits and dashes.
        name: String,
    },
    /// Choose where bare `list` and `open` look.
    Default {
        /// A declared domain.
        name: String,
    },
    /// Move a node to a domain.
    Set {
        /// Node id. Omit when using `--unplaced`.
        #[arg(conflicts_with = "unplaced")]
        node: Option<String>,
        /// A declared domain for a single node.
        name: Option<String>,
        /// Place every node whose domain is empty into this declared domain.
        #[arg(long, value_name = "DOMAIN", conflicts_with = "node")]
        unplaced: Option<String>,
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

#[allow(clippy::too_many_lines)]
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
            domain,
            parents,
            tags,
            task,
            run,
        } => c::promote(
            root,
            &entry,
            title,
            domain.as_deref(),
            &parents,
            &tags,
            task,
            run,
        )
        .map(|()| ok),
        Command::New {
            title,
            domain,
            parents,
            kill,
            status,
            tags,
            task,
            run,
        } => c::new_node(
            root,
            &title,
            domain.as_deref(),
            &parents,
            kill,
            status,
            &tags,
            task,
            run,
        )
        .map(|()| ok),
        Command::Sharpen { node, kill } => c::sharpen(root, &node, &kill).map(|()| ok),
        Command::Link { from, kind, to } => c::link(root, &from, kind, &to).map(|()| ok),
        Command::Evidence {
            node,
            verdict,
            strength,
            source,
            note,
            task,
        } => c::evidence(root, &node, verdict, strength, &source, note, task).map(|()| ok),
        Command::Cite {
            node,
            uri,
            kind,
            title,
            note,
            task,
            run,
        } => c::cite(root, &node, &uri, &kind, title, note, task, run).map(|()| ok),
        Command::Weigh {
            node,
            reference,
            verdict,
            strength,
        } => c::weigh(root, &node, &reference, verdict, strength).map(|()| ok),
        Command::Status { node, status } => c::set_status(root, &node, status).map(|()| ok),
        Command::Task {
            node,
            id,
            why,
            state,
        } => c::task(root, &node, &id, why, state).map(|()| ok),
        Command::Graduate { node, to } => c::graduate(root, &node, &to).map(|()| ok),
        Command::Trace { node, down } => c::trace(root, &node, down, json).map(|()| ok),
        Command::Impact { node } => c::impact(root, &node, json).map(|()| ok),
        Command::Open { domain, all } => c::open(root, domain.as_deref(), all, json).map(|()| ok),
        Command::Show { node } => c::show(root, &node, json).map(|()| ok),
        Command::Review { since, out } => c::review(root, since, out.as_deref(), json).map(|()| ok),
        Command::List {
            status,
            tag,
            domain,
            all,
        } => c::list(root, status, tag.as_deref(), domain.as_deref(), all, json).map(|()| ok),
        Command::Domain(cmd) => match cmd {
            DomainCommand::List => c::domain_list(root, json),
            DomainCommand::Add { name } => c::domain_add(root, &name),
            DomainCommand::Default { name } => c::domain_default(root, &name),
            DomainCommand::Set {
                node,
                name,
                unplaced,
            } => c::domain_set(root, node.as_deref(), name.as_deref(), unplaced.as_deref()),
        }
        .map(|()| ok),
        Command::Check { online } => c::check(root, json, online),
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
        assert_eq!(seen, 23, "template rows need updating for a new subcommand");
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
            ["init", "domain", "check", "completions"].as_slice(),
            &["capture", "inbox", "promote", "drop"],
            &["new", "sharpen", "status", "graduate", "link"],
            &["cite", "evidence", "weigh", "task"],
            &["show", "list", "trace", "impact"],
            &["open", "review"],
        ]
        .concat();
        assert_eq!(names, expected);
    }
}
