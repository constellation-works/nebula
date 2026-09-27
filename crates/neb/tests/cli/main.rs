#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests over a real corpus in a temporary directory.
//!
//! These drive the built binary rather than library functions, because the
//! things most likely to break are the transition guards and the exit codes,
//! and both live at the edge.
//!
//! Paths are used exactly as the temporary directory reports them and are never
//! canonicalized or compared against a resolved form. On macOS the temporary
//! directory sits under a symlink, and a checker that resolved paths would pass
//! on Linux and fail here.
//!
//! One test binary, split by area: [`harness`] holds the `Corpus` fixture and
//! the run helpers every area shares, and each other module is one area's
//! tests with the helpers only it uses.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant and corrupt corpus files directly; only the binary under test goes \
              through nebula_core's durable write helper"
)]

// The isolation every test here runs under, this process's and each child's.
#[path = "../../../nebula-core/tests/support/mod.rs"]
mod support;

mod authorship;
mod capture;
mod check;
mod cite;
mod commit;
mod containment;
mod discipline;
mod edit;
mod graph;
mod handoff;
mod harness;
mod help;
mod history;
mod ids;
mod init;
mod json;
mod listing;
mod lock;
mod migrate;
mod migrate_safety;
mod near;
mod note;
mod observatory;
mod origin;
mod promote;
mod refusals;
mod review;
mod root;
mod set_root;
mod stdout;
mod supervised_git;
mod tags;
mod trace;
mod triage;
mod unreadable;
