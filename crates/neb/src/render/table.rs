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
pub(crate) enum Target {
    /// A terminal: the aligned table, coloured when `colour` says so.
    Terminal { colour: bool },
    /// A pipe or a file: tab-separated lines, never coloured.
    Piped,
}

impl Target {
    /// stdout's target, as the output layer's gate decides it.
    pub(crate) fn stdout() -> Self {
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
pub(crate) enum Align {
    Left,
    /// For numbers, so their digits line up under the header.
    Right,
}

/// A column: its header, and how its cells are aligned.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Column {
    header: &'static str,
    align: Align,
}

impl Column {
    /// A column of text.
    pub(crate) const fn left(header: &'static str) -> Self {
        Self {
            header,
            align: Align::Left,
        }
    }

    /// A column of numbers.
    pub(crate) const fn right(header: &'static str) -> Self {
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
pub(crate) struct Cell {
    text: String,
    ink: Ink,
}

impl Cell {
    /// `text` in `role`'s colour.
    ///
    /// A control character becomes a space, so a record stays one line and
    /// its fields stay put, and `text` can carry no escape of its own. An
    /// empty or blank value is absent, and shows as `-`.
    pub(crate) fn new(role: Role, text: impl Into<String>) -> Self {
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
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self::new(Role::Neutral, text)
    }

    /// Secondary text.
    pub(crate) fn muted(text: impl Into<String>) -> Self {
        Self::new(Role::Muted, text)
    }

    /// An identifier, in bold.
    pub(crate) fn bold(text: impl Into<String>) -> Self {
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
pub(crate) struct Table<const N: usize> {
    columns: [Column; N],
    rows: Vec<[Cell; N]>,
}

impl<const N: usize> Table<N> {
    pub(crate) fn new(columns: [Column; N]) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    /// Add a record.
    pub(crate) fn row(&mut self, cells: [Cell; N]) {
        self.rows.push(cells);
    }

    /// The table for `target`, one line per record. With no records it is
    /// nothing at all, header included: an empty result leaves stdout empty,
    /// and its notice says so on stderr (§R16).
    pub(crate) fn render(&self, target: Target) -> String {
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
