//! References: `neb cite` kinds, notes and URIs, local paths that must
//! resolve, and a reference that may not carry a verdict.

use crate::harness::{Corpus, write};

#[test]
fn cite_observatory_refusal_preserves_path_and_uri_case() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    for uri in [
        "/abs/Q1",
        "../Mixed/q002.md",
        "https://example.org/Mixed/Q1",
        "file:///abs/Q1",
    ] {
        let refusal = c
            .run(&[
                "--json",
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                uri,
                "--note",
                "context",
            ])
            .usage_refusal();
        assert_eq!(refusal["code"], "invalid_observatory_id");
        assert!(
            refusal["error"].as_str().unwrap().contains(uri),
            "{refusal}"
        );
        assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
    }
    c.run(&[
        "cite",
        &id,
        "--kind",
        "observatory",
        "--uri",
        "q002",
        "--note",
        "context",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("  uri: Q002\n"), "{raw}");
}

#[test]
fn a_reference_cannot_smuggle_in_a_verdict() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace(
        "status: seed",
        "status: seed\nreferences:\n- id: r1\n  kind: paper\n  uri: http://example.com\n  added: 2026-09-06\n  verdict: supports",
    );
    write(&c.node_file(&id), &raw);
    // The separation between context and evidence is the discipline the whole
    // system exists to impose, so it fails to parse rather than merely warning.
    c.run(&["check"])
        .assert_fails()
        .says("unknown field `verdict`");
}

#[test]
fn cite_accepts_the_documented_kinds_and_refuses_other_values() {
    const ACCEPTED: [&str; 10] = [
        "paper",
        "study",
        "article",
        "note",
        "discussion",
        "book",
        "dataset",
        "thread",
        "observatory",
        "other",
    ];
    const ACCEPTED_MESSAGE: &str = "accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other";

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for kind in ACCEPTED {
        let uri = if kind == "observatory" {
            "Q002"
        } else {
            "https://example.org"
        };
        c.run(&[
            "cite", "--kind", kind, "--uri", uri, "--note", "context", &id,
        ])
        .assert_ok();
    }
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    for kind in ACCEPTED {
        assert!(
            raw.contains(&format!("  kind: {kind}")),
            "{kind} missing from:\n{raw}"
        );
    }

    // Case is not a second kind: it is lowercased, as tags are, and only
    // then checked against the vocabulary.
    for (given, uri) in [
        ("Paper", "https://example.org/case"),
        ("Observatory", "q003"),
    ] {
        c.run(&[
            "cite", "--kind", given, "--uri", uri, "--note", "context", &id,
        ])
        .assert_ok();
    }
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        raw.contains("  kind: paper\n  uri: https://example.org/case\n"),
        "{raw}"
    );
    assert!(raw.contains("  kind: observatory\n  uri: Q003\n"), "{raw}");
    assert!(
        !raw.contains("Paper") && !raw.contains("Observatory"),
        "{raw}"
    );

    let before = raw;
    for kind in ["bogus", "VERDICT", "Bogus", "", "not a kind"] {
        c.run(&[
            "cite",
            "--kind",
            kind,
            "--uri",
            "https://example.org/unexpected",
            "--note",
            "context",
            &id,
        ])
        .assert_fails()
        .says(ACCEPTED_MESSAGE);
    }
    assert_eq!(
        before,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "refused kinds must not change the node"
    );
}

#[test]
fn check_warns_about_a_legacy_unexpected_reference_kind_without_rejecting_the_file() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "paper",
        "--uri",
        "https://example.org",
        "--note",
        "context",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("  kind: paper\n", "  kind: bogus\n"),
    );

    let shown = c.run(&["show", "--json", &id]).assert_ok().stdout();
    let node: serde_json::Value = serde_json::from_str(&shown).expect("show --json is valid JSON");
    assert_eq!(node["node"]["references"][0]["kind"], "bogus");
    c.run(&["check"])
        .assert_ok()
        .says("[16]")
        .says("reference `r1` has unexpected kind `bogus`")
        .says("accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other")
        .says("0 errors, 1 warning");
}

