//! The advisory write lock that keeps three writers off one corpus.
//!
//! Three processes share one corpus: the CLI a human types at, an agent
//! session running that same CLI, and the desktop's capture box. Each node
//! file is replaced atomically, but a mutating verb is a read-modify-write
//! across steps — load, edit, save, sometimes a second file, sometimes a git
//! commit — and two of those interleaved lose an edit, leave a one-sided
//! `contradicts` edge, or settle an inbox line that has moved.
//!
//! So every op that writes holds `<root>/.lock` for its whole duration and
//! for the commit that records it, and releases it when the guard drops.
//! **Reads never take it.** [`crate::store::Corpus::open`], the queries and
//! the desktop's file watcher must never wait on a writer.
//!
//! **Nor is it held across anything slow** (STD-03 §R1): not while a person
//! types in `$EDITOR` or standard input is read, and not across the reads
//! that are only advice — the suggestions after a capture or before a
//! promotion, the close-tag notes, a `--json` view of what was written.
//! Those run before the lock is taken or after it drops. `ops::suggest` and
//! `ops::close_tags` check it in debug builds through
//! [`held_by_this_thread`].
//!
//! That rule has a consequence a writer has to answer for: the config an
//! `open` reads is a snapshot from before the lock, and a writer that waited
//! its turn opened while the writer ahead of it was still working. So a
//! writer re-reads `config.yaml` under the lock before rewriting it or
//! deciding from it — otherwise the whole-file rewrite erases the setting the
//! writer ahead just landed. The config writers on [`crate::store::Corpus`]
//! do this; its readers deliberately do not.
//!
//! Exclusion has to hold at three ranges, and no single mechanism covers all
//! three:
//!
//! - **Between processes** — `flock` on the lock file. It is advisory, and
//!   the kernel drops it when the process dies, so a crash mid-write cannot
//!   wedge the corpus the way a lock file that had to be deleted would.
//! - **Between threads of one process** — `flock` is held by the open file
//!   description, not the thread, so a second thread opening the file again
//!   would sail straight through it. A process-wide table of live locks,
//!   keyed by root, is what actually excludes them.
//! - **Within one call stack** — the CLI holds the lock across a verb *and*
//!   the commit that follows, and the op it calls takes the same lock again.
//!   That re-entry must not deadlock, so a lock this thread already holds is
//!   counted rather than re-taken.
//!
//! Contention waits briefly and then refuses. A writer that blocked forever
//! on a stuck peer would be worse than one that says so: [`Error::Locked`]
//! reaches the caller before anything is written, and the caller can retry.
//!
//! The lock file is created once, `0600`, and never removed. Unlinking it
//! would race: a second process can hold `flock` on an unlinked inode while a
//! third creates a fresh file at the same path and locks that instead, and the
//! two would not see each other.
//!
//! The same lock guards this machine's settings in `~/.config/nebula`, taken
//! on that directory through
//! [`Corpus::lock_machine_settings`](crate::store::Corpus::lock_machine_settings),
//! so a check of a setting and the write that follows it cannot interleave
//! with another writer's.
//!
//! **The lock order** (STD-03 §R3): the machine-setting lock before the corpus
//! lock, never the other way round. `init --set-root` holds the first while
//! it creates the corpus under the second, and
//! `config observatory-root <DIR> --drop-legacy` takes both in that order
//! before writing either; nothing takes the machine-setting lock while
//! holding a corpus lock. Re-entry on one thread takes nothing new, so it
//! cannot break the order.

use crate::error::{Error, Result};
use crate::fs::private_open_options;
use fs4::{FileExt, TryLockError};
use std::collections::HashMap;
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// The lock file's name, directly under the corpus root.
///
/// Dot-prefixed and outside `nodes/` and `inbox/`, so nothing that reads the
/// corpus sees it: `Corpus::load_all` takes `nodes/*.md`, the inbox reader
/// takes `inbox/`, `neb commit` stages those two plus `config.yaml` and the
/// generated `.gitignore`, and the desktop's watcher watches the same two
/// directories.
pub const LOCK_FILE: &str = ".lock";

/// How long a writer waits for the writer ahead of it before refusing.
///
/// Long enough to cover a normal verb several times over — the slowest is a
/// commit, which is a handful of git invocations — and short enough that a
/// person waiting on a prompt is told what is happening rather than left to
/// wonder.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// How often the wait retries. Short enough to be invisible next to the
/// verb it is waiting for.
const POLL: Duration = Duration::from_millis(20);

/// A held corpus write lock. Released when it drops.
///
/// Take one with [`crate::store::Corpus::lock`], or with [`Self::acquire`]
/// when there is no `Corpus` yet because the corpus is being created.
///
/// Not `Send`, like any other lock guard: re-entry is counted against the
/// thread that took it, so a guard dropped on a different thread would
/// decrement somebody else's count. A thread that wants the lock takes its
/// own.
#[derive(Debug)]
pub struct CorpusLock {
    gate: Arc<Gate>,
    /// Whether this take found the lock free rather than re-entering it.
    outermost: bool,
    not_send: PhantomData<*const ()>,
}

