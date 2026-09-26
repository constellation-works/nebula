//! The migration registry, run in memory: no step here touches a disk or
//! starts a process.

use crate::config::{Config, SCHEMA_VERSION};
use crate::migrate::{MIGRATIONS, Staged, StagedNode};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A v1 `config.yaml` with the keys v2 retired.
const V1_CONFIG: &str =
    "schema_version: 1\ncorpus_id: neb-abc123\ndomains:\n- physics\ndefault_domain: physics\n";

/// A v1 node with keys only v1 had.
const V1_NODE: &str = "---\nid: first\ntitle: First\ndomain: Physics\nstatus: testing\nkill: If it never shows up.\ncreated: 2026-08-01\nupdated: 2026-08-02\ntasks:\n- id: DANI-1\n  why: look\n---\n\nThe first.\n";

/// `config` and `nodes` staged as a migration reads them, at `/corpus`.
fn staged(config: Option<&str>, nodes: &[&str]) -> Staged {
    Staged {
        root: PathBuf::from("/corpus"),
        config: config.map(String::from),
        minted_corpus_id: None,
        nodes: nodes
            .iter()
            .enumerate()
            .map(|(i, text)| StagedNode {
                path: PathBuf::from(format!("/corpus/nodes/{i}.md")),
                original: (*text).to_string(),
                text: (*text).to_string(),
                notes: Vec::new(),
            })
            .collect(),
    }
}

/// Every step from 1 in order, one schema at a time, ending at this build's:
/// a corpus at any shipped schema has exactly one way forward, and it ends
/// where this build reads.
#[test]
fn registry_is_strictly_increasing() {
    let first = MIGRATIONS.first().expect("at least one migration");
    assert_eq!(first.from, 1, "the registry starts at the first schema");
    for migration in MIGRATIONS {
        assert_eq!(migration.to, migration.from + 1, "a step moves one schema");
    }
    for pair in MIGRATIONS.windows(2) {
        assert_eq!(pair[1].from, pair[0].to, "no gap and no overlap");
    }
    assert_eq!(
        MIGRATIONS.last().map(|last| last.to),
        Some(SCHEMA_VERSION),
        "the last step ends at this build's schema"
    );
}

/// The keys and their `schema_version` in a config's text.
fn keys(config: &str) -> (BTreeSet<String>, u64) {
    let mapping: serde_yaml_ng::Mapping = serde_yaml_ng::from_str(config).unwrap();
    let version = mapping
        .get("schema_version")
        .and_then(serde_yaml_ng::Value::as_u64)
        .expect("a schema_version");
    let keys = mapping
        .keys()
        .map(|key| key.as_str().expect("a string key").to_string())
        .collect();
    (keys, version)
}

/// A corpus migrated from v1 ends with the config a corpus created today
/// starts with, whether its v1 config had retired keys or was missing.
#[test]
fn fresh_and_migrated_configs_match() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");
    crate::store::Corpus::init(&root).unwrap();
    let fresh = std::fs::read_to_string(root.join(crate::config::FILE)).unwrap();
    let expected = keys(&fresh);
    assert_eq!(expected.1, u64::from(SCHEMA_VERSION));

    for config in [Some(V1_CONFIG), None] {
        let mut corpus = staged(config, &[V1_NODE]);
        for migration in MIGRATIONS {
            (migration.step)(&mut corpus).unwrap();
        }
        let migrated = corpus.config.expect("a migrated config");
        assert_eq!(keys(&migrated), expected, "{config:?} became {migrated}");
        let parsed: Config = serde_yaml_ng::from_str(&migrated).unwrap();
        assert_eq!(
            corpus.minted_corpus_id.is_some(),
            config.is_none(),
            "an id is minted exactly when there was none"
        );
        if config.is_some() {
            assert_eq!(parsed.corpus_id, "neb-abc123");
        }
    }
}

/// A rerun after an interrupted write applies the steps again to nodes
/// already written, so a step applied to its own output changes nothing.
#[test]
fn every_step_leaves_its_own_output_unchanged() {
    let mut corpus = staged(Some(V1_CONFIG), &[V1_NODE]);
    for migration in MIGRATIONS {
        (migration.step)(&mut corpus).unwrap();
        let once = (corpus.config.clone(), corpus.nodes[0].text.clone());
        assert_ne!(once.1, V1_NODE, "the fixture needs converting");
        (migration.step)(&mut corpus).unwrap();
        assert_eq!(
            (corpus.config.clone(), corpus.nodes[0].text.clone()),
            once,
            "step {} -> {}",
            migration.from,
            migration.to
        );
    }
}
