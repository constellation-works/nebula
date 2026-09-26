//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `nebula_core`. Nothing else in the crate knows about clap.
//!
//! Dispatch is a table. Each arm parses, makes one core call, and either
//! writes the returned value as JSON, through [`render::json`]'s view where
//! core's serialisation would leave a field out, or hands it to `render`. A
//! command that wants to do anything else belongs in core.
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

use crate::output::{self, errln, out, outln};
use crate::render::{self, json};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use nebula_core::triage::{Action, Step};
use nebula_core::{
    Citation, CommitOutcome, Corpus, CorpusLock, Direction, EdgeType, Error, Graph, Handoff,
    InboxEntry, NEAR_DEFAULT, NewNode, OBSERVATORY, OBSERVATORY_ROOT_ENV, ObservatoryLink,
    ObservatoryRoot, ObservatorySource, Origin, Promotion, Severity, Status, Triage, check, graph,
    migrate, model, ops, store,
};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

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
  config       Read or set a corpus or machine setting
  completions  Generate shell completion scripts

Inbox:
  capture      Append a thought to the inbox. One line, no decisions, no parent
  inbox        List captures that have not been promoted or dropped
  promote      Turn an inbox entry into a seed node. Suggests parents, never picks one
  drop         Discard an inbox entry. Struck through, never deleted
  triage       Work through the inbox oldest first, one key per entry

Nodes:
  new          Create a node directly, without going through the inbox
  edit         Edit a node's body in $VISUAL or $EDITOR
  sharpen      Sharpen a seed into a hypothesis by naming what would kill it
  status       Move a node to a new status, with the transition guards applied
  link         Add a typed edge between two nodes
  tag          Edit a node's tags, or list every tag with its node count
  note         Append a dated paragraph of reasoning to a node body

References:
  cite         Attach context, with a note saying why it is here
  handoff      Hand a node off to an Observatory record: cite it and close the node

Query:
  show         Show one node in full
  log          List the commits that changed a node
  list         List nodes
  near         The existing nodes closest to some text, or to a node, and how close
  trace        Walk ancestry. The feature the whole system exists for
  impact       What descends from this node, and what contradicts it
  graph        The whole corpus as nodes and edges, for a tool that draws it

Maintenance:
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
                  $NEBULA_ROOT, else the nearest corpus at or above the current directory, \
                  else ~/.config/nebula/root, else ~/.nebula.",
    disable_help_subcommand = true,
    help_template = HELP_TEMPLATE
)]
struct Cli {
    /// Corpus location. Defaults to `$NEBULA_ROOT`, else the corpus the current directory is in,
    /// else `~/.config/nebula/root`, else `~/.nebula`.
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Emit JSON instead of text, for scripts and agents.
    #[arg(long, global = true)]
    json: bool,

    /// `--no-commit` written before the verb, the spelling from when it was
    /// global. Deprecated and hidden: each verb that writes offers the flag
    /// itself. Before one of those it still skips the commit, with a warning
    /// (STD-01 §R35); before a verb that never commits it is refused, since
    /// there is nothing for it to skip (STD-01 §R28).
    #[arg(long = "no-commit", hide = true)]
    no_commit: bool,

    #[command(subcommand)]
    command: Command,

    /// The verb as typed, a setting's name included (`config commit`), for
    /// the messages that name it and the lock's holder record. Filled in by
    /// [`parse_from`].
    #[arg(skip)]
    verb: String,
}

impl Cli {
    /// What this process records as the holder of any lock it takes:
    /// `neb edit a-node`, `neb capture`. The verb and the one node or entry
    /// it names, never the text it was given, which may be private and
    /// would sit in a file every other writer reads.
    fn lock_label(&self) -> String {
        let mut label = String::from("neb");
        for word in [Some(self.verb.as_str()), self.command.target()]
            .into_iter()
            .flatten()
            .filter(|word| !word.is_empty())
        {
            label.push(' ');
            label.push_str(word);
        }
        label
    }
}

