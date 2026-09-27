//! Unit tests for `store/inbox.rs`.
//!
//! A capture's stamp picks its month file and seeds its id, so these tests
//! capture through [`Corpus::capture_at`] at fixed stamps: every run takes
//! the same ids, and no run depends on where the wall clock is.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures are planted directly, beside the helper under test"
)]

use super::corpus;
use crate::error::Error;
use crate::fs_impl::{
    Step, create_private_dir_all, create_temporary_sibling, recording, write_private_atomic,
};
use crate::ops_impl::{self as ops, Promotion};
use crate::stamp::stamp;
use crate::store::Corpus;
use crate::store::inbox::is_inbox_month_filename;
use std::collections::HashSet;
use std::ffi::OsStr;

/// A capture instant one second before a minute, an hour and a local day
/// turn over: where a test that read the wall clock was most likely to race.
const STAMP: &str = "2026-09-26T23:59:59+02:00";

/// How many ids one stamp and text hash to before an id is looked for
/// elsewhere. The chain can repeat itself, so it holds at most this many
/// distinct ids.
const HASH_CANDIDATES: usize = 64;

#[test]
fn capture_at_uses_a_free_id_after_the_hash_candidates_collide() {
    const TEXT: &str = "the intended new thought";
    let (_dir, corpus) = corpus();

    // Every capture of one text at one stamp walks the same hash chain and
    // takes the first id on it still free. Once this many hold the chain,
    // whatever it holds, the next capture has no hash candidate left.
    let occupied: Vec<String> = (0..HASH_CANDIDATES)
        .map(|_| corpus.capture_at(TEXT, STAMP).unwrap().id)
        .collect();
    let distinct: HashSet<&String> = occupied.iter().collect();
    assert_eq!(distinct.len(), HASH_CANDIDATES, "{occupied:?}");

    let captured = corpus.capture_at(TEXT, STAMP).unwrap();

    assert!(
        !occupied.contains(&captured.id),
        "{} is already taken",
        captured.id
    );
    assert_eq!(captured.at, STAMP);
    let resolved = corpus.inbox_entry(&captured.id).unwrap();
    assert_eq!(
        resolved.line, HASH_CANDIDATES,
        "the id resolves to the new line"
    );
    assert_eq!(resolved.text, TEXT);

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
    let inbox = corpus.inbox().unwrap();
    assert_eq!(inbox.0.len(), HASH_CANDIDATES);
    assert!(
        inbox.0.iter().all(|entry| entry.id != captured.id),
        "only the new capture was settled"
    );
}

#[test]
fn capture_at_ids_are_unique_across_month_files_and_older_entries_still_resolve() {
    const TEXT: &str = "the same thought";
    const OLDER: &str = "2000-01-15T09:30:00+00:00";

    // The id a capture of TEXT at STAMP takes in an empty inbox.
    let (_scratch, scratch) = corpus();
    let taken = scratch.capture_at(TEXT, STAMP).unwrap().id;

    // An older month's file already holds that id.
    let (dir, corpus) = corpus();
    let inbox = dir.path().join("corpus").join("inbox");
    create_private_dir_all(&inbox).unwrap();
    let older_file = inbox.join("2000-01.md");
    write_private_atomic(&older_file, format!("- [{taken}] {OLDER} {TEXT}\n")).unwrap();

    let second = corpus.capture_at(TEXT, STAMP).unwrap();

    assert_ne!(second.id, taken, "ids are unique across month files");
    assert_eq!(second.file, inbox.join("2026-09.md"));
    let older = corpus.inbox_entry(&taken).unwrap();
    assert_eq!(older.file, older_file);
    assert_eq!(older.at, OLDER);
    assert_eq!(older.text, TEXT);
    let newer = corpus.inbox_entry(&second.id).unwrap();
    assert_eq!(newer.file, second.file);
    assert_eq!(newer.at, STAMP);
}

/// Seven `write(2)` calls for one capture could be cut anywhere by a
/// crash. One buffer, one `write_all`, then the data flushed.
#[test]
fn a_capture_line_is_built_whole_and_written_once() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(&crate::Locations::default(), &dir.path().join("corpus")).unwrap();
    let month = dir
        .path()
        .join("corpus")
        .join("inbox")
        .join(format!("{}.md", &stamp()[..7]));
    // A month file whose last line lost its newline to a hand edit.
    std::fs::write(&month, "- [0001] 2026-09-01T00:00 an earlier thought").unwrap();

    let (entry, steps) = recording(|| corpus.capture("a thought, torn nowhere"));
    let entry = entry.unwrap();

    let line = format!("\n- [{}] {} a thought, torn nowhere\n", entry.id, entry.at);
    assert_eq!(
        steps,
        [
            Step::Write {
                path: month.clone(),
                bytes: line.clone().into_bytes(),
            },
            Step::SyncData(month.clone()),
        ],
        "the repair newline and the line are one write, then synced"
    );
    assert_eq!(
        std::fs::read_to_string(&month).unwrap(),
        format!("- [0001] 2026-09-01T00:00 an earlier thought{line}")
    );

    // The next capture needs no repair, and is still one write.
    let (entry, steps) = recording(|| corpus.capture("and another"));
    let entry = entry.unwrap();
    assert_eq!(
        steps,
        [
            Step::Write {
                path: month.clone(),
                bytes: format!("- [{}] {} and another\n", entry.id, entry.at).into_bytes(),
            },
            Step::SyncData(month.clone()),
        ]
    );
}