impl CorpusLock {
    /// Take `<root>/.lock`, waiting up to [`LOCK_WAIT`] for whoever holds it.
    ///
    /// `root` must exist; the lock file is created inside it. Fails with
    /// [`Error::Locked`] when the wait runs out, before anything is written.
    pub fn acquire(root: &Path) -> Result<Self> {
        Self::acquire_within(root, LOCK_WAIT)
    }

    /// Take it with a different bound.
    ///
    /// Five seconds is what a person at a prompt will sit through. A caller
    /// with its own idea of patience — a UI that would rather say "busy", or
    /// a test that would rather not wait — names its own.
    pub fn acquire_within(root: &Path, wait: Duration) -> Result<Self> {
        let gate = gate_for(root);
        let deadline = Instant::now() + wait;
        loop {
            if let Some(lock) = try_enter(&gate, root)? {
                return Ok(lock);
            }
            if Instant::now() >= deadline {
                return Err(Error::Locked {
                    root: root.to_path_buf(),
                });
            }
            std::thread::sleep(POLL);
        }
    }
}

impl CorpusLock {
    /// Whether this is the take that entered the critical section, rather
    /// than a re-entry by a thread already inside it.
    ///
    /// What happens once per critical section, on the way in, keys on this:
    /// finishing an interrupted write runs here and nowhere deeper, because a
    /// re-entry is a verb already under way, and finishing its own pending
    /// write out from under it would race it.
    pub(crate) fn is_outermost(&self) -> bool {
        self.outermost
    }
}

impl Drop for CorpusLock {
    fn drop(&mut self) {
        let mut held = held(&self.gate);
        let Some(inner) = held.as_mut() else { return };
        inner.depth -= 1;
        if inner.depth == 0 {
            // Dropping the file closes the description and releases the
            // `flock` with it, which is the whole of the release.
            *held = None;
        }
    }
}

/// Whether the calling thread holds `root`'s lock, at any depth.
///
/// For the reads that must never run under it (STD-03 §R1): a scan of every
/// node for suggestions or close tags is advice, and a writer holding the
/// lock across one makes every other writer wait on advice. They
/// `debug_assert!` on this, so a caller that regresses fails every debug-build
/// test that goes through it. Another thread's or process's hold is not this
/// thread's business and reads as `false`.
///
/// Keyed by `root` as given, like the gate table, and it never adds a gate.
pub(crate) fn held_by_this_thread(root: &Path) -> bool {
    let gate = gates()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(root)
        .cloned();
    let me = std::thread::current().id();
    gate.is_some_and(|gate| held(&gate).as_ref().is_some_and(|h| h.thread == me))
}

/// One gate per corpus root, for the life of the process.
///
/// Keyed by the root exactly as it was given: paths in this crate are never
/// canonicalized, and two spellings of one directory are two gates in this
/// process — which costs nothing, because `flock` still excludes them.
#[derive(Debug, Default)]
struct Gate {
    held: Mutex<Option<Held>>,
}

/// Who holds one root's lock in this process, and how deep.
#[derive(Debug)]
struct Held {
    /// The thread inside the critical section. Only it may re-enter.
    thread: ThreadId,
    /// How many live [`CorpusLock`]s that thread has taken.
    depth: usize,
    /// The open file description carrying the `flock`. Never read or
    /// written; it is held so that dropping it releases the lock.
    _file: File,
}

/// The process-wide gate table.
fn gates() -> &'static Mutex<HashMap<PathBuf, Arc<Gate>>> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Gate>>>> = OnceLock::new();
    GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gate_for(root: &Path) -> Arc<Gate> {
    let mut gates = gates().lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(gates.entry(root.to_path_buf()).or_default())
}

/// A poisoned gate is a thread that panicked mid-write. The corpus write it
/// was making is atomic per file, so the lock itself is still sound: take the
/// state as it stands rather than turning someone else's panic into ours.
fn held(gate: &Gate) -> MutexGuard<'_, Option<Held>> {
    gate.held.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One attempt. `Ok(None)` means somebody else is mid-write and the caller
