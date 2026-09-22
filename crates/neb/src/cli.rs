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
    Citation, Corpus, CorpusLock, Direction, EdgeType, Error, Graph, NEAR_DEFAULT, NewNode,
    OBSERVATORY, OBSERVATORY_ROOT_ENV, Origin, Promotion, Severity, Status, check, graph, migrate,
    ops,
};
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
  config       Read or set a corpus setting
  completions  Generate shell completion scripts

Inbox:
  capture      Append a thought to the inbox. One line, no decisions, no parent
  inbox        List captures that have not been promoted or dropped
  promote      Turn an inbox entry into a seed node. Suggests parents, never picks one
  drop         Discard an inbox entry. Struck through, never deleted

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

Query:
  show         Show one node in full
  log          List the commits that changed a node
  list         List nodes
  near         The existing nodes closest to some text, or to a node, with a score
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
                  $NEBULA_ROOT, else ~/.config/nebula/root, else ~/.nebula.",
    disable_help_subcommand = true,
    help_template = HELP_TEMPLATE
)]
struct Cli {
    /// Corpus location. Defaults to `$NEBULA_ROOT`, else `~/.config/nebula/root`, else `~/.nebula`.
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Emit JSON instead of text, for scripts and agents.
    #[arg(long, global = true)]
    json: bool,

    /// Skip the commit this once, where `neb config commit on` would make one.
    #[arg(long, global = true)]
    no_commit: bool,

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
    Migrate,

