//! Library-level tests: the graph queries and the point-of-action guards, as
//! the desktop app and the agent skill see them.
//!
//! `crates/neb/tests/cli.rs` covers the same rules through the binary and
//! asserts on messages and exit codes. These assert on the typed values, which
//! is what a consumer that is not a terminal matches on.

use nebula_core::{
    Corpus, CorpusLock, Direction, EdgeType, Error, Graph, HUMAN, NEAR_DEFAULT, NewNode, Promotion,
    ReviewRule, Status, Via, graph, ops,
};
use std::fmt::Write as _;

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = Corpus::init(&dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

fn seed(corpus: &Corpus, title: &str, parents: &[&str]) -> String {
    ops::new_node(
        corpus,
        &NewNode {
            title: title.to_string(),
            parents: parents.iter().map(ToString::to_string).collect(),
            ..NewNode::default()
        },
    )
    .expect("new node")
    .doc
    .node
    .id
}

#[test]
fn current_model_refuses_unknown_fields_in_nested_node_data() {
    for nested in [
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n",
        "references:\n- id: r1\n  kind: study\n  added: 2026-09-22\n  origin:\n    task: FIXTURE-1\n    future_field: IRREPLACEABLE\n",
        "edges:\n- type: derives-from\n  to: parent\n  future_field: IRREPLACEABLE\n",
    ] {
        let (dir, corpus) = corpus();
        let id = seed(&corpus, "Nested data", &[]);
        let path = dir.path().join("corpus/nodes").join(format!("{id}.md"));
        let original = std::fs::read_to_string(&path).unwrap();
        let edited = original.replacen("---\n\n", &format!("{nested}---\n\n"), 1);
        assert_ne!(original, edited);
        std::fs::write(&path, &edited).unwrap();

        let error = corpus.load_all().unwrap_err().to_string();
        assert!(error.contains("future_field"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    }
}

/// A diamond: `d` descends from `b` and `c`, both of which descend from `a`.
fn diamond(corpus: &Corpus) -> [String; 4] {
    let a = seed(corpus, "A", &[]);
    let b = seed(corpus, "B", &[&a]);
    let c = seed(corpus, "C", &[&a]);
    let d = seed(corpus, "D", &[&b, &c]);
    [a, b, c, d]
}

// ------------------------------------------------------------------- graph --

#[test]
fn trace_up_reports_each_ancestor_once_nearest_first() {
    let (_dir, corpus) = corpus();
    let [a, b, c, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let ids: Vec<_> = graph::trace(&graph, &d, Direction::Up)
        .unwrap()
        .0
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(ids.first(), Some(&d), "the walk starts at the node itself");
    assert_eq!(
        ids.len(),
        4,
        "a diamond reaches `a` twice but reports it once"
    );
    assert!(ids.contains(&a) && ids.contains(&b) && ids.contains(&c));
}

#[test]
fn trace_down_walks_descendants() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let walk = graph::trace(&graph, &a, Direction::Down).unwrap();
    assert_eq!(walk.0.len(), 4);
    assert_eq!(walk.0[0].id, a);
    assert!(walk.0.iter().any(|n| n.id == d));
}

#[test]
fn trace_of_an_unknown_node_is_a_typed_error() {
    let (_dir, corpus) = corpus();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    assert!(matches!(
        graph::trace(&graph, "nope", Direction::Up),
        Err(Error::NoSuchNode(id)) if id == "nope"
    ));
}

#[test]
fn impact_lists_descendants_then_contradictions() {
    let (_dir, corpus) = corpus();
    let [a, b, c, d] = diamond(&corpus);
    let rival = seed(&corpus, "Rival", &[]);
    ops::link(&corpus, &a, EdgeType::Contradicts, &rival, None).unwrap();
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let touched = graph::impact(&graph, &a).unwrap().0;
    let descendants: Vec<_> = touched
        .iter()
        .filter(|t| t.via == Via::Descends)
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(descendants.len(), 3);
    for id in [&b, &c, &d] {
        assert!(descendants.contains(&id.as_str()), "{id} descends from a");
    }
    assert!(
        touched
            .iter()
            .any(|t| t.via == Via::Contradicts && t.id == rival)
    );
}

#[test]
fn export_carries_every_node_and_edge() {
    let (_dir, corpus) = corpus();
    let [a, b, _, d] = diamond(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let out = graph::export(&graph).unwrap();
    assert_eq!(out.nodes.len(), 4);
    assert_eq!(out.edges.len(), 4, "b→a, c→a, d→b, d→c");
    assert!(out.edges.iter().all(|e| e.kind == EdgeType::DerivesFrom));
    assert!(out.edges.iter().any(|e| e.from == b && e.to == a));
    assert!(out.edges.iter().any(|e| e.from == d && e.to == b));
    assert!(out.nodes.iter().all(|n| n.status == Status::Seed));
}

#[test]
fn build_refuses_duplicate_ids() {
    let (_dir, corpus) = corpus();
    seed(&corpus, "Twin", &[]);
    let mut docs = corpus.load_all().unwrap();
    let copy = docs[0].clone();
    docs.push(copy);
    assert!(matches!(
        Graph::build(&docs),
        Err(Error::DuplicateId(id)) if id == "twin"
    ));
}

// --------------------------------------------------------------------- ops --

#[test]
fn a_node_cannot_link_to_itself() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::link(&corpus, &a, EdgeType::Refines, &a, None),
        Err(Error::SelfLoop)
    ));
}

#[test]
fn a_genealogy_edge_that_closes_a_loop_is_refused() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let err = ops::link(&corpus, &a, EdgeType::DerivesFrom, &d, None).unwrap_err();
    assert!(
        matches!(&err, Error::Cycle { from, to } if from == &a && to == &d),
        "got {err:?}"
    );
    // Refused means not written.
    let docs = corpus.load_all().unwrap();
    assert_eq!(
        graph::export(&Graph::build(&docs).unwrap())
            .unwrap()
            .edges
            .len(),
        4
    );
}

#[test]
fn a_contradiction_is_not_genealogy_so_it_may_point_anywhere() {
    let (_dir, corpus) = corpus();
    let [a, _, _, d] = diamond(&corpus);
    let changed = ops::link(&corpus, &a, EdgeType::Contradicts, &d, None).unwrap();
    assert_eq!(changed.len(), 2, "recorded on both ends");
}

#[test]
fn a_hypothesis_needs_a_kill_condition() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Hypothesis, None),
        Err(Error::NeedsKill(Status::Hypothesis))
    ));
    ops::sharpen(&corpus, &a, "it would fail if …", None).unwrap();
    assert_eq!(corpus.load(&a).unwrap().node.status, Status::Hypothesis);
}