/// `--no-commit`, flattened into each verb that writes so its `--help` offers
/// it and a read-only verb's does not.
#[derive(Args, Debug, Clone, Copy)]
struct CommitArg {
    /// Skip the commit this once, where `neb config commit on` would make one.
    #[arg(long)]
    no_commit: bool,
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
        /// Make this corpus the machine default in `~/.config/nebula/root`.
        #[arg(long)]
        set_root: bool,
        /// Replace a machine default that names a different corpus.
        #[arg(long, requires = "set_root")]
        force: bool,
    },

    /// Run the invariants. Exits non-zero on any error.
    Check,

    /// Bring a v1 corpus forward to the v2 schema, in place.
    ///
    /// Idempotent, and refused while the corpus has uncommitted git changes
    /// so the migration lands as its own commit. Evidence, task links and
    /// the removed edge kinds become references; nothing is dropped.
    Migrate {
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Read or set a corpus or machine setting.
    ///
    /// `commit` lives in the corpus's `config.yaml`, which stays
    /// machine-written: this rewrites it whole rather than inviting a hand
    /// edit. `observatory-root` lives per machine, because the corpus travels
    /// between machines and the checkout's path does not.
    Config {
        #[command(subcommand)]
        setting: ConfigSetting,
    },

    /// Generate shell completion scripts.
    ///
    /// The script is the same for every corpus and is never JSON, so
    /// `--root` and `--json` are refused here rather than ignored.
    /// `$NEBULA_ROOT` is not read.
    Completions {
        /// The shell to generate completions for.
        #[arg(conflicts_with_all = ["root", "json"])]
        shell: clap_complete::Shell,
    },

    /// Append a thought to the inbox. One line, no decisions, no parent.
    ///
    /// This is the five-second path. It deliberately asks nothing of you,
    /// because a capture step that requires decisions is a capture step you
    /// will skip at the exact moment the idea arrives. After the entry id it
    /// names the three existing nodes the thought reads closest to, for the
    /// triage that comes later; that is a suggestion, and nothing is linked.
    /// With no corpus at the root it creates one rather than refuse, and
    /// names the path it created on stderr. Text over several lines, typed,
    /// pasted or piped in with `-`, is joined onto one line with spaces. A
    /// thought already waiting in the inbox, bar case and spacing, is
    /// captured all the same, and stderr names the entry it repeats.
    Capture {
        /// Print the entry id alone, without the nearest nodes.
        #[arg(long, short)]
        quiet: bool,
        /// The thought, as you would say it out loud.
        ///
        /// Remaining words, so it can be typed without quotes. Not a trailing
        /// vararg: a flag after the text (`--quiet`, `--no-commit`) is still
        /// a flag. A dash-leading token belongs in quotes, or after `--`. A
        /// lone `-` reads the thought from standard input.
        #[arg(required = true, num_args = 1.., value_name = "TEXT|-")]
        text: Vec<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// List captures that have not been promoted or dropped.
    Inbox {
        /// Show at most N entries, oldest first, N at least 1. Defaults to
        /// every one.
        #[arg(long, value_name = "N", value_parser = count)]
        limit: Option<usize>,
    },

    /// Turn an inbox entry into a seed node. Suggests parents, never picks one.
    ///
    /// Deliberately separate from capture. Most captures should never be
    /// promoted, and dropping one is a normal outcome rather than a failure.
    /// Without `--parent` the node is written as a root and the three
    /// existing nodes it reads closest to are printed afterwards, so a parent
    /// that was defensible can still be linked; no edge is ever written from
    /// that list.
    #[command(after_long_help = "Examples:
  neb promote <entry>
  neb promote <entry> --title \"A sharper title\" --parent <id>
  neb promote <entry> --parent <id> --parent <id> --tag <tag>")]
    Promote {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
        /// Print the node id alone, without its path or the nearest nodes.
        /// `--json` carries the path.
        #[arg(long, short)]
        quiet: bool,
        /// Node title. Defaults to the captured text.
        #[arg(long)]
        title: Option<String>,
        /// Prose appended after the captured line. `-` reads standard input.
        #[arg(long, value_name = "TEXT|-", allow_hyphen_values = true)]
        body: Option<String>,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
        /// Labels.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Explicit id, overriding the title's slug. Same rules as a slug:
        /// lowercase words joined by single dashes, 60 characters or fewer.
        /// Without this or `--title`, a capture over five words gets its
        /// first five significant words, not the whole sentence.
        #[arg(long, value_name = "SLUG")]
        id: Option<String>,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Discard an inbox entry. Struck through, never deleted.
    Drop {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Work through the inbox oldest first, one key per entry.
    ///
    /// Each waiting entry is shown with its age and the nodes it reads
    /// closest to, numbered. One line then decides it: `p` promotes it as a
    /// root, `1`-`3` promotes it under that numbered node, `t` sets the title
    /// the promotion will use (`t <title>`, or `t` and then the title on the
    /// next line; a blank title goes back to the captured text), `d` drops
    /// it, `s` leaves it waiting, `q` stops, and `?` lists the keys. Nothing
    /// is linked unless a number is chosen.
    ///
    /// Each decision is the single verb it stands for, with the same
    /// refusals, the same `--by`, and, with `neb config commit on`, the same
    /// commit: one per promote or drop. On a terminal a refused key or
    /// decision is reported and the same entry asked about again. With
    /// standard input piped the keys are read one per line, and the first
    /// refusal ends the session non-zero, because the lines after it were
    /// written for an entry that did not move. End of input stops as `q`
    /// does, except while a title is waiting to be used: then it is a
    /// refusal naming the title, so it is never lost without a word. There
    /// is no `--json` form; a script runs `inbox`, `near`, `promote` and
    /// `drop` itself.
    #[command(after_long_help = "Examples:
  neb triage
  printf 'd\\ns\\nq\\n' | neb triage --no-commit
  printf 't A sharper title\\n1\\n' | neb triage --by <label>")]
    Triage {
        /// Who wrote the titles and chose the parents: `human`, or the
        /// agent's session or crew label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Create a node directly, without going through the inbox.
    New {
        /// Node title.
        title: String,
        /// The node's prose body. `-` reads standard input.
        #[arg(long, value_name = "TEXT|-", allow_hyphen_values = true)]
        body: Option<String>,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
        /// A refuted node this revives, written as a `reopens` edge. That
        /// edge is genealogy already, so do not name the node as a parent too.
        #[arg(long, value_name = "ID")]
        reopens: Option<String>,
        /// A node this contradicts, recorded on both nodes as `link` does.
        /// Repeatable.
        #[arg(long = "contradicts", value_name = "ID")]
        contradicts: Vec<String>,
        /// What would falsify this. Naming one starts the node as a hypothesis.
        #[arg(long)]
        kill: Option<String>,
        /// Labels.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Explicit id, overriding the title's slug. Same rules as a slug:
        /// lowercase words joined by single dashes, 60 characters or fewer.
        #[arg(long, value_name = "SLUG")]
        id: Option<String>,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Edit a node's body in $VISUAL or $EDITOR.
    ///
    /// The editor sees prose only, never YAML frontmatter. Every existing
    /// `## Notes` section is protected because notes are append-only. A body
    /// left as it was is not written, and stderr says so.
    Edit {
        /// Node id.
        node: String,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Sharpen a seed into a hypothesis by naming what would kill it.
    ///
    /// `--confirm` adopts the kill condition already on the node as the
    /// human's own: the text is untouched and nothing is appended, which is
    /// what takes the node off `review`'s unconfirmed list.
    Sharpen {
        /// Node id.
        node: String,
        /// The falsifier, written before anything is read.
        #[arg(long, required_unless_present = "confirm", conflicts_with = "confirm")]
        kill: Option<String>,
        /// Who wrote the kill condition: `human`, or the agent's session or
        /// crew label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL", conflicts_with = "confirm")]
        by: Option<String>,
        /// Stand behind the kill condition already there, as the human.
        #[arg(long)]
        confirm: bool,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Move a node to a new status, with the transition guards applied.
    ///
    /// `refuted` needs `--why`, saying how the kill condition fired, and is
    /// final: reviving the idea takes a new node with a `reopens` edge.
    /// `abandoned` takes `--why` optionally. A node with a kill condition
    /// cannot go back to `seed`; it reopens as a `hypothesis`.
    Status {
        /// Node id.
        node: String,
        /// Target status.
        status: StatusArg,
        /// Why it closed. Required for refuted, optional for abandoned.
        #[arg(long)]
        why: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Add a typed edge between two nodes.
    #[command(after_long_help = "Examples:
  neb link <id> derives-from <parent-id>
  neb link <id> refines <broader-id>
  neb link <id> contradicts <rival-id> --by <label>")]
    Link {
        /// Node the edge starts at.
        from: String,
        /// The relation.
        kind: EdgeKindArg,
        /// Node the edge points at.
        to: String,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Edit a node's tags, or list every tag with its node count.
    ///
    /// Tags are normalised to lowercase kebab-case on the way in. A tag new
    /// to the corpus that differs from one in use only by case or a trailing
    /// `s` is written, with a note on stderr naming the existing one. A tag
    /// already on the node to add, or not on it to remove, is noted on
    /// stderr; when that leaves the tags as they were, nothing is written.
    Tag {
        /// Node id, or `list` to show every tag in the corpus with a count.
        target: String,
        /// A tag to add. Repeatable.
        #[arg(long = "add", value_name = "TAG")]
        add: Vec<String>,
        /// A tag to remove. Repeatable.
        #[arg(long = "remove", value_name = "TAG")]
        remove: Vec<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Append a dated paragraph of reasoning to a node body.
    ///
    /// Creates a `## Notes` section at the end of the body if needed, then
    /// appends `- YYYY-MM-DD: <text>`. Repeated notes accumulate in order;
    /// earlier body text is not rewritten. Status, edges and tags are left
    /// as they are.
    Note {
        /// Node id.
        node: String,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        /// The reasoning, as they said it.
        ///
        /// Remaining words, so it can be typed without quotes. Not a trailing
        /// vararg: a flag after the text (`--no-commit`, `--by`) is still a
        /// flag. A dash-leading token belongs in quotes, or after `--`.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Attach context, with a note saying why it is here.
    #[command(after_long_help = "Examples:
  neb cite <id> --kind paper --uri https://doi.org/<doi> --note \"why it is here\"
  neb cite <id> --kind note --uri ../notes/<file>.md --note \"why it is here\"
  neb cite <id> --kind observatory --uri <record-id> --note \"why it is here\"")]
    Cite {
        /// Node id.
        node: String,
        /// Where it lives. A URL, DOI, path relative to `nodes/`, or
        /// almanac wikilink; never an absolute path.
        #[arg(long)]
        uri: Option<String>,
        /// paper, study, article, note, discussion, book, dataset, thread,
        /// observatory, other, in any case (stored lowercase). With
        /// `observatory`, `--uri` is a bare record id (`Q<nnn>`) resolved
        /// through the configured observatory root.
        #[arg(long, default_value = "other")]
        kind: String,
        /// Human-readable name.
        #[arg(long)]
        title: Option<String>,
        /// Why this is attached. The only field that matters in a year.
        #[arg(long)]
        note: Option<String>,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Hand a node off to an Observatory record: cite it and close the node.
    ///
    /// One write: an `observatory` reference to the record, and the node
    /// moved to `abandoned` with `why: handed off to <RECORD>`. A refuted or
    /// already closed node is refused. When this machine has an observatory
    /// root, the record must resolve under it; with none, the id is accepted
    /// and cannot be located yet, as with `cite --kind observatory`.
    #[command(after_long_help = "Examples:
  neb handoff <id> <record-id> --note \"the question this became\"
  neb handoff <id> <record-id> --note \"why it goes there\" --by <label>")]
    Handoff {
        /// Node id.
        node: String,
        /// The Observatory record id: Q, H, T or R followed by digits, as in
        /// `H<nnn>`.
        record: String,
        /// Why it goes there. The reference's note, and the only field that
        /// matters in a year.
        #[arg(long)]
        note: Option<String>,
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
        /// Orbit task that produced it.
        #[arg(long)]
        task: Option<String>,
        /// Orbit run that produced it.
        #[arg(long)]
        run: Option<String>,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Show one node in full.
    Show {
        /// Node id.
        node: String,
        /// Show the node at this commit hash, or at the end of this date as
        /// `neb log` dates commits, whatever the local timezone.
        #[arg(long, value_name = "HASH|YYYY-MM-DD")]
        at: Option<String>,
    },

    /// List the commits that changed a node.
    Log {
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
        /// Show at most N of the matching nodes, in corpus order, N at least
        /// 1. Defaults to every one.
        #[arg(long, value_name = "N", value_parser = count)]
        limit: Option<usize>,
    },

    /// The existing nodes closest to some text, or to a node, and how close.
    ///
    /// Word overlap over title, tags and body (BM25, title and tags weighted
    /// up; no embeddings, no network), best first. Each is banded: `strong`
    /// shares most of the query's distinctive words (a word-for-word copy is
    /// here), `some` a few, `weak` a word or two in passing. `--json` keeps
    /// the raw `score` beside the `band`. Asked about a node, a neighbour
    /// already linked to it says so, with the edge: `linked: parent
    /// (derives-from)`. Nodes sharing no word with the query are left out, so
    /// an empty answer means the thought is unlike anything here. For
    /// triage: run it on a capture, pick a parent if one is defensible,
    /// otherwise promote as a root. It suggests; `link` and `--parent` are
    /// still yours to run.
    #[command(after_long_help = "Examples:
  neb near gravity as a scarcity gradient
  neb near <id> -k 5
  neb near <id> --json")]
    Near {
        /// How many to return, at least 1.
        #[arg(
            long,
            short = 'k',
            value_name = "K",
            default_value_t = NEAR_DEFAULT,
            value_parser = count
        )]
        limit: usize,
        /// Free text, or the id of an existing node (which is then left out
        /// of the answer).
        ///
        /// Remaining words, so it can be typed without quotes. Not a trailing
        /// vararg: a flag after the query (`--limit`, `--json`) is still a
        /// flag. A dash-leading token belongs in quotes, or after `--`.
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
    },

    /// Walk ancestry. The feature the whole system exists for.
    #[command(after_long_help = "Examples:
  neb trace <id>
  neb trace <id> --down
  neb trace <id> --depth 1 --json")]
    Trace {
        /// Node id.
        node: String,
        /// Walk descendants instead of ancestors.
        #[arg(long)]
        down: bool,
        /// Stop N steps from the node: 1 is its parents, or its children
        /// with `--down`, and N is at least 1. Defaults to the whole walk.
        #[arg(long, value_name = "N", value_parser = count)]
        depth: Option<usize>,
    },

    /// What descends from this node, and what contradicts it.
    Impact {
        /// Node id.
        node: String,
    },

    /// The whole corpus as nodes and edges, for a tool that draws it.
    ///
    /// Use `--json` for structured data or `--mermaid` for a diagram that can
    /// be pasted into a document. Without either, it is refused as a usage
    /// error.
    Graph {
        /// Emit a Mermaid `graph BT` diagram.
        #[arg(long, conflicts_with = "json")]
        mermaid: bool,
        /// Limit the Mermaid diagram to this node's ancestors and descendants.
        #[arg(long, value_name = "ID", requires = "mermaid", conflicts_with = "json")]
        from: Option<String>,
    },

    /// The weekly maintenance report: stale hypotheses, untouched seeds,
    /// nodes with no references, and inbox entries waiting too long.
    ///
    /// Read-only, by the spec's hard rule: this proposes and never mutates a
    /// node, an inbox entry, or the manifest. `--short` is the quick glance:
    /// one line per thing waiting on you, and nothing else.
    Review {
        /// Only what needs attention now, one line each: hypotheses fourteen
        /// days old with no references, seeds untouched for ninety days, and
        /// inbox captures waiting fourteen days or more.
        #[arg(long, conflicts_with_all = ["since", "out"])]
        short: bool,
        /// With `--short`, only nodes carrying this tag. Repeat to require
        /// every one.
        #[arg(long = "tag", value_name = "TAG", requires = "short")]
        tags: Vec<String>,
        /// Override the day thresholds for stale hypotheses (default 30) and
        /// untouched seeds (default 90), in days, at least 0. The inbox's
        /// fourteen-day rule is unaffected.
        #[arg(long, value_parser = days)]
        since: Option<i64>,
        /// Write the report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Show at most N findings under each heading, or N lines with
        /// `--short`, N at least 1. Defaults to every one.
        #[arg(long, value_name = "N", value_parser = count)]
        limit: Option<usize>,
    },

    /// Deprecated alias for `review --short`, kept for one release so
    /// existing routines keep working. Hidden from `--help`.
    #[command(hide = true)]
    Open {
        /// Only nodes carrying this tag. Repeat to require every one.
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },
}

/// The settings `neb config` reads and writes.
#[derive(Subcommand)]
enum ConfigSetting {
    /// Where the Observatory checkout is on this machine, so
    /// `cite --kind observatory --uri <record-id>` resolves. A directory is
    /// saved to `~/.config/nebula/observatory-root`, never into the corpus,
    /// which travels between machines. Without one, prints the effective root
    /// and which setting supplied it: `$OBSERVATORY_ROOT`, else this machine's
    /// setting, else a legacy `observatory_root` key in `config.yaml`.
    ObservatoryRoot {
        /// The checkout, as an absolute path. Omit to read the current
        /// setting.
        dir: Option<PathBuf>,
        /// Remove the legacy `observatory_root` key from `config.yaml`, where
        /// an older `neb` stored one machine's path for every machine. Do it
        /// once every machine that uses the corpus has its own setting.
        #[arg(long)]
        drop_legacy: bool,
        #[command(flatten)]
        commit: CommitArg,
    },

    /// Whether each mutating verb commits the corpus afterwards, when the
    /// root is inside a git work tree. Off by default. The commit stages
    /// `nodes/`, `inbox/`, `config.yaml` and the generated `.gitignore` and
    /// commits those paths alone, whatever else is staged, as
    /// `neb <verb> <ids>`. It never pushes. `--no-commit` skips it once.
    Commit {
        /// `on` or `off`. Omit to read the current setting.
        state: Option<OnOff>,
        #[command(flatten)]
        commit: CommitArg,
    },
}

impl Command {
    /// The verb's `--no-commit`, when it offers one: `None` for a verb that
    /// never commits.
    ///
    /// No wildcard arm, so a new verb has to say here whether it writes: one
    /// that does flattens a [`CommitArg`] and returns it here, one that does
    /// not is listed as `None`.
    fn commit_arg(&self) -> Option<CommitArg> {
        match self {
            Self::Migrate { commit }
            | Self::Config {
                setting:
                    ConfigSetting::ObservatoryRoot { commit, .. } | ConfigSetting::Commit { commit, .. },
            }
            | Self::Capture { commit, .. }
            | Self::Promote { commit, .. }
            | Self::Drop { commit, .. }
            | Self::Triage { commit, .. }
            | Self::New { commit, .. }
            | Self::Edit { commit, .. }
            | Self::Sharpen { commit, .. }
            | Self::Status { commit, .. }
            | Self::Link { commit, .. }
            | Self::Tag { commit, .. }
            | Self::Note { commit, .. }
            | Self::Cite { commit, .. }
            | Self::Handoff { commit, .. } => Some(*commit),
            Self::Init { .. }
            | Self::Check
            | Self::Completions { .. }
            | Self::Inbox { .. }
            | Self::Show { .. }
            | Self::Log { .. }
            | Self::List { .. }
            | Self::Near { .. }
            | Self::Trace { .. }
            | Self::Impact { .. }
            | Self::Graph { .. }
            | Self::Review { .. }
            | Self::Open { .. } => None,
        }
    }

    /// The one node or inbox entry a writing verb names, for the lock's
    /// holder record. No wildcard arm, so a new verb says here whether it
    /// has one.
    fn target(&self) -> Option<&str> {
        match self {
            Self::Promote { entry, .. } | Self::Drop { entry, .. } => Some(entry),
            Self::Edit { node, .. }
            | Self::Sharpen { node, .. }
            | Self::Status { node, .. }
            | Self::Note { node, .. }
            | Self::Cite { node, .. }
            | Self::Handoff { node, .. } => Some(node),
            Self::Link { from, .. } => Some(from),
            Self::Tag { target, .. } => Some(target),
            Self::Init { .. }
            | Self::Check
            | Self::Migrate { .. }
            | Self::Config { .. }
            | Self::Completions { .. }
            | Self::Capture { .. }
            | Self::Inbox { .. }
            | Self::Triage { .. }
            | Self::New { .. }
            | Self::Show { .. }
            | Self::Log { .. }
            | Self::List { .. }
            | Self::Near { .. }
            | Self::Trace { .. }
            | Self::Impact { .. }
            | Self::Graph { .. }
            | Self::Review { .. }
            | Self::Open { .. } => None,
        }
    }
}

/// A boolean setting as the command line spells it.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
enum OnOff {
    On,
    Off,
}

impl From<OnOff> for bool {
    fn from(s: OnOff) -> Self {
        matches!(s, OnOff::On)
    }
}

/// A refusal the CLI exits on. Core errors become one through [`render`], so
/// the advice that names commands stays in the crate that has commands.
struct Failure(render::Refusal);

/// Refusals specific to the terminal-owned editor flow.
#[derive(Debug, thiserror::Error)]
enum EditorError {
    #[error("neither $VISUAL nor $EDITOR names an editor")]
    NotConfigured,
    #[error("editor command `{0}` is empty or has unmatched quotes")]
    InvalidCommand(String),
    #[error("could not start editor `{editor}`: {source}")]
    Start {
        editor: String,
        source: std::io::Error,
    },
    #[error("editor `{0}` exited unsuccessfully; the node was not changed")]
    Unsuccessful(String),
    #[error(
        "an existing ## Notes section was removed, reordered, or changed; use `neb note` to append notes"
    )]
    NotesChanged,
}

impl EditorError {
    /// The refusal's `code` under `--json`; exhaustive for the same reason
    /// as [`Error::code`].
    fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "editor_not_configured",
            Self::InvalidCommand(_) => "editor_invalid_command",
            Self::Start { .. } => "editor_start",
            Self::Unsuccessful(_) => "editor_unsuccessful",
            Self::NotesChanged => "notes_changed",
        }
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self(render::refusal(&e))
    }
}

impl From<serde_json::Error> for Failure {
    fn from(e: serde_json::Error) -> Self {
        Self::of("json", format!("could not write JSON: {e}"))
    }
}

impl From<output::StdoutFailed> for Failure {
    fn from(e: output::StdoutFailed) -> Self {
        Self(e.into())
    }
}

/// Input `neb triage` cannot act on.
#[derive(Debug, thiserror::Error)]
enum KeyError {
    /// A line that is not a triage key.
    #[error("`{0}` is not a triage key; use p, a candidate number, t, d, s or q (? lists them)")]
    Unknown(String),
    /// Input ended while a title for `entry` was waiting to be used: after
    /// `t <title>`, or after `t` alone, before its line (STD-01 §R27).
    #[error("{}", title_lost(entry, title.as_deref()))]
    TitleLost {
        entry: String,
        title: Option<String>,
    },
}

/// [`KeyError::TitleLost`]'s message, which says whether the title arrived.
fn title_lost(entry: &str, title: Option<&str>) -> String {
    match title {
        Some(title) => format!(
            "input ended before the title `{title}` was used for `{entry}`; nothing was promoted"
        ),
        None => {
            format!("input ended before the title for `{entry}` was given; nothing was promoted")
        }
    }
}

impl KeyError {
    /// The refusal's `code` under `--json`, exhaustive like [`Error::code`].
    fn code(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "triage_key",
            Self::TitleLost { .. } => "triage_title_lost",
        }
    }

    /// The single verb that does what the lost input was for.
    fn hint(&self) -> Option<String> {
        match self {
            Self::Unknown(_) => None,
            Self::TitleLost { entry, title } => Some(format!(
                "Promote it with that title without triage:\n  neb promote {entry} --title {}",
                title
                    .as_deref()
                    .map_or_else(|| "\"...\"".to_owned(), shell_word)
            )),
        }
    }
}

impl From<KeyError> for Failure {
    fn from(e: KeyError) -> Self {
        Self(render::Refusal::new(e.code(), e.to_string()).hinted(e.hint()))
    }
}

/// `word` as one shell word: quoted when it needs quoting, so a command
/// printed for pasting runs as shown.
fn shell_word(word: &str) -> String {
    shlex::try_quote(word).map_or_else(|_| word.to_owned(), std::borrow::Cow::into_owned)
}

impl From<EditorError> for Failure {
    fn from(e: EditorError) -> Self {
        Self::of(e.code(), e.to_string())
    }
}

impl Failure {
    /// An error raised about one node, so the hint can name it.
    fn about(e: &Error, node: &str) -> Self {
        Self(render::refusal_about(e, node))
    }

    /// Arguments that parsed but ask for nothing that can be done: a usage
    /// error, which exits 2 as clap's own do (STD-01 §R20). Clap's never get
    /// here: they exit 2, in prose, before `run`.
    fn say(message: impl Into<String>) -> Self {
        Self(render::Refusal::usage("usage", message))
    }

    /// A refusal of the CLI's own, with its `snake_case` `code` under
    /// `--json`.
    fn of(code: &'static str, message: impl Into<String>) -> Self {
        Self(render::Refusal::new(code, message))
    }
}

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
fn parse_from<I, T>(argv: I) -> std::result::Result<Cli, clap::Error>
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
pub fn main() -> ExitCode {
    let cli = parse_from(std::env::args_os()).unwrap_or_else(|e| e.exit());
    CorpusLock::label_process(&cli.lock_label());
    let json = cli.json;
    let outcome = run(cli);
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

type Outcome = std::result::Result<ExitCode, Failure>;

/// After a write that added `tags` to `node`, say on stderr which of them
/// read as a variant of a tag already in use, in every output mode.
///
/// Advice, never a refusal: there is no declared list to refuse against, and
/// `check` rule 11 reports the same drift later. The write has landed by the
/// time this runs, so a corpus that cannot be read for the comparison costs
/// the note, not the command; `check` is where an unreadable node surfaces.
/// It reads every node, so it runs once the write lock is released
/// (STD-03 §R1).
fn note_close_tags(corpus: &Corpus, node: &str, tags: &[String]) {
    let Ok(close) = ops::close_tags(corpus, node, tags) else {
        return;
    };
    for c in close {
        errln!(
            "note: tag {} is close to {} ({} node{})",
            c.tag,
            c.near,
            c.nodes,
            if c.nodes == 1 { "" } else { "s" }
        );
    }
}

/// Resolve a `--body` value, using stdin only for the explicit `-` spelling.
fn body_value(value: Option<String>) -> std::result::Result<String, Failure> {
    match value.as_deref() {
        None => Ok(String::new()),
        Some("-") => read_stdin("a body", ops::BODY_INPUT_LIMIT),
        Some(_) => Ok(value.unwrap_or_default()),
    }
}

/// The words of a `capture`, or standard input when they are a lone `-`.
///
/// Only that exact spelling reads stdin, as with `--body -`: a dash among
/// other words is part of the thought. The text is passed on as it came;
/// the core joins its lines, so a pipe and a paste store the same line.
fn capture_text(words: &[String]) -> std::result::Result<String, Failure> {
    match words {
        [dash] if dash == "-" => read_stdin("a capture", ops::CAPTURE_INPUT_LIMIT),
        _ => Ok(words.join(" ")),
    }
}

/// All of standard input, as text, refused past `limit` bytes.
///
/// Every caller reads it before opening the corpus for writing, so a pipe
/// that is slow or enormous holds no lock while it drains, and one over the
/// ceiling is refused with nothing written (STD-03 §R22).
fn read_stdin(what: &'static str, limit: usize) -> std::result::Result<String, Failure> {
    Ok(ops::read_bounded(std::io::stdin().lock(), what, limit)?)
}

/// Existing notes are immutable through `edit`; `note` is their append path.
///
/// A body can hold more than one `## Notes` section, because `note` opens a
/// fresh one rather than reach back into a section some other prose already
/// closed. Every section is checked, not just the last: an edit that rewrote
/// an earlier dated note while leaving the final section alone would
/// otherwise overwrite reasoning that was supposed to be append-only.
/// Prose outside those sections stays editable, which is what `edit` is for.
fn preserve_notes(before: &str, after: &str) -> std::result::Result<(), EditorError> {
    let before_sections = model::notes_sections(before);
    if before_sections.is_empty() {
        return Ok(());
    }
    let after_sections = model::notes_sections(after);
    if before_sections.len() != after_sections.len() {
        return Err(EditorError::NotesChanged);
    }
    for (before, after) in before_sections.iter().zip(&after_sections) {
        if before.trim_end() != after.trim_end() {
            return Err(EditorError::NotesChanged);
        }
    }
    Ok(())
}

/// The body as the editor left it, and the temporary file it is still in.
///
/// The file is deleted when this drops, so a refusal keeps the text first
/// with [`keep_refused`].
struct Edited {
    text: String,
    file: tempfile::NamedTempFile,
}

/// Let the configured editor rewrite body prose in a temporary file.
///
/// No lock is held here: a person may type for as long as they like, and
/// every other writer carries on meanwhile (STD-03 §R1). The caller checks
/// the result and saves it with [`ops::set_body_if`].
fn edit_body(body: &str) -> std::result::Result<Edited, Failure> {
    use std::io::Write;

    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .ok_or(EditorError::NotConfigured)?;
    let editor_name = editor.to_string_lossy().into_owned();
    let mut words = shlex::split(&editor_name)
        .filter(|words| !words.is_empty())
        .ok_or_else(|| EditorError::InvalidCommand(editor_name.clone()))?;
    let program = words.remove(0);
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|e| Error::io_at("creating a temporary file in", std::env::temp_dir(), e))?;
    file.write_all(body.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|e| Error::io_at("writing", file.path(), e))?;
    // The one child not run like git: it stays in the terminal's foreground
    // group with no deadline, because a person drives it (STD-03@2 §R11,
    // recorded in docs/design/lineage-graph/4_decisions.md).
    let status = ProcessCommand::new(program)
        .args(words)
        .arg(file.path())
        .status()
        .map_err(|source| EditorError::Start {
            editor: editor_name.clone(),
            source,
        })?;
    if !status.success() {
        return Err(EditorError::Unsuccessful(editor_name).into());
    }
    let text = std::fs::read_to_string(file.path())
        .map_err(|e| Error::io_at("reading", file.path(), e))?;
    Ok(Edited { text, file })
}

/// `refused`, after keeping the text the person typed (STD-03 §R30).
///
/// Everything after the editor exits can refuse — the notes check, the wait
/// for the lock, a body another writer changed, the save itself — and the
/// text is then in memory and in a temporary file about to be deleted, and
/// nowhere else. So before the refusal is returned it is kept as a new
/// owner-only file under [`Corpus::kept_edits_dir`], outside the corpus, and
/// the refusal names it. Should that fail, the temporary file is kept where it
/// is instead. Only when both fail is the text lost, and the refusal says so
/// and why.
fn keep_refused(refused: Failure, node: &str, edited: Edited) -> Failure {
    let Failure(refusal) = refused;
    Failure(match Corpus::keep_edit(node, &edited.text) {
        Ok(path) => refusal.kept_at(&path),
        Err(not_kept) => match edited.file.keep() {
            Ok((_, path)) => refusal.kept_at(&path),
            Err(also) => refusal.not_kept(&format!("{not_kept}; {}", also.error)),
        },
    })
}

/// What every mutating arm needs to decide whether to commit and how to say
/// so: `--no-commit` waives the setting once, and `--json` leaves the
/// `committed` notice out.
#[derive(Clone, Copy)]
struct CommitOpts {
    skip: bool,
    json: bool,
}

/// Open the corpus and hold its write lock until the returned guard drops.
///
/// Every mutating arm holds the lock this way, so the verb and the [`commit`]
/// that records it are one critical section and no other writer can land a
/// verb between the two. The op underneath takes the same lock again, which
/// costs nothing: it is re-entrant on one thread. A read-only arm opens with
/// plain [`Corpus::open`] and waits for nobody.
///
/// Only the write and the commit: an arm drops the guard before any advice
/// it reads afterwards (STD-03 §R1). `edit` and `promote` open without it and
/// take it themselves, because the editor and the suggestions come first.
fn open_locked(root: Option<PathBuf>) -> std::result::Result<(Corpus, CorpusLock), Failure> {
    let corpus = Corpus::open(root)?;
    let lock = corpus.lock()?;
    Ok((corpus, lock))
}

/// Say on stderr when a capture repeats a thought still waiting in the inbox.
///
/// The capture has landed either way: capture never refuses for content, and
/// the two entries are for the triage to settle. It goes to stderr so the id,
/// or the `--json` payload, on stdout reads the same as for any capture.
///
/// A side channel, so it fails open (STD-02 §R31): an inbox that cannot be
/// read back costs the note, said as a warning, and never the capture.
fn note_same_as(corpus: &Corpus, entry: &InboxEntry) {
    match corpus.inbox() {
        Ok(inbox) => {
            if let Some(earlier) = inbox.same_as(entry) {
                errln!("note: same as {}, still waiting", earlier.id);
            }
        }
        Err(e) => errln!("warning: could not check the inbox for a repeat: {e}"),
    }
}

/// The `k` nodes closest to a capture just made, for the triage after it.
///
/// A side channel, so it fails open (STD-02 §R31): the capture is in the
/// inbox and committed by the time this runs, and a `nodes/` that cannot be
/// read for suggestions costs the suggestions, said on stderr with the
/// reason, and never the capture's exit status. Run with the lock released.
fn suggestions(corpus: &Corpus, text: &str, k: usize) -> Vec<nebula_core::Neighbour> {
    ops::suggest(corpus, text, k).unwrap_or_else(|e| {
        errln!("warning: suggestions unavailable: {e}");
        Vec::new()
    })
}

/// Say where an `observatory` reference's record is on this machine: the
/// path on stdout, as part of what `cite --kind observatory` and `handoff`
/// report, or why it cannot be located on stderr. The path is the payload's
/// own, so the prose shows what `--json` carries (STD-01 §R6); `setting`
/// only says why there is none.
fn print_record_location(setting: &ObservatoryRoot, link: &ObservatoryLink) {
    let record = &link.record;
    match setting.root.as_deref() {
        None => errln!(
            "{}",
            render::notice(&format!(
                "No observatory root set, so `{record}` cannot be located. \
                 Set one with `neb config observatory-root <DIR>` or \
                 ${OBSERVATORY_ROOT_ENV}."
            ))
        ),
        Some(dir) => match &link.path {
            Some(path) => outln!("{}", render::dim(&path.display().to_string())),
            None => errln!(
                "{}",
                render::notice(&format!(
                    "`{record}` does not resolve under {}; `check` will keep \
                     saying so until the checkout has it.",
                    dir.display()
                ))
            ),
        },
    }
}

/// Say on stderr, in every mode, when the observatory root a verb resolves
/// records against is the legacy `observatory_root` key in `config.yaml`.
///
/// The key is deprecated (STD-01 §R35): it is one machine's path in a file
/// every machine shares, and it answers only while neither
/// `$OBSERVATORY_ROOT` nor this machine's setting does.
fn warn_legacy_observatory_root(corpus: &Corpus, setting: &ObservatoryRoot) {
    if setting.source == ObservatorySource::Config {
        errln!(
            "warning: `observatory_root` in {} is deprecated, and the release after 0.2.0 stops \
             reading it; set this machine's own with `neb config observatory-root <DIR>`",
            corpus.root().join("config.yaml").display()
        );
    }
}

/// The nudge after a reference written without `--note`, on stderr.
fn print_bare_note() {
    errln!(
        "{}",
        render::notice("No note. Add one saying why it is here, or this is a link that rots.")
    );
}

/// Write a result's [`render::Notice`] to stderr, unless `--json` leaves it
/// out: counts and hints are for a person, while an empty or cut result is
/// said in every mode (STD-01 §R16, §R34).
fn notify(json: bool, notice: Option<render::Notice>) {
    if let Some(line) = notice.and_then(|n| n.line(json)) {
        errln!("{line}");
    }
}

/// Commit the corpus after a write, if `config.yaml` asks for it, and say
/// so on stderr: the notice is not the verb's payload (STD-01 §R12), so
/// `E=$(neb capture -q …)` holds the id alone. `--json` leaves it out.
///
/// Runs after the verb has printed its own result, because the write has
/// already landed and a refusal here must never read as the write failing.
fn commit(
    corpus: &Corpus,
    opts: CommitOpts,
    verb: &str,
    ids: &[&str],
) -> std::result::Result<(), Failure> {
    if opts.skip {
        return Ok(());
    }
    match ops::commit(corpus, verb, ids)? {
        CommitOutcome::Committed(done) if !opts.json => {
            let short = done.hash.get(..7).unwrap_or(&done.hash);
            errln!("{}", render::notice(&format!("committed {short}")));
        }
        // Asked for and not done: said on stderr in every mode, so the
        // payload on stdout reads the same either way.
        CommitOutcome::NotARepository => errln!(
            "note: not committed: {} is not inside a git work tree",
            corpus.root().display()
        ),
        CommitOutcome::Committed(_) | CommitOutcome::Disabled | CommitOutcome::NothingToCommit => {}
    }
    Ok(())
}

/// Write rendered text to `out`. On stdout this never fails: the output
/// layer absorbs a closed pipe, so the commit after it still runs.
fn say(out: &mut impl Write, text: &str) -> std::result::Result<(), Failure> {
    out.write_all(text.as_bytes())
        .map_err(output::StdoutFailed::from)?;
    Ok(())
}

/// One line of `neb triage` input, read.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    /// A decision about the current entry.
    Act(Action),
    /// `t` alone: the title follows on the next line.
    AskTitle,
    /// `?` or `h`: list the keys again.
    Help,
    /// A blank line, which decides nothing.
    Nothing,
}

/// Read one line as a triage key. `t` is the one key that takes text.
fn parse_key(line: &str) -> std::result::Result<Key, KeyError> {
    let line = line.trim();
    let (key, rest) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(key, rest)| (key, rest.trim()));
    Ok(match (key, rest) {
        ("", _) => Key::Nothing,
        ("t", "") => Key::AskTitle,
        ("t", title) => Key::Act(Action::Title(title.to_string())),
        ("p", "") => Key::Act(Action::Promote),
        ("d", "") => Key::Act(Action::Drop),
        ("s", "") => Key::Act(Action::Skip),
        ("q", "") => Key::Act(Action::Quit),
        ("?" | "h", "") => Key::Help,
        (number, "") if number.bytes().all(|b| b.is_ascii_digit()) => {
            let number = number
                .parse()
                .map_err(|_| KeyError::Unknown(line.to_string()))?;
            Key::Act(Action::PromoteUnder(number))
        }
        _ => return Err(KeyError::Unknown(line.to_string())),
    })
}

