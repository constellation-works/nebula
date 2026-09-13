//! Corpus configuration.
//!
//! `config.yaml` at the corpus root holds exactly two things: which schema
//! the files follow and a stable id for the corpus. Nothing else is
//! configured: tags on the nodes themselves partition the corpus.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// File name at the corpus root.
pub const FILE: &str = "config.yaml";

/// The schema this build reads and writes.
pub const SCHEMA_VERSION: u32 = 2;

/// What `config.yaml` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Bumped when the file's shape changes.
    pub schema_version: u32,
    /// Stable, opaque, never edited by hand.
    pub corpus_id: String,
}

impl Config {
    /// A config for a corpus that has just been created.
    pub fn fresh(corpus_id: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            corpus_id,
        }
    }

    /// Read the config, or synthesize one for a corpus that predates it.
    ///
    /// A missing file is not an error: corpora created before the config
    /// existed must keep loading. A file at an older schema is, and the
    /// error names the command that brings it forward.
    pub fn load(root: &Path, fallback_id: impl FnOnce() -> String) -> Result<Self> {
        let path = root.join(FILE);
        if !path.exists() {
            return Ok(Self::fresh(fallback_id()));
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        // Probe the version before the strict parse, so a v1 file with its
        // extra keys gets the migrate hint rather than an unknown-field error.
        let version = schema_version_of(&raw)
            .with_context(|| format!("parsing {}", path.display()))?
            .unwrap_or(1);
        if version != SCHEMA_VERSION {
            bail!(
                "{} is schema_version {version}, and this build understands {SCHEMA_VERSION}\n\n\
                 Bring the corpus forward with:  neb migrate",
                path.display()
            );
        }
        serde_yaml_ng::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write the config atomically.
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(FILE);
        let tmp = root.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, self.render()?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The file's text, so a writer can compare before rewriting.
    pub fn render(&self) -> Result<String> {
        let mut out = String::from("# nebula corpus configuration. Not edited by hand.\n");
        out.push_str(&serde_yaml_ng::to_string(self)?);
        Ok(out)
    }
}

/// The `schema_version` of a config file of any vintage, ignoring every
/// other key. `None` when the key is absent.
pub fn schema_version_of(raw: &str) -> Result<Option<u32>> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        schema_version: Option<u32>,
    }
    let probe: Probe = serde_yaml_ng::from_str(raw)?;
    Ok(probe.schema_version)
}