#[test]
fn refuting_needs_a_reason_and_is_final() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    ops::sharpen(&corpus, &a, "kill", None).unwrap();
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Refuted, None),
        Err(Error::RefutedNeedsWhy)
    ));
    let change = ops::set_status(&corpus, &a, Status::Refuted, Some("it fired")).unwrap();
    assert_eq!(change.from, Status::Hypothesis);
    assert_eq!(
        change.doc.node.closed.as_ref().map(|c| c.why.as_str()),
        Some("it fired")
    );
    assert!(matches!(
        ops::set_status(&corpus, &a, Status::Seed, None),
        Err(Error::RefutedCannotReopen)
    ));
    assert!(
        matches!(
            ops::sharpen(&corpus, &a, "a different falsifier", None),
            Err(Error::RefutedCannotReopen)
        ),
        "rewriting the kill of a refuted node would orphan closed.why"
    );
    assert!(
        matches!(
            ops::confirm_kill(&corpus, &a),
            Err(Error::RefutedCannotReopen)
        ),
        "confirming a kill on a refuted node is the same quiet rewrite"
    );
    let after = corpus.load(&a).unwrap().node;
    assert_eq!(after.kill.as_deref(), Some("kill"));
    assert_eq!(after.status, Status::Refuted);
}

#[test]
fn an_empty_kill_condition_is_refused() {
    let (_dir, corpus) = corpus();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::sharpen(&corpus, &a, "   ", None),
        Err(Error::EmptyKill)
    ));
}

/// Authorship as a consumer that is not a terminal sees it: stored per field,
/// the human by omission, and confirmable without touching the kill text.
#[test]
fn authorship_is_per_field_and_a_human_can_confirm_a_kill() {
    let (_dir, corpus) = corpus();
    let by = Some("agent:crew-alpha");
    let parent = seed(&corpus, "Parent", &[]);

    let mine = seed(&corpus, "Mine", &[&parent]);
    let node = corpus.load(&mine).unwrap().node;
    assert_eq!(node.title_by, None, "an unattributed write is the human's");
    assert_eq!(node.edges[0].by, None);

    let theirs = ops::new_node(
        &corpus,
        &NewNode {
            title: "Theirs".into(),
            parents: vec![parent.clone()],
            kill: Some("if X".into()),
            by: by.map(String::from),
            ..NewNode::default()
        },
    )
    .unwrap()
    .doc
    .node;
    assert_eq!(theirs.title_by.as_deref(), by);
    assert_eq!(theirs.kill_by.as_deref(), by);
    assert_eq!(theirs.edges[0].by.as_deref(), by);

    // `review` asks the human to stand behind a kill somebody else wrote.
    let flagged = |corpus: &Corpus| -> Vec<String> {
        let docs = corpus.load_all().unwrap();
        graph::review(
            &Graph::build(&docs).unwrap(),
            &corpus.inbox().unwrap(),
            None,
        )
        .unwrap()
        .0
        .into_iter()
        .filter(|i| i.rule == ReviewRule::UnconfirmedKill)
        .map(|i| i.id)
        .collect()
    };
    assert_eq!(flagged(&corpus), std::slice::from_ref(&theirs.id));

    let confirmed = ops::confirm_kill(&corpus, &theirs.id).unwrap().node;
    assert_eq!(
        confirmed.kill.as_deref(),
        Some("if X"),
        "the text is as it was"
    );
    assert_eq!(confirmed.kill_by, None);
    assert_eq!(
        confirmed.title_by.as_deref(),
        by,
        "only the kill is confirmed"
    );
    assert!(flagged(&corpus).is_empty());

    // The queries state the default the file leaves out.
    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &theirs.id).unwrap();
    assert_eq!(view.node.kill_by.as_deref(), Some(HUMAN));
    assert_eq!(view.node.title_by.as_deref(), by);

    assert!(
        ops::sharpen(&corpus, &mine, "if Y", Some("crew (alpha)")).is_err(),
        "a label that would break a note line is refused"
    );
}

#[test]
fn notes_accumulate_in_order_and_unknown_nodes_are_refused() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the original thought").unwrap();
    let id = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            title: Some("A".into()),
            tags: vec!["physics".into()],
            ..Promotion::default()
        },
        0,
    )
    .unwrap()
    .doc
    .node
    .id;
    let parent = seed(&corpus, "Parent", &[]);
    ops::link(&corpus, &id, EdgeType::DerivesFrom, &parent, None).unwrap();

    let before = corpus.load(&id).unwrap();
    let status = before.node.status;
    let edges = before.node.edges.clone();
    let tags = before.node.tags.clone();
    let original = before.body.clone();

    ops::note(&corpus, &id, "first thought", None).unwrap();
    let second = ops::note(&corpus, &id, "second thought", None).unwrap();

    assert!(
        second.body.contains(original.trim()),
        "the capture is still there:\n{}",
        second.body
    );
    let first_at = second.body.find("first thought").expect("first note");
    let second_at = second.body.find("second thought").expect("second note");
    assert!(first_at < second_at, "notes accumulate in order");
    assert!(second.body.contains("## Notes"));
    assert_eq!(second.node.status, status);
    assert_eq!(second.node.edges, edges);
    assert_eq!(second.node.tags, tags);

    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &id).unwrap();
    assert_eq!(
        view.notes
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["first thought", "second thought"]
    );

    assert!(matches!(
        ops::note(&corpus, "nope", "lost", None),
        Err(Error::NoSuchNode(missing)) if missing == "nope"
    ));
}

#[test]
fn settling_an_inbox_entry_is_atomic_and_leaves_no_temporary_file() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut tmp = entry.file.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);

    corpus.settle_inbox(&entry, "dropped").unwrap();

    assert!(!tmp.exists(), "successful settlement left a temporary file");
    let leftovers: Vec<_> = std::fs::read_dir(entry.file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name != entry.file.file_name().unwrap())
        .collect();
    assert!(
        leftovers.is_empty(),
        "successful settlement left {leftovers:?} beside the month file"
    );
    assert!(
        std::fs::read_to_string(&entry.file)
            .unwrap()
            .contains(&format!("- ~~[{}]", entry.id))
    );
}

#[test]
fn an_interrupted_settlement_copy_does_not_resurrect_an_entry() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut tmp = entry.file.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let unsettled = std::fs::read(&entry.file).unwrap();

    corpus.settle_inbox(&entry, "dropped").unwrap();
    std::fs::write(&tmp, unsettled).unwrap();

    assert!(corpus.inbox().unwrap().0.is_empty());
    assert!(matches!(
        corpus.inbox_entry(&entry.id),
        Err(Error::NoSuchInboxEntry(id)) if id == entry.id
    ));
}

/// Every write in the corpus goes through one atomic replacement, so a
/// symlink planted where that replacement's temporary file would go must be
/// refused for a node, for the inbox and for the config alike. These three
/// name the same guard from the three callers that reach it.
#[cfg(unix)]
#[test]
fn a_node_write_cannot_reach_a_file_outside_the_corpus_through_its_temporary() {
    let (dir, corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let id = seed(&corpus, "Safe", &[]);
    let node = dir.path().join("corpus").join("nodes").join("safe.md");
    let planted = dir.path().join("corpus").join("nodes").join("safe.md.tmp");
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    ops::note(&corpus, &id, "probe", None).unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the note was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&node)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became the node"
    );
    let docs = corpus.load_all().unwrap();
    let view = graph::node(&Graph::build(&docs).unwrap(), &id).unwrap();
    assert_eq!(
        view.notes
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["probe"],
        "the note did not reach the node it named"
    );
}