/// `neb triage`: show the current entry, read a line, carry it out, repeat.
///
/// `interactive` is whether a person is at the keys. It decides three
/// things: whether the key legend and a prompt are shown, and whether a
/// refusal is reported and asked about again or ends the session, since
/// scripted lines after a refusal were written for an entry that did not
/// move. A commit that git refuses ends it either way, as it ends the single
/// verb: every later write would be left uncommitted the same way.
///
/// When `out` closes, nobody can see the next entry, so the session ends as
/// `q` ends it: every decision already made stays applied and committed.
fn triage(
    corpus: &Corpus,
    by: Option<String>,
    input: &mut impl BufRead,
    out: &mut impl output::Closable,
    interactive: bool,
    commits: CommitOpts,
) -> std::result::Result<(), Failure> {
    let refuse = |failure: Failure| {
        if interactive {
            errln!("{} {}", render::error_label(), failure.0.prose());
            Ok(())
        } else {
            // A line of input was refused, not the command line, so it ends
            // the session as a failure whatever the verb would call it.
            Err(Failure(failure.0.failed()))
        }
    };
    let mut session = Triage::start(corpus, by)?;
    let mut shown = false;
    let mut titling = false;
    while let Some(waiting) = session.current(corpus)? {
        if !shown {
            say(out, &render::waiting(waiting))?;
            if interactive {
                say(out, &render::triage_keys(waiting))?;
            }
            shown = true;
        }
        if interactive {
            say(out, if titling { "title> " } else { "> " })?;
            out.flush().map_err(output::StdoutFailed::from)?;
        }
        if out.is_closed() {
            break;
        }
        let mut line = String::new();
        if input
            .read_line(&mut line)
            .map_err(|source| Error::IoStdin {
                what: "a triage key",
                source,
            })?
            == 0
        {
            if interactive {
                say(out, "\n")?;
            }
            // End of input stops as `q` does, unless a title is waiting to
            // be used: stopping then would drop it without a word (STD-01
            // §R27, recorded in docs/design/lineage-graph/4_decisions.md).
            if titling || waiting.title.is_some() {
                return Err(KeyError::TitleLost {
                    entry: waiting.entry.id.clone(),
                    title: waiting.title.clone().filter(|_| !titling),
                }
                .into());
            }
            break;
        }
        let action = if std::mem::take(&mut titling) {
            Action::Title(line)
        } else {
            match parse_key(&line) {
                Ok(Key::Act(action)) => action,
                Ok(Key::AskTitle) => {
                    titling = true;
                    continue;
                }
                Ok(Key::Help) => {
                    say(out, &render::triage_keys(waiting))?;
                    continue;
                }
                Ok(Key::Nothing) => continue,
                Err(e) => {
                    refuse(e.into())?;
                    continue;
                }
            }
        };
        // One critical section per decision, as the single verb has: the
        // write and the commit that records it. Never across the wait for
        // the next line, which would hold every other writer off meanwhile.
        let writes = matches!(
            action,
            Action::Promote | Action::PromoteUnder(_) | Action::Drop
        );
        let _lock = writes.then(|| corpus.lock()).transpose()?;
        match session.apply(corpus, action) {
            Ok(step) => {
                say(out, &render::step(&step))?;
                match &step {
                    Step::Promoted { entry, created } => commit(
                        corpus,
                        commits,
                        "promote",
                        &[&entry.id, &created.doc.node.id],
                    )?,
                    Step::Dropped { entry } => {
                        commit(corpus, commits, "drop", &[&entry.id])?;
                    }
                    Step::Titled { .. } => continue,
                    Step::Skipped { .. } | Step::Quit => {}
                }
                shown = false;
            }
            Err(e) => {
                // Settled by another writer since the session began: the
                // session has moved past it, so the next entry is new.
                if nebula_core::triage::settled_elsewhere(&e) {
                    shown = false;
                }
                refuse(e.into())?;
            }
        }
    }
    say(out, &render::tally(&session.tally(), session.total()))
}

