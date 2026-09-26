//! The one error type the library returns.
//!
//! Every public function in [`crate::ops`], [`crate::graph`], [`crate::check`],
//! [`crate::migrate`] and [`crate::store`] returns `Result<_, Error>`. The
//! variants are the invariants: a caller matches on the one it cares about
//! rather than parsing a sentence. The CLI turns them into messages and exit
//! codes, the desktop turns them into UI, and neither reads a string.
//!
//! The messages here name what is wrong and nothing else. Advice that tells
//! you which command to run next is presentation, so it lives in the consumer
//! that has commands to suggest.

use crate::model::Status;
use std::path::PathBuf;

/// The library's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong reading or changing a corpus.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No node with that id exists.
    #[error("no node `{0}`")]
    NoSuchNode(String),

    /// No live inbox entry with that id: never captured, or already settled.
    #[error("no open inbox entry `{0}`")]
    NoSuchInboxEntry(String),

    /// There is no corpus where one was expected.
    #[error("no corpus at {}", .0.display())]
    NoCorpus(PathBuf),

    /// A history query requires the corpus to live inside a git work tree.
    #[error("{} is not inside a git work tree", .0.display())]
    NotGitWorkTree(PathBuf),

    /// The node did not exist at the requested commit or date.
    #[error("no node `{node}` at `{revision}`")]
    NoNodeAtRevision {
        /// The node requested.
        node: String,
        /// The hash or date requested.
        revision: String,
    },

    /// An explicit `--root` was empty. Every `join` built from it would
    /// silently resolve to the current directory, which is exactly the bug
    /// this refuses.
    #[error("--root cannot be empty")]
    EmptyRoot,

    /// A machine-local root setting already names a different corpus. The
    /// caller must opt in to replacing it rather than redirecting commands
    /// silently.
    #[error("{} already points to {}, not {}; pass --force to replace it", .path.display(), .configured.display(), .requested.display())]
    RootConfigConflict {
        /// The machine-local setting file.
        path: PathBuf,
        /// The corpus it currently names.
        configured: PathBuf,
        /// The corpus the caller asked to make the default.
        requested: PathBuf,
    },

    /// The corpus on disk follows a schema this build does not read.
    #[error("{} is schema_version {found}, and this build understands {expected}", .path.display())]
    SchemaMismatch {
        /// The config file that declared it.
        path: PathBuf,
        /// What the corpus says it is.
        found: u32,
        /// What this build reads and writes.
        expected: u32,
    },

    /// A corpus that already declares this build's schema holds a node file
    /// that does not parse under it.
    ///
    /// Raised by [`crate::migrate`] alone, and before it writes anything.
    /// Migration reads nodes through a deliberately lenient v1 model that
    /// tolerates unknown keys, because a v1 file holds retired ones; content
    /// already at the current schema gets no such latitude, since the only
    /// thing that leniency could do there is drop a field this build does
    /// not recognise.
    #[error(
        "the corpus already declares schema_version {version}, and {source}; \
         migration will not rewrite a node it cannot read, so nothing was changed"
    )]
    CurrentSchemaUnreadable {
        /// The node file that would not parse.
        path: PathBuf,
        /// The schema the corpus declares, which is this build's own.
        version: u32,
        /// Why it would not parse, as the current model complained.
        source: Box<Error>,
    },

    /// The edge would make a node its own ancestor. Genealogy is a DAG.
    #[error("that edge would make `{from}` its own ancestor")]
    Cycle {
        /// The node the edge starts at.
        from: String,
        /// The node the edge points at.
        to: String,
    },

    /// A node cannot link to itself.
    #[error("a node cannot link to itself")]
    SelfLoop,

    /// That exact edge is already recorded.
    #[error("that edge already exists")]
    DuplicateEdge,

    /// The status must name what would falsify the idea, and none is named.
    #[error("`{0}` needs a kill condition first")]
    NeedsKill(Status),

    /// An empty kill condition is not a kill condition.
    #[error("a kill condition cannot be empty; that is the whole point of it")]
    EmptyKill,

    /// A falsifier is content, not a field that `sharpen` may silently replace.
    #[error("kill condition is already `{0}`; it was not replaced")]
    KillAlreadySet(String),

    /// Refuting asserts the kill condition fired, and that has to be written.
    #[error("refuted needs a reason: say how the kill condition fired")]
    RefutedNeedsWhy,

    /// A ruled-out idea cannot quietly return to active work.
    #[error("a refuted node cannot simply reopen")]
    RefutedCannotReopen,

    /// A node that names a kill condition cannot go back to `seed`. The kill
    /// is content, so the move would keep it, and a seed carrying one is a
    /// state no verb otherwise produces. Reopening such a node is a move to
    /// `hypothesis`.
    #[error("a node with a kill condition cannot go back to seed")]
    SeedWithKill,

    /// Two nodes claim the same id.
    #[error("duplicate node id `{0}`")]
    DuplicateId(String),

    /// A node file is already there.
    #[error("node `{0}` already exists")]
    NodeExists(String),

    /// A local reference points at a path that is not there.
    #[error("`{uri}` does not resolve from {}; local references are relative to nodes/", .from.display())]
    UnresolvedUri {
        /// The reference as written.
        uri: String,
        /// The directory it was resolved against.
        from: PathBuf,
    },

    /// A local reference names a place on one machine's filesystem — an
    /// absolute path or a `file:` URI — rather than a path relative to
    /// `nodes/`. Refused whether or not it exists here: the corpus is
    /// synced between machines, and on every other one it names nothing.
    #[error("`{0}` is an absolute local path; local references are relative to nodes/")]
    AbsoluteUri(String),

    /// A status move the lifecycle does not allow, judged from the pair of
    /// statuses alone. The refusals that exist today name themselves
    /// ([`Error::NeedsKill`], [`Error::RefutedCannotReopen`] and
    /// [`Error::SeedWithKill`]); this is what
    /// a guard added later reports, and what a consumer matches on to mean
    /// "that move is not allowed" without enumerating the specific rules.
    #[error("`{from}` cannot become `{to}`")]
    InvalidTransition {
        /// Where the node is now.
        from: Status,
        /// Where it was asked to go.
        to: Status,
    },

    /// A named parent is not in the corpus.
    #[error("parent `{0}` does not exist")]
    MissingParent(String),

    /// A title with nothing in it that survives slugification.
    #[error("title `{0}` does not reduce to a usable id")]
    UnusableTitle(String),

    /// A new reference's `kind` is not in [`crate::check::REFERENCE_KINDS`].
    /// Only new writes are held to the vocabulary; `check` reports an
    /// unexpected kind already on disk as a warning instead.
    #[error(
        "`{0}` is not an accepted reference kind; accepted kinds: {accepted}",
        accepted = crate::check::REFERENCE_KINDS.join(", ")
    )]
    UnknownReferenceKind(String),

    /// An `observatory` reference's `uri` is not a bare record id.
    #[error(
        "`{0}` is not an Observatory record id: one of Q, H, T or R followed by digits, such as `Q002`"
    )]
    InvalidObservatoryId(String),

    /// A user-supplied `--id` does not follow the slug rules: lowercase
    /// words joined by single dashes, no leading, trailing, or doubled
    /// dash, 60 characters or fewer.
    #[error(
        "`{0}` is not a valid id: ids are lowercase words joined by single dashes, 60 characters or fewer"
    )]
    InvalidId(String),

    /// An id that cannot name a file under `nodes/`: it holds a path
    /// separator, a `.`/`..` or root component, a control character, or
    /// surrounding whitespace. Refused before any read or write derives a
    /// path from it, whether it came from a caller or out of a node file
    /// somebody edited by hand.
    #[error(
        "`{0}` cannot be a node id: an id names one file under nodes/, so it cannot hold a path separator, `.`, `..`, or a control character"
    )]
    UnsafeId(String),

    /// A node file's name and the id it stores disagree. Nothing is read or
    /// written: a node's id decides where a write lands, so a file that
    /// claims to be another node would make the next verb overwrite that
    /// other node.
    #[error("{} stores the id `{id}`, which is not the node its file name names; nothing was read or written", .path.display())]
    IdMismatch {
        /// The file that was read.
        path: PathBuf,
        /// The id it stores.
        id: String,
    },

    /// Another writer holds the corpus lock and did not release it within
    /// the bounded wait. Nothing was written: the refusal comes before the
    /// op reads anything, so there is no half-applied change to undo.
    #[error("another nebula writer is holding {}; nothing was written", .root.display())]
    Locked {
        /// The corpus root whose `.lock` is held.
        root: PathBuf,
    },

    /// The commit after a write was refused because something outside the
    /// corpus was already staged, and a `neb` commit must be exactly the
    /// corpus. The write itself is in place: git never rolls back a write.
    #[error(
        "{} has staged changes outside the corpus ({}); the write is in place and nothing was committed",
        .root.display(),
        .paths.join(", ")
    )]
    StagedElsewhere {
        /// The corpus root.
        root: PathBuf,
        /// What is staged, relative to the repository's top level.
        paths: Vec<String>,
    },

    /// `commit` is on, but the repository containing the corpus ignores it,
    /// so there is nothing git would ever record.
    #[error("{} is ignored by the git repository that contains it; nothing can be committed", .0.display())]
    CorpusIgnored(PathBuf),

    /// A git command did not succeed. The write it was meant to record is in
    /// place.
    #[error("git {context} failed in {}: {stderr}", .root.display())]
    Git {
        /// The corpus root the command ran in.
        root: PathBuf,
        /// Which command.
        context: String,
        /// What git said.
        stderr: String,
    },

    /// Anything else about the corpus itself: configuration, a malformed
    /// file, a value that is not one of the ones there are.
    #[error("{0}")]
    Corpus(String),

    /// Reading or writing failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Reading or writing a known corpus path failed.
    #[error("{action} {}: {source}", .path.display())]
    IoAt {
        /// What operation was attempted.
        action: &'static str,
        /// The path the operation targeted.
        path: PathBuf,
        /// The operating system's reason for refusing or failing it.
        source: std::io::Error,
    },

    /// A YAML document would not parse or render. The context says which.
    #[error("{context}: {source}")]
    Yaml {
        /// What was being read or written.
        context: String,
        /// The parser's own complaint, which names the field and the line.
        source: serde_yaml_ng::Error,
    },

    /// A JSON document would not parse or render.
    #[error("{context}: {source}")]
    Json {
        /// What was being read or written.
        context: String,
        /// The parser's own complaint.
        source: serde_json::Error,
    },
}

