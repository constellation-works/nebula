//! `neb triage`: the entry on screen, the keys that decide it, what each
//! decision did, and the session's closing count.

use super::report::neighbour;
use super::{bold, dim};
use nebula_core::triage::{Step, Tally, Waiting};
use std::fmt::Write as _;

/// The entry being decided: where it is in the session, how long it has
/// waited, what it says, and its candidate parents numbered for the keys.
pub fn waiting(w: &Waiting) -> String {
    let mut out = String::new();
    let age = match w.days {
        Some(0) => " · today".to_string(),
        Some(1) => " · 1 day".to_string(),
        Some(days) => format!(" · {days} days"),
        None => String::new(),
    };
    let _ = writeln!(
        out,
        "\n{} {} {}",
        dim(&format!("[{}/{}]", w.position, w.total)),
        bold(&w.entry.id),
        dim(&format!("{}{age}", w.entry.at))
    );
    let _ = writeln!(out, "  {}", w.entry.text);
    if w.near.is_empty() {
        let _ = writeln!(
            out,
            "  {}",
            dim("nothing near: no node shares a word with this")
        );
    }
    for (i, n) in w.near.iter().enumerate() {
        let _ = writeln!(out, "  {} {}", bold(&(i + 1).to_string()), neighbour(n));
    }
    if let Some(title) = &w.title {
        let _ = writeln!(out, "  {} {title}", dim("title:"));
    }
    out
}

/// The keys, with the parent range cut to the candidates this entry has.
pub fn triage_keys(w: &Waiting) -> String {
    let under = match w.near.len() {
        0 => String::new(),
        1 => "1 promote under it · ".to_string(),
        n => format!("1-{n} promote under that node · "),
    };
    format!(
        "{}\n",
        dim(&format!(
            "p promote as a root · {under}t title · d drop · s skip · q quit"
        ))
    )
}

/// What one decision did. Promote and drop read as their single verbs do.
pub fn step(step: &Step) -> String {
    match step {
        Step::Promoted { entry, created } => format!(
            "promoted {} -> {} {}\n",
            entry.id,
            bold(&created.doc.node.id),
            dim(&created.path.display().to_string())
        ),
        Step::Dropped { entry } => format!("dropped {}\n", bold(&entry.id)),
        Step::Titled { title: Some(title) } => format!("{} {title}\n", dim("title:")),
        Step::Titled { title: None } => format!("{}\n", dim("title: the captured text")),
        Step::Skipped { entry } => format!("{}\n", dim(&format!("skipped {}", entry.id))),
        Step::Quit => String::new(),
    }
}

/// The session's closing line, or the empty inbox it found.
pub fn tally(t: &Tally, total: usize) -> String {
    if total == 0 {
        return format!("{}\n", dim("inbox is empty"));
    }
    let waiting = t.skipped + t.untouched;
    format!(
        "\n{}\n",
        dim(&format!(
            "promoted {}, dropped {}, skipped {}; {waiting} still waiting",
            t.promoted, t.dropped, t.skipped
        ))
    )
}
