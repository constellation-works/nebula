//! `store`, through its crate-internal seams.
//!
//! A capture's stamp picks its month file and seeds its id, so these tests
//! capture through [`Corpus::capture_at`] at fixed stamps: every run takes
//! the same ids, and no run depends on where the wall clock is.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures are planted directly, beside the helper under test"
)]

use crate::config::{self, CommitSetting, Config, Declared, ObservatoryRoot};
use crate::error::{Error, Result};
use crate::fs_impl::{
    EntryKind, Links, append_private, create_private_dir_all, create_private_new,
    read_regular_bytes, read_regular_text, regular_file_at, write_private_atomic,
};
use crate::git::{self, GitAt, GitOutput};
use crate::locations::Locations;
use crate::lock::{CorpusLock, LOCK_FILE};
use crate::model::{self, Doc};
use crate::pending::PENDING_FILE;
use serde::Serialize;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use time::{
    Date, OffsetDateTime, PrimitiveDateTime, UtcOffset,
    format_description::well_known::{Iso8601, Rfc3339},
    macros::format_description,
};

use std::collections::HashSet;

use crate::ops_impl::{self as ops, Promotion};
use crate::store::Corpus;
use crate::store::collect_directory_entries;

/// A capture instant one second before a minute, an hour and a local day
/// turn over: where a test that read the wall clock was most likely to race.
const STAMP: &str = "2026-09-26T23:59:59+02:00";

/// How many ids one stamp and text hash to before an id is looked for
/// elsewhere. The chain can repeat itself, so it holds at most this many
/// distinct ids.
const HASH_CANDIDATES: usize = 64;

fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus =
        Corpus::init(&crate::Locations::default(), &dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

#[test]
fn a_failing_directory_entry_is_reported_by_every_listing() {
    let (_dir, corpus) = corpus();
    // load_all, inbox_files and migration's node listing share this helper.
    // An injected iterator failure is deterministic across filesystems.
    for (consumer, dir) in [
        ("load_all", corpus.root().join("nodes")),
        ("inbox_files", corpus.root().join("inbox")),
        ("migrate", corpus.root().join("nodes")),
    ] {
        let entries = std::iter::once(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected directory entry failure",
        )));
        let listing = collect_directory_entries(&dir, entries);
        assert!(listing.entries.is_empty());
        assert_eq!(listing.errors.len(), 1);
        let error = listing.into_strict().unwrap_err();
        assert!(
            matches!(error, Error::IoAt { path, .. } if path == dir),
            "{consumer}"
        );
    }
}

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

use crate::fs_impl::{Step, create_temporary_sibling, recording};
use crate::store::*;

/// Load and `--set-root` share this one rule, and it reads only what it
/// is handed: no `HOME`, no file.
#[test]
fn root_setting_is_validated_by_one_function() {
    let setting = Path::new("/h/.config/nebula/root");

    for contents in ["", "\n", "  \t\n"] {
        let error = Corpus::root_setting(setting, contents).unwrap_err();
        assert!(
            matches!(&error, Error::EmptyRootSetting(path) if path == setting),
            "{contents:?}: {error:?}"
        );
        assert!(
            error.to_string().contains("/h/.config/nebula/root"),
            "{error}"
        );
    }

    for contents in ["relcorpus", "relcorpus\n", "./corpus\n", "~/corpus\n"] {
        let error = Corpus::root_setting(setting, contents).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::RelativeRootSetting { setting: path, root }
                    if path == setting && root == Path::new(contents.trim())
            ),
            "{contents:?}: {error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("/h/.config/nebula/root"), "{message}");
        assert!(message.contains(contents.trim()), "{message}");
    }

    for contents in ["/srv/corpus", "/srv/corpus\n", "  /srv/my corpus \n"] {
        assert_eq!(
            Corpus::root_setting(setting, contents).unwrap(),
            PathBuf::from(contents.trim())
        );
    }

    // The write path hands it exactly what it would write.
    let root = Path::new("/srv/corpus");
    assert_eq!(
        Corpus::root_setting(setting, &Corpus::root_setting_contents(root)).unwrap(),
        root
    );
}