#[cfg(unix)]
#[test]
fn settling_an_inbox_entry_cannot_reach_a_file_outside_the_corpus() {
    let (dir, corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let entry = ops::capture(&corpus, "a thought to settle").unwrap();
    let mut planted = entry.file.as_os_str().to_os_string();
    planted.push(".tmp");
    std::os::unix::fs::symlink(&outside, std::path::PathBuf::from(planted)).unwrap();

    corpus.settle_inbox(&entry, "dropped").unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the settlement was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&entry.file)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became the month file"
    );
    assert!(corpus.inbox().unwrap().0.is_empty());
}

#[cfg(unix)]
#[test]
fn capture_refuses_a_symlinked_active_month_without_changing_its_target() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    let outside = dir.path().join("outside-inbox.md");
    std::fs::rename(&entry.file, &outside).unwrap();
    let before = std::fs::read(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &entry.file).unwrap();

    let error = ops::capture(&corpus, "must stay inside").unwrap_err();

    assert!(matches!(error, Error::Corpus(message) if message.contains("symlink")));
    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&entry.file)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn inbox_writes_refuse_a_symlinked_directory_without_changing_its_target() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    let inbox_dir = entry.file.parent().unwrap();
    let outside_dir = dir.path().join("outside-inbox");
    std::fs::rename(inbox_dir, &outside_dir).unwrap();
    let outside_month = outside_dir.join(entry.file.file_name().unwrap());
    let before = std::fs::read(&outside_month).unwrap();
    std::os::unix::fs::symlink(&outside_dir, inbox_dir).unwrap();

    let capture_error = ops::capture(&corpus, "must stay inside").unwrap_err();
    let settle_error = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    let drop_error = ops::drop(&corpus, &entry.id).unwrap_err();
    let promote_error = ops::promote(&corpus, &entry.id, &Promotion::default(), 0).unwrap_err();

    for error in [capture_error, settle_error, drop_error, promote_error] {
        assert!(matches!(error, Error::Corpus(message) if message.contains("symlink")));
    }
    assert_eq!(std::fs::read(&outside_month).unwrap(), before);
    assert!(corpus.load_all().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_config_write_cannot_reach_a_file_outside_the_corpus() {
    let (dir, mut corpus) = corpus();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let config = dir.path().join("corpus").join("config.yaml");
    let mut planted = config.as_os_str().to_os_string();
    planted.push(".tmp");
    std::os::unix::fs::symlink(&outside, std::path::PathBuf::from(planted)).unwrap();

    ops::set_commit(&mut corpus, true).unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the config was written outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(&config)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink became config.yaml"
    );
    let reopened = Corpus::open(Some(dir.path().join("corpus"))).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the setting did not reach config.yaml"
    );
}

#[test]
fn inbox_ignores_everything_except_month_markdown_files() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let inbox_dir = entry.file.parent().unwrap();

    for name in ["notes.md", "2026-00.md", "2026-13.md", "2026-09.md.tmp"] {
        std::fs::write(inbox_dir.join(name), b"not utf-8: \xff").unwrap();
    }
    std::fs::create_dir(inbox_dir.join("2000-01.md")).unwrap();

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;

        let invalid_name = std::ffi::OsString::from_vec(b"2000-01-\xff.md".to_vec());
        std::fs::write(inbox_dir.join(invalid_name), "unrelated").unwrap();
    }

    let listed = corpus.inbox().unwrap();
    assert_eq!(listed.0.len(), 1);
    assert_eq!(listed.0[0].id, entry.id);

    let captured = ops::capture(&corpus, "another thought").unwrap();
    assert_ne!(captured.id, entry.id);
    assert_eq!(corpus.inbox().unwrap().0.len(), 2);
}

#[test]
fn settling_refuses_when_the_indexed_line_has_another_entry_id() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the original thought").unwrap();
    let replacement = "- [other] 2026-09-22T08:25 another thought\n";
    std::fs::write(&entry.file, replacement).unwrap();

    let error = corpus.settle_inbox(&entry, "dropped").unwrap_err();

    assert!(
        matches!(error, Error::Corpus(message) if message == "inbox entry moved underneath us; nothing written")
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), replacement);
}

#[test]
fn multiline_capture_is_refused_without_writing() {
    let (_dir, corpus) = corpus();
    let existing = ops::capture(&corpus, "already here").unwrap();
    let before = std::fs::read_to_string(&existing.file).unwrap();

    let error = ops::capture(&corpus, "first line\nsecond line").unwrap_err();

    assert!(
        matches!(error, Error::Corpus(message) if message == "capture text must fit on one line")
    );
    assert_eq!(
        std::fs::read_to_string(&existing.file).unwrap(),
        before,
        "a refused capture must not change the inbox file"
    );
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

fn inbox_id_candidates(stamp: &str, text: &str) -> Vec<String> {
    fn fnv(s: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    let mut h = fnv(&format!("{stamp}{text}"));
    let mut ids = Vec::new();
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if !ids.contains(&id) {
            ids.push(id);
        }
        h = fnv(&format!("{h}"));
    }
    ids
}

#[test]
fn capture_uses_a_free_id_after_the_hash_candidates_collide() {
    const TEXT: &str = "the intended new thought";

    for _ in 0..3 {
        let (_dir, corpus) = corpus();
        let probe = ops::capture(&corpus, TEXT).unwrap();
        let occupied = inbox_id_candidates(&probe.at, TEXT);
        let mut fixture = String::new();
        for (i, id) in occupied.iter().enumerate() {
            writeln!(fixture, "- [{id}] {} fixture {i}", probe.at).unwrap();
        }
        std::fs::write(&probe.file, fixture).unwrap();

        let captured = ops::capture(&corpus, TEXT).unwrap();
        if captured.at != probe.at {
            continue;
        }

        assert!(!occupied.contains(&captured.id));
        assert_eq!(corpus.inbox_entry(&captured.id).unwrap().text, TEXT);
        let promoted = ops::promote(
            &corpus,
            &captured.id,
            &Promotion {
                title: Some("Intended new thought".into()),
                ..Promotion::default()
            },
            0,
        )
        .unwrap();
        assert_eq!(promoted.doc.body.trim(), TEXT);
        assert_eq!(corpus.inbox().unwrap().0.len(), occupied.len());
        return;
    }
    panic!("the clock crossed a minute during all three collision fixtures");
}

#[test]
fn capture_refuses_an_exhausted_id_namespace_without_writing() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "locate the current inbox file").unwrap();
    let mut fixture = String::new();
    for id in 0..=u16::MAX {
        writeln!(fixture, "- [{id:04x}] 2000-01-01T00:00 occupied").unwrap();
    }
    std::fs::write(&entry.file, &fixture).unwrap();

    let error = ops::capture(&corpus, "there is no id left").unwrap_err();

    assert!(
        matches!(error, Error::Corpus(message) if message == "inbox id namespace exhausted; nothing captured")
    );
    assert_eq!(
        std::fs::read_to_string(&entry.file).unwrap(),
        fixture,
        "exhaustion must be detected before appending"
    );
}