    /// Read or set a corpus setting.
    ///
    /// `config.yaml` stays machine-written: this rewrites it whole rather
    /// than inviting a hand edit. Settings: `observatory-root`, `commit`.
    Config {
        #[command(subcommand)]
        setting: ConfigSetting,
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
    /// will skip at the exact moment the idea arrives. After the entry id it
    /// names the three existing nodes the thought reads closest to, for the
    /// triage that comes later; that is a suggestion, and nothing is linked.
    Capture {
        /// Print the entry id alone, without the nearest nodes.
        #[arg(long, short)]
        quiet: bool,
        /// The thought, as you would say it out loud.
        ///
        /// Remaining words, so it can be typed without quotes. Not a trailing
        /// vararg: a flag after the text (`--quiet`, `--no-commit`) is still
        /// a flag. A dash-leading token belongs in quotes, or after `--`.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },

    /// List captures that have not been promoted or dropped.
    Inbox,

    /// Turn an inbox entry into a seed node. Suggests parents, never picks one.
    ///
    /// Deliberately separate from capture. Most captures should never be
    /// promoted, and dropping one is a normal outcome rather than a failure.
    /// Without `--parent` the node is written as a root and the three
    /// existing nodes it reads closest to are printed afterwards, so a parent
    /// that was defensible can still be linked; no edge is ever written from
    /// that list.
    Promote {
        /// Inbox entry id, from `neb inbox`.
        entry: String,
        /// Print the id and path alone, without the nearest nodes.
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
        /// The node's prose body. `-` reads standard input.
        #[arg(long, value_name = "TEXT|-", allow_hyphen_values = true)]
        body: Option<String>,
        /// A parent this descends from. Repeat for a merge.
        #[arg(long = "parent", value_name = "ID")]
        parents: Vec<String>,
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
    },

    /// Edit a node's body in $VISUAL or $EDITOR.
    ///
    /// The editor sees prose only, never YAML frontmatter. An existing
    /// `## Notes` section is protected because notes are append-only.
    Edit {
        /// Node id.
        node: String,
        /// Who edited the body. Accepted for command consistency, but body
        /// authorship is not represented in the schema and is not recorded.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
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
        /// Who authored the text: `human`, or the agent's session or crew
        /// label. Free text; defaults to `human`.
        #[arg(long, value_name = "LABEL")]
        by: Option<String>,
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
    },

    /// Attach context, with a note saying why it is here.
    Cite {
        /// Node id.
        node: String,
        /// Where it lives. A URL, DOI, path, or almanac wikilink.
        #[arg(long)]
        uri: Option<String>,
        /// paper, study, article, note, discussion, book, dataset, thread,
        /// observatory, other. With `observatory`, `--uri` is a bare record
        /// id (`Q002`) resolved through the configured observatory root.
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
    },

    /// Show one node in full.
    Show {
        /// Node id.
        node: String,
        /// Show the node at this commit hash or at the end of this date.
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
    },

    /// The existing nodes closest to some text, or to a node, with a score.
    ///
    /// Word overlap over title, tags and body (BM25, title and tags weighted
    /// up; no embeddings, no network), scored `0..=1` and best first. Nodes
    /// sharing no word with the query are left out, so an empty answer means
    /// the thought is unlike anything here. For triage: run it on a capture,
    /// pick a parent if one is defensible, otherwise promote as a root. It
    /// suggests; `link` and `--parent` are still yours to run.
    Near {
        /// How many to return.
        #[arg(long, short = 'k', value_name = "K", default_value_t = NEAR_DEFAULT)]
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
    /// Use `--json` for structured data or `--mermaid` for a diagram that can
    /// be pasted into a document. Without either, this prints a short hint.
    Graph {
        /// Emit a Mermaid `graph BT` diagram.
        #[arg(long, conflicts_with = "json")]
        mermaid: bool,
        /// Limit the Mermaid diagram to this node's ancestors and descendants.
        #[arg(long, value_name = "ID", requires = "mermaid")]
        from: Option<String>,
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

/// The settings `neb config` reads and writes.
#[derive(Subcommand)]
enum ConfigSetting {
    /// Where the Observatory checkout is, so `cite --kind observatory Q002`
    /// resolves. Without a directory, prints the effective root and where it
    /// came from. Falls back to `$OBSERVATORY_ROOT` when unset here.
    ObservatoryRoot {
        /// The checkout. Omit to read the current setting.
        dir: Option<PathBuf>,
    },

    /// Whether each mutating verb commits the corpus afterwards, when the
    /// root is inside a git work tree. Off by default. The commit stages
    /// `nodes/`, `inbox/` and `config.yaml` only, is `neb <verb> <ids>`,
    /// never pushes, and is refused (the write kept) when something outside
    /// the corpus is already staged. `--no-commit` skips it once.
    Commit {
        /// `on` or `off`. Omit to read the current setting.
        state: Option<OnOff>,
    },
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

/// A message the CLI exits on. Core errors become one through [`render`], so
/// the advice that names commands stays in the crate that has commands.
struct Failure(String);

/// Refusals specific to the terminal-owned editor flow.
#[derive(Debug)]
enum EditorError {
    NotConfigured,
    Start {
        editor: String,
        source: std::io::Error,
    },
    Unsuccessful(String),
    NotesChanged,
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str("neither $VISUAL nor $EDITOR names an editor"),
            Self::Start { editor, source } => {
                write!(f, "could not start editor `{editor}`: {source}")
            }
            Self::Unsuccessful(editor) => {
                write!(f, "editor `{editor}` exited unsuccessfully; the node was not changed")
            }
            Self::NotesChanged => f.write_str(
                "the existing ## Notes section was removed, reordered, or changed; use `neb note` to append notes",
            ),
        }
    }
}

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

impl From<EditorError> for Failure {
    fn from(e: EditorError) -> Self {
        Self(e.to_string())
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

/// Resolve a `--body` value, using stdin only for the explicit `-` spelling.
fn body_value(value: Option<String>) -> std::result::Result<String, Failure> {
    use std::io::Read;

    match value.as_deref() {
        None => Ok(String::new()),
        Some("-") => {
            let mut body = String::new();
            std::io::stdin()
                .read_to_string(&mut body)
                .map_err(Error::from)?;
            Ok(body)
        }
        Some(_) => Ok(value.unwrap_or_default()),
    }
}

/// Byte offset of the last exact `## Notes` heading in a body.
fn notes_heading(body: &str) -> Option<usize> {
    let mut found = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "## Notes" {
            found = Some(offset);
        }
        offset += line.len();
    }
    found
}

/// Existing notes are immutable through `edit`; `note` is their append path.
fn preserve_notes(before: &str, after: &str) -> std::result::Result<(), EditorError> {
    let Some(before_at) = notes_heading(before) else {
        return Ok(());
    };
    let Some(after_at) = notes_heading(after) else {
        return Err(EditorError::NotesChanged);
    };
    if before[before_at..].trim_end() != after[after_at..].trim_end() {
        return Err(EditorError::NotesChanged);
    }
    Ok(())
}

/// Let the configured editor rewrite body prose in a temporary file.
fn edit_body(body: &str) -> std::result::Result<String, Failure> {
    use std::io::Write;

    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .ok_or(EditorError::NotConfigured)?;
    let editor_name = editor.to_string_lossy().into_owned();
    let mut file = tempfile::NamedTempFile::new().map_err(Error::from)?;
    file.write_all(body.as_bytes()).map_err(Error::from)?;
    file.flush().map_err(Error::from)?;
    let status = ProcessCommand::new(&editor)
        .arg(file.path())
        .status()
        .map_err(|source| EditorError::Start {
            editor: editor_name.clone(),
            source,
        })?;
    if !status.success() {
        return Err(EditorError::Unsuccessful(editor_name).into());
    }
    let edited = std::fs::read_to_string(file.path()).map_err(Error::from)?;
    preserve_notes(body, &edited)?;
    Ok(edited)
}

/// What every mutating arm needs to decide whether to commit and how to say
/// so: `--no-commit` waives the setting once, and `--json` keeps the
/// confirmation off stdout so the payload stays parseable.
#[derive(Clone, Copy)]
struct CommitOpts {
    skip: bool,
    json: bool,
}

/// Open the corpus and hold its write lock until the returned guard drops.
///
/// Every mutating arm opens this way, so the verb and the [`commit`] that
/// records it are one critical section and no other writer can land a verb
/// between the two. The op underneath takes the same lock again, which costs
/// nothing: it is re-entrant on one thread. A read-only arm opens with plain
/// [`Corpus::open`] and waits for nobody.
fn open_locked(root: Option<PathBuf>) -> std::result::Result<(Corpus, CorpusLock), Failure> {
    let corpus = Corpus::open(root)?;
    let lock = corpus.lock()?;
    Ok((corpus, lock))
}

/// Commit the corpus after a write, if `config.yaml` asks for it.
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
    let done = ops::commit(corpus, verb, ids)?;
    if let Some(done) = done.filter(|_| !opts.json) {
        let short = done.hash.get(..7).unwrap_or(&done.hash);
        println!("{}", render::dim(&format!("committed {short}")));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // A dispatch table is one arm per verb.
fn run(cli: Cli) -> Outcome {
    let ok = ExitCode::SUCCESS;
    let root = cli.root.clone();
    let json = cli.json;
    let commits = CommitOpts {
        skip: cli.no_commit,
        json,
    };

    match cli.command {
        Command::Init {
            path,
            set_root,
            force,
        } => {
            let target = Corpus::resolve_root(path.clone().or(root.clone()))?;
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
                println!("corpus ready at {}", done.root.display());
                if set_root {
                    println!(
                        "wrote {} so every command finds it",
                        Corpus::root_config_path()?.display()
                    );
                } else if root_config_path.is_some() {
                    println!(
                        "run `neb init {} --set-root` to make this corpus the machine default",
                        target.display()
                    );
                }
            }
            if let Some(configured) = default_root_warning {
                eprintln!(
                    "warning: creating ~/.nebula while {} points to {}",
                    Corpus::root_config_path()?.display(),
                    configured.display()
                );
            }
            if let Some(configured) = shadowing_warning {
                let config_path = Corpus::root_config_path()?;
                eprintln!(
                    "warning: {} still points to {}, not {}; run `echo {} > {}` to point commands at this corpus",
                    config_path.display(),
                    configured.display(),
                    target.display(),
                    target.display(),
                    config_path.display()
                );
            }
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
                print!("{}", render::migration(&report));
            }
            // The corpus is at this build's schema now, so it opens; the
            // migration lands as its own commit when the setting is on.
            commit(&Corpus::open(root)?, commits, "migrate", &[])?;
            Ok(ok)
        }

        Command::Config {
            setting: ConfigSetting::ObservatoryRoot { dir },
        } => {
            let mut corpus = Corpus::open(root)?;
            // Only the form that sets the value writes. Reading it back
            // takes no lock and so never waits on a writer.
            let _lock = dir.is_some().then(|| corpus.lock()).transpose()?;
            let (setting, changed) = match dir {
                Some(dir) => (ops::set_observatory_root(&mut corpus, dir)?, true),
                None => (corpus.observatory_root(), false),
            };
            if json {
                out_json(&setting)?;
            } else {
                print!("{}", render::observatory_root(&setting));
            }
            if changed {
                commit(&corpus, commits, "config", &["observatory-root"])?;
            }
            Ok(ok)
        }

        Command::Config {
            setting: ConfigSetting::Commit { state },
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
                print!("{}", render::commit_setting(setting));
            }
            // Turning it on records itself; turning it off leaves the file
            // for the next commit you make by hand, because off means off.
            if changed {
                commit(&corpus, commits, "config", &["commit"])?;
            }
            Ok(ok)
        }

        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "neb", &mut std::io::stdout());
            Ok(ok)
        }

        Command::Capture { quiet, text } => {
            let text = text.join(" ");
            if text.trim().is_empty() {
                return Err(Failure::say("nothing to capture"));
            }
            // Capture must work on a corpus that does not exist yet. Being
            // told to run a setup command is precisely the friction that
            // loses the thought.
            let resolved_root = Corpus::resolve_root(root)?;
            let default_root_warning = Corpus::warning_before_default_init(&resolved_root)?;
            let corpus = Corpus::open_or_init(Some(resolved_root))?;
            let _lock = corpus.lock()?;
            if let Some(configured) = default_root_warning {
                eprintln!(
                    "warning: creating ~/.nebula while {} points to {}",
                    Corpus::root_config_path()?.display(),
                    configured.display()
                );
            }
            let k = if quiet { 0 } else { NEAR_DEFAULT };
            if json {
                let captured = ops::capture_near(&corpus, &text, k)?;
                out_json(&captured)?;
                commit(&corpus, commits, "capture", &[&captured.entry.id])?;
                return Ok(ok);
            }
            // The id goes out before `nodes/` is read for the suggestions:
            // the capture never depended on the rest of the corpus parsing,
            // and a node file that will not must not read as a lost thought.
            let entry = ops::capture(&corpus, &text)?;
            println!("{}", render::bold(&entry.id));
            print!(
                "{}",
                render::suggestions(&ops::suggest(&corpus, &entry.text, k)?)
            );
            commit(&corpus, commits, "capture", &[&entry.id])?;
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
            quiet,
            title,
            body,
            parents,
            tags,
            id,
            by,
            task,
            run,
        } => {
            let body = body_value(body)?;
            let (corpus, _lock) = open_locked(root)?;
            let k = if quiet { 0 } else { NEAR_DEFAULT };
            let created = ops::promote(
                &corpus,
                &entry,
                &Promotion {
                    title,
                    body,
                    parents,
                    tags,
                    origin: Origin::of(task, run),
                    id,
                    by,
                },
                k,
            )?;
            if json {
                out_json(&created)?;
            } else {
                println!(
                    "{} {}",
                    render::bold(&created.doc.node.id),
                    render::dim(&created.path.display().to_string())
                );
                print!("{}", render::suggestions(&created.near));
            }
            commit(&corpus, commits, "promote", &[&entry, &created.doc.node.id])?;
            Ok(ok)
        }

        Command::Drop { entry } => {
            let (corpus, _lock) = open_locked(root)?;
            let dropped = ops::drop(&corpus, &entry)?;
            if json {
                out_json(&dropped)?;
            } else {
                println!("dropped {}", render::bold(&entry));
            }
            commit(&corpus, commits, "drop", &[&entry])?;
            Ok(ok)
        }

        Command::New {
            title,
            body,
            parents,
            kill,
            tags,
            id,
            by,
            task,
            run,
        } => {
            let body = body_value(body)?;
            let (corpus, _lock) = open_locked(root)?;
            let created = ops::new_node(
                &corpus,
                &NewNode {
                    title,
                    body,
                    parents,
                    kill,
                    tags,
                    origin: Origin::of(task, run),
                    id,
                    by,
                },
            )?;
            if json {
                out_json(&created)?;
            } else {
                println!(
                    "{} {}",
                    render::bold(&created.doc.node.id),
                    render::dim(&created.path.display().to_string())
                );
            }
            commit(&corpus, commits, "new", &[&created.doc.node.id])?;
            Ok(ok)
        }

        Command::Edit { node, by } => {
            let (corpus, _lock) = open_locked(root)?;
            let before = corpus.load(&node).map_err(|e| Failure::about(&e, &node))?;
            let body = edit_body(&before.body)?;
            ops::set_body(&corpus, &node, &body, by.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                let docs = corpus.load_all()?;
                let view = graph::node(&Graph::build(&docs)?, &node)?;
                out_json(&view)?;
            } else {
                println!("{}", render::bold(&node));
            }
            commit(&corpus, commits, "edit", &[&node])?;
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
                out_json(&doc)?;
            } else {
                println!("{} kill condition confirmed as yours", render::bold(&node));
            }
            commit(&corpus, commits, "sharpen", &[&node])?;
            Ok(ok)
        }

        Command::Sharpen {
            node,
            kill,
            by,
            confirm: false,
        } => {
            // Clap's `required_unless_present` guarantees the kill is here
            // without `--confirm`; saying so beats an unwrap.
            let Some(kill) = kill else {
                return Err(Failure::say("pass --kill <KILL>, or --confirm"));
            };
            let (corpus, _lock) = open_locked(root)?;
            let doc = ops::sharpen(&corpus, &node, &kill, by.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&doc)?;
            } else {
                println!("{} is now {}", render::bold(&node), doc.node.status);
            }
            commit(&corpus, commits, "sharpen", &[&node])?;
            Ok(ok)
        }