#[test]
fn atomic_write_removes_its_temporary_file_when_rename_fails() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("destination");
    std::fs::create_dir(&destination).unwrap();

    let error = write_private_atomic(&destination, "replacement").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("writing"), "{message}");
    assert!(
        message.contains(&destination.display().to_string()),
        "{message}"
    );
    assert!(
        matches!(error, Error::IoAt { source, .. } if source.raw_os_error().is_some()),
        "the OS cause was not preserved: {message}"
    );
    assert!(
        destination.is_dir(),
        "the failed rename left the target alone"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != OsStr::new("destination"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the failed rename left temporary files: {leftovers:?}"
    );
}

/// The reported break: a symlink planted at the temporary path turned an
/// ordinary note into a write outside the corpus, and then became the node.
#[cfg(unix)]
#[test]
fn atomic_write_never_writes_through_a_temporary_planted_as_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE").unwrap();
    let destination = dir.path().join("destination.md");
    std::fs::write(&destination, "original").unwrap();
    let planted = dir.path().join("destination.md.tmp");
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    write_private_atomic(&destination, "replacement").unwrap();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE",
        "the write reached a file outside the corpus"
    );
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "replacement"
    );
    assert!(
        std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink was renamed onto the destination"
    );
    assert!(
        std::fs::symlink_metadata(&planted)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the planted path is not ours to delete"
    );
}

/// The order is the durability: bytes flushed before the rename, so the
/// name never points at a file the device has not got, and the directory
/// flushed after it, so the rename itself is not lost to a crash.
#[test]
fn write_atomic_syncs_the_file_before_rename_and_the_parent_after() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("node.md");

    let (result, steps) = recording(|| write_private_atomic(&destination, "contents"));
    result.unwrap();

    let Some(Step::Write { path: tmp, bytes }) = steps.first() else {
        panic!("the first step is not the write: {steps:?}");
    };
    assert_eq!(bytes, b"contents");
    assert_eq!(tmp.parent(), Some(dir.path()), "{}", tmp.display());
    let mut expected = vec![
        Step::Write {
            path: tmp.clone(),
            bytes: b"contents".to_vec(),
        },
        Step::SyncAll(tmp.clone()),
        Step::Rename {
            from: tmp.clone(),
            to: destination.clone(),
        },
    ];
    if cfg!(unix) {
        expected.push(Step::SyncDir(dir.path().to_path_buf()));
    }
    assert_eq!(steps, expected);
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "contents");
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

/// Crash debris and the pending record stay out of every commit, and
/// out of a person's `git add -A` once `init` has run.
#[test]
fn commits_and_ignore_rules_cover_debris_and_the_pending_record() {
    assert_eq!(
        NEVER_STAGED,
        [
            ":(exclude,glob)**/*.tmp".to_string(),
            format!(":(exclude){PENDING_FILE}")
        ]
    );
    // What the exclusion and the ignore rule both match on is the name
    // every temporary the write helper makes ends with.
    let dir = tempfile::tempdir().unwrap();
    let (tmp, _handle) = create_temporary_sibling(&dir.path().join("a.md")).unwrap();
    assert_eq!(
        tmp.extension(),
        Some(OsStr::new("tmp")),
        "{}",
        tmp.display()
    );
    assert_eq!(
        ignore_rules(),
        [
            format!("/{LOCK_FILE}"),
            format!("/{PENDING_FILE}"),
            "*.tmp".to_string()
        ]
    );
}

#[test]
fn stamp_names_the_offset_it_used() {
    let instant = time::macros::datetime!(2026-09-26 06:11:05.25 UTC);

    let fallback = format_stamp(instant, None);
    assert_eq!(fallback, "2026-09-26T06:11:05Z");
    let local = format_stamp(instant, Some(time::macros::offset!(+2)));
    assert_eq!(local, "2026-09-26T08:11:05+02:00");
    let zero = format_stamp(instant, Some(UtcOffset::UTC));
    assert_eq!(zero, "2026-09-26T06:11:05+00:00");
    for stamp in [fallback, local, zero, stamp()] {
        let parsed = OffsetDateTime::parse(&stamp, &Rfc3339)
            .unwrap_or_else(|error| panic!("{stamp}: {error}"));
        assert_eq!(parse_stamp(&stamp), Some(parsed), "{stamp}");
        assert!(
            stamp.ends_with('Z') || stamp[stamp.len() - 6..].starts_with(['+', '-']),
            "{stamp} does not name its offset"
        );
        assert_eq!(stamp.find('.'), None, "{stamp} is to the second");
    }
}

