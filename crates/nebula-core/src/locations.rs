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

use crate::error::{Error, Result};
use crate::git::GitAt;
use crate::store::Corpus;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// One environment variable read by nebula, documented once for code and
/// usage docs (STD-01 §R25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvVar {
    /// Variable name in the process environment.
    pub name: &'static str,
    /// What it controls and how empty or invalid values behave.
    pub meaning: &'static str,
    /// Behavior when unset (or empty, where empty counts as unset).
    pub default: &'static str,
    /// CLI flag that overrides this variable, if any.
    pub overridden_by: Option<&'static str>,
}

/// Home directory used for `~/.config/nebula` machine settings, the `~/.nebula` default corpus, and the `~/.local/state/nebula/edits` kept-edits fallback.
///
/// Meaning: locates machine-local settings and default user directories; unset or non-UTF-8 fails when needed.
/// Default: none (required when resolving machine settings, default root, or kept-edits fallback).
/// Overridden flag: none.
pub const HOME_ENV: &str = "HOME";

/// Shell's logical working directory spelling, used when absolute, free of `.`/`..`, and matching the OS working directory by identity so symlinked corpora keep their typed path.
///
/// Meaning: preserves symlink spellings during working-directory corpus discovery.
/// Default: falls back to the OS current working directory.
/// Overridden flag: none (`--root` skips working-directory discovery).
pub const PWD_ENV: &str = "PWD";

/// Corpus root directory path for this shell or process.
///
/// Meaning: selects the corpus root ahead of working-directory discovery; an empty value counts as unset.
/// Default: nearest corpus at or above the working directory, else `~/.config/nebula/root`, else `~/.nebula`.
/// Overridden flag: `--root <DIR>`.
pub const NEBULA_ROOT_ENV: &str = "NEBULA_ROOT";

/// Path to the Observatory checkout for resolving `observatory` references (`cite --kind observatory`, `handoff`, `check`).
///
/// Meaning: names the Observatory checkout ahead of any stored setting; an empty value counts as unset.
/// Default: `~/.config/nebula/observatory-root`, else legacy `observatory_root` in `config.yaml`, else unset.
/// Overridden flag: none (`neb config observatory-root <DIR>` sets only the machine fallback).
pub const OBSERVATORY_ROOT_ENV: &str = "OBSERVATORY_ROOT";

/// Base state directory where refused `neb edit` text is saved under `$XDG_STATE_HOME/nebula/edits`; relative paths are ignored.
///
/// Meaning: overrides the XDG state root for preserved edits when set to an absolute path.
/// Default: `$HOME/.local/state`.
/// Overridden flag: none.
pub const XDG_STATE_HOME_ENV: &str = "XDG_STATE_HOME";

/// Colon-separated directories where `git` repository discovery stops, applied both to `neb`'s repository check and passed to supervised `git` child processes.
///
/// Meaning: stops parent `.git` discovery at matching ancestors.
/// Default: unset (no discovery ceiling).
/// Overridden flag: none.
pub const GIT_CEILING_DIRECTORIES_ENV: &str = "GIT_CEILING_DIRECTORIES";

/// Orbit run identifier recorded as provenance on `new`, `promote`, `cite`, and `handoff`.
///
/// Meaning: supplies missing `origin.run` provenance; when non-empty, authored writes require explicit `--by` and `sharpen --confirm` is refused. Empty counts as unset; non-UTF-8 is refused.
/// Default: unset (no run provenance; human-authored defaults allowed).
/// Overridden flag: `--run <RUN>` for provenance; the write gate has no flag override.
pub const ORBIT_RUN_ID_ENV: &str = "ORBIT_RUN_ID";

/// Orbit task identifier recorded as provenance on `new`, `promote`, `cite`, and `handoff`.
///
/// Meaning: supplies missing `origin.task` provenance; empty counts as unset, and non-UTF-8 is refused.
/// Default: unset (no task provenance).
/// Overridden flag: `--task <TASK>`.
pub const ORBIT_TASK_ID_ENV: &str = "ORBIT_TASK_ID";

/// Read-only guard: set to `1` to refuse all corpus and machine-setting writes while keeping reads available.
///
/// Meaning: refuses every mutating command when `1`; any other non-empty value is refused on startup.
/// Default: unset or empty (writes enabled).
/// Overridden flag: none.
pub const NEBULA_READ_ONLY_ENV: &str = "NEBULA_READ_ONLY";