#[test]
fn capture_after_an_unterminated_record_stays_independent_and_promotes() {
    let (_dir, corpus) = corpus();
    let first = ops::capture(&corpus, "the first thought").unwrap();
    let unterminated = std::fs::read_to_string(&first.file)
        .unwrap()
        .trim_end_matches('\n')
        .to_string();
    std::fs::write(&first.file, unterminated).unwrap();

    let second = ops::capture(&corpus, "the second thought").unwrap();
    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), 2);
    assert_eq!(corpus.inbox_entry(&first.id).unwrap().text, first.text);
    assert_eq!(corpus.inbox_entry(&second.id).unwrap().text, second.text);

    let promoted = ops::promote(
        &corpus,
        &second.id,
        &Promotion {
            title: Some("Second thought".into()),
            ..Promotion::default()
        },
        0,
    )
    .unwrap();
    assert_eq!(promoted.doc.body.trim(), "the second thought");
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
    assert_eq!(corpus.inbox_entry(&first.id).unwrap().text, first.text);
}

#[test]
fn capture_ids_are_unique_across_month_files_and_older_entries_still_resolve() {
    for _ in 0..3 {
        let (_dir, corpus) = corpus();
        let first = ops::capture(&corpus, "the same thought").unwrap();
        let older_file = first.file.with_file_name("2000-01.md");
        std::fs::rename(&first.file, &older_file).unwrap();

        let second = ops::capture(&corpus, "the same thought").unwrap();
        if first.at != second.at {
            continue;
        }

        assert_ne!(second.id, first.id, "ids are unique across month files");
        let resolved = corpus.inbox_entry(&first.id).unwrap();
        assert_eq!(resolved.file, older_file);
        assert_eq!(resolved.text, first.text);
        return;
    }
    panic!("the clock crossed a minute during all three fixture attempts");
}

// -------------------------------------------------------------------- near --

/// A small corpus with vocabulary that overlaps in known ways: two nodes
/// about taxonomy and tags, one about search ranking, one about nothing
/// the queries below mention.
fn lexical_fixture(corpus: &Corpus) {
    for (title, tags, kill) in [
        (
            "Tags beat domains",
            vec!["design", "corpus"],
            Some("a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer"),
        ),
        ("A single global taxonomy", vec!["design"], None),
        ("Ranking decay half-life", vec!["search"], None),
        ("Proper time is a count", vec!["physics"], None),
    ] {
        ops::new_node(
            corpus,
            &NewNode {
                title: title.to_string(),
                tags: tags.into_iter().map(String::from).collect(),
                kill: kill.map(String::from),
                ..NewNode::default()
            },
        )
        .unwrap();
    }
    // Body text counts too: the ranking node argues in words a query can hit.
    ops::note(
        corpus,
        "ranking-decay-half-life",
        "a search result should lose rank as it ages, on a half-life",
        None,
    )
    .unwrap();
}

#[test]
fn near_ranks_by_shared_vocabulary_best_first() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags and domains beat a taxonomy", NEAR_DEFAULT).unwrap();
    let ids: Vec<&str> = near.0.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(
        ids,
        ["tags-beat-domains", "a-single-global-taxonomy"],
        "the two nodes sharing words, the one sharing more first; \
         the search and physics nodes share none and are left out"
    );
    assert!(
        near.0.windows(2).all(|w| w[0].score >= w[1].score),
        "best first: {near:?}"
    );
    assert!(
        near.0.iter().all(|n| n.score > 0.0 && n.score <= 1.0),
        "scores sit in 0..=1: {near:?}"
    );
    assert_eq!(
        near.0[0].tags,
        ["design", "corpus"],
        "a neighbour carries its tags, since a promotion reuses the parent's"
    );

    // Body text is read, not just the title: only the note mentions ageing.
    let near = graph::near(&graph, "results that age", NEAR_DEFAULT).unwrap();
    assert_eq!(
        near.0.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );
}

#[test]
fn near_takes_a_node_id_and_leaves_that_node_out() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags-beat-domains", NEAR_DEFAULT).unwrap();
    let ids: Vec<&str> = near.0.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !ids.contains(&"tags-beat-domains"),
        "a node is not its own neighbour: {ids:?}"
    );
    assert_eq!(
        ids.first(),
        Some(&"a-single-global-taxonomy"),
        "the other design node shares the `design` tag; nothing else does: {ids:?}"
    );
}

#[test]
fn near_is_capped_at_k_and_empty_for_a_thought_unlike_anything() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let docs = corpus.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();

    let near = graph::near(&graph, "tags taxonomy ranking", 2).unwrap();
    assert_eq!(near.0.len(), 2, "k caps the answer: {near:?}");
    assert!(
        graph::near(&graph, "tags taxonomy ranking", 0)
            .unwrap()
            .0
            .is_empty(),
        "k of zero asks for nothing"
    );

    let none = graph::near(&graph, "quantum gravity", NEAR_DEFAULT).unwrap();
    assert!(none.0.is_empty(), "no shared word, no neighbour: {none:?}");
    let stop = graph::near(&graph, "the and of", NEAR_DEFAULT).unwrap();
    assert!(stop.0.is_empty(), "stopwords alone are no query: {stop:?}");

    let empty = tempfile::tempdir().unwrap();
    let empty = Corpus::init(&empty.path().join("corpus")).unwrap();
    let docs = empty.load_all().unwrap();
    let graph = Graph::build(&docs).unwrap();
    assert!(
        graph::near(&graph, "anything", NEAR_DEFAULT)
            .unwrap()
            .0
            .is_empty(),
        "an empty corpus has no neighbours"
    );
}

#[test]
fn capture_and_promote_suggest_but_never_link() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);

    let captured = ops::capture_near(&corpus, "a taxonomy for tags", NEAR_DEFAULT).unwrap();
    assert_eq!(captured.entry.text, "a taxonomy for tags");
    assert_eq!(
        captured
            .near
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["a-single-global-taxonomy", "tags-beat-domains"],
        "capture names the nearest nodes: {:?}",
        captured.near
    );
    let quiet = ops::capture_near(&corpus, "a taxonomy for tags, again", 0).unwrap();
    assert!(quiet.near.is_empty(), "k of zero is the quiet path");

    // Promoted as a root: the suggestions come back, and the node has no edge.
    let created = ops::promote(
        &corpus,
        &captured.entry.id,
        &Promotion::default(),
        NEAR_DEFAULT,
    )
    .unwrap();
    assert_eq!(
        created.near.first().map(|n| n.id.as_str()),
        Some("a-single-global-taxonomy"),
        "{:?}",
        created.near
    );
    assert!(
        !created.near.iter().any(|n| n.id == created.doc.node.id),
        "a promotion is not its own neighbour: {:?}",
        created.near
    );
    assert!(
        created.doc.node.edges.is_empty(),
        "suggesting never links: {:?}",
        created.doc.node.edges
    );
    let on_disk = corpus.load(&created.doc.node.id).unwrap();
    assert!(on_disk.node.edges.is_empty(), "nor on disk");

    // With a parent named, there is nothing to suggest.
    let entry = ops::capture(&corpus, "ranking decay again").unwrap();
    let created = ops::promote(
        &corpus,
        &entry.id,
        &Promotion {
            parents: vec!["ranking-decay-half-life".into()],
            ..Promotion::default()
        },
        NEAR_DEFAULT,
    )
    .unwrap();
    assert!(created.near.is_empty(), "{:?}", created.near);
    assert_eq!(
        created.doc.node.parents().collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );
}

#[test]
fn opening_a_missing_corpus_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere");
    assert!(matches!(
        Corpus::open(Some(missing.clone())),
        Err(Error::NoCorpus(p)) if p == missing
    ));
}

