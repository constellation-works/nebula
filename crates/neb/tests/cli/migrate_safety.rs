//! What `neb migrate` refuses and preserves: unknown and nested v2 data, a
//! malformed config, a failing v1 migration that rewrites nothing, and a
//! dirty or unreadable git tree.

use crate::harness::{
    Corpus, break_head, git, git_commit_at, git_init, snapshot_corpus_files, write,
};
use crate::migrate::{V1_CONFIG, v1_corpus};
use std::path::{Path, PathBuf};

/// A v2 corpus whose nodes are committed, so `migrate` gets past the
/// dirty-tree refusal and the assertions are about what it does to content.
fn committed_v2_corpus() -> Corpus {
    let c = Corpus::new();
    git_init(&c.root);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "v2 corpus"]);
    c
}

/// Give `c`'s node `id` a tag in a case the current model accepts but
/// `migrate` does not render, so the v1 pass normalises it and writes the
/// file back. Returns the bytes that are now on disk.
fn with_an_unnormalised_tag(c: &Corpus, id: &str) -> String {
    let path = c.node_file(id);
    // The first `---\n\n` closes the frontmatter, so this appends to it.
    let raw =
        std::fs::read_to_string(&path)
            .unwrap()
            .replacen("---\n\n", "tags:\n- Orrery\n---\n\n", 1);
    write(&path, &raw);
    raw
}

/// The positive control for the test below: on its own, the node that test
/// expects `migrate` to leave alone really is one `migrate` would rewrite.
/// Without this, "the earlier node was not rewritten" would pass just as
/// happily against a fixture `migrate` never had any reason to touch.
#[test]
fn migrate_rewrites_an_unnormalised_tag_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let id = c.seed("an idea", "Alpha idea");
    let before = with_an_unnormalised_tag(&c, &id);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "an unnormalised tag"]);

    c.run(&["migrate"])
        .assert_ok()
        .says("tags normalised: orrery");
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_ne!(before, after);
    assert!(after.contains("- orrery"), "{after}");
}

/// A corpus already at schema 2 has nothing left to convert, so a node the
/// current model cannot read is a hand edit this build does not understand,
/// not old content to re-label. Reading it through the lenient v1 model
/// would drop the field and report the node as successfully migrated, which
/// is data loss wearing a success message.
///
/// The good node here sorts first and is one `migrate` genuinely would
/// rewrite — its tag needs normalising — so this also pins that the refusal
/// comes before the rewrite loop rather than during it. Refusing per node
/// would leave `alpha-idea` normalised on disk and the run half applied.
#[test]
fn migrate_refuses_an_unknown_node_field_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let alpha = c.seed("an idea", "Alpha idea");
    let zeta = c.seed("another idea", "Zeta idea");
    assert!(alpha < zeta, "{alpha} must sort before {zeta}");

    // Valid v2, but not in the form `migrate` renders. What it does to this
    // node alone is pinned by the test above.
    with_an_unnormalised_tag(&c, &alpha);

    let path = c.node_file(&zeta);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen("status: seed", "status: seed\nfuture_field: valuable", 1),
    );
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "hand edits"]);

    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    c.run(&["migrate"])
        .assert_fails()
        .says("schema_version 2")
        .says("future_field")
        .says("nothing was changed");

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(
        config_before,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
    // Byte for byte: the unknown field survives, and the node that sorts
    // ahead of it was never normalised.
    assert!(
        std::fs::read_to_string(c.node_file(&zeta))
            .unwrap()
            .contains("future_field: valuable")
    );
    assert!(
        std::fs::read_to_string(c.node_file(&alpha))
            .unwrap()
            .contains("- Orrery")
    );
    assert!(git(&c.root, &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn check_and_migrate_refuse_unknown_nested_v2_data_without_rewriting() {
    for nested in [
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n",
        "references:\n- id: r1\n  kind: study\n  added: 2026-09-22\n  origin:\n    task: FIXTURE-1\n    future_field: IRREPLACEABLE\n",
        "edges:\n- type: derives-from\n  to: parent\n  future_field: IRREPLACEABLE\n",
    ] {
        let c = committed_v2_corpus();
        let id = c.seed("an idea", "An idea");
        let path = c.node_file(&id);
        let raw = std::fs::read_to_string(&path).unwrap();
        let edited = raw.replacen("---\n\n", &format!("{nested}---\n\n"), 1);
        assert_ne!(raw, edited);
        write(&path, &edited);
        git(&c.root, &["add", "-A"]);
        git(&c.root, &["commit", "-q", "-m", "unknown nested data"]);

        let before = snapshot_corpus_files(&c.root);
        c.run(&["check"]).assert_fails().says("future_field");
        c.run(&["migrate"])
            .assert_fails()
            .says("schema_version 2")
            .says("future_field")
            .says("nothing was changed");
        assert_eq!(before, snapshot_corpus_files(&c.root));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }
}

#[test]
fn migrate_preflights_late_nested_unknown_before_normalising_earlier_node() {
    let c = committed_v2_corpus();
    let alpha = c.seed("first", "Alpha idea");
    let zeta = c.seed("last", "Zeta idea");
    assert!(alpha < zeta);
    with_an_unnormalised_tag(&c, &alpha);

    let path = c.node_file(&zeta);
    let raw = std::fs::read_to_string(&path).unwrap();
    let edited = raw.replacen(
        "---\n\n",
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n---\n\n",
        1,
    );
    assert_ne!(raw, edited);
    write(&path, &edited);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "late nested unknown"]);

    let before = snapshot_corpus_files(&c.root);
    c.run(&["check"]).assert_fails().says("future_field");
    c.run(&["migrate"])
        .assert_fails()
        .says("future_field")
        .says("nothing was changed");
    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    assert!(
        std::fs::read_to_string(c.node_file(&alpha))
            .unwrap()
            .contains("- Orrery")
    );
}

/// Invariant 7 is enforced by `deny_unknown_fields` on the reference, not by
/// a check, so a `verdict` on a v2 reference has to fail to parse. The v1
/// reference model tolerates one because a v1 corpus really did carry
/// weighed references; applied to a v2 corpus that tolerance would delete
/// the key and call it a migration.
#[test]
fn migrate_refuses_a_reference_verdict_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.invalid/p",
        "--note",
        "a paper",
    ])
    .assert_ok();
    let path = c.node_file(&id);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen("  kind: ", "  verdict: supports\n  kind: ", 1),
    );
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "a weighed reference"]);

    let before = snapshot_corpus_files(&c.root);
    c.run(&["migrate"])
        .assert_fails()
        .says("schema_version 2")
        .says("verdict");
    assert_eq!(before, snapshot_corpus_files(&c.root));
}

