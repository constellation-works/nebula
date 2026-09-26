//! Corpus configuration.
//!
//! `config.yaml` at the corpus root holds which schema the files follow, a
//! stable id for the corpus, whether `neb` commits the corpus after each
//! write, and, in a corpus written before it moved out, where the
//! Observatory checkout is. Nothing about the ideas is configured: tags on
//! the nodes themselves partition the corpus.
//!
//! Where the Observatory checkout is belongs to the machine, not the corpus,
//! which travels between machines: `$OBSERVATORY_ROOT`, else this machine's
//! `~/.config/nebula/observatory-root`, else, as a legacy fallback, the
//! `observatory_root` key an older `neb` wrote into `config.yaml`.
//!
//! Private to the crate. A consumer that needs to know the schema version is
//! asking about a corpus, and [`crate::store::Corpus`] is what answers that.

use crate::error::{Error, Result};
use crate::store::write_atomic;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// File name at the corpus root.
pub(crate) const FILE: &str = "config.yaml";

/// The schema this build reads and writes.
pub const SCHEMA_VERSION: u32 = 2;

/// What `config.yaml` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// Bumped when the file's shape changes.
    pub(crate) schema_version: u32,
    /// Stable, opaque, never edited by hand.
    pub(crate) corpus_id: String,
    /// Where the Observatory checkout was on whichever machine wrote it.
    /// Legacy: an older `neb config observatory-root` wrote it here, but the
    /// file travels with the corpus and the path does not travel with it.
    /// Still read, as the last fallback, so such a corpus keeps resolving;
    /// never written, only removed (`neb config observatory-root
    /// --drop-legacy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) observatory_root: Option<PathBuf>,
    /// Whether a mutating verb commits the corpus afterwards, when the root
    /// is inside a git work tree. Off by default and absent from the file
    /// until `neb config commit on` writes it, so a corpus written before
    /// the setting existed renders back byte for byte.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) commit: bool,
}

/// Whether the corpus is committed after each write, as `neb config commit`
/// reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CommitSetting {
    /// `commit` in `config.yaml`. `false` when the key is absent.
    pub enabled: bool,
}

/// Where `observatory` references resolve, and which setting said so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ObservatoryRoot {
    /// The Observatory checkout, as it was given. `None` when nothing set it.
    pub root: Option<PathBuf>,
    /// Which setting supplied it.
    pub source: ObservatorySource,
    /// The legacy `observatory_root` key in `config.yaml`, whenever the file
    /// still carries one, in effect or not: it is one machine's path in a
    /// file every machine shares, so it is worth naming even where a machine
    /// setting outranks it.
    pub legacy: Option<PathBuf>,
}

/// Where an observatory root came from, in the order the settings win.
/// `Env` and `Machine` are about this machine; `Config` is a path some
/// machine once wrote into the corpus, so it answers only when neither does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum ObservatorySource {
    /// `$OBSERVATORY_ROOT`.
    Env,
    /// This machine's `~/.config/nebula/observatory-root`, which
    /// `neb config observatory-root <DIR>` writes.
    Machine,
    /// The legacy `observatory_root` key in `config.yaml`.
    Config,
    /// None of them is set.
    Unset,
}

/// The environment variable that names the Observatory checkout, ahead of
/// every stored setting.
pub const OBSERVATORY_ROOT_ENV: &str = "OBSERVATORY_ROOT";

impl ObservatoryRoot {
    /// The effective root from each setting that could name one: the
    /// environment's value (empty counts as unset), this machine's setting,
    /// and the legacy key in `config.yaml`, which is carried along whether or
    /// not it wins.
    pub(crate) fn from_settings(
        env: Option<OsString>,
        machine: Option<PathBuf>,
        legacy: Option<PathBuf>,
    ) -> Self {
        let (root, source) = match (env.filter(|v| !v.is_empty()), machine, &legacy) {
            (Some(env), _, _) => (Some(PathBuf::from(env)), ObservatorySource::Env),
            (None, Some(machine), _) => (Some(machine), ObservatorySource::Machine),
            (None, None, Some(legacy)) => (Some(legacy.clone()), ObservatorySource::Config),
            (None, None, None) => (None, ObservatorySource::Unset),
        };
        Self {
            root,
            source,
            legacy,
        }
    }

