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

    /// Refuting asserts the kill condition fired, and that has to be written.
    #[error("refuted needs a reason: say how the kill condition fired")]
    RefutedNeedsWhy,

    /// A ruled-out idea cannot quietly return to active work.
    #[error("a refuted node cannot simply reopen")]
    RefutedCannotReopen,

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

    /// A status move the lifecycle does not allow, judged from the pair of
    /// statuses alone. The two refusals that exist today name themselves
    /// ([`Error::NeedsKill`] and [`Error::RefutedCannotReopen`]); this is what
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
}