#[allow(clippy::too_many_lines)] // A dispatch table is one arm per verb.
fn run(cli: Cli) -> Outcome {
    let ok = ExitCode::SUCCESS;
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

    match cli.command {
        Command::Init {
            path,
            set_root,
            force,
        } => {
            // Before any work: a `--root` that names another directory than
            // the path is refused here, with neither created (STD-01 §R28).
            let target = ops::init_target(root.clone(), path.clone())?;
            let root_config_path = Corpus::root_config_path_if_absent(&target)?;
            let default_root_warning = (!set_root)
                .then(|| Corpus::warning_before_default_init(&target))
                .transpose()?
                .flatten();
            let shadowing_warning = (!set_root)
                .then(|| Corpus::warning_before_shadowing_init(&target))
                .transpose()?
                .flatten();
            let done = ops::init(root, path, set_root, force)?;
            if json {
                out_json(&done)?;
            } else {
                outln!("corpus ready at {}", done.root.display());
                if set_root {
                    outln!(
                        "wrote {} so every command finds it",
                        Corpus::root_config_path()?.display()
                    );
                } else if root_config_path.is_some() {
                    errln!(
                        "{}",
                        render::notice(&format!(
                            "run `neb init {} --set-root` to make this corpus the machine default",
                            target.display()
                        ))
                    );
                }
            }
            if let Some(configured) = default_root_warning {
                errln!(
                    "warning: creating ~/.nebula while {} points to {}",
                    Corpus::root_config_path()?.display(),
                    configured.display()
                );
            }
            if let Some(configured) = shadowing_warning {
                // The command that repoints the machine default, never a
                // hand edit: absolute, since the setting is read from every
                // directory, and quoted, so it runs as printed (STD-02
                // §R26). Made absolute lexically, as capture's note is.
                let target = std::path::absolute(&target).unwrap_or(target);
                errln!(
                    "warning: {} still points to {}, not {}; run `neb init {} --set-root --force` to point commands at this corpus",
                    Corpus::root_config_path()?.display(),
                    configured.display(),
                    target.display(),
                    shell_word(&target.to_string_lossy())
                );
            }
            Ok(ok)
        }

        Command::Check => {
            let corpus = Corpus::open(root)?;
            let scan = corpus.scan()?;
            let report = check::run_scanned(&scan, &corpus)?;
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

        Command::Migrate { .. } => {
            // `migrate` takes this lock itself; held out here it also covers
            // the commit that records the migration. Only once there is a
            // corpus to lock: a missing one is `migrate`'s error to report,
            // and it names the root it looked in.
            let target = Corpus::resolve_root(root.clone())?;
            let _lock = target
                .join("nodes")
                .is_dir()
                .then(|| CorpusLock::acquire(&target))
                .transpose()?;
            let report = migrate::run(root.clone())?;
            if json {
                out_json(&report)?;
            } else {
                out!("{}", render::migration(&report));
            }
            notify(json, Some(render::migration_notice(&report)));
            // The corpus is at this build's schema now, so it opens; the
            // migration lands as its own commit when the setting is on.
            commit(&Corpus::open(root)?, commits, "migrate", &[])?;
            Ok(ok)
        }

        Command::Config {
            setting:
                ConfigSetting::ObservatoryRoot {
                    dir, drop_legacy, ..
                },
        } => {
            let mut corpus = Corpus::open(root)?;
            // Setting the root writes this machine's file and nothing in the
            // corpus, so it takes the machine-setting lock and only
            // `--drop-legacy` takes the corpus lock. Both are taken here, in
            // the order `lock.rs` documents, before either write, so a busy
            // lock refuses with nothing written. Reading the setting back
            // takes neither and so never waits on a writer.
            let _settings = dir
                .is_some()
                .then(Corpus::lock_machine_settings)
                .transpose()?;
            let _lock = drop_legacy.then(|| corpus.lock()).transpose()?;
            if let Some(dir) = &dir {
                ops::set_observatory_root(&corpus, dir)?;
            }
            let dropped = drop_legacy
                .then(|| ops::drop_legacy_observatory_root(&mut corpus))
                .transpose()?;
            let setting = corpus.observatory_root()?;
            if json {
                out_json(&setting)?;
            } else {
                out!("{}", render::observatory_root(&setting));
            }
            for note in render::observatory_root_notes(&setting, dir.is_some()) {
                notify(json, Some(note));
            }
            // Said in every mode, like a commit asked for and not made: the
            // file it names was rewritten, or was left alone (STD-01 §R30).
            if let Some(dropped) = &dropped {
                let config = corpus.root().join("config.yaml");
                notify(json, Some(render::dropped_legacy(dropped, &config)));
            }
            if dropped.is_some_and(|dropped| dropped.removed.is_some()) {
                commit(&corpus, commits, "config", &["observatory-root"])?;
            }
            Ok(ok)
        }

        Command::Config {
            setting: ConfigSetting::Commit { state, .. },
        } => {
            let mut corpus = Corpus::open(root)?;
            // As above: reading the setting is a read.
            let _lock = state.is_some().then(|| corpus.lock()).transpose()?;
            let (setting, changed) = match state {
                Some(state) => (ops::set_commit(&mut corpus, bool::from(state))?, true),
                None => (corpus.commit_setting(), false),
            };
            if json {
                out_json(&setting)?;
            } else {
                out!("{}", render::commit_setting(setting));
            }
            notify(json, render::commit_setting_hint(setting));
            // Turning it on records itself; turning it off leaves the file
            // for the next commit you make by hand, because off means off.
            if changed {
                commit(&corpus, commits, "config", &["commit"])?;
            }
            Ok(ok)
        }

        Command::Completions { shell } => {
            // Rendered whole first: `generate` panics on a failed write, and
            // the layer, not clap, decides what a closed stdout means.
            let mut script = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "neb", &mut script);
            output::stdout()
                .write_all(&script)
                .map_err(output::StdoutFailed::from)?;
            Ok(ok)
        }

        Command::Capture { quiet, text, .. } => {
            // Core's own emptiness rule, run before a corpus can be created
            // for text that would be refused anyway.
            let text = capture_text(&text)?;
            store::validate_capture(&text)?;
            // Capture must work on a corpus that does not exist yet. Being
            // told to run a setup command is precisely the friction that
            // loses the thought. A root found from the working directory is
            // one that already holds a corpus, so only an explicit or a
            // configured root can ever be created here.
            let resolved_root = Corpus::resolve_root(root)?;
            let default_root_warning = Corpus::warning_before_default_init(&resolved_root)?;
            let (corpus, created) = Corpus::open_or_init(Some(resolved_root.clone()))?;
            if let Some(configured) = default_root_warning {
                errln!(
                    "warning: creating ~/.nebula while {} points to {}",
                    Corpus::root_config_path()?.display(),
                    configured.display()
                );
            }
            // Never silent, never a question: a mistyped `--root` or
            // `$NEBULA_ROOT` would otherwise start a second corpus unnoticed.
            // Absolute so a relative typo shows where it landed; made so
            // lexically, because paths are used as given, never resolved.
            if created {
                let shown = std::path::absolute(&resolved_root).unwrap_or(resolved_root);
                errln!("note: created a new corpus at {}", shown.display());
            }
            let k = if quiet { 0 } else { NEAR_DEFAULT };
            // The capture and its commit are the critical section. The
            // suggestions are a read of every node, so they wait until the
            // lock is released (STD-03 §R1), and the id goes out before
            // them: the capture never depended on the rest of the corpus
            // parsing, and a node file that will not must not read as a
            // lost thought.
            let lock = corpus.lock()?;
            let entry = ops::capture(&corpus, &text)?;
            if !json {
                outln!("{}", render::bold(&entry.id));
            }
            let committed = commit(&corpus, commits, "capture", &[&entry.id]);
            drop(lock);
            note_same_as(&corpus, &entry);
            let near = suggestions(&corpus, &entry.text, k);
            if json {
                out_json(&json::Captured::from(&ops::Captured { entry, near }))?;
            } else {
                out!("{}", render::suggestions(&near));
            }
            committed?;
            Ok(ok)
        }

        Command::Inbox { limit } => {
            let corpus = Corpus::open(root)?;
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
        } => {
            let body = body_value(body)?;
            let promotion = Promotion {
                title,
                body,
                parents,
                tags,
                origin: Origin::of(task, run),
                id,
                by,
            };
            // The suggestions are read before the lock is taken (STD-03 §R1);
            // the promotion and its commit are the critical section.
            let corpus = Corpus::open(root)?;
            let k = if quiet { 0 } else { NEAR_DEFAULT };
            let near = ops::promotion_near(&corpus, &entry, &promotion, k)?;
            let lock = corpus.lock()?;
            let created = ops::promote_with(&corpus, &entry, &promotion, near)?;
            if json {
                out_json(&json::Created::from(&created))?;
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
            let committed = commit(&corpus, commits, "promote", &[&entry, &created.doc.node.id]);
            drop(lock);
            note_close_tags(&corpus, &created.doc.node.id, &created.doc.node.tags);
            committed?;
            Ok(ok)
        }

        Command::Drop { entry, .. } => {
            let (corpus, _lock) = open_locked(root)?;
            let dropped = ops::drop(&corpus, &entry)?;
            if json {
                out_json(&dropped)?;
            } else {
                outln!("dropped {}", render::bold(&entry));
            }
            commit(&corpus, commits, "drop", &[&entry])?;
            Ok(ok)
        }

        Command::Triage { by, .. } => {
            // Refused before anything is read: there is no one payload a
            // session of keyed decisions could honestly be.
            if json {
                return Err(Error::Interactive("triage".into()).into());
            }
            let corpus = Corpus::open(root)?;
            let interactive = output::stdin_on_terminal();
            triage(
                &corpus,
                by,
                &mut std::io::stdin().lock(),
                &mut output::stdout(),
                interactive,
                commits,
            )?;
            Ok(ok)
        }

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
        } => {
            let body = body_value(body)?;
            let (corpus, lock) = open_locked(root)?;
            let created = ops::new_node(
                &corpus,
                &NewNode {
                    title,
                    body,
                    parents,
                    reopens,
                    contradicts,
                    kill,
                    tags,
                    origin: Origin::of(task, run),
                    id,
                    by,
                },
            )
            .map_err(|e| Failure(render::refusal_for_new(&e)))?;
            if json {
                out_json(&json::Created::from(&created))?;
            } else {
                outln!(
                    "{} {}",
                    render::bold(&created.doc.node.id),
                    render::dim(&created.path.display().to_string())
                );
            }
            // A contradicted node changed too, so the commit names it.
            let node = &created.doc.node;
            let ids: Vec<&str> = std::iter::once(node.id.as_str())
                .chain(node.edges_of(EdgeType::Contradicts))
                .collect();
            let committed = commit(&corpus, commits, "new", &ids);
            drop(lock);
            note_close_tags(&corpus, &node.id, &node.tags);
            committed?;
            Ok(ok)
        }

        Command::Edit { node, .. } => {
            // No lock while the person types (STD-03 §R1). The body is
            // loaded now, edited for as long as it takes, and saved under
            // the lock only if nobody changed it meanwhile.
            let corpus = Corpus::open(root)?;
            let before = corpus.load(&node).map_err(|e| Failure::about(&e, &node))?;
            let edited = edit_body(&before.body)?;
            // Nothing typed is nothing to write: no lock, no save, no
            // commit, and `updated` stays as it was (STD-01 §R30).
            if ops::body_unchanged(&before.body, &edited.text) {
                notify(json, Some(render::unchanged(&node)));
                if json {
                    out_node_view(&corpus, &node)?;
                }
                return Ok(ok);
            }
            if let Err(e) = preserve_notes(&before.body, &edited.text) {
                return Err(keep_refused(e.into(), &node, edited));
            }
            let saved = corpus.lock().and_then(|lock| {
                ops::set_body_if(&corpus, &node, &before.body, &edited.text).map(|_| lock)
            });
            let lock = match saved {
                Ok(lock) => lock,
                Err(e) => return Err(keep_refused(Failure::about(&e, &node), &node, edited)),
            };
            drop(edited);
            if !json {
                outln!("{}", render::bold(&node));
            }
            let committed = commit(&corpus, commits, "edit", &[&node]);
            drop(lock);
            let shown = if json {
                out_node_view(&corpus, &node)
            } else {
                Ok(())
            };
            committed?;
            shown?;
            Ok(ok)
        }

        Command::Sharpen {
            node,
            confirm: true,
            ..
        } => {
            // `--kill` and `--by` conflict with `--confirm` in the clap tree,
            // so there is no text here to reconcile: confirming changes none.
            let (corpus, _lock) = open_locked(root)?;
            let doc = ops::confirm_kill(&corpus, &node).map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&json::Doc::from(&doc))?;
            } else {
                outln!("{} kill condition confirmed as yours", render::bold(&node));
            }
            commit(&corpus, commits, "sharpen", &[&node])?;
            Ok(ok)
        }

        Command::Sharpen {
            node,
            kill,
            by,
            confirm: false,
            ..
        } => {
            // Clap's `required_unless_present` guarantees the kill is here
            // without `--confirm`; saying so beats an unwrap.
            let Some(kill) = kill else {
                return Err(Failure::say("pass --kill <KILL>, or --confirm"));
            };
            let (corpus, _lock) = open_locked(root)?;
            let before = corpus.load(&node).map_err(|e| Failure::about(&e, &node))?;
            let doc = ops::sharpen(&corpus, &node, &kill, by.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&json::Doc::from(&doc))?;
            } else if before.node.status != doc.node.status {
                outln!("{} is now {}", render::bold(&node), doc.node.status);
            } else if before.node.kill.is_some() {
                outln!(
                    "{} kill condition replaced; status remains {}",
                    render::bold(&node),
                    doc.node.status
                );
            } else {
                outln!(
                    "{} kill condition set; status remains {}",
                    render::bold(&node),
                    doc.node.status
                );
            }
            commit(&corpus, commits, "sharpen", &[&node])?;
            Ok(ok)
        }

        Command::Status {
            node, status, why, ..
        } => {
            let status = Status::from(status);
            let (corpus, _lock) = open_locked(root)?;
            let changed = ops::set_status(&corpus, &node, status, why.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&json::StatusChange::from(&changed))?;
            } else {
                outln!(
                    "{} {} -> {status}",
                    render::bold(&node),
                    render::dim(&changed.from.to_string())
                );
            }
            commit(&corpus, commits, "status", &[&node])?;
            Ok(ok)
        }

        Command::Link {
            from, kind, to, by, ..
        } => {
            let (corpus, _lock) = open_locked(root)?;
            let kind = EdgeType::from(kind);
            let changed = ops::link(&corpus, &from, kind, &to, by.as_deref())?;
            if json {
                out_json(&changed.iter().map(json::Doc::from).collect::<Vec<_>>())?;
            } else {
                outln!(
                    "{} {} {}",
                    render::bold(&from),
                    render::dim(&kind.to_string()),
                    render::bold(&to)
                );
            }
            commit(&corpus, commits, "link", &[&from, &to])?;
            Ok(ok)
        }

        Command::Tag {
            target,
            add,
            remove,
            ..
        } if target == "list" && add.is_empty() && remove.is_empty() => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let counts = graph::tags(&Graph::build(&docs)?)?;
            if json {
                out_json(&counts)?;
            } else {
                out!("{}", render::tags(&counts, render::Target::stdout()));
            }
            notify(json, render::tags_notice(&counts));
            Ok(ok)
        }

        Command::Tag {
            target,
            add,
            remove,
            ..
        } => {
            if add.is_empty() && remove.is_empty() {
                return Err(Failure::say(
                    "nothing to do; pass --add <tag> or --remove <tag>",
                ));
            }
            // One write under one lock: `--remove x --add y` is one change
            // to the node's tags, not two a second writer may split.
            let (corpus, lock) = open_locked(root)?;
            let done = ops::retag(&corpus, &target, &add, &remove)?;
            let doc = &done.doc;
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
            for note in render::retag_notes(&target, &done) {
                notify(json, Some(note));
            }
            if !done.written {
                return Ok(ok);
            }
            let committed = commit(&corpus, commits, "tag", &[&target]);
            drop(lock);
            note_close_tags(&corpus, &target, &add);
            committed?;
            Ok(ok)
        }

        Command::Note { node, by, text, .. } => {
            let text = text.join(" ");
            let (corpus, lock) = open_locked(root)?;
            ops::note(&corpus, &node, &text, by.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if !json {
                outln!("{}", render::bold(&node));
            }
            let committed = commit(&corpus, commits, "note", &[&node]);
            drop(lock);
            let shown = if json {
                out_node_view(&corpus, &node)
            } else {
                Ok(())
            };
            committed?;
            shown?;
            Ok(ok)
        }

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
        } => {
            let (corpus, _lock) = open_locked(root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            // Read before the write, under `--json` too, whose payload says
            // where the record is: a broken machine setting refuses the cite
            // rather than failing it after the reference has landed. The kind
            // is compared as the write will store it, so `--kind Observatory`
            // is read here too.
            let observatory = (check::normalize_reference_kind(&kind) == OBSERVATORY)
                .then(|| corpus.observatory_root())
                .transpose()?;
            if let Some(setting) = &observatory {
                warn_legacy_observatory_root(&corpus, setting);
            }
            let cited = ops::cite_with_observatory(
                &corpus,
                &node,
                &Citation {
                    uri,
                    kind,
                    title,
                    note,
                    by,
                    origin: Origin::of(task, run),
                },
                observatory.as_ref().and_then(|s| s.root.as_deref()),
            )
            .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&json::Cited::from(&cited))?;
            } else {
                outln!("{} {}", render::bold(&node), render::bold(&cited.reference));
                if let (Some(setting), Some(link)) = (&observatory, &cited.observatory) {
                    print_record_location(setting, link);
                }
                if bare {
                    print_bare_note();
                }
            }
            commit(&corpus, commits, "cite", &[&node, &cited.reference])?;
            Ok(ok)
        }

        Command::Handoff {
            node,
            record,
            note,
            by,
            task,
            run,
            ..
        } => {
            let (corpus, _lock) = open_locked(root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            // Read before the write, under `--json` too: the record has to
            // resolve under this root, and a broken machine setting refuses
            // the hand-off rather than failing it after the node has closed.
            let setting = corpus.observatory_root()?;
            warn_legacy_observatory_root(&corpus, &setting);
            let done = ops::handoff(
                &corpus,
                &node,
                &Handoff {
                    record,
                    note,
                    by,
                    origin: Origin::of(task, run),
                },
                setting.root.as_deref(),
            )
            .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&json::HandedOff::from(&done))?;
            } else {
                outln!(
                    "{} {} -> {}, handed off to {} {}",
                    render::bold(&node),
                    render::dim(&done.from.to_string()),
                    done.doc.node.status,
                    render::bold(&done.record),
                    render::dim(&format!("({})", done.reference))
                );
                print_record_location(&setting, &done.observatory);
                if bare {
                    print_bare_note();
                }
            }
            commit(&corpus, commits, "handoff", &[&node, &done.record])?;
            Ok(ok)
        }

        Command::Show { node, at } => {
            let corpus = Corpus::open(root)?;
            let mut docs = corpus.load_all()?;
            if let Some(at) = at {
                let historical = corpus.load_at(&node, &at)?;
                let current = docs
                    .iter_mut()
                    .find(|doc| doc.node.id == node)
                    .ok_or_else(|| Error::NoSuchNode(node.clone()))?;
                *current = historical;
            }
            let observatory = corpus.observatory_root()?;
            warn_legacy_observatory_root(&corpus, &observatory);
            let view = graph::node(&Graph::build(&docs)?, &node)?
                .with_observatory(observatory.root.as_deref())?;
            if json {
                out_json(&json::NodeView::from(&view))?;
            } else {
                out!("{}", render::node(&view));
            }
            Ok(ok)
        }

        Command::Log { node } => {
            let corpus = Corpus::open(root)?;
            let history = corpus.history(&node)?;
            if json {
                out_json(&history)?;
            } else {
                out!("{}", render::history(&history, render::Target::stdout()));
            }
            notify(json, render::history_notice(&history));
            Ok(ok)
        }

        Command::List {
            status,
            tags,
            limit,
        } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let mut listing = graph::list(&Graph::build(&docs)?, status.map(Status::from), &tags)?;
            let cut = cap(&mut listing.0, limit);
            if json {
                let nodes = listing.0.iter().map(json::Node::from).collect();
                out_json(&json::List::new(nodes, limit.map(|_| cut)))?;
            } else {
                out!("{}", render::list(&listing.0, render::Target::stdout()));
            }
            notify(
                json,
                Some(render::list_notice(listing.0.len(), cut.0, docs.len())),
            );
            Ok(ok)
        }

        Command::Near { limit, query } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                return Err(Failure::say("nothing to look near"));
            }
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let (near, matched) = graph::near_counted(&Graph::build(&docs)?, &query, limit)?;
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

        Command::Trace { node, down, depth } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let direction = if down { Direction::Down } else { Direction::Up };
            let graph = Graph::build(&docs)?;
            let walk = graph::trace_within(&graph, &node, direction, depth)?;
            // A bounded walk's `total` is what the whole walk reaches.
            let cut = depth
                .map(|_| graph::trace(&graph, &node, direction))
                .transpose()?
                .map(|whole| (whole.0.len(), walk.0.len() < whole.0.len()));
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

        Command::Impact { node } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let report = graph::impact(&Graph::build(&docs)?, &node)?;
            if json {
                out_json(&report)?;
            } else {
                out!("{}", render::impact(&report, &node));
            }
            notify(json, render::impact_notice(&report));
            Ok(ok)
        }

        Command::Graph { mermaid, from } => {
            if !json && !mermaid {
                return Err(Failure::say(
                    "neb graph needs an output format; pass --json or --mermaid",
                ));
            }
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let exported = graph::export(&Graph::build(&docs)?)?;
            if mermaid {
                out!("{}", render::mermaid(&exported, from.as_deref())?);
            } else {
                out_json(&exported)?;
            }
            Ok(ok)
        }

        Command::Review {
            short: true,
            tags,
            limit,
            ..
        } => {
            short_review(root, json, &tags, limit)?;
            Ok(ok)
        }

        Command::Review {
            short: false,
            since,
            out,
            limit,
            ..
        } => {
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let mut report = graph::review(&Graph::build(&docs)?, &corpus.inbox()?, since)?;
            // Every rule's findings, counted before the cut keeps each rule's first N.
            let found = report.0.len();
            let omitted = limit.map_or_else(Vec::new, |n| report.truncate_per_rule(n));
            let text = if json {
                let listed = json::List::new(report.0, limit.map(|_| (found, !omitted.is_empty())));
                serde_json::to_string_pretty(&listed).map_err(Failure::from)?
            } else {
                render::review(
                    &report,
                    since.unwrap_or(nebula_core::HYPOTHESIS_DAYS),
                    since.unwrap_or(nebula_core::SEED_DAYS),
                    &omitted,
                )
            };
            write_report(out.as_deref(), &text)?;
            notify(json, render::review_notice(&omitted));
            Ok(ok)
        }

        Command::Open { tags } => {
            errln!("warning: `neb open` is deprecated; use `neb review --short`");
            short_review(root, json, &tags, None)?;
            Ok(ok)
        }
    }
}

