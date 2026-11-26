#![deny(clippy::print_stderr, clippy::print_stdout)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! `neb` — capture a half-formed thought in five seconds, and trace where any
//! idea came from years later.
//!
//! This crate is the terminal face of `nebula_core`, and nothing more:
//!
//! - `cli`     the clap tree and dispatch; the only module that knows clap
//! - `env`     the central environment-variable registry and usage-table generator
//! - `render`  terminal output for the values core returns
//! - `output`  the one owner of stdout and stderr; nothing else writes to them
//!
//! Every verb is one core operation: a write is one call into
//! `nebula_core::verb`, a read is `Corpus::open` plus one query. Anything
//! that reads or writes the corpus lives in `nebula_core`, so the desktop app
//! and the agent skill see the same behaviour this binary does.

mod cli;
pub mod env;
mod output;
mod render;

/// Run the `neb` CLI and return its process exit code.
pub fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(test)]
mod tests;
