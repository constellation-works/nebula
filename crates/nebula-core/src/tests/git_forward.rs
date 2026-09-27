//! Unit tests for `git_forward`.

use rustix::process::{Signal, getpid, kill_process};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::git::forward::*;

fn disposition(raw: libc::c_int) -> libc::sighandler_t {
    // SAFETY: as in `take_over`: a zeroed struct, then a query.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::sigaction(raw, std::ptr::null(), &raw mut current) },
        0
    );
    current.sa_sigaction
}

/// The signals are ours only while a git child is live, however many
/// overlap, and default again once the last one is done.
#[test]
fn held_signals_are_taken_over_while_git_is_live_and_given_back_after() {
    let term = Signal::TERM.as_raw();
    // No other git may be live in this process meanwhile, or the
    // disposition would legitimately be ours already.
    let _serial = crate::git::serial();
    assert_eq!(disposition(term), libc::SIG_DFL);
    let first = Forwarding::hold();
    let ours = record as extern "C" fn(libc::c_int) as libc::sighandler_t;
    assert_eq!(disposition(term), ours);
    let second = Forwarding::hold();
    drop(first);
    assert_eq!(disposition(term), ours, "one git is still live");
    drop(second);
    assert_eq!(disposition(term), libc::SIG_DFL);
    assert_eq!(pending(), None);
}
