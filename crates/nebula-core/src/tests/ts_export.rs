//! Writes `apps/desktop/src/types/*.ts` from the public types.
//!
//! The desktop is the consumer most likely to drift silently, because nothing
//! runs its shapes against a corpus on CI. So the TypeScript is generated from
//! the Rust rather than hand-written. `cargo test -p nebula-core --features ts`
//! runs this.

use crate::*;
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
