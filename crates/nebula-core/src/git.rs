//! The one way nebula runs git.
//!
//! Every git child goes through [`run_git`], because each one can stall on
//! something nebula does not control — a `pre-commit` hook, a credential
//! helper, a filter — and a stalled git used to take the corpus lock with it
//! and, when `neb` was killed, outlive it and commit the next writer's work
//! under its own message. So a git child here:
//!
//! - leads its own process group, so it and everything it starts are
//!   signalled as one (STD-03 §R11);
//! - has a named deadline, after which the group gets SIGTERM, a grace
//!   period, and SIGKILL if anything of it is still there (§R12, §R22);
//! - is followed by a SIGKILL sweep of its group on every way out, a clean
//!   exit included, and is reaped only after that sweep, so the group is never
//!   signalled once its leader's pid is free for reuse (§R13, §R14);
//! - has its stdout and stderr drained while it runs and captured up to a cap,
//!   with a cut marked in text and refused for parsing (§R15);
//! - runs with every variable that would point git at another repository
//!   removed, so it acts on the root it was given (STD-05 §R7).
//!
//! A timed-out git is [`Error::GitTimedOut`], never a plain failure or a
//! success. A stop signal delivered to nebula while git runs stops the group
//! the same way, and then ends nebula with that signal (see `forward`).
//!
//! The rest of the environment is passed through: git is operator-trusted,
//! so the user's identity, signing and hooks still apply (STD-05 §R10 does
//! not bind a trusted child).

use crate::error::Error;
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

#[cfg(unix)]
mod forward;
#[cfg(unix)]
mod supervise;

/// How long any git command but `commit` may run.
pub const GIT_DEADLINE: Duration = Duration::from_secs(30);

/// How long `git commit` may run. Longer, because it runs the repository's
/// hooks, and a hook that lints or tests is slow on purpose.
pub const GIT_COMMIT_DEADLINE: Duration = Duration::from_secs(120);

/// How long a git group has between SIGTERM and SIGKILL.
pub const GIT_TERMINATION_GRACE: Duration = Duration::from_secs(5);

/// The most of each output stream a git run keeps, in bytes. The rest is read
/// and discarded, so git never blocks on a full pipe.
pub const GIT_OUTPUT_CAP: usize = 1024 * 1024;

/// The variables that make git act on a repository other than the one at
/// `-C <root>`. Exactly what `git rev-parse --local-env-vars` prints on git
/// 2.43: an inherited `GIT_DIR` or `GIT_INDEX_FILE` — the ones git itself
/// exports to hooks — would otherwise send a corpus commit into whichever
/// repository the caller happened to be inside, or answer a history query
/// from it.
pub(crate) const REPOSITORY_ENV: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// Debug builds only: every git command gets this many milliseconds instead
/// of its named deadline, so an end-to-end test of the timeout does not wait
/// two minutes. Never read by a release build.
#[cfg(debug_assertions)]
const DEADLINE_ENV: &str = "NEBULA_TEST_GIT_DEADLINE_MS";

/// Run `git -C <root> <args>` under supervision.
///
/// `Ok` whenever git ran to an exit of its own, whatever its status: a
/// non-zero exit is an answer for some callers. `Err` when there is no
/// answer — git did not start, ran out of time, or was stopped by a signal
/// to nebula — and by then nothing of git's process group is left.
pub(crate) fn run_git(root: &Path, args: &[&str]) -> Result<GitOutput, RunError> {
    let deadline = deadline_for(args);
    #[cfg(unix)]
    {
        supervise::run(root, args, deadline)
    }
    #[cfg(not(unix))]
    {
        // No process groups to stop git's hooks with, so no unsupervised
        // fallback either (STD-03 §R11): git is refused, not half-run.
        let _ = (root, deadline);
        Err(RunError::Start(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "nebula runs git only where it can supervise it (Unix)",
        )))
    }
}

/// The deadline for one command: [`GIT_COMMIT_DEADLINE`] for `commit`,
/// [`GIT_DEADLINE`] for anything else, unless a debug-build test shortened it.
fn deadline_for(args: &[&str]) -> Duration {
    #[cfg(debug_assertions)]
    if let Some(shortened) = test_deadline() {
        return shortened;
    }
    if args.first() == Some(&"commit") {
        GIT_COMMIT_DEADLINE
    } else {
        GIT_DEADLINE
    }
}

/// What a git command left behind when it ran to its own exit.
#[derive(Debug)]
pub(crate) struct GitOutput {
    /// How git exited.
    pub(crate) status: ExitStatus,
    /// Its standard output, as far as the cap.
    pub(crate) stdout: Captured,
    /// Its standard error, as far as the cap.
    pub(crate) stderr: Captured,
}

