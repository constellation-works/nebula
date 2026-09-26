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
//!   where the caller pointed and nothing outside it changes. An append opens
//!   its file by name, so its caller refuses a symlink there first, as the
//!   inbox does.
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
                return Err(Error::corpus(format!(
                    "atomic write to {} failed: {error}; removing {} failed: {cleanup}",
                    path.display(),
                    tmp.display()
                )));
            }
        }
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

/// Append `bytes` to `path` in one write, then flush the data.
///
/// The whole record goes to a single `write_all` on an `O_APPEND` descriptor,
/// so two appenders cannot interleave inside it and a crash cannot leave the
/// first half of it without the second. A file this call creates is `0600`,
/// and its directory is flushed as well so the new name survives a crash.
pub(crate) fn append_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let at = |error| Error::io_at("writing", path, error);
    let (mut file, created) = match private_open_options()
        .append(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => (file, true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
            private_open_options().append(true).open(path).map_err(at)?,
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
    Err(Error::corpus(format!(
        "no free temporary name beside {} after {TEMPORARY_NAME_ATTEMPTS} tries; \
         something is creating files under them",
        path.display()
    )))
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
    /// `fsync` of a directory.
    SyncDir(PathBuf),
}

#[cfg(test)]
thread_local! {
    static STEPS: std::cell::RefCell<Option<Vec<Step>>> = const { std::cell::RefCell::new(None) };
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
    STEPS.with(|steps| {
        if let Some(steps) = steps.borrow_mut().as_mut() {
            steps.push(step);
        }
    });
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures are written directly so the helper under test is the only thing exercised"
)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_and_its_new_directories_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a").join("b");
        create_private_dir_all(&deep).unwrap();
        let file = deep.join("f");
        write_private_atomic(&file, "x").unwrap();
        assert_eq!(mode(&dir.path().join("a")), PRIVATE_DIR_MODE);
        assert_eq!(mode(&deep), PRIVATE_DIR_MODE);
        assert_eq!(mode(&file), PRIVATE_FILE_MODE);
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_directory_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("mine");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o750)).unwrap();
        create_private_dir_all(&existing.join("new")).unwrap();
        assert_eq!(
            mode(&existing),
            0o750,
            "an existing directory is not chmod'ed"
        );
        assert_eq!(mode(&existing.join("new")), PRIVATE_DIR_MODE);
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_narrows_a_wide_file_and_never_widens_a_narrow_one() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private_atomic(&file, "new").unwrap();
        assert_eq!(mode(&file), PRIVATE_FILE_MODE);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_target_is_replaced_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
        let target = dir.path().join("setting");
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        write_private_atomic(&target, "new").unwrap();

        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "IRREPLACEABLE FIXTURE"
        );
        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn an_append_creates_the_file_and_flushes_its_directory_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("log");
        let (result, first) = recording(|| append_private(&file, b"one\n"));
        result.unwrap();
        let (result, second) = recording(|| append_private(&file, b"two\n"));
        result.unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
        let dir_syncs = |steps: &[Step]| {
            steps
                .iter()
                .filter(|step| matches!(step, Step::SyncDir(_)))
                .count()
        };
        assert_eq!(dir_syncs(&first), usize::from(cfg!(unix)), "{first:?}");
        assert_eq!(dir_syncs(&second), 0, "{second:?}");
        #[cfg(unix)]
        assert_eq!(mode(&file), PRIVATE_FILE_MODE);
    }

    #[test]
    fn recording_is_off_unless_a_test_asks_for_it() {
        let dir = tempfile::tempdir().unwrap();
        write_private_atomic(&dir.path().join("f"), "x").unwrap();
        let ((), steps) = recording(|| ());
        assert!(steps.is_empty(), "{steps:?}");
    }
}
