//! Corpus configuration.
//!
//! `config.yaml` at the corpus root holds which schema the files follow, a
//! stable id for the corpus, whether `neb` commits the corpus after each
//! write, and, when one has been set, where the Observatory checkout is.
//! Nothing about the ideas is configured: tags on the nodes themselves
//! partition the corpus.
//!
//! Private to the crate. A consumer that needs to know the schema version is
//! asking about a corpus, and [`crate::store::Corpus`] is what answers that.

use crate::error::{Error, Result};
use crate::store::write_atomic;
use serde::{Deserialize, Serialize};
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
    /// Where the Observatory checkout is, so an `observatory` reference's
    /// bare record id resolves to a file on this machine. Written by
    /// `neb config observatory-root`, absent until then; `$OBSERVATORY_ROOT`
    /// stands in when it is absent.
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
    /// Which of the two settings supplied it.
    pub source: ObservatorySource,
}

/// Where an observatory root came from. `config.yaml` wins over the
/// environment, because it was set on purpose for this corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "lowercase")]
pub enum ObservatorySource {
    /// `observatory_root` in `config.yaml`.
    Config,
    /// `$OBSERVATORY_ROOT`.
    Env,
    /// Neither is set.
    Unset,
}

/// The environment variable that stands in for `observatory_root`.
pub const OBSERVATORY_ROOT_ENV: &str = "OBSERVATORY_ROOT";

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

    /// The observatory root this config names, else the environment's.
    pub(crate) fn observatory_root(&self) -> ObservatoryRoot {
        if let Some(root) = &self.observatory_root {
            return ObservatoryRoot {
                root: Some(root.clone()),
                source: ObservatorySource::Config,
            };
        }
        match std::env::var_os(OBSERVATORY_ROOT_ENV).filter(|v| !v.is_empty()) {
            Some(v) => ObservatoryRoot {
                root: Some(PathBuf::from(v)),
                source: ObservatorySource::Env,
            },
            None => ObservatoryRoot {
                root: None,
                source: ObservatorySource::Unset,
            },
        }
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