impl GitOutput {
    /// Standard output for a caller that parses it: all of it, or
    /// [`Error::Git`] when it was cut. A cut listing parsed as if it were
    /// whole would be a confident wrong answer.
    pub(crate) fn stdout_whole(&self, root: &Path, context: &str) -> Result<&[u8], Error> {
        self.stdout.whole().ok_or_else(|| Error::Git {
            root: root.to_path_buf(),
            context: context.to_string(),
            stderr: format!(
                "its output passed the {GIT_OUTPUT_CAP}-byte capture limit, so none of it was used"
            ),
        })
    }
}

/// One output stream of a git run, kept up to [`GIT_OUTPUT_CAP`] bytes.
#[derive(Debug, Default)]
pub(crate) struct Captured {
    bytes: Vec<u8>,
    /// Bytes git wrote past the cap, read and thrown away.
    dropped: u64,
    /// Whether the stream reached its end. False when something outside
    /// git's process group still held it open after git was gone.
    ended: bool,
}

impl Captured {
    /// Keep what fits under the cap and count the rest.
    fn push(&mut self, chunk: &[u8]) {
        let room = GIT_OUTPUT_CAP.saturating_sub(self.bytes.len());
        let (kept, over) = chunk.split_at(room.min(chunk.len()));
        self.bytes.extend_from_slice(kept);
        self.dropped = self
            .dropped
            .saturating_add(u64::try_from(over.len()).unwrap_or(u64::MAX));
    }

    /// Everything git wrote to the stream, or `None` when it was cut.
    pub(crate) fn whole(&self) -> Option<&[u8]> {
        (self.dropped == 0 && self.ended).then_some(self.bytes.as_slice())
    }

    /// Whether git wrote nothing at all to the stream.
    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty() && self.dropped == 0
    }

    /// The stream as text for a message: trimmed, never longer than the cap,
    /// and ending in `… [truncated N bytes]` when git said more than that.
    pub(crate) fn text(&self) -> String {
        let lossy = String::from_utf8_lossy(&self.bytes);
        let text = lossy.trim();
        // Lossy decoding can grow invalid bytes, so the cap is applied to
        // the text as well, on a character boundary.
        let mut end = text.len().min(GIT_OUTPUT_CAP);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let hidden = self
            .dropped
            .saturating_add(u64::try_from(text.len() - end).unwrap_or(u64::MAX));
        let shown = &text[..end];
        if hidden > 0 {
            format!("{shown} … [truncated {hidden} bytes]")
        } else if !self.ended {
            format!("{shown} … [cut off: the output was still held open after git exited]")
        } else {
            shown.to_string()
        }
    }
}

