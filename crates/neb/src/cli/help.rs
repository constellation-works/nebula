//! `neb --help`, grouped into sections.
//!
//! The grouped sections in `neb --help` are a render concern, not a structure
//! one. Clap's derive has no per-variant `help_heading` for subcommands
//! (`next_help_heading` is args-only and `subcommand_help_heading` only renames
//! the single `Commands:` block), so [`Cli`](super::args::Cli) carries a
//! hand-rolled `help_template` with the rows written out by hand. When adding
//! a command, add the variant to [`Command`](super::args::Command) in the
//! position its section dictates *and* add its row to the template; the tests
//! in `tests/help.rs` check the two stay in sync.

/// `neb --help`, with the subcommands grouped by lifecycle stage. Each row's
/// one-liner is the first line of that variant's doc comment, minus the
/// trailing period clap strips; `help_rows_match_the_variants` enforces it.
pub(super) const HELP_TEMPLATE: &str = "\
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
