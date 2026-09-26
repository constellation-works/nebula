//! Stop signals that reach nebula while a git child is live.
//!
//! git leads its own process group, so a Ctrl-C at the terminal, or a SIGTERM
//! or SIGHUP sent to nebula, reaches nebula alone. Left at their default,
//! those end nebula at once and orphan git, which then goes on to commit
//! whatever is staged by the time its hook returns — the next writer's work
//! included. So while any git child is live, SIGINT, SIGTERM and SIGHUP are
//! held: the handler only records which one arrived, the supervisor sees it
//! and stops its group exactly as it would on a timeout (STD-03 §R12), and
//! once the last live git has been reaped the old disposition is back and the
//! signal is delivered again, so nebula ends with that signal's conventional
//! status.
//!
//! Only a signal left at its default is taken over. One that is ignored
//! (`nohup`) stays ignored, and a handler a host application installed stays
//! that application's.

use rustix::process::{Signal, getpid, kill_process};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, PoisonError};

/// The signals held while git runs, with the names a message gives them.
const HELD: [(Signal, &str); 3] = [
    (Signal::INT, "SIGINT"),
    (Signal::TERM, "SIGTERM"),
    (Signal::HUP, "SIGHUP"),
];

/// The raw number of the held signal that arrived, or zero. Written by the
/// handler, so it is a lock-free atomic and nothing else.
static PENDING: AtomicI32 = AtomicI32::new(0);

/// How many git children are live, and the dispositions taken over while
/// any is. The count and the swap happen under this lock; nothing is held
/// under it for longer than that.
static STATE: Mutex<State> = Mutex::new(State {
    live: 0,
    replaced: Vec::new(),
});

struct State {
    live: usize,
    replaced: Vec<(libc::c_int, libc::sigaction)>,
}

/// Held for as long as one git child may be live. The first one takes the
/// signals over, the last one gives them back and delivers any that arrived.
pub(super) struct Forwarding(());

impl Forwarding {
    pub(super) fn hold() -> Self {
        let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
        if state.live == 0 {
            for (signal, _) in HELD {
                if let Some(previous) = take_over(signal.as_raw()) {
                    state.replaced.push((signal.as_raw(), previous));
                }
            }
        }
        state.live += 1;
        Self(())
    }
}

impl Drop for Forwarding {
    fn drop(&mut self) {
        let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
        state.live -= 1;
        if state.live > 0 {
            return;
        }
        for (raw, previous) in state.replaced.drain(..) {
            // SAFETY: `previous` is the action `sigaction` itself reported
            // for this signal, so reinstalling it is always valid.
            unsafe { libc::sigaction(raw, &raw const previous, std::ptr::null_mut()) };
        }
        let arrived = PENDING.swap(0, Ordering::SeqCst);
        drop(state);
        if arrived != 0 {
            deliver(arrived);
        }
    }
}

/// The name of the held signal that has arrived, if one has.
pub(super) fn pending() -> Option<&'static str> {
    let raw = PENDING.load(Ordering::SeqCst);
    HELD.iter()
        .find(|(signal, _)| signal.as_raw() == raw)
        .map(|&(_, name)| name)
}

/// Install [`record`] for `raw` when its disposition is the default, and
/// return what it replaced. `None` leaves the signal as it was.
fn take_over(raw: libc::c_int) -> Option<libc::sigaction> {
    // SAFETY: `sigaction` is plain data (integers, a signal set and, on
    // Linux, an optional function pointer), so all-zero bytes are a value.
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null new action only queries; the kernel writes `current`.
    if unsafe { libc::sigaction(raw, std::ptr::null(), &raw mut current) } != 0
        || current.sa_sigaction != libc::SIG_DFL
    {
        return None;
    }
    // SAFETY: as above.
    let mut ours: libc::sigaction = unsafe { std::mem::zeroed() };
    ours.sa_sigaction = record as extern "C" fn(libc::c_int) as libc::sighandler_t;
    ours.sa_flags = libc::SA_RESTART;
    // SAFETY: initializes the mask in place; it cannot fail on a valid pointer.
    unsafe { libc::sigemptyset(&raw mut ours.sa_mask) };
    // SAFETY: as above.
    let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: `ours` is fully initialized and its handler is
    // async-signal-safe: it does one lock-free atomic store and returns.
    (unsafe { libc::sigaction(raw, &raw const ours, &raw mut previous) } == 0).then_some(previous)
}

/// The handler: remember the signal. The supervisor polls for it.
extern "C" fn record(raw: libc::c_int) {
    PENDING.store(raw, Ordering::SeqCst);
}

/// Deliver the held signal `raw` again, now that its old disposition is back
/// and no git child is left: at its default, that ends the process.
fn deliver(raw: libc::c_int) {
    if let Some(&(signal, _)) = HELD.iter().find(|(signal, _)| signal.as_raw() == raw) {
        // Delivered before `kill` returns while this thread has it unblocked.
        // A thread that blocks it gets it when it unblocks it; until then
        // the supervisor's `Interrupted` is what the caller sees.
        let _ = kill_process(getpid(), signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