    /// Where Observatory record `record` (`Q002`, `R012`) is under the
    /// effective root. `None` when no root is set, the id is not a record id,
    /// or the checkout does not carry it; [`Self::root`] tells the first
    /// apart from the others.
    pub fn resolve(&self, record: &str) -> Option<PathBuf> {
        self.root
            .as_deref()
            .and_then(|root| crate::check::resolve_observatory(root, record))
    }
}

impl Config {
    /// A config for a corpus that has just been created.
    pub(crate) fn fresh(corpus_id: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            corpus_id,
            observatory_root: None,
            commit: false,
        }
    }

    /// The legacy `observatory_root` key, when the file carries one.
    pub(crate) fn legacy_observatory_root(&self) -> Option<&Path> {
        self.observatory_root.as_deref()
    }

    /// Read the config, or synthesize one for a corpus that predates it.
    ///
    /// A missing file is not an error: corpora created before the config
    /// existed must keep loading. A file at an older schema is, and the
    /// error says which schema was found.
    pub(crate) fn load(root: &Path, fallback_id: impl FnOnce() -> String) -> Result<Self> {
        let path = root.join(FILE);
        if !path.exists() {
            let config = Self::fresh(fallback_id());
            config.save(root)?;
            return Ok(config);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| Error::io_at("reading", &path, error))?;
        // Probe the version before the strict parse, so a v1 file with its
        // extra keys gets the migrate hint rather than an unknown-field error.
        let version = schema_version_of(&raw)
            .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))?
            .unwrap_or(1);
        if version != SCHEMA_VERSION {
            return Err(Error::SchemaMismatch {
                path,
                found: version,
                expected: SCHEMA_VERSION,
            });
        }
        serde_yaml_ng::from_str(&raw)
            .map_err(|e| Error::yaml(format!("parsing {}", path.display()), e))
    }

    /// Write the config atomically.
    pub(crate) fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(FILE);
        write_atomic(&path, self.render()?)
    }

    /// The file's text, so a writer can compare before rewriting.
    pub(crate) fn render(&self) -> Result<String> {
        let mut out = String::from("# nebula corpus configuration. Not edited by hand.\n");
        let body =
            serde_yaml_ng::to_string(self).map_err(|e| Error::yaml("rendering config.yaml", e))?;
        out.push_str(&body);
        Ok(out)
    }
}

/// The `schema_version` of a config file of any vintage, ignoring every
/// other key. `None` when the key is absent.
fn schema_version_of(raw: &str) -> std::result::Result<Option<u32>, serde_yaml_ng::Error> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        schema_version: Option<u32>,
    }
    let probe: Probe = serde_yaml_ng::from_str(raw)?;
    Ok(probe.schema_version)
}

#[cfg(test)]
mod tests {
    use super::{ObservatoryRoot, ObservatorySource};
    use std::path::PathBuf;

    fn pick(env: Option<&str>, machine: Option<&str>, legacy: Option<&str>) -> ObservatoryRoot {
        ObservatoryRoot::from_settings(
            env.map(Into::into),
            machine.map(PathBuf::from),
            legacy.map(PathBuf::from),
        )
    }

    #[test]
    fn the_environment_outranks_the_machine_setting_which_outranks_the_legacy_key() {
        let all = pick(Some("/env"), Some("/machine"), Some("/legacy"));
        assert_eq!(all.root, Some(PathBuf::from("/env")));
        assert_eq!(all.source, ObservatorySource::Env);
        assert_eq!(all.legacy, Some(PathBuf::from("/legacy")));

        let machine = pick(None, Some("/machine"), Some("/legacy"));
        assert_eq!(machine.root, Some(PathBuf::from("/machine")));
        assert_eq!(machine.source, ObservatorySource::Machine);
        assert_eq!(machine.legacy, Some(PathBuf::from("/legacy")));

        let legacy = pick(None, None, Some("/legacy"));
        assert_eq!(legacy.root, Some(PathBuf::from("/legacy")));
        assert_eq!(legacy.source, ObservatorySource::Config);

        let unset = pick(None, None, None);
        assert_eq!(unset.root, None);
        assert_eq!(unset.source, ObservatorySource::Unset);
        assert_eq!(unset.legacy, None);
    }

    #[test]
    fn an_empty_environment_value_is_no_value() {
        let setting = pick(Some(""), Some("/machine"), None);
        assert_eq!(setting.root, Some(PathBuf::from("/machine")));
        assert_eq!(setting.source, ObservatorySource::Machine);
    }

    #[test]
    fn with_no_root_nothing_resolves() {
        assert_eq!(pick(None, None, None).resolve("Q002"), None);
    }
}