/// `review --short`, and the deprecated `open` that now forwards to it:
/// what needs attention now, one line per item.
fn short_review(
    root: Option<PathBuf>,
    json: bool,
    tags: &[String],
    limit: Option<usize>,
) -> std::result::Result<(), Failure> {
    let corpus = Corpus::open(root)?;
    let docs = corpus.load_all()?;
    let mut report = graph::open(&Graph::build(&docs)?, &corpus.inbox()?, tags)?;
    let cut = cap(&mut report.0, limit);
    let notice = render::open_notice(report.0.len(), cut.0);
    if json {
        out_json(&json::List::new(report.0, limit.map(|_| cut)))?;
    } else {
        out!("{}", render::open(&report, render::Target::stdout()));
    }
    notify(json, notice);
    Ok(())
}

/// The least a count flag takes: `--limit`, `near -k` and `trace --depth`.
/// Zero would ask for an answer that is empty, or the node alone, and still
/// reads as a real one (STD-02 §R29), so it is a usage error (STD-01 §R20).
const MIN_COUNT: usize = 1;

/// The least `review --since` takes, in days.
const MIN_DAYS: i64 = 0;

/// Parse a count flag's value: at least [`MIN_COUNT`].
fn count(value: &str) -> std::result::Result<usize, String> {
    at_least(value, MIN_COUNT)
}