#[test]
fn a_legacy_stamp_reads_as_local_time_and_rfc3339_as_written() {
    let legacy = rfc3339_stamp("2026-09-01T08:00").unwrap();
    assert!(legacy.starts_with("2026-09-01T08:00:00"), "{legacy}");
    let parsed = OffsetDateTime::parse(&legacy, &Rfc3339).unwrap();
    assert_eq!(parse_stamp("2026-09-01T08:00"), Some(parsed));

    for written in ["2026-09-01T08:00:00+02:00", "2026-09-01T08:00:00.5-05:30"] {
        assert_eq!(rfc3339_stamp(written).as_deref(), Some(written));
    }
    for unreadable in ["someday", "2026-09-01", "2026-09-01T08:00:00", ""] {
        assert_eq!(rfc3339_stamp(unreadable), None, "{unreadable}");
        assert_eq!(parse_stamp(unreadable), None, "{unreadable}");
    }
}

#[test]
fn a_short_title_slugifies_whole() {
    assert_eq!(slugify("Tags beat domains"), "tags-beat-domains");
}

#[test]
fn unicode_titles_slugify_to_stable_valid_ids() {
    for (title, expected) in [
        ("Ünïcode título → ok", "ünïcode-título-ok"),
        ("시간은 프레임의 수다", "시간은-프레임의-수다"),
        ("Tags beat domains", "tags-beat-domains"),
    ] {
        let slug = slugify(title);
        assert_eq!(slug, expected);
        assert!(is_slug(&slug), "derived id is not a valid slug: {slug}");
    }
}

#[test]
fn a_slug_over_the_limit_never_ends_mid_word() {
    let word = "abcdefg"; // 7 chars, so units of 8 with the joining dash
    let title = [word; 9].join(" ");
    let slug = slugify(&title);
    assert!(slug.chars().count() <= 60, "slug is over the limit: {slug}");
    assert!(!slug.is_empty());
    assert!(
        slug.split('-').all(|w| w == word),
        "slug has a partial word: {slug}"
    );
}

/// The title behind the frozen id in the bug report: the naive
/// `.chars().take(60)` cut landed on a dash-adjacent boundary here by
/// coincidence, but the fixed rule (cut at the last dash at or before 60)
/// still applies and drops the trailing word rather than keeping a slug
/// that happens to look intact.
#[test]
fn every_surviving_word_is_whole() {
    let title = "Self-authored structure is a paved path, imposed structure is rigidity";
    let slug = slugify(title);
    assert!(slug.chars().count() <= 60);
    assert!(!slug.is_empty());
    assert!(!slug.ends_with('-'));
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    for part in slug.split('-') {
        assert!(
            words.iter().any(|w| w == part),
            "fragment `{part}` is not a whole word from the title"
        );
    }
}

/// The capture from the v0.2 evaluation: a 60-character sentence slug
/// becomes five significant words, with longer fallbacks behind it.
#[test]
fn a_long_capture_mints_a_short_id_from_its_significant_words() {
    let text = "gravity might be a scarcity gradient in some shared resource";
    let ids = capture_ids(text);
    assert_eq!(
        ids,
        [
            "gravity-scarcity-gradient-shared-resource",
            "gravity-might-be-a-scarcity-gradient-in-some-shared-resource",
        ]
    );
    assert_eq!(ids.last().unwrap(), &slugify(text), "the full slug is last");
    for id in &ids {
        assert!(is_slug(id) && is_path_safe_id(id), "{id}");
    }
}

