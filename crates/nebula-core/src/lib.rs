//! nebula's corpus, as a library.
//!
//! Three consumers sit on one corpus: a CLI a human types at, an agent that
//! runs the CLI, and a desktop app that draws the graph. This crate is what
//! they agree on. See `docs/design/v0.2/2_architecture.md`.
//!
//! The rule: **core is pure with respect to the outside world's presentation.**
//! It reads and writes corpus files and nothing else. No terminal, no colour,
//! no clap, no `println!`, no process exit codes, and no `anyhow` — every
//! public function returns [`Result<T, Error>`](Error) and every value it
//! returns is `serde::Serialize`, so the CLI's `--json` output and the
//! desktop's IPC payload are the same value and there is no second schema to
//! drift. (The CLI serialises it with every absent field stated as `null`,
//! where these types leave out what the corpus files store by omission.)
//!
//! One thing reaches past the files: git. Every git child goes through one
//! supervised runner (`git`), and while one is live the runner holds
//! SIGINT, SIGTERM and SIGHUP so that stopping nebula stops git first; the
//! signal is delivered again, with its old disposition, once git is gone.
//!
//! Module map:
//!
//! - [`model`]  the file format: [`Node`], [`Status`], [`Edge`], [`Reference`], [`Note`]
//! - [`store`]  [`Corpus`]: where it lives, loading, saving, the inbox
//! - [`fs`]     the one durable write path: fsync, rename, owner-only modes
//! - `lock`    [`CorpusLock`]: the advisory `.lock` every write holds, and reads never do
//! - `git`     the one way git is run: own process group, deadline, bounded output
//! - [`graph`]  [`Graph`] and the pure queries over it, `near` included
//! - [`ops`]    the mutations, each enforcing its point-of-action invariants
//! - [`check`]  the invariant checker, and where an `observatory` record resolves
//! - [`triage`] [`Triage`]: the inbox one entry at a time, through the ops above
//! - [`migrate`] v1 → v2, with its own lenient v1 model kept private
//!
//! The public API is exactly what this file names. A consumer that needs more
//! is a signal to add an API, not to reach in.
//!
//! ```no_run
//! use nebula_core::{Corpus, Graph, graph};
//!
//! let corpus = Corpus::open(None)?;
//! let docs = corpus.load_all()?;
//! let g = Graph::build(&docs)?;
//! for node in graph::export(&g)?.nodes {
//!     println!("{} {}", node.id, node.title);
//! }
//! # Ok::<(), nebula_core::Error>(())
//! ```

mod config;
mod error;
mod git;
mod lock;

pub mod check;
pub mod fs;
pub mod graph;
pub mod migrate;
pub mod model;
pub mod ops;
pub mod store;
pub mod triage;

pub use check::{Finding, OBSERVATORY, Report, Severity};
pub use config::{
    CommitSetting, OBSERVATORY_ROOT_ENV, ObservatoryRoot, ObservatorySource, SCHEMA_VERSION,
};
pub use error::{Error, Result};
#[cfg(debug_assertions)]
#[doc(hidden)]
pub use git::GitDeadlineOverride;
pub use git::{GIT_COMMIT_DEADLINE, GIT_DEADLINE, GIT_OUTPUT_CAP, GIT_TERMINATION_GRACE};
pub use graph::{
    Band, Direction, EdgeRecord, Graph, GraphExport, HYPOTHESIS_DAYS, INBOX_DAYS, Impact, Listing,
    NEAR_DEFAULT, Near, Neighbour, NodeSummary, NodeView, ObservatoryLink, OpenItem, OpenReport,
    ReviewItem, ReviewReport, ReviewRule, SEED_DAYS, TagCount, TagCounts, Touched, Trace, TraceHop,
    TraceNode, Via,
};
pub use lock::{CorpusLock, LOCK_FILE, LOCK_WAIT};
pub use migrate::{MigrationReport, NodeMigration};
pub use model::{Closed, Doc, Edge, EdgeType, HUMAN, Node, Note, Origin, Reference, Status};
pub use ops::{
    Captured, Citation, Cited, CloseTag, Created, HandedOff, Handoff, Initialized, NewNode,
    Promotion, StatusChange,
};
pub use store::{Committed, Corpus, HistoryEntry, Inbox, InboxEntry, Settlement};
pub use triage::Triage;

// Unit tests for the top-level modules, one file each (STD-02 §R19).
#[cfg(test)]
mod tests;

/// Writes `apps/desktop/src/types/*.ts` from the types above.
///
/// The desktop is the consumer most likely to drift silently, because nothing
/// runs its shapes against a corpus on CI. So the TypeScript is generated from
/// the Rust rather than hand-written, and this is the test that generates it:
/// `cargo test -p nebula-core --features ts`.
#[cfg(all(test, feature = "ts"))]
mod ts_export {
    use super::*;
    use ts_rs::{Config, TS};

    /// Every type that appears in a public return value or a `--json` payload.
    /// `export_all` follows each one's dependencies, so listing the roots is
    /// enough. The destination is `TS_RS_EXPORT_DIR`, set in `.cargo/config.toml`.
    #[test]
    fn writes_the_typescript_bindings() {
        let cfg = Config::from_env();
        Doc::export_all(&cfg).unwrap();
        Node::export_all(&cfg).unwrap();
        Inbox::export_all(&cfg).unwrap();
        Trace::export_all(&cfg).unwrap();
        Impact::export_all(&cfg).unwrap();
        OpenReport::export_all(&cfg).unwrap();
        ReviewReport::export_all(&cfg).unwrap();
        GraphExport::export_all(&cfg).unwrap();
        NodeView::export_all(&cfg).unwrap();
        Listing::export_all(&cfg).unwrap();
        TagCounts::export_all(&cfg).unwrap();
        Report::export_all(&cfg).unwrap();
        MigrationReport::export_all(&cfg).unwrap();
        Created::export_all(&cfg).unwrap();
        Captured::export_all(&cfg).unwrap();
        Near::export_all(&cfg).unwrap();
        Cited::export_all(&cfg).unwrap();
        CloseTag::export_all(&cfg).unwrap();
        StatusChange::export_all(&cfg).unwrap();
        HandedOff::export_all(&cfg).unwrap();
        Initialized::export_all(&cfg).unwrap();
        Direction::export_all(&cfg).unwrap();
        ObservatoryRoot::export_all(&cfg).unwrap();
        CommitSetting::export_all(&cfg).unwrap();
        Committed::export_all(&cfg).unwrap();
        HistoryEntry::export_all(&cfg).unwrap();

        assert!(
            cfg.out_dir().join("Node.ts").exists(),
            "the bindings should land in {}",
            cfg.out_dir().display()
        );
    }
}
