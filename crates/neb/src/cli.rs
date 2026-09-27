//! The command-line surface: the clap tree and the dispatch from parsed
//! arguments to `nebula_core`. Nothing else in the crate knows about clap.
//!
//! Dispatch is a table. Each arm parses, makes one core operation, and either
//! writes the returned value as JSON, through [`render::json`]'s view where
//! core's serialisation would leave a field out, or hands it to `render`. A
//! write is one call into [`nebula_core::verb`], which takes the lock, writes,
//! commits and reads the advice after; a read is [`Corpus::open`] plus one
//! query. A command that wants to do anything else belongs in core.
//!
//! The environment and the working directory are read once, in [`main`],
//! into the [`Locations`] every arm hands down: core never reads either.
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
use nebula_core::verb::{self, CiteReport, CommitPolicy, RootWarning, WriteOptions};
use nebula_core::{
    Citation, CloseTag, CommitOutcome, Corpus, CorpusLock, Direction, EdgeType, Error, Handoff,
    Locations, NEAR_DEFAULT, NewNode, OBSERVATORY_ROOT_ENV, ObservatoryLink, ObservatoryRoot,
    ObservatorySource, Promotion, Severity, Status, Triage, graph, ops,
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
pub(crate) struct Cli {
    /// Corpus location. Defaults to `$NEBULA_ROOT`, else the corpus the current directory is in,
    /// else `~/.config/nebula/root`, else `~/.nebula`.
    #[arg(long, global = true, value_name = "DIR")]
    pub(crate) root: Option<PathBuf>,

    /// Emit JSON instead of text, for scripts and agents.
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// `--no-commit` written before the verb, the spelling from when it was
    /// global. Deprecated and hidden: each verb that writes offers the flag
    /// itself. Before one of those it still skips the commit, with a warning
    /// (STD-01 §R35); before a verb that never commits it is refused, since
    /// there is nothing for it to skip (STD-01 §R28).
    #[arg(long = "no-commit", hide = true)]
    pub(crate) no_commit: bool,

    #[command(subcommand)]
    pub(crate) command: Command,

    /// The verb as typed, a setting's name included (`config commit`), for
    /// the messages that name it and the lock's holder record. Filled in by
    /// [`parse_from`].
    #[arg(skip)]
    pub(crate) verb: String,
}

impl Cli {
    /// What this process records as the holder of any lock it takes:
    /// `neb edit a-node`, `neb capture`. The verb and the one node or entry
    /// it names, never the text it was given, which may be private and
    /// would sit in a file every other writer reads.
    pub(crate) fn lock_label(&self) -> String {
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
pub(crate) struct CommitArg {
    /// Skip the commit this once, where `neb config commit on` would make one.
    #[arg(long)]
    pub(crate) no_commit: bool,
}

/// [`Status`] as a command-line value.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub(crate) enum StatusArg {
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
pub(crate) enum EdgeKindArg {
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
pub(crate) enum Command {
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
        /// Who authored the edited prose. Required under an Orbit run.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
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
pub(crate) enum ConfigSetting {
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
    pub(crate) fn commit_arg(&self) -> Option<CommitArg> {
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
pub(crate) enum OnOff {
    On,
    Off,
}

impl From<OnOff> for bool {
    fn from(s: OnOff) -> Self {
        match s {
            OnOff::On => true,
            OnOff::Off => false,
        }
    }
}

/// A refusal the CLI exits on. Core errors become one through [`render`], so
/// the advice that names commands stays in the crate that has commands.
pub(crate) struct Failure(render::Refusal);

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
pub(crate) enum KeyError {
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

/// After a write that added tags, say on stderr which of them read as a
/// variant of a tag already in use, in every output mode.
///
/// Advice, never a refusal: there is no declared list to refuse against, and
/// `check` rule 11 reports the same drift later. Core read them once the
/// write lock was released, and a corpus it could not read for the
/// comparison costs the note, not the command.
fn note_close_tags(close: &[CloseTag]) {
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
/// every other writer carries on meanwhile (STD-03 §R1). The caller saves
/// the result with [`verb::edit`], which checks it.
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
fn keep_refused(refused: Failure, corpus: &Corpus, node: &str, edited: Edited) -> Failure {
    let Failure(refusal) = refused;
    Failure(
        match Corpus::keep_edit(corpus.locations(), node, &edited.text) {
            Ok(path) => refusal.kept_at(&path),
            Err(not_kept) => match edited.file.keep() {
                Ok((_, path)) => refusal.kept_at(&path),
                Err(also) => refusal.not_kept(&format!("{not_kept}; {}", also.error)),
            },
        },
    )
}

/// What every mutating arm needs to decide whether to commit and how to say
/// so: `--no-commit` waives the setting once, and `--json` leaves the
/// `committed` notice out.
#[derive(Clone, Copy)]
pub(crate) struct CommitOpts {
    pub(crate) skip: bool,
    pub(crate) json: bool,
}

impl CommitOpts {
    /// How a verb writes: the default lock wait, and the commit unless
    /// `--no-commit` waived it.
    fn options(self) -> WriteOptions {
        WriteOptions {
            commit: if self.skip {
                CommitPolicy::Skip
            } else {
                CommitPolicy::Configured
            },
            ..WriteOptions::default()
        }
    }
}

/// Say on stderr, in every mode, when a machine setting will keep commands
/// from finding the corpus about to be created at `target`.
fn warn_root(warning: &RootWarning, target: &Path) {
    match warning {
        RootWarning::DefaultWhileConfigured {
            setting,
            configured,
        } => errln!(
            "warning: creating ~/.nebula while {} points to {}",
            setting.display(),
            configured.display()
        ),
        RootWarning::Shadowed {
            setting,
            configured,
        } => {
            // The command that repoints the machine default, never a hand
            // edit: absolute, since the setting is read from every
            // directory, and quoted, so it runs as printed (STD-02 §R26).
            // Made absolute lexically, as capture's note is.
            let target = std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf());
            errln!(
                "warning: {} still points to {}, not {}; run `neb init {} --set-root --force` to point commands at this corpus",
                setting.display(),
                configured.display(),
                target.display(),
                shell_word(&target.to_string_lossy())
            );
        }
    }
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

/// Say on stderr what the commit after a write did: the notice is not the
/// verb's payload (STD-01 §R12), so `E=$(neb capture -q …)` holds the id
/// alone. `--json` leaves the `committed` notice out.
///
/// Called after the verb has printed its own result, because the write has
/// already landed and a refused commit, returned here, must never read as
/// the write failing. `root` names the corpus in the note a commit asked for
/// and not made gets.
fn report_commit(
    root: &Path,
    opts: CommitOpts,
    commit: Option<nebula_core::Result<CommitOutcome>>,
) -> std::result::Result<(), Failure> {
    match commit.transpose()? {
        Some(CommitOutcome::Committed(done)) if !opts.json => {
            let short = done.hash.get(..7).unwrap_or(&done.hash);
            errln!("{}", render::notice(&format!("committed {short}")));
        }
        // Asked for and not done: said on stderr in every mode, so the
        // payload on stdout reads the same either way.
        Some(CommitOutcome::NotARepository) => errln!(
            "note: not committed: {} is not inside a git work tree",
            root.display()
        ),
        Some(
            CommitOutcome::Committed(_) | CommitOutcome::Disabled | CommitOutcome::NothingToCommit,
        )
        | None => {}
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
pub(crate) enum Key {
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
pub(crate) fn parse_key(line: &str) -> std::result::Result<Key, KeyError> {
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
/// Each decision is one [`Triage::decide`]: the write and its commit under
/// one lock, never held across the wait for the next line.
pub(crate) fn triage(
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
        let decided = session.decide(corpus, action, &commits.options())?;
        match decided.value {
            Ok(step) => {
                say(out, &render::step(&step))?;
                report_commit(corpus.root(), commits, decided.commit)?;
                if let Step::Titled { .. } = step {
                    continue;
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
fn run(cli: Cli, locations: &Locations) -> Outcome {
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
                            target.display()
                        ))
                    );
                }
            }
            for warning in &done.warnings {
                warn_root(warning, target);
            }
            Ok(ok)
        }

        Command::Check => {
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

        Command::Migrate { .. } => {
            let done = verb::migrate(locations, root, &commits.options())?;
            let report = &done.value.report;
            if json {
                out_json(report)?;
            } else {
                out!("{}", render::migration(report));
            }
            notify(json, Some(render::migration_notice(report)));
            report_commit(&done.value.root, commits, done.commit)?;
            Ok(ok)
        }

        Command::Config {
            setting:
                ConfigSetting::ObservatoryRoot {
                    dir, drop_legacy, ..
                },
        } => {
            // Without a change to make, this is a read and takes no lock.
            let mut corpus = Corpus::open(locations, root)?;
            let done = verb::set_observatory_root(
                &mut corpus,
                dir.as_deref(),
                drop_legacy,
                &commits.options(),
            )?;
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
            Ok(ok)
        }

        Command::Config {
            setting: ConfigSetting::Commit { state, .. },
        } => {
            let mut corpus = Corpus::open(locations, root)?;
            // Reading the setting is a read, so it takes no lock.
            let (setting, commit) = match state {
                Some(state) => {
                    let done =
                        verb::set_commit(&mut corpus, bool::from(state), &commits.options())?;
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
            let text = capture_text(&text)?;
            let k = if quiet { 0 } else { NEAR_DEFAULT };
            let done = verb::capture_at(locations, root, &text, k, &commits.options())?;
            let captured = done.value;
            if let Some(warning) = &captured.warning {
                warn_root(warning, &captured.root);
            }
            // Never silent, never a question: a mistyped `--root` or
            // `$NEBULA_ROOT` would otherwise start a second corpus unnoticed.
            // Absolute so a relative typo shows where it landed; made so
            // lexically, because paths are used as given, never resolved.
            if captured.created {
                let shown =
                    std::path::absolute(&captured.root).unwrap_or_else(|_| captured.root.clone());
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
            Ok(ok)
        }

        Command::Inbox { limit } => {
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
            Ok(ok)
        }

        Command::Drop { entry, .. } => {
            let corpus = Corpus::open(locations, root)?;
            let done = verb::drop(&corpus, &entry, &commits.options())?;
            if json {
                out_json(&done.value)?;
            } else {
                outln!("dropped {}", render::bold(&entry));
            }
            report_commit(corpus.root(), commits, done.commit)?;
            Ok(ok)
        }

        Command::Triage { by, .. } => {
            locations.write_gate(nebula_core::WriteIntent::Ordinary)?;
            // Refused before anything is read: there is no one payload a
            // session of keyed decisions could honestly be.
            if json {
                return Err(Error::Interactive("triage".into()).into());
            }
            let corpus = Corpus::open(locations, root)?;
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
            Ok(ok)
        }

        Command::Edit { node, by, .. } => {
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
            Ok(ok)
        }

        Command::Sharpen {
            node,
            confirm: true,
            ..
        } => {
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
            Ok(ok)
        }

        Command::Status {
            node, status, why, ..
        } => {
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
            Ok(ok)
        }

        Command::Link {
            from, kind, to, by, ..
        } => {
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
            Ok(ok)
        }

        Command::Tag {
            target,
            add,
            remove,
            ..
        } if target == "list" && add.is_empty() && remove.is_empty() => {
            let counts = Corpus::open(locations, root)?.query(graph::tags)?;
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
            Ok(ok)
        }

        Command::Note { node, by, text, .. } => {
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
            let corpus = Corpus::open(locations, root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            // Core reads the observatory setting before the write, under
            // `--json` too, whose payload says where the record is.
            let done = verb::cite(
                &corpus,
                &node,
                &Citation {
                    uri,
                    kind,
                    title,
                    note,
                    by,
                    origin: locations.origin(task, run)?,
                },
                &commits.options(),
            )
            .map_err(|e| Failure::about(&e, &node))?;
            let CiteReport { cited, setting } = &done.value;
            if let Some(setting) = setting {
                warn_legacy_observatory_root(&corpus, setting);
            }
            if json {
                out_json(&json::Cited::from(cited))?;
            } else {
                outln!("{} {}", render::bold(&node), render::bold(&cited.reference));
                if let (Some(setting), Some(link)) = (setting, &cited.observatory) {
                    print_record_location(setting, link);
                }
                if bare {
                    print_bare_note();
                }
            }
            report_commit(corpus.root(), commits, done.commit)?;
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
            let corpus = Corpus::open(locations, root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            // Core reads the setting before the write, under `--json` too:
            // the record has to resolve under this root.
            let done = verb::handoff(
                &corpus,
                &node,
                &Handoff {
                    record,
                    note,
                    by,
                    origin: locations.origin(task, run)?,
                },
                &commits.options(),
            )
            .map_err(|e| Failure::about(&e, &node))?;
            let handed = &done.value.done;
            let setting = &done.value.setting;
            warn_legacy_observatory_root(&corpus, setting);
            if json {
                out_json(&json::HandedOff::from(handed))?;
            } else {
                outln!(
                    "{} {} -> {}, handed off to {} {}",
                    render::bold(&node),
                    render::dim(&handed.from.to_string()),
                    handed.doc.node.status,
                    render::bold(&handed.record),
                    render::dim(&format!("({})", handed.reference))
                );
                print_record_location(setting, &handed.observatory);
                if bare {
                    print_bare_note();
                }
            }
            report_commit(corpus.root(), commits, done.commit)?;
            Ok(ok)
        }

        Command::Show { node, at } => {
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

        Command::Log { node } => {
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

        Command::List {
            status,
            tags,
            limit,
        } => {
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

        Command::Near { limit, query } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                return Err(Failure::say("nothing to look near"));
            }
            let (near, matched) = Corpus::open(locations, root)?
                .query(|graph| graph::near_counted(graph, &query, limit))?;
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

        Command::Impact { node } => {
            let report =
                Corpus::open(locations, root)?.query(|graph| graph::impact(graph, &node))?;
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
            let exported = Corpus::open(locations, root)?.query(graph::export)?;
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
            short_review(locations, root, json, &tags, limit)?;
            Ok(ok)
        }

        Command::Review {
            short: false,
            since,
            out,
            limit,
            ..
        } => {
            let corpus = Corpus::open(locations, root)?;
            let mut report = corpus.query(|graph| graph::review(graph, &corpus.inbox()?, since))?;
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
            short_review(locations, root, json, &tags, None)?;
            Ok(ok)
        }
    }
}

/// `review --short`, and the deprecated `open` that now forwards to it:
/// what needs attention now, one line per item.
fn short_review(
    locations: &Locations,
    root: Option<PathBuf>,
    json: bool,
    tags: &[String],
    limit: Option<usize>,
) -> std::result::Result<(), Failure> {
    let corpus = Corpus::open(locations, root)?;
    let mut report = corpus.query(|graph| graph::open(graph, &corpus.inbox()?, tags))?;
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
pub(crate) const MIN_COUNT: usize = 1;

/// The least `review --since` takes, in days.
pub(crate) const MIN_DAYS: i64 = 0;

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
/// `edit --json` and `note --json` print once their write is committed.
/// Core read it with the lock released (STD-03 §R1).
fn out_node_view(
    view: nebula_core::Result<nebula_core::NodeView>,
) -> std::result::Result<(), Failure> {
    out_json(&json::NodeView::from(&view?))
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