/// Registry of every environment variable read by `nebula-core` via [`Locations`].
pub const ENV_VARS: &[EnvVar] = &[
    EnvVar {
        name: NEBULA_ROOT_ENV,
        meaning: "Corpus root directory path ahead of working-directory discovery; empty counts as unset.",
        default: "Nearest corpus above cwd, else `~/.config/nebula/root`, else `~/.nebula`",
        overridden_by: Some("--root <DIR>"),
    },
    EnvVar {
        name: OBSERVATORY_ROOT_ENV,
        meaning: "Path to the Observatory checkout for resolving `observatory` references; empty counts as unset.",
        default: "`~/.config/nebula/observatory-root`, else legacy `config.yaml` key, else unset",
        overridden_by: None,
    },
    EnvVar {
        name: ORBIT_TASK_ID_ENV,
        meaning: "Orbit task identifier recorded as `origin.task` on `new`, `promote`, `cite`, and `handoff`; empty counts as unset.",
        default: "Unset (no task provenance)",
        overridden_by: Some("--task <TASK>"),
    },
    EnvVar {
        name: ORBIT_RUN_ID_ENV,
        meaning: "Orbit run identifier recorded as `origin.run`; when non-empty, authored writes require `--by` and `sharpen --confirm` is refused.",
        default: "Unset (no run provenance)",
        overridden_by: Some("--run <RUN>"),
    },
    EnvVar {
        name: NEBULA_READ_ONLY_ENV,
        meaning: "Set to `1` to refuse all corpus and machine-setting writes while keeping reads available; other non-empty values are refused.",
        default: "Unset or empty (writes enabled)",
        overridden_by: None,
    },
    EnvVar {
        name: GIT_CEILING_DIRECTORIES_ENV,
        meaning: "Colon-separated directories where `git` repository discovery stops for both `neb` and supervised `git` children.",
        default: "Unset (no discovery ceiling)",
        overridden_by: None,
    },
    EnvVar {
        name: HOME_ENV,
        meaning: "Home directory for `~/.config/nebula` settings, `~/.nebula` default corpus, and `~/.local/state/nebula/edits`.",
        default: "None (required when machine paths are needed)",
        overridden_by: None,
    },
    EnvVar {
        name: PWD_ENV,
        meaning: "Shell's logical working directory spelling, used when absolute, clean, and matching the OS cwd by identity.",
        default: "OS current working directory",
        overridden_by: None,
    },
    EnvVar {
        name: XDG_STATE_HOME_ENV,
        meaning: "Base state directory for kept edits (`$XDG_STATE_HOME/nebula/edits`); relative paths are ignored.",
        default: "`$HOME/.local/state`",
        overridden_by: None,
    },
];

/// The kind of write being checked at the one resolved-environment gate.
#[derive(Debug, Clone, Copy)]
pub enum WriteIntent<'a> {
    /// A write with no new authored words.
    Ordinary,
    /// A write of words attributed to the supplied author.
    Authored(Option<&'a str>),
    /// Adoption of a kill condition as the human's own.
    ConfirmKill,
}

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
    /// Orbit run associated with this invocation.
    pub orbit_run_id: Option<OsString>,
    /// Orbit task associated with this invocation.
    pub orbit_task_id: Option<OsString>,
    /// The optional, validated read-only switch.
    pub nebula_read_only: Option<OsString>,
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
            git_ceiling_directories: read(GIT_CEILING_DIRECTORIES_ENV),
            orbit_run_id: read(ORBIT_RUN_ID_ENV),
            orbit_task_id: read(ORBIT_TASK_ID_ENV),
            nebula_read_only: read(NEBULA_READ_ONLY_ENV),
        }
    }

    /// Validate process policy before dispatch, including read-only commands.
    pub fn validate(&self) -> Result<()> {
        match self.nebula_read_only.as_deref() {
            None => Ok(()),
            Some(v) if v.is_empty() => Ok(()),
            Some(v) if v == "1" => Ok(()),
            Some(_) => Err(Error::InvalidReadOnlyEnvironment {
                name: NEBULA_READ_ONLY_ENV,
            }),
        }
    }

    /// Check a write before its first side effect. Every writer uses this
    /// gate, either directly or through the corpus lock.
    pub fn write_gate(&self, intent: WriteIntent<'_>) -> Result<()> {
        self.validate()?;
        if self.nebula_read_only.as_deref() == Some(std::ffi::OsStr::new("1")) {
            return Err(Error::ReadOnly);
        }
        if self.orbit_run_id.as_ref().is_some_and(|v| !v.is_empty()) {
            match intent {
                WriteIntent::Authored(None) => return Err(Error::ByRequired),
                WriteIntent::Authored(Some(by)) if by.trim().is_empty() => {
                    return Err(Error::ByRequired);
                }
                WriteIntent::Ordinary | WriteIntent::Authored(Some(_)) => {}
                WriteIntent::ConfirmKill => return Err(Error::HumanOnly),
            }
        }
        Ok(())
    }

    /// Fill the absent halves of provenance from this invocation's snapshot.
    pub fn origin(
        &self,
        task: Option<String>,
        run: Option<String>,
    ) -> Result<Option<crate::Origin>> {
        let value = |v: &Option<OsString>, name| {
            v.as_ref()
                .filter(|v| !v.is_empty())
                .map(|v| {
                    v.clone()
                        .into_string()
                        .map_err(|_| Error::InvalidOriginEnvironment { name })
                })
                .transpose()
        };
        let task = match task {
            Some(task) => Some(task),
            None => value(&self.orbit_task_id, ORBIT_TASK_ID_ENV)?,
        };
        let run = match run {
            Some(run) => Some(run),
            None => value(&self.orbit_run_id, ORBIT_RUN_ID_ENV)?,
        };
        Ok(crate::Origin::of(task, run))
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

    /// `path` made absolute against the shell's working directory when it is
    /// valid, then lexically normalized through its components: repeated
    /// separators and `.` components collapse, `..` stays, and symlinks are
    /// not resolved. The CLI uses this spelling when it names a corpus.
    pub fn absolute(&self, path: &Path) -> PathBuf {
        let joined = match self.working_dir() {
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
    use crate::fs_impl::{Links, metadata};
    use std::os::unix::fs::MetadataExt;
    match (metadata(a, Links::Follow), metadata(b, Links::Follow)) {
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
