//! Unit tests for `session`.

use crate::error::IpcError;
use nebula_core::verb::{self, CommitPolicy, WriteOptions};
use nebula_core::{
    CommitOutcome, Committed, Corpus, Created, GraphExport, InboxEntry, Locations, NodeView,
    Result, graph, ops,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::session::CommitReport;

/// Git failing under commit-on is a refusal, never one of the outcomes
/// that mean nothing was asked of git.
#[test]
fn a_failed_git_is_refused_not_skipped() {
    let failed = nebula_core::Error::Git {
        root: PathBuf::from("/corpus"),
        context: "commit".into(),
        stderr: "fatal: unable to write new index file".into(),
    };
    let report = CommitReport::of(Err(failed));
    assert!(
        matches!(&report, CommitReport::Refused { error } if error.code == "git"),
        "{report:?}"
    );
}

use crate::session::matches_graph_query;

#[test]
fn matches_every_requested_field_case_insensitively() {
    for query in ["n42", "IDEA", "evidence", "REFUTED"] {
        assert!(matches_graph_query(
            "n42",
            "An idea",
            "new evidence",
            "refuted",
            &query.to_lowercase()
        ));
    }
    assert!(!matches_graph_query(
        "n42",
        "An idea",
        "new evidence",
        "refuted",
        "absent"
    ));
}
