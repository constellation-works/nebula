//! `neb migrate` from v1: relabelled evidence, tasks and edges, mapped
//! statuses, and a second run that changes nothing.

use crate::harness::{Corpus, git, snapshot_corpus_files, write};

/// A corpus in v1 form: declared domains, evidence with verdicts and
/// strengths, a task link, a weighed reference, the removed edge kinds, and
/// the statuses v2 collapses. Written by hand so the fixture is exactly what
/// v0.1 wrote and nothing in the current binary can shape it.
pub(super) const V1_CONFIG: &str = r"# nebula corpus configuration. `neb domain` edits this.
schema_version: 1
corpus_id: neb-abc123
domains:
- general
- physics
default_domain: physics
";

const V1_WAKE: &str = r"---
id: wake-retardation
title: Retardation in the scarcity wake
domain: physics
status: supported
created: 2026-09-07
updated: 2026-09-10
kill: If the wake timescale is frame-independent, this is dead.
tags:
- Orrery
edges:
- type: derives-from
  to: gravity-as-scarcity
- type: depends-on
  to: gravity-as-scarcity
evidence:
- id: ev1
  verdict: supports
  strength: strong
  source: sim://boosted-source/run-3
  date: 2026-09-08
  note: |-
    Boosted source shows a lag.
    Frame dependence still to be checked.
  origin:
    task: DANI-10001
- id: ev2
  verdict: undermines
  strength: anecdote
  source: https://example.org/objection
  date: 2026-09-09
references:
- id: r1
  kind: study
  uri: https://example.org/time-dilation
  title: Gravitational time dilation
  note: The constraint any wake timescale has to survive.
  added: 2026-09-07
  promoted_to: ev1
tasks:
- id: DANI-10001
  state: open
  why: run the boosted-source sim at three velocities
---

The wake lags the source.
";

const V1_GRAVITY: &str = r"---
id: gravity-as-scarcity
title: Gravity as scarcity
domain: physics
status: testing
created: 2026-09-01
updated: 2026-09-05
kill: If a dense region shows no pull at all.
evidence:
- id: ev1
  verdict: inconclusive
  strength: suggestive
  source: doi:10.1000/scarcity
  date: 2026-09-04
  note: Looked, learned nothing.
---

Space might have a density of something.
";

const V1_OLD: &str = r"---
id: old-idea
title: Old idea
domain: general
status: graduated
created: 2026-08-01
updated: 2026-08-20
kill: If nobody downstream wants it.
graduated_to: principia://theory/old-idea
---

It went downstream.
";

const V1_DEAD: &str = r"---
id: dead-idea
title: Dead idea
domain: general
status: refuted
created: 2026-08-01
updated: 2026-08-15
kill: If the effect vanishes under control.
edges:
- type: undermines
  to: old-idea
evidence:
- id: ev1
  verdict: undermines
  strength: strong
  source: https://example.org/control-run
  date: 2026-08-15
  note: It vanished under control.
---

The effect vanished.
";

pub(super) fn v1_corpus() -> Corpus {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    write(&root.join("config.yaml"), V1_CONFIG);
    for (id, text) in [
        ("wake-retardation", V1_WAKE),
        ("gravity-as-scarcity", V1_GRAVITY),
        ("old-idea", V1_OLD),
        ("dead-idea", V1_DEAD),
    ] {
        write(&root.join("nodes").join(format!("{id}.md")), text);
    }
    Corpus { dir, root }
}

#[test]
fn a_v1_corpus_refuses_to_open_until_migrated() {
    let c = v1_corpus();
    c.run(&["list"])
        .assert_fails()
        .says("schema_version 1")
        .says("Bring the corpus forward with:  neb migrate");
    c.run(&["check"]).assert_fails().says("neb migrate");
}