/// should wait; `Err` is the lock file itself failing to open or lock.
fn try_enter(gate: &Arc<Gate>, root: &Path) -> Result<Option<CorpusLock>> {
    let mut held = held(gate);
    let me = std::thread::current().id();
    match held.as_mut() {
        // Already inside on this thread: the CLI holding the lock across a
        // verb and its commit, with the op taking it again underneath.
        Some(inner) if inner.thread == me => {
            inner.depth += 1;
            Ok(Some(CorpusLock {
                gate: Arc::clone(gate),
                outermost: false,
                not_send: PhantomData,
            }))
        }
        // Another thread of this process is mid-write.
        Some(_) => Ok(None),
        None => {
            // `truncate(false)`: the file is a lock, not a record. Nothing
            // is ever written into it, and emptying it would be a write to
            // a file another process may hold open. Created `0600`.
            let path = root.join(LOCK_FILE);
            let file = private_open_options()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .map_err(|error| Error::io_at("opening", &path, error))?;
            match FileExt::try_lock(&file) {
                Ok(()) => {
                    *held = Some(Held {
                        thread: me,
                        depth: 1,
                        _file: file,
                    });
                    Ok(Some(CorpusLock {
                        gate: Arc::clone(gate),
                        outermost: true,
                        not_send: PhantomData,
                    }))
                }
                // Another process is mid-write.
                Err(TryLockError::WouldBlock) => Ok(None),
                Err(TryLockError::Error(error)) => Err(Error::io_at("locking", path, error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn the_lock_file_is_created_under_the_root_and_outlives_the_guard() {
        let dir = root();
        let lock = CorpusLock::acquire(dir.path()).expect("first acquire");
        assert!(dir.path().join(LOCK_FILE).exists());
        drop(lock);
        assert!(
            dir.path().join(LOCK_FILE).exists(),
            "the file stays; deleting it would race a process holding the old inode"
        );
        CorpusLock::acquire(dir.path()).expect("the next writer gets in");
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_file_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = root();
        drop(CorpusLock::acquire(dir.path()).expect("acquire"));
        let mode = std::fs::metadata(dir.path().join(LOCK_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_lock_file_that_cannot_be_opened_is_named() {
        let dir = root();
        let missing = dir.path().join("missing");
        let error = CorpusLock::acquire(&missing).expect_err("no directory to hold the lock");
        assert!(
            matches!(
                &error,
                Error::IoAt { action: "opening", path, .. } if *path == missing.join(LOCK_FILE)
            ),
            "expected the lock path in {error:?}"
        );
    }

    /// The CLI holds the lock across a verb and the commit that records it,
    /// and the op it calls takes the same lock again. Without re-entry that
    /// second take would wait on the first for five seconds and then refuse.
    #[test]
    fn one_thread_may_take_the_same_lock_again() {
        let dir = root();
        let outer = CorpusLock::acquire(dir.path()).expect("outer");
        let inner = CorpusLock::acquire_within(dir.path(), Duration::ZERO).expect("re-entry");
        assert!(outer.is_outermost() && !inner.is_outermost());
        drop(inner);

        // The outer guard still holds it: only the last drop releases.
        let gate = gate_for(dir.path());
        assert_eq!(held(&gate).as_ref().map(|h| h.depth), Some(1));
        drop(outer);
        assert!(held(&gate).is_none());
    }

    #[test]
    fn held_by_this_thread_reports_only_the_holding_thread() {
        let dir = root();
        assert!(!held_by_this_thread(dir.path()), "nobody holds it yet");

        let outer = CorpusLock::acquire(dir.path()).expect("outer");
        let inner = CorpusLock::acquire(dir.path()).expect("re-entry");
        assert!(held_by_this_thread(dir.path()));
        let path = dir.path().to_path_buf();
        let elsewhere = std::thread::spawn(move || held_by_this_thread(&path))
            .join()
            .expect("the other thread did not panic");
        assert!(!elsewhere, "another thread does not hold it");

        // Still held until the last guard goes.
        drop(inner);
        assert!(held_by_this_thread(dir.path()));
        drop(outer);
        assert!(!held_by_this_thread(dir.path()));

        let other = root();
        let _held = CorpusLock::acquire(dir.path()).expect("again");
        assert!(
            !held_by_this_thread(other.path()),
            "holding one root is not holding another"
        );
    }

    #[test]
    fn a_second_thread_waits_and_then_refuses_with_the_root_it_wanted() {
        let dir = root();
        let held_by_us = CorpusLock::acquire(dir.path()).expect("held");
        let path = dir.path().to_path_buf();

        let refused = std::thread::spawn(move || {
            CorpusLock::acquire_within(&path, Duration::from_millis(50)).map(|_| ())
        })
        .join()
        .expect("the waiter did not panic");

        assert!(
            matches!(&refused, Err(Error::Locked { root }) if root == dir.path()),
            "got {refused:?}"
        );
        drop(held_by_us);

        // And once it is free, another thread gets in. The guard is not
        // `Send`, so it is taken and dropped over there rather than handed
        // back.
        let path = dir.path().to_path_buf();
        std::thread::spawn(move || {
            CorpusLock::acquire_within(&path, Duration::from_millis(50)).map(|_| ())
        })
        .join()
        .expect("no panic")
        .expect("the lock is free again");
    }
}