#[test]
fn opening_a_configless_corpus_persists_its_synthesized_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    let config = root.join("config.yaml");
    assert!(!config.exists());

    Corpus::open(Some(root.clone())).unwrap();
    let first = std::fs::read_to_string(&config).unwrap();
    let first_id = first
        .lines()
        .find_map(|line| line.strip_prefix("corpus_id: "))
        .expect("persisted corpus_id");

    Corpus::open(Some(root)).unwrap();
    let second = std::fs::read_to_string(config).unwrap();
    let second_id = second
        .lines()
        .find_map(|line| line.strip_prefix("corpus_id: "))
        .expect("reloaded corpus_id");
    assert_eq!(second_id, first_id);
}

// ------------------------------------------------------------------ commit --

/// Run git in `dir`, asserting it succeeded; stdout as text.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("running git");
    assert!(
        out.status.success(),
        "git {args:?} failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository at `dir` with an identity, so `neb`'s commits do not depend
/// on the developer's global git configuration.
fn git_init(dir: &std::path::Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "neb-test"]);
    git(dir, &["config", "user.email", "neb-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// The paths a commit touched, relative to the repository's top level.
fn committed_paths(dir: &std::path::Path, rev: &str) -> Vec<String> {
    git(dir, &["show", "--name-only", "--format=", rev])
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn head(dir: &std::path::Path) -> String {
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

#[test]
fn commit_is_off_by_default_and_off_means_nothing_is_committed() {
    // Not a repository at all: with the setting off, git is never consulted.
    let (_plain, plain) = corpus();
    assert!(!plain.commit_setting().enabled);
    seed(&plain, "A", &[]);
    assert!(matches!(ops::commit(&plain, "new", &["a"]), Ok(None)));

    // In a repository, off still means the write is left uncommitted.
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    seed(&corpus, "A", &[]);
    assert!(matches!(ops::commit(&corpus, "new", &["a"]), Ok(None)));
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? nodes/"),
        "the write is there, not staged, not committed"
    );

    // The setting is a `config.yaml` key that is absent until turned on, so
    // a file written before it existed renders back unchanged.
    let config = || std::fs::read_to_string(root.join("config.yaml")).unwrap();
    assert!(!config().contains("commit"), "{}", config());
    assert!(ops::set_commit(&mut corpus, true).unwrap().enabled);
    assert!(config().contains("commit: true"), "{}", config());
    assert!(!ops::set_commit(&mut corpus, false).unwrap().enabled);
    assert!(!config().contains("commit"), "{}", config());
}

#[test]
fn commit_on_records_each_write_as_one_commit_of_corpus_paths_only() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    let start = ops::commit(&corpus, "config", &["commit"])
        .unwrap()
        .expect("turning it on is itself recorded");
    assert_eq!(start.message, "neb config commit");
    assert_eq!(
        committed_paths(&root, "HEAD"),
        [".gitignore", "config.yaml"]
    );

    // A capture creates inbox/, a promote writes a node and settles the
    // capture: one commit each, naming what the verb touched.
    let entry = ops::capture(&corpus, "a thought").unwrap();
    let captured = ops::commit(&corpus, "capture", &[&entry.id])
        .unwrap()
        .expect("a commit");
    assert_eq!(captured.message, format!("neb capture {}", entry.id));
    assert_eq!(captured.hash, head(&root));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    let id = ops::promote(&corpus, &entry.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let promoted = ops::commit(&corpus, "promote", &[&entry.id, &id])
        .unwrap()
        .expect("a commit");
    assert_eq!(promoted.message, format!("neb promote {} {id}", entry.id));
    let mut paths = committed_paths(&root, "HEAD");
    paths.sort();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.iter().any(|p| p == &format!("nodes/{id}.md")));
    assert!(paths.iter().any(|p| p.starts_with("inbox/")));

    // A stray file under the root that is not a corpus path is left alone,
    // and a write that changed nothing produces no commit.
    std::fs::write(root.join("scratch.txt"), "not the corpus\n").unwrap();
    let before = head(&root);
    assert!(matches!(ops::commit(&corpus, "note", &[&id]), Ok(None)));
    assert_eq!(head(&root), before);
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? scratch.txt"),
        "scratch.txt is neither staged nor committed"
    );
    assert!(
        git(
            &root,
            &[
                "status",
                "--porcelain",
                "--",
                "nodes",
                "inbox",
                "config.yaml"
            ]
        )
        .is_empty()
    );
    assert!(
        git(&root, &["remote"]).is_empty(),
        "nothing to push to, and nothing pushed"
    );
}

#[test]
fn a_commit_is_refused_when_something_outside_the_corpus_is_staged_and_the_write_stays() {
    // The corpus is nested in a larger repository, the shape the refusal
    // exists for: a file staged elsewhere must not ride in a `neb` commit.
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join("README.md"), "theirs\n").unwrap();
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    ops::commit(&corpus, "config", &["commit"])
        .unwrap()
        .unwrap();
    assert_eq!(committed_paths(&outer, "HEAD"), ["corpus/config.yaml"]);

    std::fs::write(outer.join("README.md"), "theirs, edited\n").unwrap();
    git(&outer, &["add", "README.md"]);
    let before = head(&outer);
    let a = seed(&corpus, "A", &[]);
    let err = ops::commit(&corpus, "new", &[&a]).unwrap_err();
    assert!(
        matches!(&err, Error::StagedElsewhere { root: r, paths } if r == &root && paths == &["README.md"]),
        "got {err:?}"
    );
    assert_eq!(head(&outer), before, "nothing was committed");
    assert!(
        corpus.node_path(&a).unwrap().exists(),
        "the write is never rolled back because of git"
    );
    let status = git(&outer, &["status", "--porcelain"]);
    assert!(
        status.contains("M  README.md"),
        "their staging is intact:\n{status}"
    );
    assert!(
        status.contains("?? corpus/nodes/"),
        "the node was not even staged:\n{status}"
    );

    // Once their change is out of the way, the next commit sweeps the node.
    git(&outer, &["commit", "-q", "-m", "theirs"]);
    let done = ops::commit(&corpus, "new", &[&a]).unwrap().unwrap();
    assert_eq!(done.message, format!("neb new {a}"));
    assert_eq!(
        committed_paths(&outer, "HEAD"),
        [format!("corpus/nodes/{a}.md")]
    );
}

#[test]
fn staged_corpus_paths_with_unicode_spaces_and_newlines_are_unambiguous() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    git_init(&outer);
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    ops::commit(&corpus, "config", &["commit"])
        .unwrap()
        .unwrap();

    let names = ["시간.md", "two words.md", "two\nlines.md"];
    for name in names {
        std::fs::write(root.join("inbox").join(name), "fixture\n").unwrap();
    }
    git(&outer, &["add", "--", "corpus/inbox"]);

    let id = seed(&corpus, "A", &[]);
    ops::commit(&corpus, "new", &[&id]).unwrap().unwrap();

    let committed = git(&outer, &["show", "--name-only", "--format=", "-z", "HEAD"]);
    let paths: Vec<&str> = committed
        .split('\0')
        .filter(|path| !path.is_empty())
        .collect();
    for name in names {
        assert!(
            paths.contains(&format!("corpus/inbox/{name}").as_str()),
            "missing {name:?} from {paths:?}"
        );
    }
    assert!(paths.contains(&format!("corpus/nodes/{id}.md").as_str()));
}

