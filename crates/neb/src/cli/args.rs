// size: `Command` is one cohesive clap tree, a documented variant per verb.
//! The clap tree: [`Cli`], its [`Command`]s and their settings, and the
//! bounded value parsers every count flag goes through.
//!
//! [`StatusArg`] and [`EdgeKindArg`] exist because core does not depend on
//! clap: they are the `ValueEnum` wrappers that keep `--help` listing the
//! possible values and the shell completions offering them.

use super::help::{ABOUT, HELP_TEMPLATE};
use clap::{Args, Parser, Subcommand, ValueEnum};
use nebula_core::{EdgeType, NEAR_DEFAULT, Status};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "neb",
    version,
    about = ABOUT,
    long_about = ABOUT,
    after_help = "The corpus lives outside this repository. It is found via --root, else \
                  $NEBULA_ROOT, else the nearest corpus at or above the current directory, \
                  else ~/.config/nebula/root, else ~/.nebula.\n\n\
                  Use `neb help` or `neb help <COMMAND>` for full help.",
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
        #[arg(long, value_parser = days, allow_negative_numbers = true)]
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
