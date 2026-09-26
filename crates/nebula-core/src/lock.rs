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
//! two would not see each other. It is opened without following a symlink, so
//! a link planted at `.lock` is refused rather than written through.
//!
//! **Who holds it** (STD-03 §R7). Just after `flock` succeeds, the holder
//! writes one short record into the lock file itself — its PID, when it took
//! the lock, and a label saying which writer it is (`neb edit a-node`,
//! `desktop capture`) — and empties the file again as the guard drops. A
//! writer that times out reads that record into [`Error::Locked`], so the
//! refusal names who to wait for. The record is **diagnostic only**: `flock`
//! is the one authority on whether the lock is held, and nothing here decides
//! ownership or liveness from the record or ever signals its PID
//! (STD-03 §R14). It is written after the lock is taken and cleared before it
//! is released, so a waiter can catch it empty, or still holding a crashed
//! holder's record; an empty, oversized or unreadable record is
//! `holder: None`, which is still contention, never a free lock. Keeping it
//! in `.lock` keeps it where commits and the checker already never look.
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
use crate::fs::{no_follow, overwrite_in_place, private_open_options, read_capped};
use fs4::{FileExt, TryLockError};
use std::collections::HashMap;
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

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

/// The label a holder records when its process never named itself.
const DEFAULT_LABEL: &str = "nebula";

/// The most of a holder record a waiter reads. A record is a few dozen
/// bytes; anything larger is not one, and is not read into memory to find
/// that out.
const RECORD_CAP: usize = 4096;

/// The longest label a holder records, in characters. Enough for a verb and
/// the id it names; a longer one is cut rather than refused, because the
/// label only explains a wait.
const LABEL_CAP: usize = 160;

/// Who holds a lock, as the holder recorded it in the lock file.
///
/// **Diagnostic only** (STD-03 §R7, §R14): it names who to wait for in an
/// [`Error::Locked`] and nothing more. Whether the lock is held is decided by
/// `flock` alone, never by this record; the PID is never signalled and never
/// probed for liveness. It can be stale — a holder that crashed leaves its
/// record behind until the next holder overwrites it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockHolder {
    /// The holding process's id.
    pub pid: u32,
    /// When it took the lock, to the second.
    pub since: SystemTime,
    /// Which writer it is: `neb edit a-node`, `desktop capture`, or
    /// `nebula` when the writer did not say.
    pub label: String,
}

impl LockHolder {
    /// [`Self::since`] as RFC 3339 in UTC, as the record stores it.
    pub fn since_rfc3339(&self) -> String {
        OffsetDateTime::from(self.since)
            .format(&Rfc3339)
            .unwrap_or_else(|_| "an unreadable time".to_owned())
    }
}

/// The holder record as it sits in the lock file: one line of JSON.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    pid: u32,
    since: String,
    label: String,
}