        Command::Status { node, status, why } => {
            let status = Status::from(status);
            // `--why` is this command's flag, so what it does and does not
            // apply to is this command's rule to state.
            if status.is_open() && why.as_ref().is_some_and(|w| !w.trim().is_empty()) {
                return Err(Failure::say("--why only applies to refuted or abandoned"));
            }
            let (corpus, _lock) = open_locked(root)?;
            let changed = ops::set_status(&corpus, &node, status, why.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                out_json(&changed)?;
            } else {
                println!(
                    "{} {} -> {status}",
                    render::bold(&node),
                    render::dim(&changed.from.to_string())
                );
            }
            commit(&corpus, commits, "status", &[&node])?;
            Ok(ok)
        }

        Command::Link { from, kind, to, by } => {
            let (corpus, _lock) = open_locked(root)?;
            let kind = EdgeType::from(kind);
            let changed = ops::link(&corpus, &from, kind, &to, by.as_deref())?;
            if json {
                out_json(&changed)?;
            } else {
                println!(
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
            // One lock over both edits: `--remove x --add y` is one change
            // to the node's tags, not two a second writer may split.
            let (corpus, _lock) = open_locked(root)?;
            let mut doc = ops::tag_remove(&corpus, &target, &remove)?;
            if !add.is_empty() {
                doc = ops::tag_add(&corpus, &target, &add)?;
            }
            if json {
                out_json(&doc)?;
            } else {
                let shown = if doc.node.tags.is_empty() {
                    render::dim("(no tags)")
                } else {
                    doc.node.tags.join(", ")
                };
                println!("{} {shown}", render::bold(&target));
            }
            commit(&corpus, commits, "tag", &[&target])?;
            Ok(ok)
        }

        Command::Note { node, by, text } => {
            let text = text.join(" ");
            if text.trim().is_empty() {
                return Err(Failure::say("nothing to note"));
            }
            let (corpus, _lock) = open_locked(root)?;
            ops::note(&corpus, &node, &text, by.as_deref())
                .map_err(|e| Failure::about(&e, &node))?;
            if json {
                let docs = corpus.load_all()?;
                let view = graph::node(&Graph::build(&docs)?, &node)?;
                out_json(&view)?;
            } else {
                println!("{}", render::bold(&node));
            }
            commit(&corpus, commits, "note", &[&node])?;
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
        } => {
            let (corpus, _lock) = open_locked(root)?;
            let bare = note.as_ref().is_none_or(|n| n.trim().is_empty());
            let cited = ops::cite(
                &corpus,
                &node,
                &Citation {
                    uri,
                    kind: kind.clone(),
                    title,
                    note,
                    by,
                    origin: Origin::of(task, run),
                },
            )?;
            if json {
                out_json(&cited)?;
            } else {
                println!("{} {}", render::bold(&node), render::bold(&cited.reference));
                if kind == OBSERVATORY {
                    let setting = corpus.observatory_root();
                    let record = cited
                        .doc
                        .node
                        .references
                        .iter()
                        .find(|r| r.id == cited.reference)
                        .and_then(|r| r.uri.clone())
                        .unwrap_or_default();
                    match setting.root.as_deref() {
                        None => println!(
                            "\n{}",
                            render::dim(&format!(
                                "No observatory root set, so `{record}` cannot be located. \
                                 Set one with `neb config observatory-root <DIR>` or \
                                 ${OBSERVATORY_ROOT_ENV}."
                            ))
                        ),
                        Some(dir) => match check::resolve_observatory(dir, &record) {
                            Some(path) => println!("{}", render::dim(&path.display().to_string())),
                            None => println!(
                                "\n{}",
                                render::dim(&format!(
                                    "`{record}` does not resolve under {}; `check` will keep \
                                     saying so until the checkout has it.",
                                    dir.display()
                                ))
                            ),
                        },
                    }
                }
                if bare {
                    println!(
                        "\n{}",
                        render::dim(
                            "No note. Add one saying why it is here, or this is a link that rots."
                        )
                    );
                }
            }
            commit(&corpus, commits, "cite", &[&node, &cited.reference])?;
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
            let observatory = corpus.observatory_root().root;
            let view =
                graph::node(&Graph::build(&docs)?, &node)?.with_observatory(observatory.as_deref());
            if json {
                out_json(&view)?;
            } else {
                print!("{}", render::node(&view));
            }
            Ok(ok)
        }

        Command::Log { node } => {
            let corpus = Corpus::open(root)?;
            let history = corpus.history(&node)?;
            if json {
                out_json(&history)?;
            } else {
                print!("{}", render::history(&history));
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

        Command::Near { limit, query } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                return Err(Failure::say("nothing to look near"));
            }
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let near = graph::near(&Graph::build(&docs)?, &query, limit)?;
            if json {
                out_json(&near)?;
            } else {
                print!("{}", render::near(&near));
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

        Command::Graph { mermaid, from } => {
            if !json && !mermaid {
                println!(
                    "neb graph needs an output format; run:  neb graph --json  or  neb graph --mermaid"
                );
                return Ok(ExitCode::from(2));
            }
            let corpus = Corpus::open(root)?;
            let docs = corpus.load_all()?;
            let exported = graph::export(&Graph::build(&docs)?)?;
            if mermaid {
                print!(
                    "{}",
                    render::mermaid(&exported, from.as_deref()).map_err(Failure::say)?
                );
            } else {
                out_json(&exported)?;
            }
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
        assert_eq!(seen, 26, "template rows need updating for a new subcommand");
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
        assert!(text.contains("--no-commit"));
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
            ["init", "check", "migrate", "config", "completions"].as_slice(),
            &["capture", "inbox", "promote", "drop"],
            &["new", "edit", "sharpen", "status", "link", "tag", "note"],
            &["cite"],
            &["show", "log", "list", "near", "trace", "impact", "graph"],
            &["open", "review"],
        ]
        .concat();
        assert_eq!(names, expected);
    }

    /// The wrappers are the only place a status or an edge kind is spelled
    /// for the command line, so they have to agree with the core types they
    /// stand in for.
    fn parse_cli(args: &[&str]) -> Result<Cli, String> {
        Cli::try_parse_from(std::iter::once("neb").chain(args.iter().copied()))
            .map_err(|e| e.render().to_string())
    }

    /// `capture`, `note` and `near` take remaining words as the thought, but
    /// a flag after those words is still a flag. Swallowing `--quiet` or
    /// `--no-commit` into the text is how the corpus got silently polluted.
    #[test]
    fn trailing_flags_after_free_text_are_flags() {
        let cli = parse_cli(&["capture", "an idea", "--quiet"]).expect("capture --quiet");
        assert!(!cli.no_commit);
        match cli.command {
            Command::Capture { quiet, text } => {
                assert!(quiet);
                assert_eq!(text, ["an idea"]);
            }
            _ => panic!("expected capture"),
        }

        let cli =
            parse_cli(&["capture", "an", "idea", "--no-commit"]).expect("capture --no-commit");
        assert!(cli.no_commit);
        match cli.command {
            Command::Capture { quiet, text } => {
                assert!(!quiet);
                assert_eq!(text, ["an", "idea"]);
            }
            _ => panic!("expected capture"),
        }

        let cli = parse_cli(&["note", "alpha-beta", "a thought", "--no-commit"])
            .expect("note --no-commit");
        assert!(cli.no_commit);
        match cli.command {
            Command::Note { node, by, text } => {
                assert_eq!(node, "alpha-beta");
                assert_eq!(by, None);
                assert_eq!(text, ["a thought"]);
            }
            _ => panic!("expected note"),
        }

        let cli = parse_cli(&["near", "some query", "--no-commit"]).expect("near --no-commit");
        assert!(cli.no_commit);
        match cli.command {
            Command::Near { limit, query } => {
                assert_eq!(limit, NEAR_DEFAULT);
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
        assert!(!cli.no_commit);
        match cli.command {
            Command::Capture { quiet, text } => {
                assert!(!quiet);
                assert_eq!(text, ["an idea", "--quiet"]);
            }
            _ => panic!("expected capture"),
        }
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
