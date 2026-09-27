//! The one place that asks what the streams are attached to and whether to
//! colour them (STD-01 §R17), and the colour roles that say what a colour
//! means (§R18).
//!
//! The environment is read once, into [`Terminal`], the first time anything
//! asks. Nothing else in the crate reads `NO_COLOR`, `TERM` or
//! `CLICOLOR_FORCE`, or asks whether a stream is a terminal;
//! `scripts/check-terminal-guard.sh` fails CI if anything does.
//!
//! Colour is decided per stream: `neb list 2>errors.txt` on a terminal
//! colours the listing and leaves the file plain, and `neb list | less`
//! still colours an `error:` that reaches the terminal.
//!
//! There is no width here: `neb` never truncates or wraps to fit a terminal,
//! so it has no use for one (§R15).

use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::sync::OnceLock;

/// A stream that text is painted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

/// Which standard streams are terminals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Ttys {
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
}

/// What the process was started with, as far as output cares: which streams
/// are terminals, and what the environment says about colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Terminal {
    pub(crate) ttys: Ttys,
    /// `NO_COLOR` is set and not empty. An empty one asks for nothing.
    no_color: bool,
    /// `TERM=dumb`: a terminal that cannot show an escape.
    dumb: bool,
}

impl Terminal {
    /// Read from `var`, a lookup standing in for the environment.
    ///
    /// `CLICOLOR_FORCE` is not read. STD-01 ranks it below `NO_COLOR` and
    /// `TERM=dumb`, and lets it colour a terminal only, which is already
    /// coloured without it; so it can change nothing here, and a pipe is
    /// never coloured whatever it says.
    pub(crate) fn from_env(var: impl Fn(&str) -> Option<OsString>, ttys: Ttys) -> Self {
        Self {
            ttys,
            no_color: var("NO_COLOR").is_some_and(|v| !v.is_empty()),
            dumb: var("TERM").is_some_and(|v| v == "dumb"),
        }
    }

    /// The process's own environment and streams.
    fn read() -> Self {
        Self::from_env(
            |name| std::env::var_os(name),
            Ttys {
                stdin: io::stdin().is_terminal(),
                stdout: io::stdout().is_terminal(),
                stderr: io::stderr().is_terminal(),
            },
        )
    }

    /// Whether text written to `stream` is coloured: only on a terminal that
    /// can show it, and only when `NO_COLOR` does not say otherwise.
    pub(crate) fn colours(self, stream: Stream) -> bool {
        let tty = match stream {
            Stream::Stdout => self.ttys.stdout,
            Stream::Stderr => self.ttys.stderr,
        };
        tty && !self.dumb && !self.no_color
    }
}

/// The process's [`Terminal`], read the first time it is asked for.
fn terminal() -> &'static Terminal {
    static READ: OnceLock<Terminal> = OnceLock::new();
    READ.get_or_init(Terminal::read)
}

/// Whether text written to `stream` is coloured.
pub(crate) fn colours(stream: Stream) -> bool {
    terminal().colours(stream)
}

/// Whether stdout is a terminal, so a human is reading it as it is written.
pub(crate) fn stdout_on_terminal() -> bool {
    terminal().ttys.stdout
}

/// Whether stdin is a terminal, so a person is at the keys.
pub(crate) fn stdin_on_terminal() -> bool {
    terminal().ttys.stdin
}

/// What a colour means. Every coloured value maps to one of these, in
/// [`Role::of`], and each role is one basic-16 SGR code in [`Role::sgr`].
///
/// A colour never carries meaning alone: every coloured value is also
/// spelled out, so with the escapes stripped nothing is lost but emphasis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Settled well.
    Ok,
    /// Live, and owing a look.
    Warn,
    /// Dead, or broken.
    Error,
    /// Live, and asking for nothing yet.
    Active,
    /// Secondary: settled, weak, or said in passing.
    Muted,
    /// Plain text.
    Neutral,
}

/// Domain values by kind and canonical token, and the role each is shown
/// in. A value missing from here is [`Role::Neutral`].
const ROLES: &[(&str, &str, Role)] = &[
    ("status", "seed", Role::Active),
    ("status", "hypothesis", Role::Warn),
    ("status", "refuted", Role::Error),
    ("status", "abandoned", Role::Muted),
    ("severity", "error", Role::Error),
    ("severity", "warn", Role::Warn),
    ("band", "strong", Role::Active),
    ("band", "some", Role::Neutral),
    ("band", "weak", Role::Muted),
    ("check", "clean", Role::Ok),
    ("check", "warnings", Role::Warn),
    ("check", "errors", Role::Error),
];

impl Role {
    /// The role of a domain value: `Role::of("status", "seed")`.
    pub(crate) fn of(kind: &str, value: &str) -> Self {
        ROLES
            .iter()
            .find(|(k, v, _)| *k == kind && *v == value)
            .map_or(Self::Neutral, |(.., role)| *role)
    }

    /// The SGR parameters for the role, or `None` for plain text.
    pub(crate) fn sgr(self) -> Option<&'static str> {
        match self {
            Self::Ok => Some("32"),
            Self::Warn => Some("33"),
            Self::Error => Some("31;1"),
            Self::Active => Some("36"),
            Self::Muted => Some("2"),
            Self::Neutral => None,
        }
    }
}

/// `text` in `role`'s colour when `stream` is coloured, else as it is.
pub(crate) fn paint(stream: Stream, role: Role, text: &str) -> String {
    painted(colours(stream), role.sgr(), text)
}

/// `text` in bold when `stream` is coloured: emphasis for an identifier,
/// which is no colour and so no role.
pub(crate) fn bold(stream: Stream, text: &str) -> String {
    painted(colours(stream), Some("1"), text)
}

/// `text` in `role`'s colour when `on`, else as it is: for a renderer handed
/// the gate's answer rather than asking it, as a table is.
pub(crate) fn paint_if(on: bool, role: Role, text: &str) -> String {
    painted(on, role.sgr(), text)
}

/// `text` in bold when `on`, else as it is. See [`paint_if`].
pub(crate) fn bold_if(on: bool, text: &str) -> String {
    painted(on, Some("1"), text)
}

pub(crate) fn painted(on: bool, sgr: Option<&str>, text: &str) -> String {
    match sgr.filter(|_| on) {
        Some(code) => format!("\x1b[{code}m{text}\x1b[0m"),
        None => text.to_string(),
    }
}
