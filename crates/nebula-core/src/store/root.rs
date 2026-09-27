//! Where a corpus is, and the settings this machine keeps beside it.
//!
//! Root resolution (`--root`, `NEBULA_ROOT`, the corpus the working directory
//! is in, `~/.config/nebula/root`, `~/.nebula`), the corpus marker discovery
//! looks for, the machine settings under `~/.config/nebula` and the lock that
//! serializes writes to them, kept edits, and the corpus settings in
//! `config.yaml` that are read or rewritten after open.

use super::Corpus;
use super::inbox::is_inbox_month_filename;
use super::links::is_node_file_name;
use crate::config::{self, CommitSetting, Config, Declared, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::fs_impl::{create_private_dir_all, create_private_new, write_private_atomic};
use crate::id::is_path_safe_id;
use crate::locations::Locations;
use crate::lock::CorpusLock;
use std::path::{Path, PathBuf};
use time::{OffsetDateTime, macros::format_description};

impl Corpus {
    /// Where a corpus would be, given `--root`, else `NEBULA_ROOT`, else the
    /// corpus the working directory is in, else the configured root, else
    /// `~/.nebula`.
    ///
    /// The working directory sits below the two settings that are typed on
    /// purpose and above the two that are standing machine defaults, the way
    /// git finds its repository: standing inside a corpus is itself a choice
    /// of corpus. See [`Self::discover`] for what counts as being inside one.
    ///
    /// Every answer but `--root` comes from `locations`, never from the
    /// process this runs in.
    pub fn resolve_root(locations: &Locations, explicit: Option<PathBuf>) -> Result<PathBuf> {
        if let Some(p) = explicit {
            if p.as_os_str().is_empty() {
                return Err(Error::EmptyRoot);
            }
            return Ok(p);
        }
        if let Some(p) = locations.nebula_root() {
            return Ok(p);
        }
        if let Some(p) = locations.working_dir().and_then(|cwd| Self::discover(&cwd)) {
            return Ok(p);
        }
        if let Some(p) = Self::configured_root(locations)? {
            return Ok(p);
        }
        Self::default_root(locations)
    }

    /// The nearest directory at or above `start` with `nodes/` and positive
    /// corpus evidence; `None` when no ancestor has both. Evidence is either
    /// a `config.yaml` with a top-level `schema_version` or `corpus_id` key
    /// (regardless of value), an unparseable/unreadable config, or, when the
    /// config is absent, a real `inbox/` directory or regular `.lock` file.
    /// The latter markers identify config-less legacy or damaged corpora.
    /// A lone `nodes/`, a parsable foreign config, and config without nodes
    /// fall through to the configured or default corpus.
    ///
    /// Discovery identifies the target, not whether its config is healthy.
    /// Opening validates the config and refuses a missing or malformed one
    /// at this root instead of silently falling through to another corpus.
    /// Migration can therefore find a legacy corpus from before config.yaml.
    ///
    /// The nearest corpus wins. The walk is lexical and never resolves a
    /// symlink: the root keeps the spelling of `start`. With the same evidence,
    /// a `nodes` symlink (even dangling) also stops the walk so opening refuses
    /// it by name. Config symlinks and special files are never read; they stop
    /// discovery as unreadable configs so opening can name the refusal.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        start
            .ancestors()
            .filter(|dir| !dir.as_os_str().is_empty())
            .find(|dir| holds_corpus(dir))
            .map(Path::to_path_buf)
    }

    /// The root used when no command, environment, or machine setting names one.
    pub(crate) fn default_root(locations: &Locations) -> Result<PathBuf> {
        Ok(locations.home()?.join(".nebula"))
    }

    /// The machine-local file that records a non-default corpus root.
    pub fn root_config_path(locations: &Locations) -> Result<PathBuf> {
        Ok(Self::machine_settings_dir(locations)?.join("root"))
    }

    /// Where this machine's settings live: `~/.config/nebula`.
    pub fn machine_settings_dir(locations: &Locations) -> Result<PathBuf> {
        Ok(locations.home()?.join(".config").join("nebula"))
    }

    /// Where an edit that could not be saved is kept:
    /// `$XDG_STATE_HOME/nebula/edits`, else `~/.local/state/nebula/edits`.
    ///
    /// Outside every corpus on purpose. The text is a person's, typed against
    /// a node the save was refused for, so it belongs to this machine and not
    /// to the corpus it would sync or commit into. A relative
    /// `$XDG_STATE_HOME` is ignored, as the XDG base-directory rules ask.
    pub fn kept_edits_dir(locations: &Locations) -> Result<PathBuf> {
        let state = match locations.xdg_state_home() {
            Some(dir) => dir,
            None => locations.home()?.join(".local").join("state"),
        };
        Ok(state.join("nebula").join("edits"))
    }

    /// Keep `text`, typed for node `id` and refused a save, as a new
    /// `<id>-<UTC stamp>.md` owner-only under [`Self::kept_edits_dir`], and
    /// return where it went.
    ///
    /// This is how a refused `neb edit` keeps what the person typed before
    /// its temporary file goes (STD-03 §R30). A file is never replaced: a
    /// second edit kept in the same second takes the next free `-N` suffix.
    pub fn keep_edit(locations: &Locations, id: &str, text: &str) -> Result<PathBuf> {
        /// Names tried before giving up. Only a second refused edit of the
        /// same node in the same second takes a suffix at all.
        const ATTEMPTS: u32 = 16;

        if !is_path_safe_id(id) {
            return Err(Error::UnsafeId(id.to_string()));
        }
        let dir = Self::kept_edits_dir(locations)?;
        create_private_dir_all(&dir)?;
        let stamp = OffsetDateTime::now_utc()
            .format(format_description!(
                "[year][month][day]T[hour][minute][second]Z"
            ))
            .unwrap_or_default();
        for n in 0..ATTEMPTS {
            let name = match n {
                0 => format!("{id}-{stamp}.md"),
                n => format!("{id}-{stamp}-{n}.md"),
            };
            let path = dir.join(name);
            match create_private_new(&path, text) {
                Ok(()) => return Ok(path),
                Err(Error::IoAt { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(Error::NoFreeKeepName {
            id: id.to_string(),
            dir,
            attempts: ATTEMPTS,
        })
    }

    /// Take the lock that serializes writes to this machine's settings,
    /// `~/.config/nebula/.lock`, creating the directory `0700` if needed.
    ///
    /// The same advisory lock a corpus write takes, on a different
    /// directory, so it waits the same bounded time and refuses with
    /// [`Error::Locked`] naming `~/.config/nebula`. A writer that checks a
    /// setting and then writes it holds this across both, so two of them
    /// cannot both pass the check. Taken before any corpus lock, never after
    /// one; see the lock order in `lock.rs`.
    #[must_use = "the machine-setting lock is released when this guard drops"]
    pub fn lock_machine_settings(locations: &Locations) -> Result<CorpusLock> {
        locations.write_gate(crate::locations::WriteIntent::Ordinary)?;
        let dir = Self::machine_settings_dir(locations)?;
        create_private_dir_all(&dir)?;
        CorpusLock::acquire(&dir)
    }

    /// Read the configured corpus root, if this machine has one.
    ///
    /// A setting that is there but fails [`Self::root_setting`] is refused
    /// by name rather than skipped or used: skipping would send every
    /// command to `~/.nebula` without a word, and a relative path would name
    /// a different corpus from each working directory (STD-02 §R28).
    pub(crate) fn configured_root(locations: &Locations) -> Result<Option<PathBuf>> {
        let path = Self::root_config_path(locations)?;
        match read_machine_setting(&path)? {
            Some(raw) => Self::root_setting(&path, &raw).map(Some),
            None => Ok(None),
        }
    }

    /// The one rule for what the corpus root setting at `path` may hold:
    /// a single absolute path, surrounding whitespace aside. Loading the
    /// setting and writing it both go through here, so the file can only be
    /// given what it will later be read as (STD-02 §R24, §R28).
    ///
    /// Pure over its arguments: `path` only names the file in the error.
    pub(crate) fn root_setting(path: &Path, contents: &str) -> Result<PathBuf> {
        let root = contents.trim();
        if root.is_empty() {
            return Err(Error::EmptyRootSetting(path.to_path_buf()));
        }
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            return Err(Error::RelativeRootSetting {
                setting: Some(path.to_path_buf()),
                root,
            });
        }
        Ok(root)
    }

    /// What the corpus root setting holds when it names `root`.
    pub(crate) fn root_setting_contents(root: &Path) -> String {
        format!("{}\n", root.display())
    }

    /// The machine-local file that records where the Observatory checkout
    /// is, beside [`Self::root_config_path`]. A machine setting rather than a
    /// corpus one, because the corpus travels between machines and the
    /// checkout's path does not.
    pub(crate) fn observatory_root_config_path(locations: &Locations) -> Result<PathBuf> {
        Ok(Self::machine_settings_dir(locations)?.join("observatory-root"))
    }

    /// Read this machine's Observatory checkout setting, if it has one.
    ///
    /// A machine with no `HOME` has no machine settings, which is not an
    /// error: `$OBSERVATORY_ROOT` and an explicit `--root` still work there.
    /// A `HOME` that is set but unreadable is not that machine, so it is
    /// refused rather than read as "no setting" (STD-02 §R29). A file that
    /// is present but empty or relative is refused rather than skipped,
    /// because falling through to the legacy key would resolve records
    /// against another machine's path without a word.
    pub(crate) fn configured_observatory_root(locations: &Locations) -> Result<Option<PathBuf>> {
        let path = match Self::observatory_root_config_path(locations) {
            Ok(path) => path,
            Err(Error::HomeUnset) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(raw) = read_machine_setting(&path)? else {
            return Ok(None);
        };
        let root = PathBuf::from(raw.trim());
        if !root.is_absolute() {
            return Err(Error::RelativeObservatoryRoot {
                root,
                setting: Some(path),
            });
        }
        Ok(Some(root))
    }

    /// Make `dir` this machine's Observatory checkout, for every corpus.
    ///
    /// Writes `~/.config/nebula/observatory-root` and nothing under any
    /// corpus. The directory is not required to exist yet; `check` says so
    /// when a reference fails to resolve under it. It must be absolute, since
    /// the setting is read from whatever directory a command runs in.
    pub(crate) fn write_observatory_root_config(
        locations: &Locations,
        dir: &Path,
    ) -> Result<PathBuf> {
        locations.write_gate(crate::locations::WriteIntent::Ordinary)?;
        if !dir.is_absolute() {
            return Err(Error::RelativeObservatoryRoot {
                root: dir.to_path_buf(),
                setting: None,
            });
        }
        let path = Self::observatory_root_config_path(locations)?;
        create_private_dir_all(&Self::machine_settings_dir(locations)?)?;
        write_private_atomic(&path, format!("{}\n", dir.display()))?;
        Ok(path)
    }

    /// The machine-local setting that a new non-default corpus will write.
    pub fn root_config_path_if_absent(
        locations: &Locations,
        root: &Path,
    ) -> Result<Option<PathBuf>> {
        if root == Self::default_root(locations)? {
            return Ok(None);
        }
        let path = Self::root_config_path(locations)?;
        if path.exists() {
            return Ok(None);
        }
        Ok(Some(path))
    }

    /// Make `root` the machine-local default corpus.
    ///
    /// Refuses to replace a different configured root unless `force` is set.
    /// Paths are compared as given, like every other corpus path. The check
    /// and the write happen under [`Self::lock_machine_settings`], and the
    /// file is replaced whole: a symlink at `root` is replaced by a regular
    /// file, and whatever it pointed at is left alone.
    pub(crate) fn write_root_config(
        locations: &Locations,
        root: &Path,
        force: bool,
    ) -> Result<PathBuf> {
        let _lock = Self::lock_machine_settings(locations)?;
        let path = Self::root_config_path(locations)?;
        Self::check_root_config(locations, root, force)?;
        create_private_dir_all(&Self::machine_settings_dir(locations)?)?;
        write_private_atomic(&path, Self::root_setting_contents(root))?;
        Ok(path)
    }

    /// Refuse a conflicting machine-local default before initializing a
    /// corpus. The write is deliberately separate so a refused `--set-root`
    /// cannot create or rewrite the requested corpus first.
    ///
    /// `root` is checked by [`Self::root_setting`] as the contents it would
    /// be written as, the same function that reads it back. A setting already
    /// there that fails that rule names no corpus, since every command
    /// refuses it, so replacing it redirects nothing and needs no `force`:
    /// it is the remedy that refusal suggests.
    pub(crate) fn check_root_config(locations: &Locations, root: &Path, force: bool) -> Result<()> {
        let path = Self::root_config_path(locations)?;
        Self::root_setting(&path, &Self::root_setting_contents(root)).map_err(|mut error| {
            // The same validation rule applies, but this value came from
            // the caller, not from the setting file we have yet to read.
            if let Error::RelativeRootSetting { setting, .. } = &mut error {
                *setting = None;
            }
            error
        })?;
        let configured = match Self::configured_root(locations) {
            Ok(configured) => configured,
            Err(Error::EmptyRootSetting(_) | Error::RelativeRootSetting { .. }) => None,
            Err(error) => return Err(error),
        };
        if let Some(configured) = configured
            && configured != root
            && !force
        {
            return Err(Error::RootConfigConflict {
                path,
                configured,
                requested: root.to_path_buf(),
            });
        }
        Ok(())
    }

    /// A conflicting machine setting is worth naming before creating the
    /// legacy default corpus. The normal resolver cannot reach this state,
    /// but an explicit `--root ~/.nebula` can.
    pub fn warning_before_default_init(
        locations: &Locations,
        root: &Path,
    ) -> Result<Option<PathBuf>> {
        if root != Self::default_root(locations)? || root.exists() {
            return Ok(None);
        }
        Ok(Self::configured_root(locations)?.filter(|configured| configured != root))
    }

    /// A configured root that names a *different* directory than the one
    /// about to be initialized is worth naming even when the target is not
    /// the legacy default: `init` would otherwise succeed silently and every
    /// later command would keep resolving to the old corpus, leaving the new
    /// one orphaned. `None` when there is no configured root, the target
    /// already matches it, or the target is the default (covered by
    /// [`Self::warning_before_default_init`] instead).
    pub fn warning_before_shadowing_init(
        locations: &Locations,
        root: &Path,
    ) -> Result<Option<PathBuf>> {
        if root == Self::default_root(locations)? {
            return Ok(None);
        }
        Ok(Self::configured_root(locations)?.filter(|configured| configured != root))
    }

    /// The config `init` keeps, or `None` when it is to write a fresh one.
    ///
    /// A config that is there is validated before anything is created, so a
    /// malformed or incompatible one is refused without mutation, while a
    /// valid identity and settings survive an interrupted init.
    pub(super) fn config_to_keep(root: &Path) -> Result<Option<Config>> {
        match config::declared(root)? {
            Declared::Current { config, .. } => Ok(Some(config)),
            Declared::Missing if holds_content(root)? => Err(Error::MissingConfig {
                path: root.join(config::FILE),
            }),
            Declared::Missing => Ok(None),
            Declared::Older { version, .. } | Declared::Newer { version } => {
                Err(config::schema_mismatch(root, version))
            }
        }
    }

    /// Where `observatory` references resolve on this machine:
    /// `$OBSERVATORY_ROOT`, else this machine's
    /// `~/.config/nebula/observatory-root`, else the legacy `observatory_root`
    /// key in `config.yaml`, else nowhere. The result names which one
    /// answered, and carries the legacy key whenever the file has one.
    ///
    /// The paths are used as given and never canonicalized, like every other
    /// path here.
    ///
    /// The environment is the one this corpus was opened with and the machine
    /// file is read now; the legacy key is
    /// the snapshot [`Self::open`] took, which is what a read wants: it
    /// answers without waiting on a writer. Fails only on a machine setting
    /// that is unreadable, empty or relative.
    pub fn observatory_root(&self) -> Result<ObservatoryRoot> {
        let env = self
            .locations
            .observatory_root
            .clone()
            .filter(|v| !v.is_empty());
        // The environment outranks the machine file, which is then not read
        // at all: a machine whose file is broken still works while the
        // variable is exported.
        let machine = match env {
            Some(_) => None,
            None => Self::configured_observatory_root(&self.locations)?,
        };
        Ok(ObservatoryRoot::from_settings(
            env,
            machine,
            self.config.legacy_observatory_root().map(Path::to_path_buf),
        ))
    }

    /// Remove the legacy `observatory_root` key from `config.yaml`, returning
    /// the path it held. When the file on disk no longer carries it, nothing
    /// is written and this returns `None`.
    ///
    /// The file stays machine-written: this rewrites it whole, header and
    /// all, rather than editing a line — which is why the reload comes
    /// first: this removes one key and must carry every other key across as
    /// it stands under the caller's lock, not as it stood when the corpus was
    /// opened.
    pub(crate) fn drop_legacy_observatory_root(&mut self) -> Result<Option<PathBuf>> {
        self.reload_config()?;
        let removed = self.config.observatory_root.take();
        if removed.is_some() {
            self.config.save(&self.root)?;
        }
        Ok(removed)
    }

    /// Whether a write is followed by a commit, per `config.yaml`.
    ///
    /// The snapshot [`Self::open`] took, like the legacy key in
    /// [`Self::observatory_root`]: a read of the setting never waits on a
    /// writer.
    pub fn commit_setting(&self) -> CommitSetting {
        CommitSetting {
            enabled: self.config.commit,
        }
    }

    /// Record in `config.yaml` whether writes are committed. Rewrites the
    /// file whole, like every other setting, and so reloads first for the
    /// same reason [`Self::drop_legacy_observatory_root`] does.
    pub(crate) fn set_commit(&mut self, enabled: bool) -> Result<()> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        self.reload_config()?;
        self.config.commit = enabled;
        self.config.save(&self.root)
    }
}

/// Machine settings may be symlinks, but their targets must be regular files.
/// The shared reader checks before opening, opens nonblocking on Unix, then
/// checks the descriptor before reading, including when a target was swapped.
fn read_machine_setting(path: &Path) -> Result<Option<String>> {
    let Some(bytes) = crate::fs_impl::read_regular_bytes(path, crate::fs_impl::Links::Follow)?
    else {
        return Ok(None);
    };
    String::from_utf8(bytes).map(Some).map_err(|error| {
        Error::io_at(
            "reading",
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        )
    })
}

/// Check the nodes directory entry without resolving the corpus root. A root
/// reached through a symlink is supported, but `nodes/` must be a real
/// directory entry so node paths cannot reach a different tree.
pub(crate) fn refuse_nodes_symlink(root: &Path) -> Result<()> {
    let path = root.join("nodes");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Error::NodesSymlink(path)),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io_at("inspecting", &path, error)),
    }
}

