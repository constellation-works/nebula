//! `neb` — capture a half-formed thought in five seconds, and trace where any
//! idea came from years later.
//!
//! Module map, outermost first:
//!
//! - `cli`      the clap tree and dispatch; the only module that knows clap
//! - `commands` one function per verb, grouped by what they act on
//! - `check`    the invariant checker
//! - `render`   terminal output
//! - `corpus`   the data layer: schema, storage, configuration
//!
//! Dependencies point downward only. `corpus` knows nothing above it, which is
//! what would let it become a library crate if a second consumer appeared.

mod check;
mod cli;
mod commands;
mod corpus;
mod render;

fn main() -> std::process::ExitCode {
    cli::main()
}
