//! The one way nebula puts bytes on disk.
//!
//! Everything nebula keeps — nodes, the inbox, `config.yaml`, `.gitignore`,
//! the lock, `~/.config/nebula`, the desktop's `settings.json` — is created
//! and written through this module, so the steps that make a write durable and private
//! cannot drift between copies (STD-03 §R5). `clippy.toml` refuses
//! `std::fs::write` and `std::fs::create_dir_all` everywhere else, which is
//! what keeps this the only copy.
//!
//! Three guarantees, all here and nowhere else:
//!
//! - **Durable.** A file is replaced whole: written to a temporary sibling,
//!   flushed with `fsync`, renamed over the target, and then the directory
//!   that holds it is flushed too, so the rename itself survives a crash. An
//!   append is one `write` of the whole line followed by `fdatasync`, so a
//!   crash can lose the line but never tear it.
//! - **Owner-only.** Files are created `0600` and directories `0700`,
//!   set at creation rather than left to the umask (STD-05 §R8). A rewrite
//!   replaces the file with the temporary one, which was created `0600`, so a
//!   rewrite can narrow a file's mode and never widens it. A directory that
//!   already exists is left exactly as its owner made it.
//! - **A replace never writes through a symlink.** The temporary file is
//!   created with `O_EXCL`, and `rename` replaces a symlink at the target
//!   rather than following it, so a replace always leaves a regular file
//!   where the caller pointed and nothing outside it changes.
//! - **A file opened by name is a regular file.** An append, the lock and
//!   every read of a corpus file go through [`open_regular`]: the entry is
//!   judged with `lstat` before it is opened, opened `O_NOFOLLOW` and
//!   `O_NONBLOCK`, and judged again on the descriptor, so a symlink, a FIFO,
//!   a device or a directory is refused as [`Error::NotRegularFile`] and
//!   nothing is read from or written through it (STD-05 §R7).
//!
//! On platforms without Unix modes the modes are not set and directories are
//! not flushed; the rename is still atomic.

use crate::error::{Error, Result};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Note a step for `recording`. Expands to nothing outside tests, so the
/// seam costs a release build nothing and cannot change what it does.
macro_rules! observe {
    ($step:expr) => {
        #[cfg(test)]
        record($step);
    };
}

/// The mode every file nebula creates gets: read and write for the owner.
pub const PRIVATE_FILE_MODE: u32 = 0o600;

/// The mode every directory nebula creates gets: the owner alone.
pub const PRIVATE_DIR_MODE: u32 = 0o700;

/// Replace `path` with `contents`, durably and owner-only.
///
/// The bytes go to a fresh temporary file beside `path`, which is flushed and
/// then renamed over it; the directory holding it is flushed after the
/// rename. A reader sees the old file or the new one, never half of either,
/// and a crash after this returns cannot bring the old one back.
///
/// The temporary file is created, never opened by name a second time, so the
/// bytes go to the file this call made and to nothing else. Writing by name
/// would follow whatever already answers to it: a symlink planted at the
/// temporary path is a write straight through the corpus wall, and the rename
/// that follows would then install the symlink as the node. A symlink at
/// `path` itself is replaced by a regular file; its target is never written.
///
/// The temporary file is removed when either writing or renaming fails, so a
/// failed write does not leave debris that could be mistaken for corpus data.
/// A failure to flush the directory after the rename is reported as such: the
/// new contents are in place by then, but not yet known to be durable.
pub fn write_private_atomic(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let (tmp, file) = create_temporary_sibling(path)?;
    let result =
        write_sync_and_close(file, &tmp, contents.as_ref()).and_then(|()| rename(&tmp, path));
    if let Err(error) = result {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => {
                return Err(Error::TempCleanupFailed {
                    path: path.to_path_buf(),
                    tmp,
                    write: error,
                    cleanup,
                });
            }
        }
        return Err(Error::io_at("writing", path, error));
    }
    sync_parent(path)
}

/// Create `path` holding `contents`, durably and owner-only, where no file of
/// that name exists yet.
///
/// For a file that must never replace another, such as an edit kept after a
/// refused save: a second one kept under the same name would lose the first.
/// The name is opened `create_new` (`O_EXCL`), so a name that is taken — a
/// symlink included — is refused as [`Error::IoAt`] with the source
/// [`std::io::ErrorKind::AlreadyExists`], and the caller picks another. The
/// bytes are flushed, then the directory. A write that fails part-way removes
/// the file it created rather than leave half of it behind.
pub fn create_private_new(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let file = private_open_options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| Error::io_at("creating", path, error))?;
    if let Err(error) = write_sync_and_close(file, path, contents.as_ref()) {
        // This call created the file a moment ago, so it is ours to remove;
        // if even that fails, the write's own error is the one to report.
        let _ = std::fs::remove_file(path);
        return Err(Error::io_at("writing", path, error));
    }
    sync_parent(path)
}