/// Why a git command gave no answer.
#[derive(Debug)]
pub(crate) enum RunError {
    /// git could not be started: not installed, not executable, or no
    /// process could be created.
    Start(std::io::Error),
    /// git started, but watching it failed: it could not be reaped, or did
    /// not exit even after SIGKILL.
    Supervise(std::io::Error),
    /// It ran past its deadline and its process group was stopped.
    TimedOut(Duration),
    /// A stop signal reached nebula while it ran, so its process group was
    /// stopped before nebula ends with that signal.
    Interrupted(&'static str),
}

impl RunError {
    /// The library error for this, about `git <context>` in `root`.
    pub(crate) fn into_error(self, root: &Path, context: &str) -> Error {
        let failed = |context: &str, stderr: String| Error::Git {
            root: root.to_path_buf(),
            context: context.to_string(),
            stderr,
        };
        match self {
            // The context has always been `start` here: git never ran.
            Self::Start(error) => failed("start", error.to_string()),
            Self::Supervise(error) => failed(context, format!("supervising git failed: {error}")),
            Self::TimedOut(after) => Error::GitTimedOut {
                root: root.to_path_buf(),
                context: context.to_string(),
                after,
            },
            Self::Interrupted(signal) => {
                failed(context, format!("stopped because nebula received {signal}"))
            }
        }
    }
}

#[cfg(debug_assertions)]
thread_local! {
    static DEADLINE_OVERRIDE: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// A deadline a debug-build test set, on this thread or in the environment.
#[cfg(debug_assertions)]
fn test_deadline() -> Option<Duration> {
    DEADLINE_OVERRIDE.with(std::cell::Cell::get).or_else(|| {
        std::env::var(DEADLINE_ENV)
            .ok()?
            .parse()
            .ok()
            .map(Duration::from_millis)
    })
}

/// Test seam, debug builds only: while this guard lives, every git command
/// the current thread runs gets `deadline` in place of [`GIT_DEADLINE`] and
/// [`GIT_COMMIT_DEADLINE`], so a test of a hung hook finishes in seconds.
/// Per thread, so tests running in parallel do not shorten each other's git.
/// It does not exist in a release build, so it can never be a user's knob.
#[cfg(debug_assertions)]
#[doc(hidden)]
#[must_use = "the deadline is back to normal as soon as the guard drops"]
#[derive(Debug)]
pub struct GitDeadlineOverride {
    previous: Option<Duration>,
    not_send: std::marker::PhantomData<*const ()>,
}

#[cfg(debug_assertions)]
impl GitDeadlineOverride {
    /// Shorten every git deadline on this thread to `deadline`.
    pub fn new(deadline: Duration) -> Self {
        Self {
            previous: DEADLINE_OVERRIDE.with(|cell| cell.replace(Some(deadline))),
            not_send: std::marker::PhantomData,
        }
    }
}

#[cfg(debug_assertions)]
impl Drop for GitDeadlineOverride {
    fn drop(&mut self) {
        DEADLINE_OVERRIDE.with(|cell| cell.set(self.previous));
    }
}

/// Unit tests that run git, or look at the signal dispositions a live git
/// changes, take this so they never overlap.
#[cfg(test)]
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured(bytes: &[u8]) -> Captured {
        let mut out = Captured::default();
        out.push(bytes);
        out.ended = true;
        out
    }

    #[test]
    fn output_under_the_cap_is_whole_and_unmarked() {
        let out = captured(b"  fatal: not a git repository\n");
        assert_eq!(out.whole(), Some(&b"  fatal: not a git repository\n"[..]));
        assert_eq!(out.text(), "fatal: not a git repository");
        assert!(!out.is_empty());
        assert!(captured(b"").is_empty());
    }

    #[test]
    fn output_past_the_cap_is_kept_to_the_cap_refused_whole_and_marked() {
        let out = captured(&vec![b'x'; GIT_OUTPUT_CAP + 10]);
        assert_eq!(out.bytes.len(), GIT_OUTPUT_CAP);
        assert_eq!(out.whole(), None, "a cut stream is never parsed");
        let text = out.text();
        assert!(
            text.ends_with(" … [truncated 10 bytes]"),
            "{}",
            &text[GIT_OUTPUT_CAP..]
        );
        assert_eq!(text.len(), GIT_OUTPUT_CAP + " … [truncated 10 bytes]".len());
    }

    #[test]
    fn a_cap_that_falls_inside_a_character_cuts_before_it() {
        // Three-byte characters from the start, so the cap lands mid-character.
        let mut bytes = "가".repeat(GIT_OUTPUT_CAP / 3 + 1).into_bytes();
        bytes.truncate(GIT_OUTPUT_CAP + 1);
        let text = captured(&bytes).text();
        let shown = text.split(" … [truncated").next().unwrap();
        assert!(shown.len() <= GIT_OUTPUT_CAP);
        assert!(shown.chars().all(|c| c == '가'));
    }

    #[test]
    fn a_stream_that_never_ended_is_not_whole_and_says_so() {
        let mut out = Captured::default();
        out.push(b"partial");
        assert_eq!(out.whole(), None);
        assert!(
            out.text().starts_with("partial … [cut off"),
            "{}",
            out.text()
        );
    }

    #[test]
    fn commit_gets_the_longer_deadline_and_the_override_is_per_thread() {
        // A fresh thread, so no other test's override is in force on it.
        let clean = std::thread::spawn(|| {
            assert_eq!(deadline_for(&["commit", "-q"]), GIT_COMMIT_DEADLINE);
            assert_eq!(deadline_for(&["log"]), GIT_DEADLINE);
            {
                let _short = GitDeadlineOverride::new(Duration::from_millis(5));
                assert_eq!(deadline_for(&["commit"]), Duration::from_millis(5));
                let other = std::thread::spawn(|| deadline_for(&["commit"]));
                assert_eq!(other.join().unwrap(), GIT_COMMIT_DEADLINE);
            }
            assert_eq!(deadline_for(&["commit"]), GIT_COMMIT_DEADLINE);
        });
        clean.join().unwrap();
    }

    /// The list is git's own: every variable this git says selects a
    /// repository is scrubbed. A newer git that adds one fails here, which
    /// is the prompt to add it.
    #[cfg(unix)]
    #[test]
    fn every_repository_variable_git_names_is_scrubbed() {
        let _serial = serial();
        let dir = std::env::temp_dir();
        let out = run_git(&dir, &["rev-parse", "--local-env-vars"]).expect("git runs");
        assert!(out.status.success(), "{}", out.stderr.text());
        let named = String::from_utf8(out.stdout.whole().unwrap().to_vec()).unwrap();
        let named: Vec<&str> = named.lines().collect();
        assert!(!named.is_empty());
        for name in named {
            assert!(REPOSITORY_ENV.contains(&name), "{name} is not scrubbed");
        }
    }

    #[test]
    fn the_scrub_list_names_each_variable_once() {
        let mut names = REPOSITORY_ENV.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), REPOSITORY_ENV.len());
        assert!(names.iter().all(|name| name.starts_with("GIT_")));
    }
}
