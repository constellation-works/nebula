//! Unit tests for `render_table`.

use crate::output::{self, Role, Stream};
use std::fmt::Write as _;

use crate::render::table::*;

const TERMINAL: Target = Target::Terminal { colour: false };

/// Where each column of `line` starts: after a gutter of two or more
/// spaces, in a line whose cells hold no spaces of their own.
fn starts(line: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let bytes = line.as_bytes();
    for i in 2..bytes.len() {
        if bytes[i] != b' ' && bytes[i - 1] == b' ' && bytes[i - 2] == b' ' {
            starts.push(i);
        }
    }
    starts
}

fn fruit() -> Table<3> {
    let mut t = Table::new([
        Column::left("NAME"),
        Column::left("COLOUR"),
        Column::right("COUNT"),
    ]);
    t.row([Cell::bold("fig"), Cell::plain("purple"), Cell::muted("5")]);
    t.row([
        Cell::bold("clementine"),
        Cell::plain("orange"),
        Cell::muted("150"),
    ]);
    t.row([Cell::bold("kiwi"), Cell::plain("green"), Cell::muted("12")]);
    t
}

#[test]
fn terminal_table_has_header_and_two_space_gutters() {
    let drawn = fruit().render(TERMINAL);
    let lines: Vec<&str> = drawn.lines().collect();
    assert_eq!(lines.len(), 4, "a header and three records: {drawn:?}");
    assert_eq!(lines[0], "NAME        COLOUR  COUNT");
    assert_eq!(lines[1], "fig         purple      5");
    assert_eq!(lines[2], "clementine  orange    150");
    assert_eq!(lines[3], "kiwi        green      12");
    // Every column starts in the same place on every line, and the
    // widest cell sets it: `clementine`, then the two-space gutter.
    for line in &lines {
        assert_eq!(starts(line)[..2], [0, "clementine".len() + 2], "{line:?}");
        assert!(!line.contains('\t') && !line.ends_with(' '), "{line:?}");
    }
}

#[test]
fn numbers_are_right_aligned() {
    let drawn = fruit().render(TERMINAL);
    let lines: Vec<&str> = drawn.lines().collect();
    assert!(lines[0].ends_with("COUNT"), "{:?}", lines[0]);
    let end = lines[0].len();
    for (line, count) in lines[1..].iter().zip(["5", "150", "12"]) {
        assert_eq!(line.len(), end, "{line:?}");
        assert!(line.ends_with(&format!(" {count}")), "{line:?}");
    }
}

#[test]
fn absent_cell_renders_as_dash() {
    let mut t = Table::new([Column::left("ID"), Column::left("TAGS")]);
    t.row([Cell::bold("untagged"), Cell::muted("")]);
    t.row([Cell::bold("blank"), Cell::muted("  ")]);
    t.row([Cell::bold("tagged"), Cell::muted("a,b")]);
    assert_eq!(
        t.render(TERMINAL),
        "ID        TAGS\nuntagged  -\nblank     -\ntagged    a,b\n"
    );
    assert_eq!(
        t.render(Target::Piped),
        "untagged\t-\nblank\t-\ntagged\ta,b\n"
    );
}

/// STD-01 §R15: with no width known, nothing is cut.
#[test]
fn no_width_means_no_truncation() {
    let title = "long ".repeat(100);
    let title = title.trim_end();
    assert!(title.len() >= 499);
    let mut t = Table::new([Column::left("ID"), Column::left("TITLE")]);
    t.row([Cell::bold("a"), Cell::plain(title)]);
    for target in [TERMINAL, Target::Terminal { colour: true }, Target::Piped] {
        let drawn = t.render(target);
        assert!(drawn.contains(title), "{target:?}: {drawn:?}");
        assert!(!drawn.contains('…'), "{target:?}: {drawn:?}");
    }
}

#[test]
fn piped_table_is_tab_separated_without_header() {
    let drawn = fruit().render(Target::Piped);
    assert_eq!(
        drawn,
        "fig\tpurple\t5\nclementine\torange\t150\nkiwi\tgreen\t12\n"
    );
    let lines: Vec<&str> = drawn.lines().collect();
    assert_eq!(lines.len(), 3, "no header: {drawn:?}");
    for line in lines {
        assert_eq!(line.split('\t').count(), 3, "{line:?}");
    }
    assert!(!drawn.contains("\x1b["), "{drawn:?}");
    assert!(!drawn.contains("COUNT"), "{drawn:?}");
}

#[test]
fn colour_paints_cells_but_not_the_alignment() {
    let drawn = fruit().render(Target::Terminal { colour: true });
    let first = drawn.lines().nth(1).unwrap();
    assert_eq!(
        first,
        "\x1b[1mfig\x1b[0m         purple      \x1b[2m5\x1b[0m"
    );
    assert!(!drawn.lines().next().unwrap().contains('\x1b'), "{drawn:?}");
}

/// A tab, a newline or an escape in a value would break the record into
/// more fields or lines, or reach the terminal as a control sequence.
#[test]
fn control_characters_in_a_cell_become_spaces() {
    let mut t = Table::new([Column::left("ID"), Column::left("TEXT")]);
    t.row([Cell::bold("x"), Cell::plain("one\ttwo\nthree\x1b[31m")]);
    assert_eq!(t.render(Target::Piped), "x\tone two three [31m\n");
    assert_eq!(t.render(TERMINAL).lines().count(), 2);
}

/// An empty result is an empty stdout, in both forms (§R16).
#[test]
fn no_records_is_no_output() {
    let t = Table::new([Column::left("ID")]);
    assert_eq!(t.render(TERMINAL), "");
    assert_eq!(t.render(Target::Piped), "");
}
