//! The output layer: the only code in the crate that writes to stdout or
//! stderr, and the only code that names either stream (STD-02 §R15).
//! `scripts/check-terminal-guard.sh` fails CI if anything else does.
//!
//! Everything else writes through these:
//!
//! - `out!` and `outln!` write to stdout, as `print!` and `println!` did;
//! - `errln!` writes a line to stderr, as `eprintln!` did;
//! - [`stdout`] is a [`Write`] handle on stdout, for code that takes one,
//!   such as `triage` and the completion script.
//!
//! The streams are explicit in the names, so moving a line from one stream to
//! the other is a one-word change.
//!
//! **A closed stdout ends the command silently, never the write under it.**
//! `neb list | head -1` closes the pipe while `neb` is still writing. The
//! first write that fails latches stdout shut: it and every later write
//! return as though they had succeeded, and nothing more is written. So the
//! verb runs to its end, including the commit that records a write, and
//! `main` exits as the verb decided, with nothing on stderr (STD-01 §R13).
//! `SIGPIPE` stays ignored, as Rust leaves it: its default action would kill
//! the process between a write and its commit.
//!
//! A failure other than a closed pipe latches too, for the same reason, and
//! [`finish`] hands it back so `main` refuses with it once the verb is done.
//! Writes to stderr are best effort: when stderr is gone there is nowhere to
//! say so, and a warning must never cost the command.

use crate::render::Refusal;
use std::fmt;
use std::io::{self, ErrorKind, IsTerminal, Write};
use std::sync::{Mutex, PoisonError};

/// Write to stdout, formatted as `print!` formats.
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::output::write_out(format_args!($($arg)*))
    };
}

/// Write a line to stdout, formatted as `println!` formats.
macro_rules! outln {
    ($($arg:tt)*) => {
        $crate::output::write_out(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Write a line to stderr, formatted as `eprintln!` formats. Never fails.
macro_rules! errln {
    ($($arg:tt)*) => {
        $crate::output::write_err(format_args!("{}\n", format_args!($($arg)*)))
    };
}

pub(crate) use {errln, out, outln};

/// Whether stdout has stopped taking writes, shared by every handle on it.
static STDOUT: Latch = Latch::new();

/// A [`Write`] handle on stdout, through the latch.
///
/// It never returns an error: a failed write stops stdout and reads as
/// success (see the module docs). Ask [`Closable::is_closed`] whether anyone
/// is still reading.
pub fn stdout() -> Stdout {
    Stdout(())
}

/// See [`stdout`].
pub struct Stdout(());

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        STDOUT.write(&mut io::stdout().lock(), buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        STDOUT.flush(&mut io::stdout().lock())
    }
}

/// A writer that can tell when nobody reads it any more, so a session writing
/// to it knows to stop asking questions.
pub trait Closable: Write {
    /// Whether writes have stopped reaching a reader.
    fn is_closed(&self) -> bool;
}

impl Closable for Stdout {
    fn is_closed(&self) -> bool {
        !STDOUT.is_open()
    }
}

/// A buffer is always read: it is what the caller inspects afterwards.
impl Closable for Vec<u8> {
    fn is_closed(&self) -> bool {
        false
    }
}

/// Whether stdout is a terminal, for `render`'s colour decision.
pub fn stdout_is_terminal() -> bool {
    io::stdout().is_terminal()
}

/// Flush stdout, and hand back a write to it that failed for any reason but
/// a closed pipe. `main` calls this once, after the verb has run.
pub fn finish() -> Result<(), StdoutFailed> {
    // The latch has already absorbed any error; what it recorded is below.
    let _ = STDOUT.flush(&mut io::stdout().lock());
    STDOUT.failure()
}

/// The target of `out!` and `outln!`.
#[doc(hidden)]
pub fn write_out(args: fmt::Arguments<'_>) {
    // `Stdout` returns no error but `Interrupted`, which `write_fmt` retries.
    let _ = stdout().write_fmt(args);
}

/// The target of `errln!`.
#[doc(hidden)]
pub fn write_err(args: fmt::Arguments<'_>) {
    best_effort(&mut io::stderr().lock(), args);
}

/// Write to stderr, or to a stand-in for it, and ignore a failure: there is
/// nowhere left to report one.
fn best_effort(err: &mut impl Write, args: fmt::Arguments<'_>) {
    let _ = err.write_fmt(args);
}

/// A write to stdout failed with something other than a closed pipe: a full
/// disk behind a redirect, say. `main` refuses with it.
#[derive(Debug)]
pub struct StdoutFailed(io::Error);

impl fmt::Display for StdoutFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not write to stdout: {}", self.0)
    }
}

/// The refusal `main` reports, with `code` `stdout` under `--json`.
impl From<StdoutFailed> for Refusal {
    fn from(e: StdoutFailed) -> Self {
        Refusal::new("stdout", e.to_string())
    }
}