/// This process's label, for every lock taken without one of its own.
static PROCESS_LABEL: OnceLock<String> = OnceLock::new();

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
#[must_use = "the corpus lock is released when this guard drops"]
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
        Self::acquire_as(root, wait, process_label())
    }

    /// Take it with a bound and a label of its own, which the holder record
    /// carries instead of the process's (see [`Self::label_process`]).
    ///
    /// For a process with more than one kind of writer, like the desktop,
    /// whose commands each say which they are. A take that re-enters a lock
    /// this thread already holds records nothing: the outermost label stays.
    pub fn acquire_as(root: &Path, wait: Duration, label: &str) -> Result<Self> {
        let gate = gate_for(root);
        let deadline = Instant::now() + wait;
        loop {
            if let Some(lock) = try_enter(&gate, root, label)? {
                return Ok(lock);
            }
            if Instant::now() >= deadline {
                return Err(Error::Locked {
                    root: root.to_path_buf(),
                    holder: read_holder(&root.join(LOCK_FILE)),
                });
            }
            std::thread::sleep(POLL);
        }
    }

    /// Name this process in the record of every lock it takes without a
    /// label of its own: `neb edit a-node`, say, set once from the parsed
    /// command line. The first call wins and later ones are ignored, so a
    /// label cannot change under a lock already held. A process that never
    /// calls this records `nebula`.
    pub fn label_process(label: &str) {
        let _ = PROCESS_LABEL.set(clean_label(label));
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
            // Empty the holder record while the lock is still held, so the
            // next waiter does not read this one as its holder. A failure
            // costs only that diagnosis, and a guard never fails a drop.
            let _ = overwrite_in_place(&inner.file, b"");
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
    /// The open file description carrying the `flock`. Written only to
    /// record and clear the holder; dropping it releases the lock.
    file: File,
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
fn try_enter(gate: &Arc<Gate>, root: &Path, label: &str) -> Result<Option<CorpusLock>> {
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
            // `truncate(false)`: another process may hold the lock and have
            // its record in the file, and only a holder writes it. Created
            // `0600`, and never through a symlink: the record written below
            // would otherwise empty whatever the link names.
            let path = root.join(LOCK_FILE);
            let file = no_follow(
                private_open_options()
                    .create(true)
                    .write(true)
                    .truncate(false),
            )
            .open(&path)
            .map_err(|error| Error::io_at("opening", &path, error))?;
            match FileExt::try_lock(&file) {
                Ok(()) => {
                    record_holder(&file, label);
                    *held = Some(Held {
                        thread: me,
                        depth: 1,
                        file,
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

/// Write this process into the lock file it has just locked.
///
/// Diagnostic only, so a failure is not the writer's problem: the lock is
/// held either way, and a waiter that finds no record says the holder is
/// unidentified. Only a regular file is written, so a FIFO or device planted
/// at the name is locked but never written to.
fn record_holder(file: &File, label: &str) {
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return;
    }
    let since = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::now_utc());
    let Ok(since) = since.format(&Rfc3339) else {
        return;
    };
    let record = Record {
        pid: std::process::id(),
        since,
        label: clean_label(label),
    };
    let Ok(mut line) = serde_json::to_vec(&record) else {
        return;
    };
    line.push(b'\n');
    let _ = overwrite_in_place(file, &line);
}

/// The holder a waiter reads out of `path`, or `None` when there is no
/// usable record: empty, oversized, a symlink, unparsable, or with a label
/// no holder would write. `None` still means the lock is held.
fn read_holder(path: &Path) -> Option<LockHolder> {
    let bytes = read_capped(path, RECORD_CAP).ok()??;
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    if record.label.is_empty() || record.label != clean_label(&record.label) {
        return None;
    }
    let since = OffsetDateTime::parse(&record.since, &Rfc3339).ok()?;
    Some(LockHolder {
        pid: record.pid,
        since: since.into(),
        label: record.label,
    })
}

/// A label as a record stores it: no control characters, which a terminal
/// rendering the refusal would act on, cut to [`LABEL_CAP`] characters, and
/// [`DEFAULT_LABEL`] when nothing is left.
fn clean_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(LABEL_CAP)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        DEFAULT_LABEL.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// The label of a lock taken without one.
fn process_label() -> &'static str {
    PROCESS_LABEL.get().map_or(DEFAULT_LABEL, String::as_str)
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
        drop(CorpusLock::acquire(dir.path()).expect("the next writer gets in"));
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

    /// Hold `root`'s lock as `label` on another thread, run `waiter` on
    /// this one while it is held, then let the holder go.
    fn while_held_elsewhere<T>(root: &Path, label: &str, waiter: impl FnOnce() -> T) -> T {
        let (held_tx, held_rx) = std::sync::mpsc::sync_channel(0);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(0);
        std::thread::scope(|scope| {
            let holder = scope.spawn(move || {
                let lock = CorpusLock::acquire_as(root, LOCK_WAIT, label).expect("the holder");
                held_tx.send(()).expect("the waiter is listening");
                // Until the waiter is done, or gone.
                let _ = done_rx.recv();
                drop(lock);
            });
            held_rx.recv().expect("the holder took the lock");
            let out = waiter();
            done_tx.send(()).expect("the holder is waiting");
            holder.join().expect("the holder did not panic");
            out
        })
    }

    /// Replace the lock file's contents by hand, the way a torn write, a
    /// crash or a stranger would leave them.
    fn scribble(root: &Path, bytes: &[u8]) {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(root.join(LOCK_FILE))
            .and_then(|mut file| file.write_all(bytes))
            .expect("scribbling on the lock file");
    }

    #[test]
    fn a_timeout_names_the_holder() {
        let dir = root();
        let refused = while_held_elsewhere(dir.path(), "test holder", || {
            CorpusLock::acquire_within(dir.path(), Duration::from_millis(50)).map(|_| ())
        });
        let Err(Error::Locked {
            root,
            holder: Some(holder),
        }) = &refused
        else {
            panic!("expected a named holder, got {refused:?}");
        };
        assert_eq!(root, dir.path());
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(holder.label, "test holder");
        assert!(holder.since <= SystemTime::now(), "{holder:?}");
        assert!(
            SystemTime::now()
                .duration_since(holder.since)
                .is_ok_and(|age| age < Duration::from_secs(60)),
            "taken just now: {holder:?}"
        );
        assert!(
            refused
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("`test holder`, pid "),
            "the message names it: {refused:?}"
        );
    }

    /// The record only explains a wait. Whatever the file holds, a lock
    /// somebody has is refused as held — never taken as free because its
    /// record could not be read.
    #[test]
    fn an_unreadable_holder_record_is_still_contention() {
        let oversized = format!(
            r#"{{"pid":1,"since":"2026-09-26T00:00:00Z","label":"{}"}}"#,
            "x".repeat(RECORD_CAP)
        );
        let cases: [(&str, &[u8]); 6] = [
            ("garbage", b"\xff\x00 not a record at all"),
            ("empty", b""),
            ("truncated", br#"{"pid":1,"since":"2026-09-"#),
            (
                "a bad time",
                br#"{"pid":1,"since":"yesterday","label":"neb"}"#,
            ),
            (
                "a control character",
                b"{\"pid\":1,\"since\":\"2026-09-26T00:00:00Z\",\"label\":\"\\u001b[2J\"}",
            ),
            ("oversized", oversized.as_bytes()),
        ];
        for (case, bytes) in cases {
            let dir = root();
            let refused = while_held_elsewhere(dir.path(), "test holder", || {
                scribble(dir.path(), bytes);
                CorpusLock::acquire_within(dir.path(), Duration::from_millis(50)).map(|_| ())
            });
            assert!(
                matches!(&refused, Err(Error::Locked { holder: None, .. })),
                "{case}: got {refused:?}"
            );
            assert!(
                refused
                    .unwrap_err()
                    .to_string()
                    .contains("(an unidentified writer)"),
                "{case}"
            );
        }
    }

    #[test]
    fn the_record_is_cleared_on_release_and_not_rewritten_on_reentry() {
        let dir = root();
        let path = dir.path().join(LOCK_FILE);
        let outer = CorpusLock::acquire_as(dir.path(), Duration::ZERO, "outer").expect("outer");
        let recorded = read_holder(&path).expect("the holder recorded itself");
        assert_eq!(recorded.label, "outer");
        assert_eq!(recorded.pid, std::process::id());

        let inner = CorpusLock::acquire_as(dir.path(), Duration::ZERO, "inner").expect("re-entry");
        assert_eq!(read_holder(&path), Some(recorded.clone()), "re-entry wrote");
        drop(inner);
        assert_eq!(
            read_holder(&path),
            Some(recorded),
            "an inner release cleared the outer holder's record"
        );

        drop(outer);
        assert_eq!(std::fs::metadata(&path).expect("the file stays").len(), 0);
    }

    /// The record is written into the file the lock opened, so a symlink at
    /// `.lock` would have it empty the link's target. It is refused instead,
    /// and the target is left as it was.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_lock_file_is_refused_and_its_target_untouched() {
        let dir = root();
        let target = dir.path().join("precious");
        scribble_new(&target, b"keep me");
        std::os::unix::fs::symlink(&target, dir.path().join(LOCK_FILE)).expect("symlink");

        let error = CorpusLock::acquire(dir.path()).expect_err("a symlinked lock file");
        assert!(
            matches!(
                &error,
                Error::IoAt {
                    action: "opening",
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(std::fs::read(&target).expect("target"), b"keep me");
    }

    #[cfg(unix)]
    fn scribble_new(path: &Path, bytes: &[u8]) {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| file.write_all(bytes))
            .expect("writing a fixture");
    }

    #[test]
    fn a_label_is_recorded_without_control_characters_and_within_bounds() {
        assert_eq!(clean_label("neb edit a-node"), "neb edit a-node");
        assert_eq!(clean_label("neb edit \u{1b}[2J"), "neb edit ?[2J");
        assert_eq!(clean_label("  "), DEFAULT_LABEL);
        assert_eq!(clean_label(&"x".repeat(1000)).chars().count(), LABEL_CAP);
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
            matches!(&refused, Err(Error::Locked { root, .. }) if root == dir.path()),
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