#[test]
fn commit_on_in_a_corpus_the_containing_repository_ignores_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join(".gitignore"), "corpus/\n").unwrap();
    ops::set_commit(&mut corpus, true).unwrap();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::commit(&corpus, "new", &[&a]),
        Err(Error::CorpusIgnored(r)) if r == root
    ));
    assert!(corpus.node_path(&a).unwrap().exists());
}

// -------------------------------------------- settings written under a lock --

/// `Corpus::open` reads `config.yaml` without the lock, because a read must
/// never wait on a writer. A writer that then waits its turn is holding a
/// snapshot from before the writer ahead of it finished, and every config
/// write rewrites the file whole — so without a reload under the lock the
/// waiter carries the stale copy of every key it is not setting back to disk
/// and the other writer's setting is gone without a word.
///
/// Two `Corpus` handles on one thread rather than two threads: the interleave
/// under test is a *stale open*, not contention, and sequencing it by hand is
/// what makes the loss deterministic. `crates/neb/tests/cli.rs` runs the same
/// shape across two processes against a really held `flock`.
#[test]
fn a_setting_landed_since_open_survives_the_next_writers_rewrite() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let observatory = dir.path().join("observatory");

    // The waiter opens — and so snapshots the config — before the writer
    // ahead of it in the queue has written anything.
    let mut waiting = Corpus::open(Some(root.clone())).unwrap();
    assert!(!waiting.commit_setting().enabled);

    // The writer ahead finishes its own config write and lets go.
    let mut ahead = Corpus::open(Some(root.clone())).unwrap();
    assert!(ops::set_commit(&mut ahead, true).unwrap().enabled);

    // The waiter gets in and sets a different key.
    let setting = ops::set_observatory_root(&mut waiting, observatory.clone()).unwrap();
    assert_eq!(setting.root.as_deref(), Some(observatory.as_path()));

    let reopened = Corpus::open(Some(root)).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the commit setting written after the waiter opened was erased by its rewrite"
    );
    assert_eq!(
        reopened.observatory_root().root.as_deref(),
        Some(observatory.as_path()),
        "and the waiter's own setting must still have landed"
    );
}

/// The same loss in the other direction, so neither setting is merely the one
/// that happens to be written last.
#[test]
fn a_stale_commit_write_keeps_the_observatory_root_written_since_it_opened() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let observatory = dir.path().join("observatory");

    let mut waiting = Corpus::open(Some(root.clone())).unwrap();
    assert!(waiting.observatory_root().root.is_none());

    let mut ahead = Corpus::open(Some(root.clone())).unwrap();
    ops::set_observatory_root(&mut ahead, observatory.clone()).unwrap();

    assert!(ops::set_commit(&mut waiting, true).unwrap().enabled);

    let reopened = Corpus::open(Some(root)).unwrap();
    assert_eq!(
        reopened.observatory_root().root.as_deref(),
        Some(observatory.as_path()),
        "the observatory root written after the waiter opened was erased"
    );
    assert!(reopened.commit_setting().enabled);
}

/// Whether a write is recorded is a question about the configuration in
/// force, not the one this corpus happened to open with. A verb that waited
/// out a writer who turned the setting on commits; one that waited out a
/// writer who turned it off does not.
#[test]
fn the_commit_decision_follows_the_setting_on_disk_not_the_one_at_open() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);

    // Opened while the setting was off.
    let stale_off = Corpus::open(Some(root.clone())).unwrap();
    assert!(!stale_off.commit_setting().enabled);

    // Another writer turns it on and records that.
    let mut ahead = Corpus::open(Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, true).unwrap();
    ops::commit(&ahead, "config", &["commit"])
        .unwrap()
        .expect("turning it on records itself");

    let entry = ops::capture(&stale_off, "a thought the setting now says to record").unwrap();
    let done = ops::commit(&stale_off, "capture", &[&entry.id])
        .unwrap()
        .expect("the setting in force is on, so the write is recorded");
    assert_eq!(done.message, format!("neb capture {}", entry.id));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    // And the other way: opened while it was on, but off by the time the
    // verb lands.
    let stale_on = Corpus::open(Some(root.clone())).unwrap();
    assert!(stale_on.commit_setting().enabled);
    let mut ahead = Corpus::open(Some(root.clone())).unwrap();
    ops::set_commit(&mut ahead, false).unwrap();

    let before = head(&root);
    let entry = ops::capture(&stale_on, "a thought the setting now says to leave").unwrap();
    assert!(matches!(
        ops::commit(&stale_on, "capture", &[&entry.id]),
        Ok(None)
    ));
    assert_eq!(head(&root), before, "nothing was committed");
    assert!(
        git(&root, &["status", "--porcelain"]).contains(" M inbox/"),
        "the capture is on disk, just not recorded"
    );
}

// -------------------------------------------------------------------- lock --

/// Two writers editing one node's tags at the same time. Each op is a load,
/// an edit and a save; without the lock the second load sees the corpus as it
/// was before the first save, and the later rename wins — one tag survives and
/// the other is silently gone.
///
/// Threads rather than processes here: this is what the in-process half of
/// the lock is for, since `flock` is held by the open file description and
/// would let a second thread of one process straight through.
/// `crates/neb/tests/cli.rs` runs the same race across two spawned binaries,
/// which is what exercises `flock` itself.
#[test]
fn two_concurrent_tag_adds_on_one_node_both_survive() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "A node two writers will tag", &[]);
    let root = dir.path().join("corpus");

    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        for tag in ["alpha", "beta"] {
            let (root, start, id) = (root.clone(), &start, id.clone());
            scope.spawn(move || {
                let corpus = Corpus::open(Some(root)).expect("open");
                start.wait();
                ops::tag_add(&corpus, &id, &[tag.to_string()]).expect("tag");
            });
        }
    });

    let mut tags = corpus.load(&id).unwrap().node.tags;
    tags.sort();
    assert_eq!(tags, ["alpha", "beta"], "one writer's tag was lost");
}

/// Past the bounded wait the writer refuses rather than proceeding, and
/// refuses *before* touching the node: the error is the whole outcome.
#[test]
fn a_write_against_a_held_lock_refuses_and_changes_nothing() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "A node nobody gets to edit", &[]);
    let before = std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap();
    let root = dir.path().join("corpus");

    // Held on another thread, because the lock is re-entrant on the thread
    // that already has it — which is what lets the CLI hold it across a verb
    // and the commit that records it.
    let held = CorpusLock::acquire(&root).unwrap();
    let refused = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let corpus = Corpus::open(Some(root.clone())).expect("open");
                // The op uses the real bound, so this waits the full five
                // seconds before refusing. That wait is the thing under test.
                ops::tag_add(&corpus, &id, &["never".to_string()])
            })
            .join()
            .expect("the waiting writer did not panic")
    });

    assert!(
        matches!(&refused, Err(Error::Locked { root: r }) if r == &root),
        "got {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(corpus.node_path(&id).unwrap()).unwrap(),
        before,
        "the refusal came before the write, so the node is untouched"
    );

    // And a reader never waited on any of it.
    assert!(corpus.load(&id).is_ok());
    assert!(Corpus::open(Some(root.clone())).is_ok());
    drop(held);
}