/// What a stream has done so far.
#[derive(Debug)]
enum State {
    /// Every write so far has landed.
    Open,
    /// The reader went away (`EPIPE`).
    Closed,
    /// A write failed some other way.
    Failed(io::Error),
}

/// Stops a stream at its first failed write, so that no failure to write
/// output can end a command early.
struct Latch(Mutex<State>);

impl Latch {
    const fn new() -> Self {
        Self(Mutex::new(State::Open))
    }

    /// Run `op` against the stream while it is open. A failure latches it
    /// and reads as success; later calls do nothing. `Interrupted` is not a
    /// failure: it goes back to the caller, whose `write_all` retries it.
    fn pass<T>(&self, op: impl FnOnce() -> io::Result<T>) -> io::Result<Option<T>> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*state, State::Open) {
            return Ok(None);
        }
        match op() {
            Ok(done) => Ok(Some(done)),
            Err(e) if e.kind() == ErrorKind::Interrupted => Err(e),
            Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                *state = State::Closed;
                Ok(None)
            }
            Err(e) => {
                *state = State::Failed(e);
                Ok(None)
            }
        }
    }

    fn write(&self, stream: &mut impl Write, buf: &[u8]) -> io::Result<usize> {
        Ok(self.pass(|| stream.write(buf))?.unwrap_or(buf.len()))
    }

    fn flush(&self, stream: &mut impl Write) -> io::Result<()> {
        self.pass(|| stream.flush()).map(drop)
    }

    fn is_open(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        matches!(*state, State::Open)
    }

    /// The failure that stopped the stream, unless it was a closed pipe,
    /// which is how a pipe is supposed to end. Taken once.
    fn failure(&self) -> Result<(), StdoutFailed> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*state, State::Failed(_)) {
            return Ok(());
        }
        match std::mem::replace(&mut *state, State::Closed) {
            State::Failed(e) => Err(StdoutFailed(e)),
            State::Open | State::Closed => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream that fails every call with `kind` and counts the calls that
    /// reached it.
    struct Failing {
        kind: ErrorKind,
        calls: usize,
    }

    impl Failing {
        fn new(kind: ErrorKind) -> Self {
            Self { kind, calls: 0 }
        }
    }

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            Err(self.kind.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.calls += 1;
            Err(self.kind.into())
        }
    }

    #[test]
    fn broken_pipe_is_latched_and_later_writes_are_skipped() {
        let latch = Latch::new();
        let mut stream = Failing::new(ErrorKind::BrokenPipe);
        assert_eq!(latch.write(&mut stream, b"first").unwrap(), 5);
        assert_eq!(stream.calls, 1);
        assert!(!latch.is_open());

        assert_eq!(latch.write(&mut stream, b"second").unwrap(), 6);
        latch.flush(&mut stream).unwrap();
        assert_eq!(stream.calls, 1, "nothing reaches a closed stream");
        assert!(latch.failure().is_ok(), "a closed pipe is no failure");
    }

    #[test]
    fn other_stdout_errors_become_a_refusal() {
        let latch = Latch::new();
        let mut stream = Failing::new(ErrorKind::StorageFull);
        assert_eq!(latch.write(&mut stream, b"payload").unwrap(), 7);
        latch.write(&mut stream, b"more").unwrap();
        assert_eq!(stream.calls, 1, "a failed stream is written no further");

        let refused = Refusal::from(latch.failure().unwrap_err());
        assert_eq!(refused.code, "stdout");
        let said = &refused.message;
        assert!(said.starts_with("could not write to stdout: "), "{said}");
        assert!(
            said.ends_with(&io::Error::from(ErrorKind::StorageFull).to_string()),
            "{said}"
        );
        assert!(latch.failure().is_ok(), "reported once");
    }

    #[test]
    fn an_open_stream_writes_through_and_has_no_failure() {
        let latch = Latch::new();
        let mut buf = Vec::new();
        assert_eq!(latch.write(&mut buf, b"one\n").unwrap(), 4);
        latch.flush(&mut buf).unwrap();
        assert_eq!(buf, b"one\n");
        assert!(latch.is_open());
        assert!(latch.failure().is_ok());
        assert!(latch.is_open(), "asking about a failure leaves it open");
    }

    #[test]
    fn interrupted_is_retried_rather_than_latched() {
        let latch = Latch::new();
        let mut stream = Failing::new(ErrorKind::Interrupted);
        let e = latch.write(&mut stream, b"x").unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Interrupted);
        assert!(latch.is_open());
    }

    #[test]
    fn a_failed_stderr_write_never_panics() {
        for kind in [ErrorKind::BrokenPipe, ErrorKind::Other] {
            let mut stream = Failing::new(kind);
            best_effort(&mut stream, format_args!("warning: {}\n", "lost"));
            assert_eq!(stream.calls, 1);
        }
    }
}
