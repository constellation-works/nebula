//! Terminal output.
//!
//! Colour is applied only when stdout is a terminal and `NO_COLOR` is unset, so
//! piping into a file or an agent yields clean text.

mod tree;

use crate::corpus::{Doc, Status};
use std::io::IsTerminal;
use std::sync::OnceLock;

pub use tree::Tree;

fn colour() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

/// Wrap text in an ANSI code, or return it untouched when colour is off.
pub fn paint(code: &str, s: &str) -> String {
    if colour() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Dim text, for anything secondary.
pub fn dim(s: &str) -> String {
    paint("2", s)
}

/// Bold text, for identifiers.
pub fn bold(s: &str) -> String {
    paint("1", s)
}

/// A status badge, coloured by whether the node still asks anything of you.
pub fn status_badge(s: Status) -> String {
    let code = match s {
        Status::Seed => "36",       // cyan, unformed
        Status::Hypothesis => "33", // yellow, live and owing a look
        Status::Refuted => "31",    // red, dead
        Status::Abandoned => "2",   // dim, settled
    };
    paint(code, &format!("{s:<10}"))
}

/// One node as a single line.
pub fn line(doc: &Doc) -> String {
    let n = &doc.node;
    let tags = if n.tags.is_empty() {
        String::new()
    } else {
        format!(" {}", dim(&format!("[{}]", n.tags.join(", "))))
    };
    format!(
        "{} {}{tags} {}",
        status_badge(n.status),
        bold(&n.id),
        dim(&n.title)
    )
}