/// The strict read is scoped to corpora that declare the current schema. A
/// v1 one keeps the leniency it was written for: its retired keys are
/// exactly what migration is there to drop, so an unrecognised key in a v1
/// file must still convert rather than refuse.
#[test]
fn migrate_still_drops_an_unknown_field_from_a_v1_corpus() {
    let c = v1_corpus();
    let node = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&node).unwrap();
    write(
        &node,
        &raw.replacen(
            "status: supported",
            "status: supported\nretired_v1_key: gone",
            1,
        ),
    );

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    assert!(
        !std::fs::read_to_string(&node)
            .unwrap()
            .contains("retired_v1_key")
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn migrate_keeps_valid_v1_origin_mapping_with_retired_nested_keys() {
    let c = v1_corpus();
    let path = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&path).unwrap();
    let edited = raw
        .replacen(
            "    task: DANI-10001\n",
            "    task: DANI-10001\n    retired_v1_origin_key: gone\n",
            1,
        )
        .replacen(
            "---\n\n",
            "origin:\n  task: DANI-10002\n  retired_v1_origin_key: gone\n---\n\n",
            1,
        );
    assert_ne!(raw, edited);
    write(&path, &edited);

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    let migrated = std::fs::read_to_string(path).unwrap();
    assert!(migrated.contains("task: DANI-10001"), "{migrated}");
    assert!(migrated.contains("task: DANI-10002"), "{migrated}");
    assert!(!migrated.contains("retired_v1_origin_key"), "{migrated}");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn migrate_refuses_a_malformed_config_before_changing_any_corpus_file() {
    let c = v1_corpus();
    let config = c.root.join("config.yaml");
    write(&config, "schema_version: 1\ncorpus_id: [not, a, string]\n");
    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(&config).unwrap();

    c.run(&["migrate"])
        .assert_fails()
        .says("parsing")
        .says("config.yaml");

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(config_before, std::fs::read_to_string(&config).unwrap());
}

/// Every entry under `root` but the write lock, with a file's bytes, a
/// directory as `dir` and a symlink as its target. What a refused migration
/// is compared on: the lock is the one thing a run may leave behind.
fn every_file_but_lock(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == root.join(".lock") {
                continue;
            }
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            let content = if kind.is_symlink() {
                format!("-> {}", std::fs::read_link(&path).unwrap().display()).into_bytes()
            } else if kind.is_dir() {
                pending.push(path.clone());
                b"dir".to_vec()
            } else {
                std::fs::read(&path).unwrap()
            };
            out.insert(path, content);
        }
    }
    out
}