/// The lock file is the one thing under the root that is not the corpus, so
/// nothing that reads or records the corpus may see it.
#[test]
fn the_lock_file_is_neither_committed_nor_checked() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    ops::commit(&corpus, "config", &["commit"])
        .unwrap()
        .unwrap();

    let id = seed(&corpus, "A node whose write took the lock", &[]);
    ops::commit(&corpus, "new", &[&id]).unwrap().unwrap();
    assert!(
        root.join(nebula_core::LOCK_FILE).exists(),
        "the write took the lock, so the file is there to be excluded"
    );

    assert_eq!(committed_paths(&root, "HEAD"), [format!("nodes/{id}.md")]);
    assert!(
        !git(&root, &["ls-files"]).contains(nebula_core::LOCK_FILE),
        "the lock file is not tracked"
    );
    assert!(
        !git(&root, &["status", "--porcelain"]).contains(".lock"),
        "the ignored lock is absent from repository status"
    );
    assert_eq!(git(&root, &["check-ignore", ".lock"]), ".lock\n");

    let docs = corpus.load_all().unwrap();
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert_eq!(report.nodes, 1, "the lock file is not read as a node");
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

// ---------------------------------------------------------------- node ids --

/// Rewrite the id a node file stores, the way a hand edit would.
fn rewrite_stored_id(path: &std::path::Path, to: &str) {
    let raw = std::fs::read_to_string(path).expect("node file");
    let (first, rest) = raw.split_once('\n').expect("frontmatter");
    assert!(first == "---", "a node file starts with `---`");
    let rewritten = rest
        .lines()
        .map(|line| {
            if line.starts_with("id: ") {
                format!("id: {to}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("---\n{rewritten}\n")).expect("rewrite");
}

/// A stored id that is not one file name never becomes a `Doc`, so no verb
/// downstream can derive a destination from it. The file that proved this
/// necessary said `id: ../../escaped` and a `note` wrote `escaped.md` beside
/// the corpus root.
#[test]
fn a_stored_id_that_leaves_nodes_fails_to_load_and_writes_nothing() {
    for escape in ["../../escaped", "../escaped", "nodes/elsewhere", ".."] {
        let (dir, corpus) = corpus();
        let id = seed(&corpus, "Safe", &[]);
        let path = corpus.node_path(&id).unwrap();
        rewrite_stored_id(&path, escape);

        let refused = ops::note(&corpus, &id, "a fixture note", None);
        assert!(
            matches!(&refused, Err(Error::IdMismatch { path: p, id: stored }) if p == &path && stored == escape),
            "`{escape}` got {refused:?}"
        );
        assert!(corpus.load(&id).is_err(), "`{escape}` still loads");
        assert!(corpus.load_all().is_err(), "`{escape}` survives a scan");
        let outside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        assert_eq!(outside, ["corpus"], "`{escape}` wrote beside the corpus");
    }
}

/// An absolute id is the same escape spelled differently, and is refused the
/// same way rather than replacing the join wholesale.
#[test]
fn a_stored_id_that_is_an_absolute_path_fails_to_load() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "Safe", &[]);
    let elsewhere = dir.path().join("escaped");
    rewrite_stored_id(
        &corpus.node_path(&id).unwrap(),
        &elsewhere.display().to_string(),
    );

    assert!(ops::note(&corpus, &id, "a fixture note", None).is_err());
    assert!(!elsewhere.with_extension("md").exists(), "wrote outside");
}

/// The quieter half of the same bug: a stored id that *is* a valid id, but
/// another node's. `save` writes where the id says, so loading this file and
/// noting on it overwrote the node it named.
#[test]
fn a_node_file_that_claims_another_nodes_id_is_refused_before_the_write() {
    let (_dir, corpus) = corpus();
    let safe = seed(&corpus, "Safe", &[]);
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let before = std::fs::read_to_string(&victim_path).unwrap();
    rewrite_stored_id(&corpus.node_path(&safe).unwrap(), &victim);

    let refused = ops::note(&corpus, &safe, "a fixture note", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &corpus.node_path(&safe).unwrap() && id == &victim),
        "got {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        before,
        "the other node is untouched"
    );
    // A scan finds nodes by path, so it refuses from the other direction
    // rather than answering every query from the wrong file.
    assert!(matches!(corpus.load_all(), Err(Error::IdMismatch { .. })));
}

/// The same bug without the hand edit. Copying a node file leaves two files
/// whose text is equal down to the byte, and agreement was once decided by
/// comparing that text — so `nodes/safe.md`, a copy of `nodes/victim.md`,
/// vouched for the `victim` it stored and `note safe` appended to the other
/// node. Equal contents are not one file, and never were the question.
#[test]
fn a_copy_of_another_node_file_does_not_authorize_the_id_it_stores() {
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let safe_path = corpus.node_path("safe").unwrap();
    std::fs::copy(&victim_path, &safe_path).expect("copy");
    let before = std::fs::read_to_string(&victim_path).unwrap();
    assert_eq!(
        std::fs::read_to_string(&safe_path).unwrap(),
        before,
        "the fixture is a byte-for-byte copy"
    );

    let refused = ops::note(&corpus, "safe", "a fixture note", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &safe_path && id == &victim),
        "got {refused:?}"
    );
    // Both doors: the id names the copy, and a scan finds it by path.
    assert!(
        matches!(corpus.load("safe"), Err(Error::IdMismatch { .. })),
        "a read answered from the wrong file"
    );
    assert!(
        matches!(corpus.load_all(), Err(Error::IdMismatch { .. })),
        "a scan read it as the node it claims"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_path).unwrap(),
        before,
        "the node the copy named is untouched"
    );
    assert_eq!(
        std::fs::read_to_string(&safe_path).unwrap(),
        before,
        "and so is the file that was asked for"
    );
}

