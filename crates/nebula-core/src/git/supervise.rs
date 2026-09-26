//! One git child, from spawn to reap.
//!
//! One thread does all of it: `poll` on the two pipes with a timeout, so the
//! output is drained while git runs (STD-03 §R15), the deadline and a stop
//! signal are noticed within [`POLL_SLICE`], and no wait here is unbounded
//! (§R22). The leader's exit is observed with `waitid(WNOWAIT)`, which leaves
//! it a zombie: its pid, and so the group's id, cannot be reused until it is
//! reaped, and it is reaped only after the group has been swept (§R14).
//!
//! A consequence of holding the leader unreaped: `killpg(pgid, 0)` succeeds
//! for as long as the zombie is there, so the group can never be seen empty
//! while it is. The grace period after SIGTERM therefore also ends once git
//! has exited and nothing holds its output open any more. That only ends the
//! politeness early; the SIGKILL sweep that follows is unconditional.

use super::forward::{self, Forwarding};
use super::{Captured, GIT_TERMINATION_GRACE, GitOutput, REPOSITORY_ENV, RunError};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, kill_process_group, test_kill_process_group, waitid,
};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The longest the supervisor sleeps before looking again at the deadline,
/// a stop signal and the leader.
const POLL_SLICE: Duration = Duration::from_millis(50);

/// The first pause when there is no pipe left to wait on, doubled up to
/// [`POLL_SLICE`]: git usually exits a moment after closing its output, and a
/// commit is a handful of git runs that should not each lose a whole slice.
const FIRST_IDLE: Duration = Duration::from_millis(1);

/// After the sweep, how long the pipes may stay open. Only a process that
/// left git's group (`setsid`) can still hold them; its output is not waited
/// for.
const DRAIN_DEADLINE: Duration = Duration::from_secs(1);

/// Read size for one ready pipe: a quarter of a Linux pipe buffer, and on
/// the stack.
const CHUNK: usize = 16 * 1024;