/// Whether `root` holds anything a corpus is made of: a node file under
/// `nodes/` or a month file under `inbox/`. What tells a root an init was
/// interrupted in, which is still empty, from a corpus that has lost its
/// config.
fn holds_content(root: &Path) -> Result<bool> {
    for (dir, is_content) in [
        ("nodes", is_node_file_name as fn(&Path) -> bool),
        ("inbox", |path: &Path| {
            path.file_name().is_some_and(is_inbox_month_filename)
        }),
    ] {
        let dir = root.join(dir);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(Error::io_at("reading", &dir, error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| Error::io_at("reading", &dir, error))?;
            if is_content(&entry.path()) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Whether `dir` has both halves of the discovery marker; see `discover`.
fn holds_corpus(dir: &Path) -> bool {
    if !std::fs::symlink_metadata(dir.join("nodes"))
        .is_ok_and(|metadata| metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return false;
    }
    match crate::fs_impl::read_regular_text(&dir.join(config::FILE)) {
        Ok(Some(raw)) => match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&raw) {
            Ok(value) => value.get("schema_version").is_some() || value.get("corpus_id").is_some(),
            // Integrity check: do not redirect a damaged corpus's writes.
            Err(_) => true,
        },
        Ok(None) => {
            std::fs::symlink_metadata(dir.join("inbox")).is_ok_and(|metadata| metadata.is_dir())
                || std::fs::symlink_metadata(dir.join(".lock"))
                    .is_ok_and(|metadata| metadata.is_file())
        }
        // Keep unsafe/unreadable entries at this root for opening to refuse.
        Err(_) => true,
    }
}
