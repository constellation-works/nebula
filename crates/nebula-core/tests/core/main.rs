#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Library-level tests: the graph queries and the point-of-action guards, as
//! the desktop app and the agent skill see them.
//!
//! `crates/neb/tests/cli/` covers the same rules through the binary and
//! asserts on messages and exit codes. These assert on the typed values, which
//! is what a consumer that is not a terminal matches on.
//!
//! One test binary, split by area: [`harness`] holds `corpus()`, `seed()` and
//! the other fixtures every area shares, and each other module is one area's
//! tests with the helpers only it uses.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant and corrupt corpus files directly; only the code under test goes \
              through nebula_core's durable write helper"
)]

// This process runs with a temporary `HOME` and git environment, set before
// any test thread starts, and every child comes from its builder.
#[path = "../support/mod.rs"]
mod support;

mod authorship;
mod capture;
mod cite;
mod commit;
mod config;
mod graph;
mod harness;
mod ids;
mod locations;
mod lock;
mod near;
mod open;
mod ops;
mod origin;
mod promote;
mod supervised_git;
mod tags;
mod triage;