#[test]
fn a_reference_with_no_note_warns_and_a_noted_one_does_not() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &bare,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "",
    ])
    .assert_ok();
    c.run(&["check"])
        .assert_ok()
        .says("[10]")
        .says("reference `r1` has no note saying why it is here")
        .says(&bare)
        .says("0 errors, 1 warning");

    let omitted = Corpus::new();
    let omitted_id = omitted.seed("another idea", "Another idea");
    omitted
        .run(&[
            "cite",
            &omitted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
        ])
        .assert_ok();
    omitted
        .run(&["check"])
        .assert_ok()
        .says("[10]")
        .says("0 errors, 1 warning");

    let noted = Corpus::new();
    let noted_id = noted.seed("a third idea", "A third idea");
    noted
        .run(&[
            "cite",
            &noted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
            "--note",
            "explains the mechanism",
        ])
        .assert_ok();
    noted
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

#[test]
fn a_discussion_reference_may_omit_its_uri_but_other_kinds_may_not() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");

    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussion",
        "--note",
        "came from the ideation session",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("kind: discussion"), "{raw}");
    assert!(
        !raw.contains("uri:"),
        "URI-less discussion should omit uri:\n{raw}"
    );
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    c.run(&["cite", &id, "--kind", "paper", "--note", "missing URI"])
        .assert_fails()
        .says("a reference of kind `paper` needs a URI")
        .says("--uri");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussions",
        "--note",
        "missing URI",
    ])
    .assert_fails()
    .says("a reference of kind `discussions` needs a URI");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "paper",
        "--uri",
        "",
        "--note",
        "empty URI",
    ])
    .assert_fails()
    .says("a reference of kind `paper` needs a URI");

    // The kind's case is normalised before it decides whether a URI is
    // needed, so `Discussion` is a discussion.
    c.run(&["cite", &id, "--kind", "Discussion"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(!raw.contains("Discussion"), "{raw}");
    c.run(&["check"])
        .assert_ok()
        .says("reference `r2` has no note saying why it is here")
        .says("0 errors, 1 warning");

    let invalid = Corpus::new();
    let invalid_id = invalid.seed("another idea", "Another idea");
    invalid
        .run(&[
            "cite",
            &invalid_id,
            "--kind",
            "paper",
            "--uri",
            "https://example.org",
            "--note",
            "context",
        ])
        .assert_ok();
    let raw = std::fs::read_to_string(invalid.node_file(&invalid_id)).unwrap();
    write(
        &invalid.node_file(&invalid_id),
        &raw.replace("  uri: https://example.org\n", "  uri: ''\n"),
    );
    invalid
        .run(&["check"])
        .assert_fails()
        .says("kind `paper` but no URI")
        .says("1 error, 0 warnings");
}

/// A citation with no URI, or a blank one, has its own code. Core's message
/// names no flag, since the desktop has none; the CLI's hint names `--uri`.
#[test]
fn cite_without_uri_has_a_specific_code() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for args in [
        vec!["--json", "cite", &id, "--kind", "paper"],
        vec!["--json", "cite", &id, "--kind", "paper", "--uri", "  "],
    ] {
        let refused = c.run(&args).usage_refusal();
        assert_eq!(refused["code"], "uri_required", "{args:?}: {refused}");
        let said = refused["error"].as_str().unwrap();
        assert!(!said.contains("--"), "a flag in core's message: {said}");
        assert!(
            refused["hint"].as_str().unwrap().contains("--uri"),
            "{refused}"
        );
    }
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

#[test]
fn a_local_uri_must_resolve_and_external_urls_never_trip_it() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    // Rule 8 at the point of action: `cite` refuses a path to nothing.
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_fails()
        .says("does not resolve");

    std::fs::create_dir_all(c.root.join("nodes").join("notes")).unwrap();
    write(
        &c.root.join("nodes").join("notes").join("missing.md"),
        "here now",
    );
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // And in `check`, for the file that went missing later: an error, since
    // a citation to nothing is a broken record rather than an untidy one.
    std::fs::remove_file(c.root.join("nodes").join("notes").join("missing.md")).unwrap();
    c.run(&["check"])
        .assert_fails()
        .says("[8]")
        .says("points at a path that does not resolve: ./notes/missing.md")
        .says("1 error,");

    // Schemes and URLs are never resolved, so none of these trip rule 8.
    let online = Corpus::new();
    let online_id = online.seed("an online idea", "An online idea");
    for uri in [
        "https://example.org/paper",
        "doi:10.1000/x",
        "orbit:DANI-10345",
        "[[almanac/some-page]]",
    ] {
        online
            .run(&["cite", &online_id, "--uri", uri, "--note", "n"])
            .assert_ok();
    }
    online
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

/// Rule 8 at `cite`: a path that is absolute here names nothing on the other
/// machines the corpus is synced to, so it is refused even when it exists,
/// and the refusal says what to write instead. The relative spelling of the
/// same file is accepted.
#[test]
fn cite_refuses_an_absolute_local_path_even_when_it_exists() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let beside = c.root.join("studies").join("x.md");
    std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
    write(&beside, "a study");
    let absolute = beside.to_str().unwrap().to_string();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for uri in [
        absolute.clone(),
        format!("file://{absolute}"),
        format!("FILE://{absolute}"),
        format!("file:{absolute}"),
        "/nonexistent/x.md".to_string(),
        "C:/studies/x.md".to_string(),
    ] {
        c.run(&["cite", &id, "--kind", "study", "--uri", &uri, "--note", "n"])
            .assert_fails()
            .says(&format!(
                "`{uri}` is an absolute local path; local references are relative to nodes/"
            ))
            .says("Cite it by a path relative to nodes/")
            .says(&format!(
                "neb cite {id} --kind observatory --uri <record-id>"
            ));
    }
    assert_eq!(
        before,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "a refused citation must not change the node"
    );

    c.run(&[
        "cite",
        &id,
        "--kind",
        "study",
        "--uri",
        "../studies/x.md",
        "--note",
        "n",
    ])
    .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// Rule 8 in `check`: an absolute path already in the corpus — hand written,
/// or carried over from a v1 `evidence` source — still loads, and is a
/// warning whether or not it resolves on this machine, since resolving here
/// is what cannot be judged. URLs and scheme handles never trip it.
#[test]
fn check_warns_about_an_absolute_local_path_already_in_the_corpus() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for uri in [
        "https://example.org/a",
        "http://example.org/b",
        "mailto:someone@example.org",
        "orbit:ORB-13049",
        "neb:another-idea",
    ] {
        c.run(&["cite", &id, "--uri", uri, "--note", "n"])
            .assert_ok();
    }
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    let existing = c.root.join("config.yaml");
    let existing = existing.to_str().unwrap();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace(
            "  uri: https://example.org/a\n",
            &format!("  uri: {existing}\n"),
        )
        .replace(
            "  uri: http://example.org/b\n",
            "  uri: file:///nonexistent/x.md\n",
        ),
    );

    let shown = c.run(&["show", "--json", &id]).assert_ok().stdout();
    let node: serde_json::Value = serde_json::from_str(&shown).expect("show --json is valid JSON");
    assert_eq!(node["node"]["references"][0]["uri"], existing);
    c.run(&["check"])
        .assert_ok()
        .says("[8]")
        .says(&format!(
            "reference `r1` uses an absolute local path, which resolves on this machine only: \
             {existing}"
        ))
        .says(
            "reference `r2` uses an absolute local path, which resolves on this machine only: \
             file:///nonexistent/x.md",
        )
        .says("0 errors, 2 warnings");

    let out = c.run(&["--json", "check"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json is valid JSON");
    let findings = v["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 2, "{findings:?}");
    for f in findings {
        assert_eq!(f["rule"], 8);
        assert_eq!(f["level"], "warn");
        assert_eq!(f["node"], id);
    }
}