/// Spawn `git -C <root> <args>` as its own group and see it through.
pub(super) fn run(root: &Path, args: &[&str], deadline: Duration) -> Result<GitOutput, RunError> {
    // Held before the spawn, so a signal from here on is never lost, and
    // dropped only after the group is gone, so it is delivered after that.
    let forwarding = Forwarding::hold();
    if let Some(signal) = forward::pending() {
        return Err(RunError::Interrupted(signal));
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Never wait on a person: there may not be one, and nothing here
        // could answer a credential prompt anyway.
        .env("GIT_TERMINAL_PROMPT", "0")
        .process_group(0);
    for name in REPOSITORY_ENV {
        command.env_remove(name);
    }
    let child = command.spawn().map_err(RunError::Start)?;
    let outcome = Supervisor::new(child).run(deadline);
    drop(forwarding);
    outcome
}

/// Why the supervisor stopped git before it exited on its own.
#[derive(Debug, Clone, Copy)]
enum Stop {
    TimedOut,
    Signal(&'static str),
}

struct Supervisor {
    child: Child,
    /// The group git leads: its own pid, since it was spawned with
    /// `process_group(0)`.
    group: Pid,
    stdout: Stream<ChildStdout>,
    stderr: Stream<ChildStderr>,
}

impl Supervisor {
    fn new(mut child: Child) -> Self {
        let group = Pid::from_child(&child);
        Self {
            stdout: Stream::new(child.stdout.take()),
            stderr: Stream::new(child.stderr.take()),
            group,
            child,
        }
    }

    fn run(mut self, deadline: Duration) -> Result<GitOutput, RunError> {
        let stop = self.wait_or_stop(Instant::now() + deadline);
        // Whatever ended the wait, nothing of the group outlives this call
        // (§R13): a hook's `sleep &` after a clean exit included.
        self.sweep();
        let reaped = self.reap();
        self.drain_until(Instant::now() + DRAIN_DEADLINE);
        match stop {
            Some(Stop::TimedOut) => Err(RunError::TimedOut(deadline)),
            Some(Stop::Signal(signal)) => Err(RunError::Interrupted(signal)),
            None => Ok(GitOutput {
                status: reaped?,
                stdout: self.stdout.finish(),
                stderr: self.stderr.finish(),
            }),
        }
    }

    /// Drain the output until the leader exits, or until the deadline or a
    /// stop signal, then SIGTERM the group and wait out the grace period.
    /// Returns why git was stopped, or `None` when it exited on its own.
    fn wait_or_stop(&mut self, deadline: Instant) -> Option<Stop> {
        let mut stop = None;
        let mut grace_ends = None;
        let mut idle = FIRST_IDLE;
        loop {
            let exited = self.leader_exited();
            match grace_ends {
                None if exited => return None,
                None => {
                    let reason = forward::pending()
                        .map(Stop::Signal)
                        .or_else(|| (Instant::now() >= deadline).then_some(Stop::TimedOut));
                    if reason.is_some() {
                        self.signal_group(Signal::TERM);
                        // A member stopped by job control would sit on the
                        // SIGTERM until the SIGKILL; let it act on it now.
                        self.signal_group(Signal::CONT);
                        grace_ends = Some(Instant::now() + GIT_TERMINATION_GRACE);
                        stop = reason;
                    }
                }
                Some(ends) => {
                    if (exited && self.output_closed()) || Instant::now() >= ends {
                        return stop;
                    }
                }
            }
            self.pump(grace_ends.unwrap_or(deadline), &mut idle);
        }
    }

    /// SIGKILL whatever of the group is left. Survival is the group's own
    /// answer, and anything but "no such group" counts as alive (§R12).
    /// Called only while the leader is unreaped, so the group id is still
    /// the one git was given (§R14).
    fn sweep(&self) {
        if !matches!(test_kill_process_group(self.group), Err(Errno::SRCH)) {
            self.signal_group(Signal::KILL);
        }
    }

    /// Wait, bounded, for the swept leader to be gone, then reap it.
    fn reap(&mut self) -> Result<ExitStatus, RunError> {
        let give_up = Instant::now() + GIT_TERMINATION_GRACE;
        let mut idle = FIRST_IDLE;
        while !self.leader_exited() {
            if Instant::now() >= give_up {
                // Stuck in the kernel past SIGKILL. It stays unreaped rather
                // than block nebula, and is never signalled again.
                return Err(RunError::Supervise(std::io::Error::other(
                    "git did not exit after SIGKILL",
                )));
            }
            std::thread::sleep(idle);
            idle = (idle * 2).min(POLL_SLICE);
        }
        self.child.wait().map_err(RunError::Supervise)
    }

    /// Whether the leader has exited, observed without reaping it. An
    /// interrupted or failed probe reads as "not yet": the deadline still
    /// bounds the wait.
    fn leader_exited(&self) -> bool {
        matches!(
            waitid(
                WaitId::Pid(self.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            ),
            Ok(Some(_))
        )
    }

    fn signal_group(&self, signal: Signal) {
        // The leader is unreaped whenever this runs, so the group exists and
        // is ours; there is nothing useful to do with a refusal.
        let _ = kill_process_group(self.group, signal);
    }

    fn output_closed(&self) -> bool {
        self.stdout.pipe.is_none() && self.stderr.pipe.is_none()
    }

    /// Read what the pipes have until `until`, or for at most one slice.
    fn pump(&mut self, until: Instant, idle: &mut Duration) {
        let wait = until
            .saturating_duration_since(Instant::now())
            .min(POLL_SLICE);
        if self.output_closed() {
            std::thread::sleep(wait.min(*idle));
            *idle = (*idle * 2).min(POLL_SLICE);
            return;
        }
        let (out_ready, err_ready) = {
            // Which stream each entry is, beside the entries `poll` fills.
            let mut streams = Vec::with_capacity(2);
            let mut fds = Vec::with_capacity(2);
            if let Some(pipe) = &self.stdout.pipe {
                streams.push(true);
                fds.push(PollFd::new(pipe, PollFlags::IN));
            }
            if let Some(pipe) = &self.stderr.pipe {
                streams.push(false);
                fds.push(PollFd::new(pipe, PollFlags::IN));
            }
            let timeout = Timespec::try_from(wait).unwrap_or(Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            });
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) => {}
                // A signal: the caller looks at it straight away.
                Err(Errno::INTR) => return,
                Err(_) => {
                    std::thread::sleep(wait);
                    return;
                }
            }
            // Readable, at its end, or failed: a read settles each.
            let ready = |stdout: bool| {
                streams
                    .iter()
                    .zip(&fds)
                    .any(|(is_stdout, fd)| *is_stdout == stdout && !fd.revents().is_empty())
            };
            (ready(true), ready(false))
        };
        if out_ready {
            self.stdout.read_once();
        }
        if err_ready {
            self.stderr.read_once();
        }
    }

    /// After the sweep: read what is left, until both pipes end or `until`.
    fn drain_until(&mut self, until: Instant) {
        let mut idle = FIRST_IDLE;
        while !self.output_closed() && Instant::now() < until {
            self.pump(until, &mut idle);
        }
    }
}

/// One pipe from git and what has been read from it.
struct Stream<R> {
    pipe: Option<R>,
    captured: Captured,
}

impl<R: Read> Stream<R> {
    fn new(pipe: Option<R>) -> Self {
        Self {
            pipe,
            captured: Captured::default(),
        }
    }

    /// One read from a pipe `poll` said is ready, so it does not block.
    fn read_once(&mut self) {
        let Some(pipe) = self.pipe.as_mut() else {
            return;
        };
        let mut chunk = [0; CHUNK];
        match pipe.read(&mut chunk) {
            Ok(0) => {
                self.pipe = None;
                self.captured.ended = true;
            }
            Ok(n) => self.captured.push(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            // Unreadable: stop reading. The capture stays marked unfinished.
            Err(_) => self.pipe = None,
        }
    }

    /// What was read. A pipe still open is closed here, unfinished.
    fn finish(self) -> Captured {
        self.captured
    }
}