/// Create `path` and any missing parents, each one `0700`.
///
/// A directory that already exists, `path` included, is left alone: its mode
/// is its owner's choice, and a corpus root the user made before running
/// `init` keeps whatever they gave it.
pub fn create_private_dir_all(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(PRIVATE_DIR_MODE);
    }
    builder
        .create(path)
        .map_err(|error| Error::io_at("creating", path, error))
}

/// Options that create a file `0600`, and nothing else yet.
///
/// Every file nebula opens for writing starts here, so a new way of opening
/// one inherits the mode rather than the umask. Callers add the access and
/// creation flags they need.
pub fn private_open_options() -> OpenOptions {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(PRIVATE_FILE_MODE);
    }
    options
}

/// What an entry turned out to be when a regular file was required.
///
/// [`Error::NotRegularFile`] carries it, so the refusal says what is in the
/// way rather than the errno the open would have given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryKind {
    /// A symbolic link, whether or not it leads anywhere.
    Symlink,
    /// A directory.
    Directory,
    /// A named pipe: a read would wait for a writer that may never come.
    Fifo,
    /// A Unix domain socket.
    Socket,
    /// A block or character device, such as `/dev/zero`, which a read would
    /// never finish.
    Device,
    /// Something else the platform has.
    Other,
}

impl EntryKind {
    /// What `file_type` is, or `None` when it is a regular file.
    fn of(file_type: std::fs::FileType) -> Option<Self> {
        if file_type.is_file() {
            return None;
        }
        if file_type.is_symlink() {
            return Some(Self::Symlink);
        }
        if file_type.is_dir() {
            return Some(Self::Directory);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt as _;
            if file_type.is_fifo() {
                return Some(Self::Fifo);
            }
            if file_type.is_socket() {
                return Some(Self::Socket);
            }
            if file_type.is_block_device() || file_type.is_char_device() {
                return Some(Self::Device);
            }
        }
        Some(Self::Other)
    }
}

impl std::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Symlink => "a symlink",
            Self::Directory => "a directory",
            Self::Fifo => "a FIFO",
            Self::Socket => "a socket",
            Self::Device => "a device",
            Self::Other => "a special file",
        })
    }
}

/// How [`open_regular`] treats a symlink in the last component of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Links {
    /// Refuse it. Every entry below the corpus root is judged this way.
    Refuse,
    /// Follow it and judge what it names. For machine settings and for
    /// `.gitignore`, whose symlink `init` replaces with a copy of its target.
    Follow,
}

/// Refuse `path` as [`Error::NotRegularFile`] unless `file_type` is a
/// regular file.
fn require_regular(path: &Path, file_type: std::fs::FileType) -> Result<()> {
    match EntryKind::of(file_type) {
        None => Ok(()),
        Some(found) => Err(Error::NotRegularFile {
            path: path.to_path_buf(),
            found,
        }),
    }
}

/// Open `path` with `options`, provided it is a regular file, or one
/// `options` creates.
///
/// The entry is judged three times, and each closes a gap the others leave
/// (STD-05 §R7):
///
/// - **Before the open**, on its own metadata, so a device or a FIFO is
///   refused without being opened at all: opening one can block, or have
///   effects of its own.
/// - **By the open**, which on Unix carries `O_NOFOLLOW` (under
///   [`Links::Refuse`]) and `O_NONBLOCK`, so an entry swapped in after that
///   look cannot redirect it: the kernel refuses a symlink with `ELOOP`, and
///   a FIFO without a reader cannot hold the open. Either failure is reported
///   as what is there now, not as the errno. `O_NONBLOCK` changes nothing for
///   a regular file.
/// - **On the descriptor**, which is the one resolution every later read,
///   write or lock uses.
///
/// Nothing is resolved or canonicalized: `path` is used as given, so a corpus
/// reached through a symlinked root opens its files beneath it as usual; only
/// the last component is judged. An entry that is not there is left to the
/// open, which creates it or fails with [`std::io::ErrorKind::NotFound`] as
/// [`Error::IoAt`] labelled `action`.
pub(crate) fn open_regular(
    path: &Path,
    options: &mut OpenOptions,
    links: Links,
    action: &'static str,
) -> Result<File> {
    let judge = |path: &Path| match links {
        Links::Refuse => std::fs::symlink_metadata(path),
        Links::Follow => std::fs::metadata(path),
    };
    match judge(path) {
        Ok(metadata) => require_regular(path, metadata.file_type())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io_at("inspecting", path, error)),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let no_follow = match links {
            Links::Refuse => libc::O_NOFOLLOW,
            Links::Follow => 0,
        };
        options.custom_flags(no_follow | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| {
        judge(path)
            .ok()
            .and_then(|metadata| require_regular(path, metadata.file_type()).err())
            .unwrap_or_else(|| Error::io_at(action, path, error))
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| Error::io_at("inspecting", path, error))?;
    require_regular(path, metadata.file_type())?;
    Ok(file)
}

/// Open or create a private, regular, singly linked advisory-lock file.
///
/// Does not acquire the advisory lock or change existing contents. The guarded
/// open refuses symlinks and special files before any blocking I/O. Call
/// [`validate_private_lock`] again after acquiring the lock and before changing
/// its holder record, since waiting may have allowed another link to appear.
pub fn open_private_lock(path: &Path) -> Result<File> {
    let file = open_regular(
        path,
        private_open_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false),
        Links::Refuse,
        "opening lock",
    )?;
    validate_private_lock(&file, path)?;
    Ok(file)
}

