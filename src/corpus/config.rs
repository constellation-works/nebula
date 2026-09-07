//! Corpus configuration.
//!
//! `config.yaml` at the corpus root declares the domains a corpus is
//! partitioned into and which one bare commands scope to. A domain is a view
//! inside one corpus, not a boundary between corpora: edges cross domains
//! freely, because a ranking observation feeding a physics hypothesis is the
//! kind of link the graph exists to find. The boundary that does need separate
//! storage, work against personal, is a separate corpus with its own config.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// File name at the corpus root.
pub const FILE: &str = "config.yaml";

/// The domain a fresh corpus starts with.
pub const INITIAL_DOMAIN: &str = "general";

/// What `config.yaml` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Bumped when the file's shape changes.
    pub schema_version: u32,
    /// Stable, opaque, never edited by hand.
    pub corpus_id: String,
    /// Every domain a node may belong to. Closed set: a node naming anything
    /// else fails `check`, which is what keeps `principia` from drifting into
    /// `Principia` and `physics` over a year of typing.
    pub domains: Vec<String>,
    /// Where bare `list` and `open` look when more than one domain exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_domain: Option<String>,
}

impl Config {
    /// A config for a corpus that has just been created.
    pub fn fresh(corpus_id: String) -> Self {
        Self {
            schema_version: 1,
            corpus_id,
            domains: vec![INITIAL_DOMAIN.to_string()],
            default_domain: Some(INITIAL_DOMAIN.to_string()),
        }
    }

    /// Read the config, or synthesize one for a corpus that predates it.
    ///
    /// A missing file is not an error: corpora created before domains existed
    /// must keep loading, and `check` will tell them what to fix.
    pub fn load(root: &Path, fallback_id: impl FnOnce() -> String) -> Result<Self> {
        let path = root.join(FILE);
        if !path.exists() {
            return Ok(Self::fresh(fallback_id()));
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Self =
            serde_yaml_ng::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        if cfg.schema_version != 1 {
            bail!(
                "{} is schema_version {}, and this build understands 1",
                path.display(),
                cfg.schema_version
            );
        }
        if cfg.domains.is_empty() {
            bail!("{} declares no domains", path.display());
        }
        if let Some(d) = &cfg.default_domain
            && !cfg.has(d)
        {
            bail!(
                "{} names default domain `{d}`, which is not declared",
                path.display()
            );
        }
        Ok(cfg)
    }

    /// Write the config atomically.
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(FILE);
        let tmp = root.join(format!("{FILE}.tmp"));
        let mut out = String::from("# nebula corpus configuration. `neb domain` edits this.\n");
        out.push_str(&serde_yaml_ng::to_string(self)?);
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Whether a domain is declared.
    pub fn has(&self, domain: &str) -> bool {
        self.domains.iter().any(|d| d == domain)
    }

    /// Fail unless a domain is declared, naming the alternatives.
    pub fn require(&self, domain: &str) -> Result<()> {
        if self.has(domain) {
            return Ok(());
        }
        bail!(
            "no domain `{domain}`; declared: {}\n\nAdd one with:  neb domain add {domain}",
            self.domains.join(", ")
        )
    }

    /// The domain a new node lands in when none was named.
    ///
    /// One declared domain is unambiguous. Several with a default is the
    /// common case. Several without a default is a genuine decision, and the
    /// error says so rather than guessing.
    pub fn resolve_new(&self, explicit: Option<&str>) -> Result<String> {
        if let Some(d) = explicit {
            self.require(d)?;
            return Ok(d.to_string());
        }
        if let [only] = self.domains.as_slice() {
            return Ok(only.clone());
        }
        if let Some(d) = &self.default_domain {
            return Ok(d.clone());
        }
        bail!(
            "several domains and no default; pass --domain or set one with:  neb domain default <name>\n\ndeclared: {}",
            self.domains.join(", ")
        )
    }

    /// The domain a read command scopes to: `None` means the whole corpus.
    ///
    /// Bare commands narrow to the default domain only when there is more
    /// than one, so a single-domain corpus never sees the machinery.
    pub fn resolve_view(&self, explicit: Option<&str>, all: bool) -> Result<Option<String>> {
        if all {
            return Ok(None);
        }
        if let Some(d) = explicit {
            self.require(d)?;
            return Ok(Some(d.to_string()));
        }
        if self.domains.len() > 1 {
            return Ok(self.default_domain.clone());
        }
        Ok(None)
    }
}

/// A domain name is a slug: what fits in a filename, a flag, and a tag.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !ok {
        bail!("domain `{name}` must be lowercase letters, digits and dashes");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(domains: &[&str], default: Option<&str>) -> Config {
        Config {
            schema_version: 1,
            corpus_id: "neb-test".into(),
            domains: domains.iter().map(ToString::to_string).collect(),
            default_domain: default.map(String::from),
        }
    }

    #[test]
    fn one_domain_needs_no_decision() {
        let c = cfg(&["general"], None);
        assert_eq!(c.resolve_new(None).unwrap(), "general");
        assert_eq!(c.resolve_view(None, false).unwrap(), None);
    }

    #[test]
    fn several_domains_without_a_default_is_a_decision() {
        let c = cfg(&["work", "personal"], None);
        assert!(c.resolve_new(None).is_err());
        assert_eq!(c.resolve_new(Some("work")).unwrap(), "work");
        assert!(c.resolve_new(Some("nope")).is_err());
        assert_eq!(c.resolve_view(None, false).unwrap(), None);
    }

    #[test]
    fn a_default_scopes_reads_and_all_crosses() {
        let c = cfg(&["work", "personal"], Some("personal"));
        assert_eq!(c.resolve_new(None).unwrap(), "personal");
        assert_eq!(
            c.resolve_view(None, false).unwrap().as_deref(),
            Some("personal")
        );
        assert_eq!(c.resolve_view(None, true).unwrap(), None);
        assert_eq!(
            c.resolve_view(Some("work"), false).unwrap().as_deref(),
            Some("work")
        );
    }

    #[test]
    fn names_are_slugs() {
        assert!(validate_name("principia").is_ok());
        assert!(validate_name("ranking-signals").is_ok());
        assert!(validate_name("Principia").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("-x").is_err());
    }
}