impl Error {
    /// The variant's name, as a stable string for a consumer that is not
    /// Rust: `neb --json` reports it as the refusal's `kind`, so a script
    /// matches `NoSuchNode` rather than parsing the message.
    ///
    /// Spelled out rather than derived, and exhaustive on purpose. A new
    /// variant does not compile until it names itself here, and renaming a
    /// variant does not quietly rename a kind that scripts already match.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NoSuchNode(_) => "NoSuchNode",
            Self::NoSuchInboxEntry(_) => "NoSuchInboxEntry",
            Self::NoCorpus(_) => "NoCorpus",
            Self::NotGitWorkTree(_) => "NotGitWorkTree",
            Self::NoNodeAtRevision { .. } => "NoNodeAtRevision",
            Self::EmptyRoot => "EmptyRoot",
            Self::RootConfigConflict { .. } => "RootConfigConflict",
            Self::SchemaMismatch { .. } => "SchemaMismatch",
            Self::CurrentSchemaUnreadable { .. } => "CurrentSchemaUnreadable",
            Self::Cycle { .. } => "Cycle",
            Self::SelfLoop => "SelfLoop",
            Self::DuplicateEdge => "DuplicateEdge",
            Self::NeedsKill(_) => "NeedsKill",
            Self::EmptyKill => "EmptyKill",
            Self::KillAlreadySet(_) => "KillAlreadySet",
            Self::RefutedNeedsWhy => "RefutedNeedsWhy",
            Self::RefutedCannotReopen => "RefutedCannotReopen",
            Self::SeedWithKill => "SeedWithKill",
            Self::DuplicateId(_) => "DuplicateId",
            Self::NodeExists(_) => "NodeExists",
            Self::UnresolvedUri { .. } => "UnresolvedUri",
            Self::AbsoluteUri(_) => "AbsoluteUri",
            Self::InvalidTransition { .. } => "InvalidTransition",
            Self::MissingParent(_) => "MissingParent",
            Self::UnusableTitle(_) => "UnusableTitle",
            Self::UnknownReferenceKind(_) => "UnknownReferenceKind",
            Self::InvalidObservatoryId(_) => "InvalidObservatoryId",
            Self::InvalidId(_) => "InvalidId",
            Self::UnsafeId(_) => "UnsafeId",
            Self::IdMismatch { .. } => "IdMismatch",
            Self::Locked { .. } => "Locked",
            Self::StagedElsewhere { .. } => "StagedElsewhere",
            Self::CorpusIgnored(_) => "CorpusIgnored",
            Self::Git { .. } => "Git",
            Self::Corpus(_) => "Corpus",
            Self::Io(_) => "Io",
            Self::IoAt { .. } => "IoAt",
            Self::Yaml { .. } => "Yaml",
            Self::Json { .. } => "Json",
        }
    }

    /// A YAML failure, labelled with what was being read.
    pub(crate) fn yaml(context: impl Into<String>, source: serde_yaml_ng::Error) -> Self {
        Self::Yaml {
            context: context.into(),
            source,
        }
    }

    /// Anything about the corpus that has no variant of its own.
    pub(crate) fn corpus(message: impl Into<String>) -> Self {
        Self::Corpus(message.into())
    }

    /// An I/O failure labelled with the path the user can inspect or repair.
    pub(crate) fn io_at(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::IoAt {
            action,
            path: path.into(),
            source,
        }
    }
}