/// Refuse to mutate an advisory-lock descriptor with an unsafe file type or
/// link count. `path` labels errors; all checks use the already open descriptor.
///
/// This detects existing aliases, not a hostile same-user process racing a
/// new hard link between this check and the write. No lock inode is replaced
/// or unlinked: doing so could split contenders across different inodes.
/// Platforms without a verifiable link count fail closed.
pub fn validate_private_lock(file: &File, path: &Path) -> Result<()> {
    let metadata = file
        .metadata()
        .map_err(|error| Error::io_at("inspecting lock", path, error))?;
    require_regular(path, metadata.file_type())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(Error::io_at(
                "validating lock",
                path,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "lock must have exactly one hard link; preserve its contents and remove extra aliases before retrying",
                ),
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    Err(Error::io_at(
        "validating lock",
        path,
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "cannot verify the lock file's hard-link count on this platform",
        ),
    ))
}

/// The regular file at `path`, opened to read, or `None` when there is none.
fn open_to_read(path: &Path, links: Links) -> Result<Option<File>> {
    match open_regular(path, OpenOptions::new().read(true), links, "reading") {
        Ok(file) => Ok(Some(file)),
        Err(Error::IoAt { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The text of the regular file at `path`, or `None` when nothing is there.
///
/// Every read of a file below the corpus root comes through here, so a
/// symlink, a FIFO, a device or a directory at `path` is
/// [`Error::NotRegularFile`] and nothing is read from it. See
/// [`open_regular`].
pub(crate) fn read_regular_text(path: &Path) -> Result<Option<String>> {
    use std::io::Read as _;
    let Some(mut file) = open_to_read(path, Links::Refuse)? else {
        return Ok(None);
    };
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| Error::io_at("reading", path, error))?;
    Ok(Some(text))
}

/// The bytes of the regular file at `path`, or `None` when nothing is there;
/// a symlink is followed or refused as `links` says.
pub(crate) fn read_regular_bytes(path: &Path, links: Links) -> Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let Some(mut file) = open_to_read(path, links)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| Error::io_at("reading", path, error))?;
    Ok(Some(bytes))
}

/// Whether a regular file is at `path`: `false` when nothing is, and
/// [`Error::NotRegularFile`] when something else is. Judged without following
/// a symlink, and without opening anything.
pub(crate) fn regular_file_at(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => require_regular(path, metadata.file_type()).map(|()| true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io_at("inspecting", path, error)),
    }
}

/// Append `bytes` to `path` in one write, then flush the data.
///
/// The whole record goes to a single `write_all` on an `O_APPEND` descriptor,
/// so two appenders cannot interleave inside it and a crash cannot leave the
/// first half of it without the second. A file this call creates is `0600`,
/// and its directory is flushed as well so the new name survives a crash.
///
/// A file that is already there is opened through [`open_regular`], so the
/// append never lands on the far side of a symlink, and a FIFO or a device
/// there is refused rather than written to. Creating one is `O_EXCL`, which
/// refuses a name that is taken, a symlink included.
pub(crate) fn append_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let at = |error| Error::io_at("writing", path, error);
    let (mut file, created) = match private_open_options()
        .append(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => (file, true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
            open_regular(
                path,
                private_open_options().append(true),
                Links::Refuse,
                "writing",
            )?,
            false,
        ),
        Err(error) => return Err(at(error)),
    };
    file.write_all(bytes).map_err(at)?;
    observe!(Step::Write {
        path: path.to_path_buf(),
        bytes: bytes.to_vec(),
    });
    file.sync_data().map_err(at)?;
    observe!(Step::SyncData(path.to_path_buf()));
    if created {
        sync_parent(path)?;
    }
    Ok(())
}

