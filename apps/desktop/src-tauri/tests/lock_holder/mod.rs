//! A second process holding a corpus's write lock until told to let go.
//!
//! `flock` is held by the open file description, so only another process
//! contends with it the way `neb` or an agent does. The holder is this test
//! binary started with [`ROOT_VAR`] set: the constructor below sees it before
//! `main`, takes the lock, prints [`HELD`], waits for stdin to close, and
//! exits. libtest never starts in the child, so no test is re-run there, and
//! there is no `#[test]` that silently passes in the parent's own run
//! (STD-03 §R19, STD-04 §R8). The child comes from the isolating builder and
//! is owned by a [`ChildGuard`] (STD-03 §R18).
//!
//! `roundtrip.rs` and `commands.rs` include this by path, beside `support`.

use crate::support::{self, ChildGuard};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Instant;

/// Set on the child to the root whose lock it holds.
const ROOT_VAR: &str = "NEBULA_DESKTOP_TEST_HOLD_LOCK";

/// What the child prints once the lock is held.
const HELD: &str = "NEBULA_TEST_LOCK_HELD";

/// In the child only: hold the lock, then exit without reaching `main`.
#[ctor::ctor]
unsafe fn hold_the_lock_when_asked() {
    let Some(root) = std::env::var_os(ROOT_VAR) else {
        return;
    };
    let code = match hold(Path::new(&root)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("lock holder: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn hold(root: &Path) -> Result<(), String> {
    let _lock = nebula_core::CorpusLock::acquire(root).map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{HELD}")
        .and_then(|()| stdout.flush())
        .map_err(|e| e.to_string())?;
    let mut ignored = Vec::new();
    std::io::stdin()
        .read_to_end(&mut ignored)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Another process holding `root`'s lock. Dropping it kills the holder;
/// [`LockHolder::release`] lets it exit cleanly and checks that it did.
pub struct LockHolder {
    child: ChildGuard,
}

impl LockHolder {
    /// Start the holder and wait, up to [`support::DEADLINE`], until it has
    /// the lock.
    pub fn start(root: &Path) -> Self {
        let exe = std::env::current_exe().expect("the test binary's path");
        let mut child = ChildGuard::spawn(
            support::command(exe, support::home())
                .env(ROOT_VAR, root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped()),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        // Lines are read on a thread so the wait for the signal has a
        // deadline; the guard kills the holder if it never comes.
        let (send, lines) = mpsc::channel();
        let stdout = BufReader::new(child.take_stdout());
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let until = Instant::now() + support::DEADLINE;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match lines.recv_timeout(left) {
                Ok(line) if line == HELD => return Self { child },
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("lock holder exited early"),
                Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                    "lock holder did not take the lock within {:?}",
                    support::DEADLINE
                ),
            }
        }
    }

    /// Close the holder's stdin and wait for it to let go and exit 0.
    pub fn release(mut self) {
        drop(self.child.take_stdin());
        let status = self
            .child
            .wait(support::DEADLINE)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(status.success(), "lock holder: {status}");
    }
}
