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