/// Remove `path` and flush its directory, so the removal survives a crash
/// as a write would.
///
/// For a file nebula made and has just established is its own, such as the
/// pending-write record it read back and parsed (STD-03 §R29): nothing here
/// decides ownership, so no caller may reach it with a path found by name
/// alone. A file that is already gone is not an error; the outcome the
/// caller wanted is on disk either way.
pub(crate) fn remove_private(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::io_at("removing", path, error)),
    }
    observe!(Step::Remove(path.to_path_buf()));
    sync_parent(path)
}

/// Replace an open file's contents in place: truncate it, then write `bytes`
/// from the start in one write, with no flush.
///
/// For the lock's holder record and nothing else (see `lock.rs`). The record
/// lives in the lock file itself, which is never replaced: renaming a new
/// file over it would move the `flock` to an inode no other writer opens. And
/// it is not durable state, so STD-03 §R5's rule that a durable file is never
/// rewritten in place does not reach it: it is cleared on release, means
/// nothing after a crash, and a reader that finds it torn or empty treats it
/// as no record at all. Written through the handle the caller holds, never
/// by name, so it cannot follow a symlink planted since the open.
pub(crate) fn overwrite_in_place(file: &File, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::{Seek as _, SeekFrom};
    file.set_len(0)?;
    if !bytes.is_empty() {
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(bytes)?;
    }
    Ok(())
}

/// Read `path` whole if it is a regular file of at most `cap` bytes.
///
/// For a small record another process may be rewriting, like the lock's
/// holder record. It never follows a symlink at `path` (`O_NOFOLLOW`), never
/// blocks opening something that is not a regular file (a FIFO planted at
/// the name), and never reads more than `cap` bytes plus one, however large
/// the file has grown. `Ok(None)` when the file is not a regular file or is
/// larger than `cap`.
pub(crate) fn read_capped(path: &Path, cap: usize) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        // Both flags at once: `custom_flags` replaces rather than adds.
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() <= cap).then_some(bytes))
}

/// Write the whole of `contents`, flush it to the device, and close the file,
/// so the rename that follows moves complete bytes nobody still holds open.
fn write_sync_and_close(
    mut file: File,
    #[cfg_attr(
        not(test),
        allow(unused_variables, reason = "named only for the test seam")
    )]
    tmp: &Path,
    contents: &[u8],
) -> std::io::Result<()> {
    file.write_all(contents)?;
    observe!(Step::Write {
        path: tmp.to_path_buf(),
        bytes: contents.to_vec(),
    });
    file.sync_all()?;
    observe!(Step::SyncAll(tmp.to_path_buf()));
    Ok(())
}

fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)?;
    observe!(Step::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
    });
    Ok(())
}

/// Flush the directory holding `path`, so a name just created or renamed in
/// it is on the device and not only in the page cache.
fn sync_parent(path: &Path) -> Result<()> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    sync_dir(dir).map_err(|error| Error::io_at("syncing", dir, error))
}

/// A filesystem that cannot flush a directory at all answers `EINVAL` or
/// `ENOTSUP`, and has no stronger guarantee to give; the rename already
/// happened, so that is not a failed write.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    match File::open(dir).and_then(|handle| handle.sync_all()) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) => {}
        Err(error) => return Err(error),
    }
    observe!(Step::SyncDir(dir.to_path_buf()));
    Ok(())
}

/// Windows cannot open a directory as a file to flush it, and NTFS journals
/// the rename itself.
#[cfg(not(unix))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Unix twin can fail, and callers treat both alike"
)]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// How many names a single write tries before giving up. Two names collide
/// only when another writer picks the same counter in the same nanosecond
/// under the same process id, or when something is planting files under each
/// name as fast as they are tried. A few attempts cover the first and a bound
/// keeps the second from spinning.
const TEMPORARY_NAME_ATTEMPTS: u32 = 8;