#[test]
fn collision_fallbacks_add_one_significant_word_at_a_time() {
    let ids = capture_ids("the map is not the territory but the atlas is a map of maps");
    assert_eq!(
        ids,
        [
            "map-not-territory-atlas-map",
            "map-not-territory-atlas-map-maps",
            "the-map-is-not-the-territory-but-the-atlas-is-a-map-of-maps",
        ]
    );
    assert!(
        ids[0].split('-').count() <= CAPTURE_ID_WORDS,
        "the first choice is within the bound: {}",
        ids[0]
    );
}

#[test]
fn a_short_capture_keeps_the_slug_it_always_had() {
    for text in [
        "search ranking decays with age",
        "the human's own words",
        "a thought",
        "시간은 프레임의 수다",
    ] {
        assert_eq!(capture_ids(text), [slugify(text)], "{text}");
    }
}

/// Words past the 60-character cut of the full slug still count: the
/// short id is built from the whole sentence, then capped.
#[test]
fn a_capture_id_draws_on_words_past_the_full_slugs_cut() {
    let text = "it is what it is and it was what it was and so it goes on and on forever";
    // Every word but `goes` and `forever` is a stop word, so those two
    // and nothing else carry the id.
    assert_eq!(capture_ids(text)[0], "goes-forever");
    assert!(!slugify(text).contains("forever"), "{}", slugify(text));

    let stop_only = "it is what it is and so it was";
    assert_eq!(capture_ids(stop_only)[0], "it-is-what-it-is");
}

#[test]
fn a_capture_that_reduces_to_nothing_has_no_id() {
    assert!(capture_ids("→ … !!").is_empty());
    assert!(capture_ids(&"a".repeat(61)).is_empty());
}

#[test]
fn a_title_with_no_dash_in_the_first_60_chars_reduces_to_empty() {
    // One long run with no separator: there is no dash to cut at, so the
    // whole thing reduces to nothing rather than a truncated fragment
    // standing in for the title.
    let title = "a".repeat(61);
    assert_eq!(slugify(&title), "");
}

/// The rule an id has to satisfy before it is joined into a path. Every
/// spelling here that escapes `nodes/` reached a file outside the corpus
/// before this existed.
#[test]
fn an_id_that_is_not_one_file_name_is_refused() {
    for id in [
        "../../escaped",
        "../escaped",
        "..",
        ".",
        "./escaped",
        "nodes/other",
        "a\\b",
        "/etc/passwd",
        "/absolute",
        "",
        " ",
        " leading",
        "trailing ",
        "new\nline",
        "nul\0byte",
    ] {
        assert!(!is_path_safe_id(id), "accepted `{id}`");
    }
}

/// The rule is about path structure, not about which alphabet an idea was
/// named in: every id a verb has ever derived stays valid.
#[test]
fn an_ordinary_id_including_a_unicode_one_is_accepted() {
    for id in [
        "safe",
        "self-authored-structure",
        "ünïcode-título-ok",
        "시간은-프레임의-수다",
        "..leading-dots",
        "a",
    ] {
        assert!(is_path_safe_id(id), "refused `{id}`");
        assert_eq!(
            Path::new(id).components().count(),
            1,
            "`{id}` is more than one component"
        );
    }
    // Everything `slugify` produces satisfies the weaker rule, which is
    // what keeps the two checks from ever disagreeing about a new node.
    for title in [
        "Tags beat domains",
        "Ünïcode título → ok",
        "시간은 프레임의 수다",
    ] {
        let slug = slugify(title);
        assert!(is_slug(&slug) && is_path_safe_id(&slug), "{slug}");
    }
}

#[test]
fn is_slug_matches_what_slugify_would_produce() {
    assert!(is_slug("self-authored-structure"));
    assert!(is_slug(&"a".repeat(60)));
    assert!(!is_slug(""));
    assert!(!is_slug("Has-Capitals"));
    assert!(!is_slug("trailing-"));
    assert!(!is_slug("-leading"));
    assert!(!is_slug("double--dash"));
    assert!(!is_slug("has space"));
    assert!(!is_slug(&"a".repeat(61)));
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
        assert_eq!(crate::store::capture_line(text), line, "joining {text:?}");
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