#[test]
fn a_temporary_sibling_is_fresh_each_time_and_hides_from_corpus_listings() {
    let dir = tempfile::tempdir().unwrap();
    let node = dir.path().join("safe.md");
    let (first, _handle) = create_temporary_sibling(&node).unwrap();
    let (second, _handle) = create_temporary_sibling(&node).unwrap();

    assert_ne!(
        first, second,
        "a stale temporary must not wedge the next write"
    );
    for tmp in [&first, &second] {
        assert_eq!(tmp.parent(), node.parent(), "the temporary is a sibling");
        assert!(
            tmp.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("safe.md."),
            "the temporary does not name its destination: {}",
            tmp.display()
        );
        assert_eq!(
            tmp.extension(),
            Some(OsStr::new("tmp")),
            "`load_all` reads every `.md` in `nodes`, so a temporary may not be one"
        );
    }

    let (month, _handle) = create_temporary_sibling(&dir.path().join("2026-09.md")).unwrap();
    assert!(
        !is_inbox_month_filename(month.file_name().unwrap()),
        "the inbox would read this temporary as a month file: {}",
        month.display()
    );
}

// Store writers are crate-private. These cases used to live in the integration
// suite, which can no longer call them.
use crate::Settlement;

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
        Err(Error::InboxEntrySettled { id, settlement: Settlement::Dropped }) if id == entry.id
    ));
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
        assert!(
            matches!(&error, Error::InboxSymlink(path) if path == inbox_dir),
            "{error}"
        );
    }
    assert_eq!(std::fs::read(&outside_month).unwrap(), before);
    assert!(corpus.load_all().unwrap().is_empty());
}

/// An entry's file is the caller's to set, and settling writes it. Only
/// `<root>/inbox/<month>.md` is taken: a path that merely starts with the
/// inbox can climb out of it with `..`, and the entry's line copied there
/// would be struck in a file that is not the inbox's (STD-05 §R6).
#[test]
fn settling_refuses_an_entry_file_that_leaves_the_inbox_by_name() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let content = std::fs::read_to_string(&entry.file).unwrap();
    let root = dir.path().join("corpus");
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir(&outside).unwrap();
    let month = entry.file.file_name().unwrap();
    std::fs::write(root.join(month), &content).unwrap();
    std::fs::write(outside.join(month), &content).unwrap();

    for file in [
        root.join("inbox").join("..").join(month),
        root.join("inbox")
            .join("..")
            .join("..")
            .join("elsewhere")
            .join(month),
        outside.join(month),
    ] {
        let mut foreign = entry.clone();
        foreign.file = file.clone();
        let error = corpus.settle_inbox(&foreign, "dropped").unwrap_err();
        assert!(
            matches!(&error, Error::InboxEntryForeign { file: named, .. } if *named == file),
            "{error}"
        );
    }
    assert_eq!(std::fs::read_to_string(root.join(month)).unwrap(), content);
    assert_eq!(
        std::fs::read_to_string(outside.join(month)).unwrap(),
        content
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), content);
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

/// `inbox/` is judged by `lstat`, and so is the month file, but nothing
/// between them is: an entry whose file sits in a symlinked directory under
/// the inbox is refused by its shape rather than followed out of the corpus.
#[cfg(unix)]
#[test]
fn settling_refuses_an_entry_file_below_a_symlinked_inbox_subdirectory() {
    let (dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the live thought").unwrap();
    let content = std::fs::read_to_string(&entry.file).unwrap();
    let outside = dir.path().join("outside-inbox");
    std::fs::create_dir(&outside).unwrap();
    let month = entry.file.file_name().unwrap();
    std::fs::write(outside.join(month), &content).unwrap();
    let planted = entry.file.parent().unwrap().join("sub");
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    let mut foreign = entry.clone();
    foreign.file = planted.join(month);
    let error = corpus.settle_inbox(&foreign, "dropped").unwrap_err();

    assert!(
        matches!(&error, Error::InboxEntryForeign { file, .. } if *file == foreign.file),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join(month)).unwrap(),
        content
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), content);
    corpus.settle_inbox(&entry, "dropped").unwrap();
    assert!(corpus.inbox().unwrap().0.is_empty());
}

#[test]
fn settling_refuses_when_the_indexed_line_has_another_entry_id() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the original thought").unwrap();
    let replacement = "- [other] 2026-09-22T08:25 another thought\n";
    std::fs::write(&entry.file, replacement).unwrap();

    let error = corpus.settle_inbox(&entry, "dropped").unwrap_err();

    assert!(
        matches!(&error, Error::InboxEntryChanged { id, file, line: 0 } if *id == entry.id && *file == entry.file),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), replacement);
}