/// Parse a day threshold: at least [`MIN_DAYS`].
fn days(value: &str) -> std::result::Result<i64, String> {
    at_least(value, MIN_DAYS)
}

/// A whole number no smaller than `min`. Every bounded flag refuses through
/// here, so clap reports the flag and this one wording for all of them.
fn at_least<T>(value: &str, min: T) -> std::result::Result<T, String>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display + Copy,
{
    match value.parse::<T>() {
        Ok(n) if n >= min => Ok(n),
        _ => Err(format!("must be a whole number, at least {min}")),
    }
}

/// Cut a listing to `--limit`, when one was given, and return how many
/// matched before the cut and whether it dropped any, so the text can say
/// how much was left out. `--json` puts both in the [`json::List`] envelope
/// whenever the flag is given (STD-01 §R34). The flag is at least
/// [`MIN_COUNT`], so a cut never empties a listing that had anything in it.
fn cap<T>(items: &mut Vec<T>, limit: Option<usize>) -> (usize, bool) {
    let total = items.len();
    if let Some(n) = limit {
        items.truncate(n);
    }
    (total, items.len() < total)
}

/// Pretty JSON on stdout, which is what `--json` means everywhere.
fn out_json<T: serde::Serialize>(v: &T) -> std::result::Result<(), Failure> {
    outln!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// `node` as `show --json` prints it, over the corpus as it is now: what
/// `edit --json` and `note --json` print once their write is committed. It
/// builds the whole graph, so it runs with the lock released
/// (STD-03 §R1).
fn out_node_view(corpus: &Corpus, node: &str) -> std::result::Result<(), Failure> {
    let docs = corpus.load_all()?;
    let view = graph::node(&Graph::build(&docs)?, node)?;
    out_json(&json::NodeView::from(&view))
}

/// Send a rendered report to a file, or print it, per `--out`.
#[allow(
    clippy::disallowed_methods,
    reason = "`--out` names a report file of the user's choosing, not nebula state, so it is \
              written where and how they asked rather than through the private durable helper"
)]
fn write_report(out: Option<&Path>, text: &str) -> std::result::Result<(), Failure> {
    match out {
        Some(path) => {
            std::fs::write(path, format!("{text}\n"))
                .map_err(|e| Error::io_at("writing", path, e))?;
            // stdout stays empty, so the file is named on stderr, in every
            // mode: a write says what it wrote (STD-01 §R30).
            errln!("{}", render::notice(&format!("wrote {}", path.display())));
        }
        None => outln!("{text}"),
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

    /// Every visible subcommand clap knows about has a row in the template,
    /// and the row's text is the variant's own one-liner, so the two cannot
    /// drift. Hidden aliases are the exception, and have no row.
    #[test]
    fn help_rows_match_the_variants() {
        let flat = squash(&help());
        let mut seen = 0;
        for sub in Cli::command()
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
        {
            let about = sub.get_about().map(ToString::to_string).unwrap_or_default();
            let row = squash(&format!("{} {about}", sub.get_name()));
            assert!(flat.contains(&row), "help is missing the row {row:?}");
            seen += 1;
        }
        assert_eq!(seen, 27, "template rows need updating for a new subcommand");
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
        assert!(
            !text.contains("--no-commit"),
            "--no-commit belongs to the verbs that write"
        );
        assert!(text.contains("The corpus lives outside this repository"));
    }

    /// Whether `neb <path...> --help` lists `--no-commit` as an option, as
    /// opposed to prose that merely mentions it.
    fn offers_no_commit(path: &[&str]) -> bool {
        let mut cmd = Cli::command();
        cmd.build();
        let mut at = &mut cmd;
        for name in path {
            at = at
                .find_subcommand_mut(name)
                .unwrap_or_else(|| panic!("no subcommand {name}"));
        }
        at.render_long_help()
            .to_string()
            .lines()
            .any(|l| l.trim_start().starts_with("--no-commit"))
    }

    /// `--no-commit` is offered by exactly the verbs that can write, which
    /// are exactly the ones [`Command::no_commit`] reads it from.
    #[test]
    fn no_commit_is_offered_only_by_verbs_that_write() {
        let writes: [&[&str]; 16] = [
            &["migrate"],
            &["config", "observatory-root"],
            &["config", "commit"],
            &["capture"],
            &["promote"],
            &["drop"],
            &["triage"],
            &["new"],
            &["edit"],
            &["sharpen"],
            &["status"],
            &["link"],
            &["tag"],
            &["note"],
            &["cite"],
            &["handoff"],
        ];
        for path in writes {
            assert!(
                offers_no_commit(path),
                "{path:?} writes, so its help offers --no-commit"
            );
        }
        let reads: [&[&str]; 13] = [
            &["init"],
            &["check"],
            &["config"],
            &["completions"],
            &["inbox"],
            &["show"],
            &["log"],
            &["list"],
            &["near"],
            &["trace"],
            &["impact"],
            &["graph"],
            &["review"],
        ];
        for path in reads {
            assert!(
                !offers_no_commit(path),
                "{path:?} only reads, so its help does not offer --no-commit"
            );
        }
        let visible = Cli::command()
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
            .count();
        // Every verb is in one list; `config` is in `reads` as itself and in
        // `writes` as its two settings.
        assert_eq!(writes.len() - 2 + reads.len(), visible);
    }

    /// Whether the verb's own `--no-commit` was given.
    fn skips(command: &Command) -> bool {
        command.commit_arg().is_some_and(|c| c.no_commit)
    }

    /// The lock's holder record names the verb as typed and the one node or
    /// entry it acts on, and never the text a verb was given.
    #[test]
    fn the_lock_label_is_the_verb_and_its_target_only() {
        let label = |argv: &[&str]| parse_cli(argv).expect("parses").lock_label();
        assert_eq!(label(&["edit", "a-node"]), "neb edit a-node");
        assert_eq!(
            label(&["--root", "/c", "tag", "n", "--add", "x"]),
            "neb tag n"
        );
        assert_eq!(label(&["link", "a", "refines", "b"]), "neb link a");
        assert_eq!(label(&["drop", "i1", "--no-commit"]), "neb drop i1");
        assert_eq!(
            label(&["capture", "a", "private", "thought"]),
            "neb capture"
        );
        assert_eq!(label(&["new", "A private title", "--body", "b"]), "neb new");
        assert_eq!(
            label(&["config", "observatory-root", "/obs"]),
            "neb config observatory-root"
        );
        assert_eq!(label(&["migrate"]), "neb migrate");
    }

    /// After a writing verb the flag is that verb's; before any verb it is
    /// the old global spelling, which `run` warns about or refuses; after a
    /// read-only verb it is an unknown argument rather than a silent no-op.
    #[test]
    fn no_commit_parses_where_it_means_something() {
        let after = parse_cli(&["drop", "i1", "--no-commit"]).expect("drop --no-commit");
        assert!(!after.no_commit && skips(&after.command));
        let config = parse_cli(&["config", "commit", "on", "--no-commit"]).expect("config");
        assert!(skips(&config.command));
        let before = parse_cli(&["--no-commit", "drop", "i1"]).expect("--no-commit drop");
        assert!(before.no_commit && !skips(&before.command));
        let plain = parse_cli(&["drop", "i1"]).expect("drop");
        assert!(!plain.no_commit && !skips(&plain.command));
        // What `run` names in its warning or its refusal.
        assert_eq!(before.verb, "drop");
        assert_eq!(config.verb, "config commit");
        let read = parse_cli(&["--no-commit", "list"]).expect("--no-commit list");
        assert!(read.no_commit && read.command.commit_arg().is_none());
        assert_eq!(read.verb, "list");
        // Neither spelling is global, so `parse_from`'s check for a verb
        // argument conflicting with a global before the verb never sees one,
        // alone or beside `--json`.
        let root = Cli::command();
        let hidden = root
            .get_arguments()
            .find(|a| a.get_long() == Some("no-commit"))
            .expect("the pre-verb spelling");
        assert!(hidden.is_hide_set() && !hidden.is_global_set());
        for args in [
            ["--json", "--no-commit", "drop", "i1"].as_slice(),
            &["--no-commit", "--json", "triage"],
            &["--json", "drop", "i1", "--no-commit"],
            &["drop", "--json", "i1", "--no-commit"],
        ] {
            let cli = parse_cli(args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
            assert!(
                cli.json && (cli.no_commit || skips(&cli.command)),
                "{args:?}"
            );
        }
        for args in [
            ["show", "x", "--no-commit"].as_slice(),
            &["trace", "x", "--no-commit"],
            &["review", "--no-commit"],
        ] {
            let err = parse_cli(args).err().unwrap_or_default();
            assert!(
                err.contains("unexpected argument '--no-commit'"),
                "{args:?}: {err}"
            );
        }
    }

    /// `--limit` and `--depth` are optional: without them nothing is cut.
    #[test]
    fn output_bounds_default_to_everything() {
        assert!(matches!(
            parse_cli(&["list"]).map(|c| c.command),
            Ok(Command::List { limit: None, .. })
        ));
        assert!(matches!(
            parse_cli(&["inbox"]).map(|c| c.command),
            Ok(Command::Inbox { limit: None })
        ));
        assert!(matches!(
            parse_cli(&["review"]).map(|c| c.command),
            Ok(Command::Review { limit: None, .. })
        ));
        assert!(matches!(
            parse_cli(&["trace", "x"]).map(|c| c.command),
            Ok(Command::Trace { depth: None, .. })
        ));
        assert!(matches!(
            parse_cli(&["review", "--short", "--limit", "3"]).map(|c| c.command),
            Ok(Command::Review {
                short: true,
                limit: Some(3),
                ..
            })
        ));
        let err = parse_cli(&["list", "--limit", "all"])
            .err()
            .unwrap_or_default();
        assert!(err.contains("--limit"), "{err}");
    }

    /// Every count flag goes through the one bounded parser, found by walking
    /// the whole command tree so a new `--limit` cannot opt out: each refuses
    /// the value under its minimum, takes the minimum, and says it in `--help`.
    #[test]
    fn count_flags_share_one_lower_bound() {
        fn walk(cmd: &clap::Command, found: &mut Vec<(String, clap::Arg)>) {
            for arg in cmd.get_arguments() {
                let bounded = matches!(arg.get_long(), Some("limit" | "depth" | "since"))
                    || arg.get_short() == Some('k');
                if bounded {
                    found.push((cmd.get_name().to_owned(), arg.clone()));
                }
            }
            for sub in cmd.get_subcommands() {
                walk(sub, found);
            }
        }
        let mut found = Vec::new();
        walk(&Cli::command(), &mut found);

        let mut seen = Vec::new();
        for (verb, arg) in &found {
            let flag = arg.get_long().unwrap_or_default();
            let (below, least) = if flag == "since" {
                ("-1", MIN_DAYS.to_string())
            } else {
                ("0", MIN_COUNT.to_string())
            };
            // The argument alone, as its verb declares it, so the verb's
            // other required arguments and conflicts are not in the way.
            let parse = |value: &str| {
                clap::Command::new("probe")
                    .arg(arg.clone())
                    .try_get_matches_from(["probe".to_owned(), format!("--{flag}={value}")])
            };
            let refused = parse(below).expect_err(&format!("{verb} --{flag} {below}"));
            assert!(
                refused.to_string().contains(&format!("at least {least}")),
                "{verb} --{flag}: {refused}"
            );
            assert!(parse(&least).is_ok(), "{verb} --{flag} {least}");
            let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
            assert!(
                help.contains(&format!("at least {least}")),
                "{verb} --{flag} help does not state its minimum: {help}"
            );
            seen.push(format!("{verb} --{flag}"));
        }
        seen.sort();
        assert_eq!(
            seen,
            [
                "inbox --limit",
                "list --limit",
                "near --limit",
                "review --limit",
                "review --since",
                "trace --depth",
            ],
            "the bounded flags this walk found"
        );
    }

    /// `open` is a deprecated alias for `review --short`: it still parses,
    /// but `--help` no longer offers it.
    #[test]
    fn the_deprecated_open_alias_is_hidden() {
        let open = Cli::command()
            .find_subcommand("open")
            .cloned()
            .expect("`open` still parses");
        assert!(open.is_hide_set(), "`open` must be hidden from --help");
        assert!(!help().contains("\n  open "), "`open` has no row in --help");
        assert!(matches!(
            parse_cli(&["open", "--tag", "physics"]).map(|c| c.command),
            Ok(Command::Open { tags }) if tags == ["physics"]
        ));
    }

    /// `--short` is its own view with its own thresholds, so it refuses the
    /// full report's `--since` and `--out`; `--tag` belongs to it alone.
    #[test]
    fn review_short_owns_tag_and_refuses_since_and_out() {
        assert!(matches!(
            parse_cli(&["review", "--short", "--tag", "a", "--tag", "b"]).map(|c| c.command),
            Ok(Command::Review { short: true, tags, .. }) if tags == ["a", "b"]
        ));
        for args in [
            ["review", "--short", "--since", "7"].as_slice(),
            &["review", "--short", "--out", "r.md"],
        ] {
            let err = parse_cli(args).err().unwrap_or_default();
            assert!(err.contains("cannot be used with"), "{args:?}: {err}");
        }
        let err = parse_cli(&["review", "--tag", "a"])
            .err()
            .unwrap_or_default();
        assert!(err.contains("--short"), "--tag without --short: {err}");
    }

    /// `graph`'s formats conflict with the global `--json` on either side of
    /// the verb, with the error clap gives when `--json` follows it.
    #[test]
    fn graph_formats_refuse_json_before_or_after_the_verb() {
        for (before, after) in [
            (
                ["--json", "graph", "--mermaid"].as_slice(),
                ["graph", "--mermaid", "--json"].as_slice(),
            ),
            (
                &["--json", "graph", "--mermaid", "--from", "x"],
                &["graph", "--mermaid", "--from", "x", "--json"],
            ),
            (
                &["--json", "graph", "--from", "x", "--mermaid"],
                &["graph", "--from", "x", "--mermaid", "--json"],
            ),
        ] {
            let Err(refused) = parse_from(std::iter::once("neb").chain(before.iter().copied()))
            else {
                panic!("{before:?} parsed");
            };
            assert_eq!(
                refused.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{before:?}"
            );
            let err = refused.render().to_string();
            assert!(err.contains("cannot be used with '--json'"), "{err}");
            assert_eq!(Err(err), parse_cli(after).map(|_| ()), "{after:?}");
        }
        assert!(parse_cli(&["--json", "graph"]).is_ok_and(|cli| cli.json));
        assert!(matches!(
            parse_cli(&["--no-commit", "graph", "--mermaid", "--from", "x"]).map(|c| c.command),
            Ok(Command::Graph { mermaid: true, from: Some(from) }) if from == "x"
        ));
    }

    /// `completions` refuses `--root` and `--json`, which it would ignore, on
    /// either side of the verb, with the error clap gives after it.
    #[test]
    fn completions_refuse_root_and_json_before_or_after_the_verb() {
        for (before, after, global) in [
            (
                ["--json", "completions", "bash"].as_slice(),
                ["completions", "bash", "--json"].as_slice(),
                "--json",
            ),
            (
                &["--root", "x", "completions", "bash"],
                &["completions", "bash", "--root", "x"],
                "--root <DIR>",
            ),
        ] {
            let Err(refused) = parse_from(std::iter::once("neb").chain(before.iter().copied()))
            else {
                panic!("{before:?} parsed");
            };
            assert_eq!(
                refused.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{before:?}"
            );
            let err = refused.render().to_string();
            assert!(
                err.contains(&format!("cannot be used with '{global}'")),
                "{err}"
            );
            assert_eq!(Err(err), parse_cli(after).map(|_| ()), "{after:?}");
        }
        assert!(parse_cli(&["completions", "bash"]).is_ok());
    }

    /// The declared conflicts between a verb's argument and a global flag.
    /// Clap checks these only when the flag follows the verb and
    /// [`parse_from`] covers the other side; each one here needs a case in
    /// the test above and in the CLI tests.
    #[test]
    fn global_conflicts_are_the_ones_tested() {
        fn walk(cmd: &clap::Command, found: &mut Vec<String>) {
            for sub in cmd.get_subcommands() {
                for arg in sub.get_arguments().filter(|a| !a.is_global_set()) {
                    for other in sub.get_arg_conflicts_with(arg) {
                        if other.is_global_set() {
                            found.push(format!("{} {} {}", sub.get_name(), arg, other));
                        }
                    }
                }
                walk(sub, found);
            }
        }
        let mut cmd = Cli::command();
        cmd.build();
        let mut found = Vec::new();
        walk(&cmd, &mut found);
        assert_eq!(
            found,
            [
                "completions <SHELL> --root <DIR>",
                "completions <SHELL> --json",
                "graph --mermaid --json",
                "graph --from <ID> --json"
            ]
        );
    }

    /// The variant order mirrors the template's section order, so a command
    /// left out of the template would still surface next to its group.
    #[test]
    fn variant_order_matches_the_template() {
        let names: Vec<_> = Cli::command()
            .get_subcommands()
            .filter(|s| !s.is_hide_set())
            .map(|s| s.get_name().to_string())
            .collect();
        let expected = [
            ["init", "check", "migrate", "config", "completions"].as_slice(),
            &["capture", "inbox", "promote", "drop", "triage"],
            &["new", "edit", "sharpen", "status", "link", "tag", "note"],
            &["cite", "handoff"],
            &["show", "log", "list", "near", "trace", "impact", "graph"],
            &["review"],
        ]
        .concat();
        assert_eq!(names, expected);
    }

    /// The wrappers are the only place a status or an edge kind is spelled
    /// for the command line, so they have to agree with the core types they
    /// stand in for.
    fn parse_cli(args: &[&str]) -> Result<Cli, String> {
        parse_from(std::iter::once("neb").chain(args.iter().copied()))
            .map_err(|e| e.render().to_string())
    }

    /// `capture`, `note` and `near` take remaining words as the thought, but
    /// a flag after those words is still a flag. Swallowing `--quiet` or
    /// `--no-commit` into the text is how the corpus got silently polluted.
    #[test]
    fn trailing_flags_after_free_text_are_flags() {
        let cli = parse_cli(&["capture", "an idea", "--quiet"]).expect("capture --quiet");
        assert!(!skips(&cli.command));
        match cli.command {
            Command::Capture { quiet, text, .. } => {
                assert!(quiet);
                assert_eq!(text, ["an idea"]);
            }
            _ => panic!("expected capture"),
        }

        let cli =
            parse_cli(&["capture", "an", "idea", "--no-commit"]).expect("capture --no-commit");
        assert!(skips(&cli.command));
        match cli.command {
            Command::Capture { quiet, text, .. } => {
                assert!(!quiet);
                assert_eq!(text, ["an", "idea"]);
            }
            _ => panic!("expected capture"),
        }

        let cli = parse_cli(&["note", "alpha-beta", "a thought", "--no-commit"])
            .expect("note --no-commit");
        assert!(skips(&cli.command));
        match cli.command {
            Command::Note { node, by, text, .. } => {
                assert_eq!(node, "alpha-beta");
                assert_eq!(by, None);
                assert_eq!(text, ["a thought"]);
            }
            _ => panic!("expected note"),
        }

        let cli = parse_cli(&["near", "some query", "--limit", "2"]).expect("near --limit");
        match cli.command {
            Command::Near { limit, query } => {
                assert_eq!(limit, 2);
                assert_eq!(query, ["some query"]);
            }
            _ => panic!("expected near"),
        }

        let Err(err) = parse_cli(&["near", "some query", "--quiet"]) else {
            panic!("near has no --quiet")
        };
        assert!(
            err.contains("--quiet"),
            "unknown trailing flag should be named: {err}"
        );

        let cli = parse_cli(&["capture", "--", "an idea", "--quiet"]).expect("capture -- escape");
        assert!(!skips(&cli.command));
        match cli.command {
            Command::Capture { quiet, text, .. } => {
                assert!(!quiet);
                assert_eq!(text, ["an idea", "--quiet"]);
            }
            _ => panic!("expected capture"),
        }
    }

    /// Each key is one action, `t` is the only one that takes text, and
    /// anything else is refused by name rather than guessed at.
    #[test]
    fn triage_keys_parse_to_their_actions() {
        for (line, key) in [
            ("p\n", Key::Act(Action::Promote)),
            ("  2 ", Key::Act(Action::PromoteUnder(2))),
            ("d", Key::Act(Action::Drop)),
            ("s", Key::Act(Action::Skip)),
            ("q", Key::Act(Action::Quit)),
            ("t", Key::AskTitle),
            (
                "t  A title  here ",
                Key::Act(Action::Title("A title  here".into())),
            ),
            ("?", Key::Help),
            ("h", Key::Help),
            ("   \n", Key::Nothing),
        ] {
            assert_eq!(parse_key(line).unwrap(), key, "{line:?}");
        }
        for line in [
            "x",
            "pp",
            "p now",
            "d 2",
            "+1",
            "-1",
            "1.5",
            "99999999999999999999999",
        ] {
            let Err(KeyError::Unknown(said)) = parse_key(line) else {
                panic!("{line:?} should be refused")
            };
            assert_eq!(said, line.trim());
        }
    }

    /// On a terminal the keys are listed and prompted for, and a refusal is
    /// reported and the same entry asked about again rather than ending the
    /// session, which is what lets a person correct a slip.
    #[test]
    fn interactive_triage_prompts_and_asks_again_after_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = Corpus::init(&dir.path().join("corpus")).unwrap();
        let entry = ops::capture(&corpus, "a thought worth keeping").unwrap();
        let mut input = std::io::Cursor::new("x\n7\nt\n!!!\np\nt Worth keeping\np\n");
        let mut out = Vec::new();
        let commits = CommitOpts {
            skip: false,
            json: false,
        };
        let done = triage(&corpus, None, &mut input, &mut out, true, commits);
        assert!(done.is_ok(), "refusals do not end a terminal session");
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("p promote as a root"), "{out}");
        assert!(out.contains("title> "), "{out}");
        assert_eq!(
            out.matches("\n[1/1]").count(),
            1,
            "the entry is shown once, then asked about again:\n{out}"
        );
        assert!(
            out.contains(&format!("promoted {} -> worth-keeping", entry.id)),
            "{out}"
        );
        assert!(corpus.inbox().unwrap().0.is_empty());
    }

    /// A screen that goes away once the first decision is reported.
    struct GoesAfterFirstStep(Vec<u8>);

    impl Write for GoesAfterFirstStep {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl output::Closable for GoesAfterFirstStep {
        fn is_closed(&self) -> bool {
            String::from_utf8_lossy(&self.0).contains("dropped ")
        }
    }

    /// When the screen closes mid-session, the session ends as `q` ends it:
    /// the decision already made stays, and the keys typed after it are
    /// never applied to entries nobody saw.
    #[test]
    fn triage_ends_as_quit_does_when_its_screen_closes() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = Corpus::init(&dir.path().join("corpus")).unwrap();
        let first = ops::capture(&corpus, "the first thought").unwrap();
        let second = ops::capture(&corpus, "the second thought").unwrap();
        let mut input = std::io::Cursor::new("d\nd\n");
        let mut out = GoesAfterFirstStep(Vec::new());
        let commits = CommitOpts {
            skip: true,
            json: false,
        };
        let done = triage(&corpus, None, &mut input, &mut out, false, commits);
        assert!(done.is_ok(), "a closed screen is no refusal");
        let waiting: Vec<_> = corpus
            .inbox()
            .unwrap()
            .0
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(waiting, [second.id], "only {} was dropped", first.id);
    }

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
