//! `neb` — capture a half-formed thought in five seconds, and trace where any
//! idea came from years later.
//!
//! This crate is the terminal face of `nebula_core`, and nothing more:
//!
//! - `cli`     the clap tree and dispatch; the only module that knows clap
//! - `render`  terminal output for the values core returns
//!
//! Every verb is one core call. Anything that reads or writes the corpus
//! lives in `nebula_core`, so the desktop app and the agent skill see the
//! same behaviour this binary does.

mod cli;
mod render;

fn main() -> std::process::ExitCode {
    cli::main()
}