/// The line gone and the line changed are two failure modes, and each is
/// its own refusal naming the entry and its file (STD-02 §R26).
#[test]
fn a_missing_and_a_changed_inbox_line_are_distinct_refusals() {
    let (_dir, corpus) = corpus();
    ops::capture(&corpus, "first thought").unwrap();
    let entry = ops::capture(&corpus, "second thought").unwrap();
    assert_eq!(entry.line, 1);
    let before = std::fs::read_to_string(&entry.file).unwrap();

    // The file now ends before the entry's line.
    let first_line = format!("{}\n", before.lines().next().unwrap());
    std::fs::write(&entry.file, &first_line).unwrap();
    let missing = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    assert!(
        matches!(&missing, Error::InboxEntryMissing { id, file, line: 1 } if *id == entry.id && *file == entry.file),
        "{missing}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), first_line);

    // The line is there, holding other text.
    let changed_text = format!("{first_line}- [beef] 2026-09-22T08:25 someone else's\n");
    std::fs::write(&entry.file, &changed_text).unwrap();
    let changed = corpus.settle_inbox(&entry, "dropped").unwrap_err();
    assert!(
        matches!(&changed, Error::InboxEntryChanged { id, file, line: 1 } if *id == entry.id && *file == entry.file),
        "{changed}"
    );
    assert_eq!(std::fs::read_to_string(&entry.file).unwrap(), changed_text);

    assert_ne!(missing.code(), changed.code());
    for error in [&missing, &changed] {
        let said = error.to_string();
        assert!(said.contains(&entry.id), "{said}");
        assert!(said.contains(&entry.file.display().to_string()), "{said}");
    }
}

/// A legacy stamp has no offset. It is read as local time, and settling the
/// entry strikes the line through with the stamp exactly as it was written.
#[test]
fn settling_a_legacy_entry_keeps_its_stamp_as_written() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "a new thought").unwrap();
    let legacy = entry.file.with_file_name("2000-01.md");
    std::fs::write(&legacy, "- [abcd] 2026-09-01T08:00 old thought\n").unwrap();

    let old = corpus.inbox_entry("abcd").unwrap();
    assert!(old.at.starts_with("2026-09-01T08:00:00"), "{}", old.at);
    assert!(
        time::OffsetDateTime::parse(&old.at, &time::format_description::well_known::Rfc3339)
            .is_ok(),
        "{}",
        old.at
    );
    ops::drop(&corpus, "abcd").unwrap();

    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        "- ~~[abcd] 2026-09-01T08:00 old thought~~ dropped\n"
    );
    assert_eq!(corpus.inbox_entry(&entry.id).unwrap().at, entry.at);
}

#[test]
fn capture_line_joins_every_line_break_into_one_space() {
    for (text, line) in [
        ("one line", "one line"),
        ("  padded  ", "padded"),
        ("a\nb\n", "a b"),
        ("a\r\nb\r\n", "a b"),
        ("a\rb", "a b"),
        ("a  \n\n\t  b", "a b"),
        ("\n\nleading and trailing\n\n", "leading and trailing"),
        ("inner  spacing\tstays\nput", "inner  spacing\tstays put"),
        ("", ""),
        (" \n\r\n\t ", ""),
    ] {
        assert_eq!(
            crate::store::inbox::capture_line(text),
            line,
            "joining {text:?}"
        );
    }
}

/// Ids are unique among waiting entries only, so a struck-through line can
/// share its id with a later one, and a hand edit can leave an outcome that
/// no verb wrote.
#[test]
fn a_settled_id_resolves_to_its_latest_recorded_outcome() {
    let (_dir, corpus) = corpus();
    let live = ops::capture(&corpus, "waiting").unwrap();
    std::fs::write(
        live.file.with_file_name("2000-01.md"),
        format!(
            "- ~~[abcd] 2000-01-01T00:00 first~~ dropped\n\
             - ~~[abcd] 2000-01-02T00:00 a ~~struck~~ word~~ -> second-node\n\
             - ~~[beef] 2000-01-03T00:00 edited~~ merged elsewhere\n\
             - ~~[f00d] 2000-01-04T00:00 empty~~ ->\n\
             - ~~[{}] 2000-01-05T00:00 older~~ dropped\n",
            live.id
        ),
    )
    .unwrap();

    assert!(matches!(
        corpus.inbox_entry("abcd"),
        Err(Error::InboxEntrySettled { settlement: Settlement::Promoted(node), .. })
            if node == "second-node"
    ));
    for unrecognised in ["beef", "f00d"] {
        assert!(matches!(
            corpus.inbox_entry(unrecognised),
            Err(Error::NoSuchInboxEntry(id)) if id == unrecognised
        ));
    }
    assert_eq!(
        corpus.inbox_entry(&live.id).unwrap().text,
        "waiting",
        "a waiting entry wins over a settled one with its id"
    );
}