/// A v1 node `migrate` can convert, sorting first.
const V1_FIRST: &str = "---\nid: a-first\ntitle: First\ndomain: Physics\nstatus: testing\nkill: If it never shows up.\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe first.\n";

/// A v1 node it cannot, sorting after it: `bogus` was never a v1 status.
const V1_BOGUS: &str = "---\nid: b-second\ntitle: Second\nstatus: bogus\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe second.\n";

/// A v1 corpus of `nodes` and, when given, `config`, outside git.
fn v1_corpus_of(config: Option<&str>, nodes: &[(&str, &str)]) -> Corpus {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    if let Some(config) = config {
        write(&root.join("config.yaml"), config);
    }
    for (id, text) in nodes {
        write(&root.join("nodes").join(format!("{id}.md")), text);
    }
    Corpus { dir, root }
}

/// Every node is converted in memory before the first write, so a node that
/// cannot be converted late in the corpus refuses the run with the earlier
/// ones untouched, and the run after the fix reports every node it changed.
#[test]
fn a_failing_v1_migration_rewrites_nothing() {
    let c = v1_corpus_of(
        Some(V1_CONFIG),
        &[("a-first", V1_FIRST), ("b-second", V1_BOGUS)],
    );
    let before = every_file_but_lock(&c.root);

    c.run(&["migrate"])
        .assert_fails()
        .says("b-second.md")
        .says("status `bogus` is not a v1 status");
    assert_eq!(before, every_file_but_lock(&c.root));

    write(
        &c.node_file("b-second"),
        &V1_BOGUS.replace("status: bogus", "status: seed"),
    );
    // `a-first` is converted by this run, notes and all; `b-second` is
    // already in v2 form now, so it is examined and left alone.
    c.run(&["migrate"])
        .assert_ok()
        .says("domain `Physics` -> tag `physics`")
        .says("status testing -> hypothesis")
        .says("1 of 2 nodes rewritten");
    c.run(&["check"]).assert_ok().says("2 nodes, 0 errors");
}

/// A corpus an older `neb` stamped at schema 2 over v1 nodes cannot be told
/// apart from a hand edit for certain, so it is still refused; the refusal
/// says which file looks like v1 and how to repair it without the tool.
#[test]
fn a_corpus_already_stamped_v2_over_v1_nodes_names_the_hand_repair() {
    let c = v1_corpus_of(
        Some("schema_version: 2\ncorpus_id: neb-abc123\n"),
        &[("a-first", V1_FIRST)],
    );
    let config = c.root.join("config.yaml");
    let before = every_file_but_lock(&c.root);

    for args in [&["migrate"][..], &["list"]] {
        let run = c
            .run(args)
            .assert_fails()
            .says("a-first.md")
            .says("`domain`")
            .says(config.to_str().unwrap());
        if args[0] == "migrate" {
            run.says("schema_version: 1").says("neb migrate");
        } else {
            run.says("neb check");
        }
        assert_eq!(before, every_file_but_lock(&c.root), "`neb {args:?}` wrote");
    }
    let refused = c.run(&["--json", "list"]).refusal();
    assert_eq!(refused["code"], "unreadable_nodes", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("v1_node_under_current_schema")
    );

    // The repair it names works.
    write(&config, "schema_version: 1\ncorpus_id: neb-abc123\n");
    c.run(&["migrate"])
        .assert_ok()
        .says("1 of 1 node rewritten");
    let migrated = std::fs::read_to_string(&config).unwrap();
    assert!(migrated.contains("corpus_id: neb-abc123"), "{migrated}");
    c.run(&["list"]).assert_ok().says("a-first");
}

/// A corpus from before `config.yaml` existed migrates, and the report says
/// the id it now has was minted rather than carried over. Running it again
/// changes nothing.
#[test]
fn migrate_reports_a_minted_corpus_id() {
    let c = v1_corpus_of(None, &[("a-first", V1_FIRST)]);
    let out = c.run(&["--json", "migrate"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["config_rewritten"], true, "{report}");
    let minted = report["minted_corpus_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no minted id: {report}"));
    assert!(minted.starts_with("neb-"), "{report}");
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains(&format!("corpus_id: {minted}")), "{config}");

    let before = every_file_but_lock(&c.root);
    let out = c.run(&["--json", "migrate"]).assert_ok().stdout();
    let again: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(again["config_rewritten"], false, "{again}");
    assert_eq!(
        again["minted_corpus_id"],
        serde_json::Value::Null,
        "{again}"
    );
    assert_eq!(again["rewritten"], serde_json::json!([]), "{again}");
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(before, every_file_but_lock(&c.root));

    let plain = v1_corpus_of(None, &[("a-first", V1_FIRST)]);
    plain
        .run(&["migrate"])
        .assert_ok()
        .says("minted corpus_id neb-");
}

