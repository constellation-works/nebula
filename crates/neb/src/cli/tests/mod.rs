//! Unit tests for `cli`, one file per source file (STD-02 §R19);
//! `module_root` holds those for `cli/mod.rs` itself. Being siblings of the
//! modules they test, they reach what those expose to `cli`
//! (`pub(super)`, `pub(crate)`), never their private items.

#![allow(unused_imports)]

mod args;
mod help;
mod module_root;
mod triage;

use super::args::Cli;
use super::parse_from;

/// `neb <args>` through [`parse_from`], a refusal as clap renders it.
fn parse_cli(args: &[&str]) -> Result<Cli, String> {
    parse_from(std::iter::once("neb").chain(args.iter().copied()))
        .map_err(|e| e.render().to_string())
}
