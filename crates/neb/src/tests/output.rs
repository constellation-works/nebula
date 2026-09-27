//! Unit tests for `output`.

use crate::render::Refusal;
use std::fmt;
use std::io::{self, ErrorKind, Write};
use std::sync::{Mutex, PoisonError};

use crate::output::*;

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