#[test]
fn migrate_refuses_a_dirty_git_tree() {
    let c = v1_corpus();
    git(&c.root, &["init", "-q"]);
    c.run(&["migrate"])
        .assert_fails()
        .says("uncommitted changes");
    // Refused before anything was written.
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 1"), "{config}");

    git(&c.root, &["add", "-A"]);
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
            "v1 corpus",
        ],
    );
    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// Whether the tree is clean is unknown when git cannot read the repository,
/// and unknown is not clean: `migrate` refuses before it rewrites a node.
#[test]
fn migrate_refuses_when_git_cannot_report_status() {
    let c = v1_corpus();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "v1 corpus");
    break_head(&c.root);
    let before = snapshot_corpus_files(&c.root);
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    let refused = c.run(&["--json", "migrate"]).refusal();
    assert_eq!(refused["code"], "git", "{refused}");
    assert_eq!(snapshot_corpus_files(&c.root), before);
    assert_eq!(
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap(),
        config
    );
}

#[test]
fn migrate_rerun_is_exact_no_op_in_a_dirty_git_tree() {
    for commit in [false, true] {
        let c = v1_corpus();
        if commit {
            write(
                &c.root.join("config.yaml"),
                &format!("{V1_CONFIG}commit: true\n"),
            );
        }
        git_init(&c.root);
        git(&c.root, &["add", "-A"]);
        git_commit_at(&c.root, "2020-01-01", "v1 corpus");
        c.run(&["migrate"]).assert_ok();
        let immediate = c.run(&["migrate"]).assert_ok();
        assert_eq!(immediate.stderr(), "already at schema 2; nothing changed\n");
        assert_eq!(immediate.stdout(), "");
        // Both staged and unstaged user work must survive a no-op, even
        // when automatic commits are enabled.
        let node = c.node_file("wake-retardation");
        let raw = std::fs::read_to_string(&node).unwrap();
        write(&node, &format!("{raw}\nA later thought.\n"));
        git(&c.root, &["add", "nodes"]);
        write(&c.root.join("untracked.txt"), "unrelated work");
        let before = snapshot_corpus_files(&c.root);
        let status = git(&c.root, &["status", "--porcelain"]);
        let head = git(&c.root, &["rev-parse", "HEAD"]);
        let run = c.run(&["migrate"]).assert_ok();
        assert_eq!(run.stderr(), "already at schema 2; nothing changed\n");
        assert_eq!(run.stdout(), "");
        assert_eq!(before, snapshot_corpus_files(&c.root));
        assert_eq!(status, git(&c.root, &["status", "--porcelain"]));
        assert_eq!(head, git(&c.root, &["rev-parse", "HEAD"]));
    }
}

#[test]
fn migrate_installs_the_init_ignore_rules() {
    let initialized = Corpus::new();
    let expected = std::fs::read(initialized.root.join(".gitignore")).unwrap();
    let c = v1_corpus();
    git_init(&c.root);
    git(&c.root, &["add", "-A"]);
    git_commit_at(&c.root, "2020-01-01", "v1 corpus");
    c.run(&["migrate"]).assert_ok();
    assert_eq!(std::fs::read(c.root.join(".gitignore")).unwrap(), expected);
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(!git(&c.root, &["status", "--porcelain"]).contains(".lock"));
}

#[test]
fn migrate_preserves_existing_ignore_entries() {
    let c = v1_corpus();
    write(&c.root.join(".gitignore"), "# personal rules\nprivate/");
    c.run(&["migrate"]).assert_ok();
    assert_eq!(
        std::fs::read_to_string(c.root.join(".gitignore")).unwrap(),
        "# personal rules\nprivate/\n/.lock\n/.pending\n*.tmp\n"
    );
}

#[test]
fn migrate_refuses_an_invalid_ignore_file_before_rewriting_nodes() {
    let c = v1_corpus();
    std::fs::create_dir(c.root.join(".gitignore")).unwrap();
    let before = every_file_but_lock(&c.root);
    let refused = c.run(&["--json", "migrate"]).refusal();
    assert_eq!(refused["code"], "not_regular_file", "{refused}");
    assert_eq!(before, every_file_but_lock(&c.root));
}