/// Create a temporary file beside `path` and hand back its name and its open
/// handle.
///
/// `create_new` is the guard: it opens with `O_CREAT | O_EXCL`, which refuses
/// a name that already exists instead of following it, so a symlink sitting at
/// the temporary path is a refusal rather than a write to its target. The
/// handle comes back with the name because reopening by name afterwards would
/// hand the same opening back to whoever won the race. The file is created
/// `0600`, and it becomes the target on rename, so this is also what keeps a
/// rewrite from widening a file.
///
/// The name carries a nonce, and not only for the race. A fixed name that
/// `O_EXCL` refuses would wedge every later write to that file behind one
/// stale temporary left by a killed process, and nothing here deletes what it
/// did not create. `.tmp` stays the extension so a temporary that does outlive
/// a crash stays invisible to `load_all` and to the inbox, which both match on
/// the name.
///
/// The name is built by appending to `path` as given, so a corpus reached
/// through a symlinked root writes beside the file the caller named. Nothing
/// is resolved or canonicalized.
pub(crate) fn create_temporary_sibling(path: &Path) -> Result<(PathBuf, File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    for _ in 0..TEMPORARY_NAME_ATTEMPTS {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{:x}-{count:x}-{nanos:x}.tmp", std::process::id()));
        let tmp = PathBuf::from(name);
        match private_open_options()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Error::io_at("writing", path, error)),
        }
    }
    Err(Error::NoFreeTempName {
        path: path.to_path_buf(),
        attempts: TEMPORARY_NAME_ATTEMPTS,
    })
}

/// One thing the helpers did to the disk, as a test sees it.
///
/// Recorded after the step succeeded, in the order the steps ran, and only
/// while a test on the same thread is [`recording`]. Outside tests the
/// recording compiles away.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// One `write_all` of these bytes to this file.
    Write { path: PathBuf, bytes: Vec<u8> },
    /// `fsync` of a file.
    SyncAll(PathBuf),
    /// `fdatasync` of a file.
    SyncData(PathBuf),
    /// A rename over the target.
    Rename { from: PathBuf, to: PathBuf },
    /// A file removed.
    Remove(PathBuf),
    /// `fsync` of a directory.
    SyncDir(PathBuf),
}

#[cfg(test)]
thread_local! {
    static STEPS: std::cell::RefCell<Option<Vec<Step>>> = const { std::cell::RefCell::new(None) };
    static CRASH: std::cell::RefCell<Option<CrashAt>> = const { std::cell::RefCell::new(None) };
}

/// Where [`crashing_at`] stops the process, as a test names it.
#[cfg(test)]
type CrashAt = Box<dyn Fn(&Step) -> bool>;

/// What [`crashing_at`] unwinds with, so it can tell its own stop from a
/// real panic in the code under test.
#[cfg(test)]
struct Crashed;

/// Run `f`, stopping it dead right after the first step `at` matches, the
/// way a kill or a power cut would: that step is on disk and nothing after
/// it runs. `Err(())` when it stopped there, `Ok` with what `f` returned when
/// no step matched.
///
/// The stop is an unwind that no panic hook sees, so a test prints nothing
/// for it, and code that runs while it unwinds can ask
/// [`std::thread::panicking`] to behave as a dead process would: a guard
/// that would clean up on an ordinary error does nothing. A real panic in
/// `f` is not swallowed; it carries on up. Deterministic: the stop is chosen
/// by the step, never by timing (STD-04, STD-03 §R9's fault injection).
#[cfg(test)]
pub(crate) fn crashing_at<T>(
    at: impl Fn(&Step) -> bool + 'static,
    f: impl FnOnce() -> T,
) -> std::result::Result<T, ()> {
    CRASH.with(|crash| *crash.borrow_mut() = Some(Box::new(at)));
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CRASH.with(|crash| crash.borrow_mut().take());
    match out {
        Ok(out) => Ok(out),
        Err(payload) if payload.is::<Crashed>() => Err(()),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Run `f` and return what it returned with the steps it took, in order.
#[cfg(test)]
pub(crate) fn recording<T>(f: impl FnOnce() -> T) -> (T, Vec<Step>) {
    STEPS.with(|steps| *steps.borrow_mut() = Some(Vec::new()));
    let out = f();
    let steps = STEPS.with(|steps| steps.borrow_mut().take().unwrap_or_default());
    (out, steps)
}

#[cfg(test)]
fn record(step: Step) {
    let crash = CRASH.with(|crash| {
        let mut crash = crash.borrow_mut();
        // Once only: a stopped process takes no further steps, and the
        // unwind that follows may run code that takes some.
        if crash.as_ref().is_some_and(|at| at(&step)) {
            crash.take()
        } else {
            None
        }
    });
    STEPS.with(|steps| {
        if let Some(steps) = steps.borrow_mut().as_mut() {
            steps.push(step);
        }
    });
    if crash.is_some() {
        std::panic::resume_unwind(Box::new(Crashed));
    }
}
