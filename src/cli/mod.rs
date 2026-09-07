//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `commands`. Nothing else in the crate depends on clap.

use crate::corpus::{EdgeType, Status, Strength, Verdict};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

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
                  $NEBULA_ROOT, else ~/.nebula."
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

    /// Discard an inbox entry. Struck through, never deleted.
    Drop {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
    },

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
    },

    /// Sharpen a seed into a hypothesis by naming what would kill it.
    Sharpen {
        /// Node id.
        node: String,
        /// The falsifier, written before any evidence arrives.
        #[arg(long)]
        kill: String,
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

    /// Move a node to a new status, with the transition guards applied.
    Status {
        /// Node id.
        node: String,
        /// Target status.
        status: Status,
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

    /// Hand a node downstream to principia or orbit-research.
    Graduate {
        /// Node id.
        node: String,
        /// Where it went.
        #[arg(long)]
        to: String,
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

    /// Declare domains and move nodes between them.
    ///
    /// A domain is a view inside one corpus, so `trace` and `impact` cross
    /// them freely. The boundary that needs separate storage, work against
    /// personal, is a separate corpus.
    #[command(subcommand)]
    Domain(DomainCommand),

    /// Run the invariants. Exits non-zero on any error.
    Check,
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
        /// Node id.
        node: String,
        /// A declared domain.
        name: String,
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
            domain,
            parents,
            tags,
        } => c::promote(root, &entry, title, domain.as_deref(), &parents, &tags).map(|()| ok),
        Command::New {
            title,
            domain,
            parents,
            kill,
            status,
            tags,
        } => c::new_node(
            root,
            &title,
            domain.as_deref(),
            &parents,
            kill,
            status,
            &tags,
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
        } => c::cite(root, &node, &uri, &kind, title, note).map(|()| ok),
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
            DomainCommand::Set { node, name } => c::domain_set(root, &node, &name),
        }
        .map(|()| ok),
        Command::Check => c::check(root, json),
    }
}
