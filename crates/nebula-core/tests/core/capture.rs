//! Capture and the inbox: month files that are not regular, blank and
//! multiline captures, id exhaustion, repeated and settled entries, and writes
//! that may not reach outside the corpus.

use crate::harness::{
    corpus, live_inbox, mkfifo, not_regular, process_locations, seed, within_two_seconds,
};
use nebula_core::{Corpus, Error, Graph, Promotion, Settlement, Status, graph, ops};
use std::fmt::Write as _;

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

    assert!(matches!(error, Error::InboxSymlink(path) if path == entry.file));
    assert_eq!(std::fs::read(&outside).unwrap(), before);
    assert!(
        std::fs::symlink_metadata(&entry.file)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Capture reads the month file before it appends, and a FIFO there held
/// that read until something wrote to it. It is refused instead.
#[cfg(unix)]
#[test]
fn capture_refuses_a_fifo_month_file_without_blocking() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "existing thought").unwrap();
    std::fs::remove_file(&entry.file).unwrap();
    mkfifo(&entry.file);

    let refused = within_two_seconds(move || ops::capture(&corpus, "must not block"));
    assert!(
        not_regular(&refused, &entry.file, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );
}

/// `init` copies a symlinked `.gitignore`'s target on purpose, but only a
/// regular file's: a FIFO at `.gitignore`, or at the end of its link, is
/// refused rather than read.
#[cfg(unix)]
#[test]
fn init_refuses_a_fifo_gitignore_without_blocking() {
    let (dir, _corpus) = corpus();
    let root = dir.path().join("corpus");
    let gitignore = root.join(".gitignore");
    std::fs::remove_file(&gitignore).unwrap();
    mkfifo(&gitignore);
    let refused = within_two_seconds({
        let root = root.clone();
        move || Corpus::init(&process_locations(), &root)
    });
    assert!(
        not_regular(&refused, &gitignore, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );

    let fifo = dir.path().join("outside-fifo");
    std::fs::rename(&gitignore, &fifo).unwrap();
    std::os::unix::fs::symlink(&fifo, &gitignore).unwrap();
    let refused = within_two_seconds(move || Corpus::init(&process_locations(), &root));
    assert!(
        not_regular(&refused, &gitignore, nebula_core::fs::EntryKind::Fifo),
        "{refused:?}"
    );
    assert!(
        std::fs::symlink_metadata(&gitignore)
            .unwrap()
            .file_type()
            .is_symlink()
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
fn whitespace_only_capture_is_refused_without_writing() {
    let (_dir, corpus) = corpus();
    let existing = ops::capture(&corpus, "already here").unwrap();
    let before = std::fs::read_to_string(&existing.file).unwrap();

    for text in ["", "   ", "\n", " \r\n\t\n "] {
        let error = ops::capture(&corpus, text).unwrap_err();
        assert!(
            matches!(&error, Error::EmptyCapture),
            "capturing {text:?}: {error}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&existing.file).unwrap(),
        before,
        "a refused capture must not change the inbox file"
    );
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

/// Each of these rules is spelled once, here, so every surface reports the
/// same code for it; `neb`'s test of the same name checks the CLI agrees
/// (STD-02 §R24). None of them touches the node.
#[test]
fn blank_capture_note_and_open_status_reason_are_typed_refusals() {
    let (dir, corpus) = corpus();
    let id = seed(&corpus, "An idea", &[]);
    let node_file = dir.path().join("corpus/nodes").join(format!("{id}.md"));
    let before = std::fs::read_to_string(&node_file).unwrap();

    let capture = ops::capture(&corpus, "   ").unwrap_err();
    assert!(matches!(capture, Error::EmptyCapture), "{capture}");
    assert_eq!(capture.code(), "empty_capture");
    let note = ops::note(&corpus, &id, "  \n ", None).unwrap_err();
    assert!(matches!(note, Error::EmptyNote), "{note}");
    assert_eq!(note.code(), "empty_note");

    for open in [Status::Seed, Status::Hypothesis] {
        let reason = ops::set_status(&corpus, &id, open, Some("y")).unwrap_err();
        assert!(
            matches!(reason, Error::ReasonOnOpenStatus(status) if status == open),
            "{reason}"
        );
        assert_eq!(reason.code(), "reason_on_open_status");
    }
    // The arguments decide it, so a node that does not exist is no excuse.
    assert!(matches!(
        ops::set_status(&corpus, "nope", Status::Seed, Some("y")),
        Err(Error::ReasonOnOpenStatus(Status::Seed))
    ));

    assert_eq!(std::fs::read_to_string(&node_file).unwrap(), before);
    assert!(corpus.inbox().unwrap().0.is_empty());
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

    assert!(matches!(error, Error::InboxIdsExhausted));
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
    assert_eq!(live_inbox(&corpus, &first.id).text, first.text);
    assert_eq!(live_inbox(&corpus, &second.id).text, second.text);

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
    assert_eq!(live_inbox(&corpus, &first.id).text, first.text);
}

#[test]
fn a_repeated_capture_names_the_earliest_entry_still_waiting() {
    let (_dir, corpus) = corpus();
    let first = ops::capture(&corpus, "Tags beat  domains").unwrap();
    let unrelated = ops::capture(&corpus, "tags beat a domain").unwrap();
    let second = ops::capture(&corpus, "tags BEAT domains").unwrap();
    let third = ops::capture(&corpus, "  TAGS\tbeat domains ").unwrap();

    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), 4, "a duplicate is still captured");
    let same = |entry| inbox.same_as(entry).map(|e| e.id.clone());
    assert_eq!(same(&second), Some(first.id.clone()), "case is folded");
    assert_eq!(same(&third), Some(first.id.clone()), "whitespace is folded");
    assert_eq!(
        same(&unrelated),
        None,
        "a different word is a different thought"
    );
}

#[test]
fn a_settled_capture_is_not_a_duplicate() {
    let (_dir, corpus) = corpus();
    let dropped = ops::capture(&corpus, "a thought once dropped").unwrap();
    ops::drop(&corpus, &dropped.id).unwrap();
    let promoted = ops::capture(&corpus, "a thought once promoted").unwrap();
    ops::promote(&corpus, &promoted.id, &Promotion::default(), 0).unwrap();

    let again = [
        ops::capture(&corpus, "a thought once dropped").unwrap(),
        ops::capture(&corpus, "A thought once promoted").unwrap(),
    ];

    let inbox = corpus.inbox().unwrap();
    for entry in &again {
        assert!(
            inbox.same_as(entry).is_none(),
            "only waiting entries count: {entry:?}"
        );
    }
}

#[test]
fn promote_and_drop_on_a_settled_entry_say_how_it_was_settled() {
    let (dir, corpus) = corpus();
    let kept = ops::capture(&corpus, "worth keeping").unwrap();
    let node = ops::promote(&corpus, &kept.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let tossed = ops::capture(&corpus, "not worth keeping").unwrap();
    ops::drop(&corpus, &tossed.id).unwrap();
    let inbox_before = std::fs::read_to_string(&kept.file).unwrap();
    let nodes_before = corpus.load_all().unwrap().len();

    for error in [
        ops::promote(&corpus, &kept.id, &Promotion::default(), 0).unwrap_err(),
        ops::drop(&corpus, &kept.id).unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            format!("`{}` was already promoted to `{node}`", kept.id)
        );
        assert!(matches!(
            error,
            Error::InboxEntrySettled { id, settlement: Settlement::Promoted(to) }
                if id == kept.id && to == node
        ));
    }
    for error in [
        ops::promote(&corpus, &tossed.id, &Promotion::default(), 0).unwrap_err(),
        ops::drop(&corpus, &tossed.id).unwrap_err(),
    ] {
        assert_eq!(
            error.to_string(),
            format!("`{}` was already dropped", tossed.id)
        );
        assert!(matches!(
            error,
            Error::InboxEntrySettled { id, settlement: Settlement::Dropped } if id == tossed.id
        ));
    }
    assert!(matches!(
        ops::drop(&corpus, "zzzz"),
        Err(Error::NoSuchInboxEntry(id)) if id == "zzzz"
    ));

    assert_eq!(
        std::fs::read_to_string(&kept.file).unwrap(),
        inbox_before,
        "a refusal writes nothing to the inbox"
    );
    assert_eq!(corpus.load_all().unwrap().len(), nodes_before);
    assert!(
        dir.path()
            .join("corpus/nodes")
            .join(format!("{node}.md"))
            .is_file()
    );
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
    let reopened = Corpus::open(&process_locations(), Some(dir.path().join("corpus"))).unwrap();
    assert!(
        reopened.commit_setting().enabled,
        "the setting did not reach config.yaml"
    );
}

#[test]
fn multiline_capture_is_stored_as_one_inbox_line() {
    let (_dir, corpus) = corpus();
    let existing = ops::capture(&corpus, "already here").unwrap();

    let entry = ops::capture(&corpus, "first line\nsecond line\r\nthird\rfourth\n").unwrap();

    assert_eq!(entry.text, "first line second line third fourth");
    let raw = std::fs::read_to_string(&entry.file).unwrap();
    assert_eq!(raw.lines().count(), 2, "one line per entry:\n{raw}");
    assert_eq!(entry.line, 1);
    assert_eq!(
        raw.lines().nth(entry.line).unwrap(),
        format!("- [{}] {} {}", entry.id, entry.at, entry.text)
    );
    assert_eq!(corpus.inbox().unwrap().0.len(), 2);
    assert_eq!(live_inbox(&corpus, &entry.id).text, entry.text);
    assert_eq!(live_inbox(&corpus, &existing.id).text, existing.text);
}