/// One file under two names in `nodes/`. The names once opened the same inode
/// and so agreed, but the write that followed replaced `victim.md` through a
/// rename: `safe.md` kept the old bytes under a name that no longer matched
/// anything, and the next load of either refused. The alias is refused before
/// the write, whichever name the node is reached through.
#[cfg(unix)]
#[test]
fn a_hard_linked_alias_is_refused_before_a_write_can_split_it() {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &std::path::Path| {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let safe_path = corpus.node_path("safe").unwrap();
    std::fs::hard_link(&victim_path, &safe_path).expect("hard link");
    let before = std::fs::read_to_string(&victim_path).unwrap();
    let linked = identity(&victim_path);

    let names_the_alias = |refused: &nebula_core::Result<_>| matches!(refused, Err(Error::IdMismatch { path, id }) if path == &safe_path && id == &victim);
    let through_alias = ops::note(&corpus, "safe", "ALIAS-NOTE", None);
    assert!(names_the_alias(&through_alias), "got {through_alias:?}");
    let through_own_name = ops::note(&corpus, &victim, "OWN-NAME-NOTE", None);
    assert!(
        names_the_alias(&through_own_name),
        "got {through_own_name:?}"
    );
    let scanned = corpus.load_all();
    assert!(
        matches!(&scanned, Err(Error::IdMismatch { path, id }) if path == &safe_path && id == &victim),
        "got {scanned:?}"
    );
    assert!(matches!(corpus.load("safe"), Err(Error::IdMismatch { .. })));
    assert!(matches!(
        corpus.load(&victim),
        Err(Error::IdMismatch { .. })
    ));

    assert_eq!(identity(&victim_path), linked, "no write replaced the node");
    assert_eq!(identity(&safe_path), linked, "the pair was not split");
    assert_eq!(std::fs::read_to_string(&victim_path).unwrap(), before);
}

/// A symlink alias stays linked across a write, but a scan read the node
/// twice and refused a duplicate id while `load` through the alias succeeded.
/// Both doors now refuse the alias itself, as a mismatch, before any write.
#[cfg(unix)]
#[test]
fn a_symlinked_alias_is_refused_by_load_and_by_a_scan_alike() {
    let (_dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    let victim_path = corpus.node_path(&victim).unwrap();
    let alias_path = corpus.node_path("alias").unwrap();
    std::os::unix::fs::symlink(format!("{victim}.md"), &alias_path).expect("symlink");
    let before = std::fs::read_to_string(&victim_path).unwrap();

    let refused = ops::note(&corpus, "alias", "SYM-NOTE", None);
    assert!(
        matches!(&refused, Err(Error::IdMismatch { path, id }) if path == &alias_path && id == &victim),
        "got {refused:?}"
    );
    let scanned = corpus.load_all();
    assert!(
        matches!(&scanned, Err(Error::IdMismatch { path, id }) if path == &alias_path && id == &victim),
        "a scan refused something other than the alias: {scanned:?}"
    );
    assert_eq!(std::fs::read_to_string(&victim_path).unwrap(), before);
    assert_eq!(corpus.load(&victim).unwrap().node.id, victim);
}

/// Only a name a scan would read is an alias. A hard link from outside
/// `nodes/`, as a `cp -al` backup makes, leaves the node one entry there.
#[cfg(unix)]
#[test]
fn a_hard_link_outside_the_nodes_directory_does_not_refuse_the_node() {
    let (dir, corpus) = corpus();
    let victim = seed(&corpus, "Victim", &[]);
    std::fs::hard_link(
        corpus.node_path(&victim).unwrap(),
        dir.path().join("backup.md"),
    )
    .expect("hard link");

    ops::note(&corpus, &victim, "a fixture note", None).expect("note");
    assert_eq!(corpus.load_all().expect("scan").len(), 1);
}

/// The one reason two spellings may name one node, pinned to the case it was
/// written for. A volume may store a file name in a different Unicode
/// normalization than the id the file stores, and a scan reading that name
/// back must still recognize the node rather than refuse it. macOS does this
/// and Linux does not, so the case is *detected* rather than assumed: where
/// the volume answers both spellings with one file, the node still loads.
#[test]
fn a_file_name_stored_in_another_normalization_still_loads_and_scans() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Ünïcode título → ok", &[]);
    assert_eq!(id, "ünïcode-título-ok");
    let composed = corpus.node_path(&id).unwrap();
    // The same word decomposed: each accent written as a combining mark.
    let decomposed = composed.with_file_name("u\u{308}ni\u{308}code-ti\u{301}tulo-ok.md");
    std::fs::rename(&composed, &decomposed).expect("rename to the decomposed spelling");

    if !composed.exists() {
        // A byte-exact volume: the two spellings are two names, the node the
        // id points at is simply gone, and there is nothing to agree with.
        assert!(matches!(corpus.load(&id), Err(Error::NoSuchNode(_))));
        return;
    }

    assert_eq!(
        corpus
            .load(&id)
            .expect("the id still opens the node")
            .node
            .id,
        id,
        "the stored id is the composed spelling the volume did not keep"
    );
    let docs = corpus
        .load_all()
        .expect("a scan accepts the stored spelling");
    assert_eq!(docs.len(), 1, "one file, read once");
    assert_eq!(docs[0].node.id, id);
    ops::note(&corpus, &id, "a fixture note", None).expect("and a write still lands");
}

/// A caller-supplied id is a path the moment it is used, whether it came from
/// a terminal, the desktop, or an agent.
#[test]
fn a_caller_supplied_id_that_is_not_one_file_name_is_refused() {
    let (dir, corpus) = corpus();
    seed(&corpus, "Safe", &[]);
    // A real node file outside the corpus: the refusal must not depend on
    // there being nothing to find.
    let secret = dir.path().join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(
        secret.join("leak.md"),
        "---\nid: leak\ntitle: fixture node outside the corpus\nstatus: seed\n\
         created: 2026-09-22\nupdated: 2026-09-22\n---\n\nfixture body\n",
    )
    .unwrap();
    let absolute = secret.join("leak").display().to_string();

    for id in ["../../secret/leak", "../secret/leak", &absolute, "..", ""] {
        assert!(
            matches!(corpus.node_path(id), Err(Error::UnsafeId(_))),
            "node_path accepted `{id}`"
        );
        for refused in [
            ops::note(&corpus, id, "a fixture note", None).err(),
            ops::sharpen(&corpus, id, "a fixture kill", None).err(),
            ops::tag_add(&corpus, id, &["fixture".to_string()]).err(),
            corpus.load(id).err(),
            corpus.history(id).err(),
        ] {
            assert!(
                matches!(refused, Some(Error::UnsafeId(_) | Error::NotGitWorkTree(_))),
                "`{id}` got {refused:?}"
            );
        }
    }
    assert!(
        !corpus.node_path("leak").unwrap().exists(),
        "nothing outside the root was read into the corpus"
    );
}

/// The rule is about path structure, not about the alphabet: a Unicode id
/// still captures, loads, saves and checks. macOS stores a file name in a
/// normalization of its own choosing, so this is also where a scan that
/// compared names byte for byte would fail on one platform and pass on the
/// other.
#[test]
fn a_unicode_id_still_round_trips_through_a_write() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Ünïcode título → ok", &[]);
    assert_eq!(id, "ünïcode-título-ok");
    let korean = seed(&corpus, "시간은 프레임의 수다", &[]);

    ops::note(&corpus, &id, "a fixture note", None).expect("note");
    ops::note(&corpus, &korean, "a fixture note", None).expect("note");
    assert_eq!(corpus.load(&id).unwrap().node.id, id);
    let docs = corpus.load_all().expect("a scan reads both back");
    assert_eq!(docs.len(), 2);
    let report = nebula_core::check::run(&Graph::build(&docs).unwrap(), &corpus).unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

/// macOS temporary directories sit under a symlink, so a corpus reached
/// through one is the normal case there rather than an exotic one. Nothing
/// in the id rules canonicalizes, so the path is used as given and the same
/// verbs work.
#[cfg(unix)]
#[test]
fn a_corpus_reached_through_a_symlinked_root_still_writes_and_loads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let real = dir.path().join("real");
    let corpus = Corpus::init(&real).expect("init");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    let linked = Corpus::open(Some(link.clone())).expect("open through the link");
    let id = seed(&linked, "Reached through a link", &[]);
    ops::note(&linked, &id, "a fixture note", None).expect("note");

    assert_eq!(
        linked.node_path(&id).unwrap(),
        link.join("nodes").join(format!("{id}.md")),
        "the path is the one given, not a resolved one"
    );
    assert!(
        corpus.load(&id).is_ok(),
        "the same node through the real path"
    );
    assert_eq!(linked.load_all().unwrap().len(), 1);
}
