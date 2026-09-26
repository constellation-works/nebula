//! List-shaped output: one kind of table, drawn by one renderer.
//!
//! On a terminal a table is borderless (STD-01 §R14): one header row, then
//! one line per record, columns two spaces apart, numbers right-aligned under
//! their header, and `-` in an absent cell. Anything else gets one
//! tab-separated line per record, with no header and no colour (§R9), and
//! every column present, so `cut -f` never shifts.
//!
//! Nothing is cut to fit. `neb` knows no terminal width, and with none a
//! table never truncates (§R15): a column is as wide as its longest cell, and
//! a long cell makes a long line.
//!
//! Which form, and whether it is coloured, is the caller's [`Target`], which
//! [`Target::stdout`] takes from the output layer's one gate. The renderer
//! never asks the environment, so a test hands it either form.
//!
//! `list`, `inbox`, `near`, `tag list`, `review --short` and `log` are
//! tables. `trace` (a tree on a terminal, its own lines otherwise), `impact`,
//! `check`, `show` and `review`'s markdown are not, and neither are the
//! `near:` block under `capture` and `promote` or `triage`'s candidates.

use crate::output::{self, Role, Stream};
use std::fmt::Write as _;

/// Where a table is going, as far as its form is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A terminal: the aligned table, coloured when `colour` says so.
    Terminal { colour: bool },
    /// A pipe or a file: tab-separated lines, never coloured.
    Piped,
}

impl Target {
    /// stdout's target, as the output layer's gate decides it.
    pub fn stdout() -> Self {
        if output::stdout_on_terminal() {
            Self::Terminal {
                colour: output::colours(Stream::Stdout),
            }
        } else {
            Self::Piped
        }
    }
}

/// Which side of its column a cell sits against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    /// For numbers, so their digits line up under the header.
    Right,
}

/// A column: its header, and how its cells are aligned.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    header: &'static str,
    align: Align,
}

impl Column {
    /// A column of text.
    pub const fn left(header: &'static str) -> Self {
        Self {
            header,
            align: Align::Left,
        }
    }

    /// A column of numbers.
    pub const fn right(header: &'static str) -> Self {
        Self {
            header,
            align: Align::Right,
        }
    }
}

/// How a cell is shown when colour is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Role(Role),
    /// Emphasis for an identifier, which is no colour and so no role.
    Bold,
}

/// One value in a row.
#[derive(Debug, Clone)]
pub struct Cell {
    text: String,
    ink: Ink,
}

impl Cell {
    /// `text` in `role`'s colour.
    ///
    /// A control character becomes a space, so a record stays one line and
    /// its fields stay put, and `text` can carry no escape of its own. An
    /// empty or blank value is absent, and shows as `-`.
    pub fn new(role: Role, text: impl Into<String>) -> Self {
        let text: String = text
            .into()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let text = if text.trim().is_empty() {
            "-".to_string()
        } else {
            text
        };
        Self {
            text,
            ink: Ink::Role(role),
        }
    }

    /// Plain text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(Role::Neutral, text)
    }

    /// Secondary text.
    pub fn muted(text: impl Into<String>) -> Self {
        Self::new(Role::Muted, text)
    }

    /// An identifier, in bold.
    pub fn bold(text: impl Into<String>) -> Self {
        Self {
            ink: Ink::Bold,
            ..Self::plain(text)
        }
    }

    /// Characters a terminal shows for the cell. Each is taken as one column
    /// wide, which a wide script is not; such a row is still whole, only
    /// out of line.
    fn width(&self) -> usize {
        self.text.chars().count()
    }

    fn painted(&self, colour: bool) -> String {
        match self.ink {
            Ink::Role(role) => output::paint_if(colour, role, &self.text),
            Ink::Bold => output::bold_if(colour, &self.text),
        }
    }
}

/// A table of `N` columns, built a row at a time and rendered once.
#[derive(Debug, Clone)]
pub struct Table<const N: usize> {
    columns: [Column; N],
    rows: Vec<[Cell; N]>,
}

impl<const N: usize> Table<N> {
    pub fn new(columns: [Column; N]) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    /// Add a record.
    pub fn row(&mut self, cells: [Cell; N]) {
        self.rows.push(cells);
    }

    /// The table for `target`, one line per record. With no records it is
    /// nothing at all, header included: an empty result leaves stdout empty,
    /// and its notice says so on stderr (§R16).
    pub fn render(&self, target: Target) -> String {
        let mut out = String::new();
        if self.rows.is_empty() {
            return out;
        }
        match target {
            Target::Piped => {
                for row in &self.rows {
                    let fields: Vec<&str> = row.iter().map(|c| c.text.as_str()).collect();
                    let _ = writeln!(out, "{}", fields.join("\t"));
                }
            }
            Target::Terminal { colour } => {
                let widths: [usize; N] = std::array::from_fn(|i| {
                    self.rows
                        .iter()
                        .map(|row| row[i].width())
                        .fold(self.columns[i].header.chars().count(), usize::max)
                });
                let header = self
                    .columns
                    .iter()
                    .map(|c| (c.header.to_string(), c.header.chars().count()));
                self.line(&mut out, &widths, header);
                for row in &self.rows {
                    let cells = row.iter().map(|c| (c.painted(colour), c.width()));
                    self.line(&mut out, &widths, cells);
                }
            }
        }
        out
    }

    /// One aligned line of `(shown, width)` cells, two spaces between
    /// columns and nothing after the last.
    fn line(
        &self,
        out: &mut String,
        widths: &[usize; N],
        cells: impl Iterator<Item = (String, usize)>,
    ) {
        for (i, (shown, width)) in cells.enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            let pad = " ".repeat(widths[i] - width);
            match self.columns[i].align {
                Align::Right => {
                    out.push_str(&pad);
                    out.push_str(&shown);
                }
                Align::Left if i + 1 == N => out.push_str(&shown),
                Align::Left => {
                    out.push_str(&shown);
                    out.push_str(&pad);
                }
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
