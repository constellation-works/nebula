//! Corpus configuration.
//!
//! `config.yaml` at the corpus root holds exactly two things: which schema
//! the files follow and a stable id for the corpus. Nothing else is
//! configured: tags on the nodes themselves partition the corpus.
//!
//! Private to the crate. A consumer that needs to know the schema version is
//! asking about a corpus, and [`crate::store::Corpus`] is what answers that.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
}

impl Config {
    /// A config for a corpus that has just been created.
    pub(crate) fn fresh(corpus_id: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            corpus_id,
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
            return Ok(Self::fresh(fallback_id()));
        }
        let raw = std::fs::read_to_string(&path)?;
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
        let tmp = root.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, self.render()?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
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
