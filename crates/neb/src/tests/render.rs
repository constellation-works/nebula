//! Unit tests for `render`.

use crate::output::{self, Role, Stream};
use crate::render::table::{Cell, Column, Table};
use nebula_core::{EdgeType, Error, GraphExport, HistoryEntry, Node, Status};
use std::collections::HashSet;
use std::fmt::Write as _;

use crate::render::*;

/// The text with every ANSI escape removed, so a test reads what a
/// terminal shows whether or not colour happens to be on.
pub(crate) fn visible(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(|c| *c == 'm');
        } else {
            out.push(c);
        }
    }
    out
}

/// The badge is a column: every status comes out the same width, so the
/// id after it starts at the same place on every line.
#[test]
fn status_badges_are_padded_to_one_width() {
    for s in [
        Status::Seed,
        Status::Hypothesis,
        Status::Refuted,
        Status::Abandoned,
    ] {
        let badge = visible(&status_badge(s));
        assert_eq!(badge.chars().count(), 10, "{badge:?}");
        assert!(badge.starts_with(&s.to_string()), "{badge:?}");
    }
    assert_eq!(visible(&status_badge(Status::Seed)), "seed      ");
}

/// A count is for a person, and `--json` leaves it out; an empty or cut
/// listing is said in every mode.
#[test]
fn list_notices_say_in_every_mode_only_what_a_script_needs() {
    let said = |n: &Notice, json| n.line(json).map(|l| visible(&l));
    let whole = list_notice(3, 3, 4);
    assert_eq!(said(&whole, false).as_deref(), Some("3 of 4 nodes"));
    assert_eq!(said(&whole, true), None);
    for (notice, text) in [
        (list_notice(0, 0, 4), "no nodes match"),
        (
            list_notice(1, 4, 4),
            "1 of 4 nodes shown; raise --limit for more",
        ),
        (
            list_notice(1, 3, 4),
            "1 of 3 matching nodes shown, of 4 in all; raise --limit for more",
        ),
    ] {
        assert_eq!(said(&notice, false).as_deref(), Some(text));
        assert_eq!(said(&notice, true).as_deref(), Some(text));
    }
}

#[test]
fn counts_agree_with_their_nouns() {
    assert_eq!(count(0_usize, "warning"), "0 warnings");
    assert_eq!(count(1_usize, "warning"), "1 warning");
    assert_eq!(count(2_usize, "warning"), "2 warnings");
    assert_eq!(count(1_i64, "day"), "1 day");
}
