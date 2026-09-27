//! Nodes and settings nebula cannot read: every reader names the file and
//! what is wrong with it, and capture still lands beside one.

use crate::harness::{Corpus, corpus_repo, log, write};

/// A malformed node file under `nodes/`, with commits on.
fn corpus_with_a_broken_node() -> Corpus {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    c.seed("an idea", "An idea");
    write(&c.node_file("broken"), "this node lost its frontmatter\n");
    c
}

/// Suggestions are a declared side channel (STD-02 §R31): a `nodes/` that
/// cannot be read for them costs the suggestions, said on stderr with the
/// file, and never the capture. It is committed and exits 0, so nobody
/// retries it and captures the thought twice.
#[test]
fn capture_with_an_unreadable_node_file_still_commits_and_exits_zero() {
    let c = corpus_with_a_broken_node();

    let run = c.run(&["capture", "x"]).assert_ok();
    let id = run.stdout_trim();
    assert_eq!(id.len(), 4, "the id is on stdout: {}", run.stdout());
    let stderr = run.stderr();
    assert!(stderr.contains("suggestions unavailable"), "{stderr}");
    assert!(stderr.contains("broken.md"), "names the file: {stderr}");
    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
}

/// The same under `--json`: the entry is on stdout, so an agent knows the
/// capture landed.
#[test]
fn json_capture_with_an_unreadable_node_file_reports_the_entry() {
    let c = corpus_with_a_broken_node();

    let run = c.run(&["--json", "capture", "json thought"]).assert_ok();
    let out: serde_json::Value = serde_json::from_str(&run.stdout())
        .unwrap_or_else(|e| panic!("stdout is the entry ({e}): {}", run.stdout()));
    assert_eq!(out["entry"]["text"], "json thought", "{out}");
    assert_eq!(out["near"], serde_json::json!([]), "{out}");
    assert!(run.stderr().contains("broken.md"), "{}", run.stderr());
    let id = out["entry"]["id"].as_str().unwrap();
    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
}

#[test]
fn every_graph_reader_names_a_node_with_structurally_invalid_frontmatter() {
    for (contents, message) in [
        (
            "this node lost its opening delimiter\n",
            "missing YAML frontmatter",
        ),
        (
            "---\nid: broken\ntitle: Broken\nstatus: seed\ncreated: 2026-09-01\nupdated: 2026-09-01\n\nthe closing delimiter was lost\n",
            "frontmatter is not terminated",
        ),
    ] {
        let c = Corpus::new();
        let healthy = c.seed("a healthy node", "Healthy");
        let broken = c.node_file("broken");
        write(&broken, contents);
        let path = broken.display().to_string();

        for args in [
            vec!["check"],
            vec!["list"],
            vec!["show", healthy.as_str()],
            vec!["graph", "--json"],
        ] {
            c.run(&args).assert_fails().says(message).says(&path);
        }
    }
}

/// A directory where a node file belongs is refused by name as what it is,
/// by every reader, rather than surfacing as the OS's `Is a directory`.
#[test]
fn every_corpus_reader_names_a_non_regular_node_as_what_it_is() {
    let c = Corpus::new();
    let healthy = c.seed("a healthy node", "Healthy");
    let broken = c.node_file("broken");
    std::fs::create_dir(&broken).unwrap();
    let path = broken.display().to_string();

    for args in [
        vec!["check"],
        vec!["list"],
        vec!["show", healthy.as_str()],
        vec!["graph", "--mermaid"],
        vec!["trace", healthy.as_str()],
        vec!["review"],
        vec!["review", "--short"],
        vec!["near", "thing"],
        vec!["tag", "list"],
    ] {
        c.run(&args)
            .assert_fails()
            .says(&format!("{path} is a directory, not a regular file"));
    }
    assert_eq!(
        c.run(&["--json", "list"]).refusal()["code"],
        "unreadable_nodes"
    );
}

#[test]
fn opening_a_non_regular_config_names_it_as_what_it_is() {
    let c = Corpus::new();
    let config = c.root.join("config.yaml");
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    let path = config.display().to_string();

    c.run(&["list"])
        .assert_fails()
        .says(&format!("{path} is a directory, not a regular file"));
}

#[cfg(unix)]
#[test]
fn a_node_write_failure_names_the_destination_and_preserves_the_os_cause() {
    use std::os::unix::fs::PermissionsExt;

    let c = Corpus::new();
    let nodes = c.root.join("nodes");
    let mut permissions = std::fs::metadata(&nodes).unwrap().permissions();
    permissions.set_mode(0o500);
    std::fs::set_permissions(&nodes, permissions).unwrap();

    let destination = c.node_file("cannot-land");
    let run = c.run(&["new", "Cannot land", "--id", "cannot-land"]);

    let mut permissions = std::fs::metadata(&nodes).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&nodes, permissions).unwrap();

    run.assert_fails()
        .says("writing")
        .says(&destination.display().to_string())
        .says("Permission denied");
}