#[test]
fn migrate_relabels_evidence_tasks_and_edges_into_references() {
    let c = v1_corpus();
    let first = c
        .run(&["migrate"])
        .assert_ok()
        .says("wake-retardation")
        .says("domain `physics` -> tag `physics`")
        .says("evidence ev1 -> reference r2")
        .says("evidence ev2 -> reference r3")
        .says("task DANI-10001 -> reference r4")
        .says("edge depends-on gravity-as-scarcity -> reference r5")
        .says("status supported -> hypothesis")
        .says("status testing -> hypothesis")
        .says("status graduated -> abandoned (graduated to principia://theory/old-idea)")
        .says("config.yaml schema_version -> 2")
        .says("4 of 4 nodes rewritten")
        .stdout();
    assert!(!first.contains("nothing changed"), "{first}");

    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 2"), "{config}");
    assert!(config.contains("corpus_id: neb-abc123"), "{config}");
    assert!(
        !config.contains("domains") && !config.contains("default_domain"),
        "{config}"
    );

    let wake = std::fs::read_to_string(c.node_file("wake-retardation")).unwrap();
    for gone in [
        "domain:",
        "evidence:",
        "tasks:",
        "promoted_to",
        "type: depends-on",
        "verdict:",
        "strength:",
    ] {
        assert!(!wake.contains(gone), "`{gone}` survived migration:\n{wake}");
    }
    assert!(wake.contains("status: hypothesis"), "{wake}");
    assert!(wake.contains("tags:\n- orrery\n- physics\n"), "{wake}");
    assert!(
        wake.contains("- type: derives-from\n  to: gravity-as-scarcity\n"),
        "{wake}"
    );
    // The verdict and strength survive as a prefix on the note, the note's
    // first line becomes the title, and the evidence provenance is kept.
    assert!(
        wake.contains(
            "- id: r2\n  kind: other\n  uri: sim://boosted-source/run-3\n  \
             title: Boosted source shows a lag.\n  note: |-\n    \
             [supports/strong] Boosted source shows a lag.\n    \
             Frame dependence still to be checked.\n  added: 2026-09-08\n  \
             origin:\n    task: DANI-10001\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r3\n  kind: other\n  uri: https://example.org/objection\n  \
             note: '[undermines/anecdote]'\n  added: 2026-09-09\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r4\n  kind: other\n  uri: orbit:DANI-10001\n  title: DANI-10001\n  \
             note: '[open] run the boosted-source sim at three velocities'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r5\n  kind: other\n  uri: neb:gravity-as-scarcity\n  \
             title: gravity-as-scarcity\n  note: '[depends-on] gravity-as-scarcity'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains("The wake lags the source."),
        "prose survives:\n{wake}"
    );
    // The migration never masquerades as an edit.
    assert!(wake.contains("updated: 2026-09-10"), "{wake}");
    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
}

#[test]
fn migrate_maps_statuses_and_is_a_no_op_the_second_time() {
    let c = v1_corpus();
    c.run(&["migrate"]).assert_ok();

    let gravity = std::fs::read_to_string(c.node_file("gravity-as-scarcity")).unwrap();
    assert!(gravity.contains("status: hypothesis"), "{gravity}");
    assert!(
        gravity.contains("[inconclusive/suggestive] Looked, learned nothing."),
        "{gravity}"
    );
    assert!(gravity.contains("tags:\n- physics\n"), "{gravity}");

    let old = std::fs::read_to_string(c.node_file("old-idea")).unwrap();
    assert!(old.contains("status: abandoned"), "{old}");
    assert!(
        old.contains(
            "closed:\n  why: graduated to principia://theory/old-idea\n  at: 2026-08-20\n"
        ),
        "{old}"
    );
    assert!(!old.contains("graduated_to"), "{old}");

    let dead = std::fs::read_to_string(c.node_file("dead-idea")).unwrap();
    assert!(dead.contains("status: refuted"), "{dead}");
    assert!(dead.contains("closed:\n  why: refuted under v1"), "{dead}");
    assert!(
        dead.contains("[undermines/strong] It vanished under control."),
        "{dead}"
    );
    assert!(dead.contains("note: '[undermines] old-idea'"), "{dead}");

    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
    let listed = c.run(&["list", "--tag", "physics"]).assert_ok().stdout();
    assert!(
        listed.contains("wake-retardation") && listed.contains("gravity-as-scarcity"),
        "{listed}"
    );

    // Second run: nothing to do, nothing touched.
    let before = snapshot_corpus_files(&c.root);
    let before_config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(
        before_config,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
}

/// A v2 corpus can hold a `kind: discussion` reference with no `uri`
/// (allowed since DANI-10593). `migrate` reads every node through the v1
/// model to detect what needs rewriting, and that model must tolerate a
/// missing `uri` too, or a legal v2 node makes migration fail outright.
#[test]
fn migrate_is_a_no_op_on_a_uri_less_discussion_reference() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussion",
        "--note",
        "a chat with the human",
    ])
    .assert_ok();

    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(!before.contains("uri:"), "{before}");
    let updated_before = before
        .lines()
        .find(|l| l.starts_with("updated:"))
        .unwrap()
        .to_string();

    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");

    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(before, after, "migrate must not touch a v2 node");
    let updated_after = after
        .lines()
        .find(|l| l.starts_with("updated:"))
        .unwrap()
        .to_string();
    assert_eq!(updated_before, updated_after, "updated must not be bumped");
}

#[test]
fn migrate_refuses_a_future_schema_without_changing_any_corpus_file() {
    let c = v1_corpus();
    let node = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&node).unwrap();
    write(
        &node,
        &raw.replacen(
            "status: supported",
            "status: supported\nfuture_field: valuable",
            1,
        ),
    );
    let config = c.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &raw.replacen("schema_version: 1", "schema_version: 3", 1),
    );
    git(&c.root, &["init", "-q"]);
    git(
        &c.root,
        &[
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "add",
            "-A",
        ],
    );
    git(
        &c.root,
        &[
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "future corpus",
        ],
    );

    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(&config).unwrap();
    let run = c
        .run(&["migrate"])
        .assert_fails()
        .says("schema_version 3")
        .says("This corpus was written by a newer nebula. Upgrade this build.");
    let output = format!("{}{}", run.stdout(), run.stderr());
    assert!(
        !output.contains("Bring the corpus forward with:  neb migrate"),
        "future schema must not suggest migration:\n{output}"
    );

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(config_before, std::fs::read_to_string(&config).unwrap());
    assert!(
        std::fs::read_to_string(node)
            .unwrap()
            .contains("future_field: valuable")
    );
}
