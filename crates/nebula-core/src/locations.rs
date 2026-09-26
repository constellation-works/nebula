//! Where things are on this machine, as a surface resolved them.
//!
//! Core never reads the process environment or asks the OS for the working
//! directory (STD-02 §R3). Each surface builds one [`Locations`] when it
//! starts, with [`Locations::from_reader`] over its own environment and
//! working directory, and passes it down: every resolver, the [`Corpus`]
//! it opens, and the git children that corpus runs take their answers from
//! it. A test builds one by hand, so it resolves roots against a fixture
//! without touching the process it runs in.
//!
//! The variable names are spelled here and nowhere else in the workspace:
//! a surface hands over a reader, never a name (STD-02 §R24).
//!
//! [`Corpus`]: crate::Corpus

use crate::config::OBSERVATORY_ROOT_ENV;
use crate::error::{Error, Result};
use crate::git::{CEILING_ENV, GitAt};
use crate::store::Corpus;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The home directory: machine settings, `~/.nebula` and kept edits.
const HOME_ENV: &str = "HOME";
/// The shell's name for the working directory.
const PWD_ENV: &str = "PWD";
/// A corpus root named for this shell or process.
const NEBULA_ROOT_ENV: &str = "NEBULA_ROOT";
/// Where the XDG base-directory rules put state: kept edits go under it.
const XDG_STATE_HOME_ENV: &str = "XDG_STATE_HOME";

/// The resolved environment core works from: what the process environment
/// held, and where the process was, when a surface started.
///
/// Every field is optional because every one of them may be missing on a
/// real machine, and what a missing one means is decided where it is used,
/// in one place each: an unset `HOME` is [`Error::HomeUnset`] for a
/// resolver that needs it and no machine setting for one that does not.
/// An empty `NEBULA_ROOT` or `OBSERVATORY_ROOT` counts as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locations {
    /// `$HOME`, as the environment held it.
    pub home: Option<OsString>,
    /// The working directory, as the OS reported it. `None` when there was
    /// none to speak of, as when it had been removed.
    pub cwd: Option<PathBuf>,
    /// `$PWD`, the shell's spelling of the working directory. Used only when
    /// [`Self::working_dir`] accepts it.
    pub pwd: Option<OsString>,
    /// `$NEBULA_ROOT`.
    pub nebula_root: Option<OsString>,
    /// `$OBSERVATORY_ROOT`.
    pub observatory_root: Option<OsString>,
    /// `$XDG_STATE_HOME`.
    pub xdg_state_home: Option<OsString>,
    /// `$GIT_CEILING_DIRECTORIES`: where git's repository discovery stops.
    /// Core decides whether a corpus has a repository the way git would, and
    /// hands every git child this same value, so the two always agree.
    pub git_ceiling_directories: Option<OsString>,
}

impl Locations {
    /// Build the value from an environment `read` by variable name and the
    /// working directory `cwd`. A surface passes a reader over its own
    /// process environment and the working directory the OS reports for it;
    /// a test passes whatever it wants resolved.
    ///
    /// Each variable is read once, here, so a surface that builds this at
    /// start works from one snapshot for the rest of its life.
    pub fn from_reader(
        mut read: impl FnMut(&str) -> Option<OsString>,
        cwd: Option<PathBuf>,
    ) -> Self {
        Self {
            home: read(HOME_ENV),
            cwd,
            pwd: read(PWD_ENV),
            nebula_root: read(NEBULA_ROOT_ENV),
            observatory_root: read(OBSERVATORY_ROOT_ENV),
            xdg_state_home: read(XDG_STATE_HOME_ENV),
            git_ceiling_directories: read(CEILING_ENV),
        }
    }

    /// `$HOME`, refused by what is actually wrong with it: unset, or set to
    /// something that is not UTF-8 (STD-02 §R26).
    pub fn home(&self) -> Result<PathBuf> {
        match &self.home {
            None => Err(Error::HomeUnset),
            Some(home) => home
                .clone()
                .into_string()
                .map(PathBuf::from)
                .map_err(Error::HomeNotUnicode),
        }
    }

    /// The corpus a surface with no `--root` works on: `$NEBULA_ROOT`, else
    /// the corpus the working directory is in, else the configured root,
    /// else `~/.nebula`. [`Corpus::resolve_root`] with no explicit root, for
    /// a surface such as the desktop that has none to give.
    pub fn corpus_root(&self) -> Result<PathBuf> {
        Corpus::resolve_root(self, None)
    }

    /// `$NEBULA_ROOT`, when it names anything.
    pub(crate) fn nebula_root(&self) -> Option<PathBuf> {
        self.nebula_root
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }

    /// `$XDG_STATE_HOME`, when it is absolute: a relative one is ignored, as
    /// the XDG base-directory rules ask.
    pub(crate) fn xdg_state_home(&self) -> Option<PathBuf> {
        self.xdg_state_home
            .as_ref()
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
    }

    /// `path` made absolute against [`Self::cwd`], lexically, as
    /// `std::path::absolute` would against the process's: `.` components
    /// go, `..` stays, and nothing is resolved. As given when it is already
    /// absolute or there is no working directory to join it to.
    pub(crate) fn absolute(&self, path: &Path) -> PathBuf {
        let joined = match &self.cwd {
            Some(cwd) if !path.is_absolute() => cwd.join(path),
            _ => path.to_path_buf(),
        };
        joined
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .collect()
    }

    /// Git run at `root`, with this environment's ceilings.
    pub(crate) fn git_at<'a>(&'a self, root: &'a Path) -> GitAt<'a> {
        GitAt {
            root,
            ceilings: self.git_ceiling_directories.as_deref(),
        }
    }

    /// The working directory as the shell names it.
    ///
    /// [`Self::pwd`] when it is absolute, has no `.` or `..` in it, and is
    /// the same directory as [`Self::cwd`] by identity, which is the rule
    /// `pwd -L` follows; otherwise [`Self::cwd`]. A corpus reached through a
    /// symlink is then found under the spelling the user typed rather than
    /// the one the kernel resolved, because paths are used as given. `None`
    /// when there is no working directory to speak of: there is nothing to
    /// discover from.
    pub fn working_dir(&self) -> Option<PathBuf> {
        let cwd = self.cwd.as_ref()?;
        let logical = self.pwd.as_ref().map(PathBuf::from).filter(|pwd| {
            pwd.is_absolute()
                && !pwd
                    .components()
                    .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
                && same_dir(pwd, cwd)
        });
        Some(logical.unwrap_or_else(|| cwd.clone()))
    }
}

/// Whether two paths name the same directory, by identity rather than by
/// spelling.
#[cfg(unix)]
fn same_dir(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Without inode identity there is no cheap way to tell, so `$PWD` is never
/// trusted and the OS's answer stands.
#[cfg(not(unix))]
fn same_dir(_: &Path, _: &Path) -> bool {
    false
}
