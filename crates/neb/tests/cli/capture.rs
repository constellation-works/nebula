//! `neb capture` and the inbox: stdin and its limits, blank and multiline
//! text, inbox stamps, id candidates and exhaustion, repeated and settled
//! captures.

use crate::harness::{Corpus, Run, run_from_home, write};
use std::fmt::Write as _;
use std::path::PathBuf;

#[test]
fn capture_then_promote_leaves_the_original_text_in_the_node() {
    let c = Corpus::new();
    let entry = c
        .run(&[
            "capture",
            "ranking decay looks like a half-life, not a cliff",
        ])
        .assert_ok();
    let id = entry.stdout_trim();
    c.run(&["inbox"]).assert_ok().says(&id).says("half-life");

    let node = c
        .run(&["promote", &id, "--title", "Ranking decay half-life"])
        .assert_ok();
    let node_id = node.stdout_trim();
    assert_eq!(node_id, "ranking-decay-half-life");

    let body = std::fs::read_to_string(c.node_file(&node_id)).unwrap();
    assert!(
        body.contains("half-life, not a cliff"),
        "capture text should survive promotion"
    );
}

/// The lines of the one inbox month file a fresh corpus has captured into.
fn inbox_lines(c: &Corpus) -> Vec<String> {
    let files: Vec<PathBuf> = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let [month] = files.as_slice() else {
        panic!("expected one inbox month file, found {files:?}");
    };
    std::fs::read_to_string(month)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn capture_dash_reads_stdin_and_joins_its_lines() {
    let c = Corpus::new();

    let id = c
        .run_with_stdin(&["capture", "-", "--quiet"], "a\nb\n")
        .assert_ok()
        .stdout_trim();

    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 1, "one line per entry: {lines:?}");
    assert!(lines[0].starts_with(&format!("- [{id}] ")), "{lines:?}");
    assert!(lines[0].ends_with(" a b"), "{lines:?}");

    let captured = c
        .run_with_stdin(&["capture", "-", "--json"], "one\r\n\r\n  two\r\n")
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(captured["entry"]["text"], "one two");
    assert_eq!(inbox_lines(&c).len(), 2);
}

/// `capture -` reads at most 64 KiB, and one byte more is refused before the
/// corpus is opened for writing: the lock held here would otherwise turn it
/// into a five-second wait and a `locked` refusal, and nothing is written.
#[test]
fn oversized_stdin_capture_is_refused_before_locking() {
    let c = Corpus::new();
    c.run(&["capture", "-q", "already waiting"]).assert_ok();
    let before = inbox_lines(&c);
    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    let over = "a".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT + 1);
    let run = c.run_with_stdin(&["--json", "capture", "-"], &over);
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "input_too_large", "{refusal}");
    let error = refusal["error"].as_str().unwrap();
    assert!(error.contains("65536 bytes"), "names the limit: {error}");
    assert!(
        refusal["hint"].as_str().unwrap().contains("64 KiB"),
        "{refusal}"
    );

    drop(held);
    assert_eq!(inbox_lines(&c), before, "nothing was captured");
}

/// Count UTF-8 bytes and joining spaces, refusing before any lock.
#[test]
fn capture_argument_limit_matches_stdin_before_locking() {
    let c = Corpus::new();
    c.run(&["capture", "-q", "already waiting"]).assert_ok();
    let before = inbox_lines(&c);
    let held = nebula_core::CorpusLock::acquire(&c.root).unwrap();
    let half = "é".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT / 4);
    let over = format!("{half} {half}");
    let expected = c
        .run_with_stdin(&["--json", "capture", "-"], &over)
        .refusal();
    assert_eq!(expected["code"], "input_too_large");
    for words in [vec![over.as_str()], vec![half.as_str(), half.as_str()]] {
        let mut args = vec!["--json", "capture"];
        args.extend(words);
        let actual = c.run(&args).refusal();
        assert_eq!(
            actual, expected,
            "arguments and stdin have the same refusal"
        );
    }
    drop(held);
    assert_eq!(inbox_lines(&c), before, "nothing was captured");
}

#[test]
fn capture_argument_at_byte_limit_is_accepted() {
    let c = Corpus::new();
    let text = "é".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT / 2);
    c.run(&["capture", "-q", &text]).assert_ok();
    assert!(inbox_lines(&c)[0].ends_with(&text));
}

/// `--body -` reads at most 1 MiB, refused the same way before any lock.
#[test]
fn oversized_stdin_body_is_refused_before_locking() {
    let c = Corpus::new();
    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    let over = "b".repeat(nebula_core::ops::BODY_INPUT_LIMIT + 1);
    let run = c.run_with_stdin(&["--json", "new", "Too long", "--body", "-"], &over);
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "input_too_large", "{refusal}");
    assert!(
        refusal["error"].as_str().unwrap().contains("1048576 bytes"),
        "{refusal}"
    );

    drop(held);
    assert!(!c.node_file("too-long").exists(), "no node was written");
}

/// Exactly the limit is not over it: taken whole, for a capture and a body.
#[test]
fn stdin_at_the_limit_is_accepted() {
    let c = Corpus::new();
    let thought = "a".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT);
    let id = c
        .run_with_stdin(&["capture", "-q", "-"], &thought)
        .assert_ok()
        .stdout_trim();
    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 1, "{} lines", lines.len());
    assert!(lines[0].starts_with(&format!("- [{id}] ")));
    assert!(
        lines[0].ends_with(&format!(" {thought}")),
        "the capture is whole"
    );

    let body = "b".repeat(nebula_core::ops::BODY_INPUT_LIMIT);
    c.run_with_stdin(&["new", "At the limit", "--body", "-"], &body)
        .assert_ok();
    let node = std::fs::read_to_string(c.node_file("at-the-limit")).unwrap();
    assert!(node.ends_with(&format!("{body}\n")), "the body is whole");
}

#[test]
fn multiline_capture_text_is_joined_onto_one_line() {
    let c = Corpus::new();

    let captured = c
        .run(&["capture", "first line\nsecond line", "--quiet", "--json"])
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(captured["entry"]["text"], "first line second line");

    // A lone `-` is stdin; a dash among other words is part of the thought.
    c.run(&["capture", "--quiet", "--", "left", "-", "right"])
        .assert_ok();

    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 2, "one line per entry: {lines:?}");
    assert!(lines[0].ends_with(" first line second line"), "{lines:?}");
    assert!(lines[1].ends_with(" left - right"), "{lines:?}");
    c.run(&["inbox"])
        .assert_ok()
        .says("first line second line")
        .says("left - right");
}

#[test]
fn empty_capture_is_refused_without_writing() {
    let c = Corpus::new();

    for input in ["", "\n", " \r\n\t\n "] {
        c.run_with_stdin(&["capture", "-"], input)
            .assert_fails()
            .says("nothing to capture");
    }
    c.run(&["capture", " \n \r\n "])
        .assert_fails()
        .says("nothing to capture");

    assert_eq!(
        std::fs::read_dir(c.root.join("inbox")).unwrap().count(),
        0,
        "a refused capture must not create an inbox record"
    );
    c.run(&["inbox", "--json"]).assert_ok().says("[]");
}

/// Capture creates a missing corpus, but not for a thought that is not there:
/// empty standard input is refused before anything is written.
#[test]
fn empty_stdin_capture_creates_no_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = Corpus {
        root: dir.path().join("fresh"),
        dir,
    };

    fresh
        .run_with_stdin(&["capture", "-"], "\n\n")
        .assert_fails()
        .says("nothing to capture");

    assert!(
        !fresh.root.exists(),
        "a refused capture must not init a corpus"
    );
}

/// Blank text to a root that does not exist yet is refused with core's code
/// for it, and the root is still not there: the rule runs before a corpus
/// can be created for it (STD-02 §R24, §R34).
#[test]
fn blank_capture_to_a_missing_root_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let missing = dir.path().join("not-yet");

    let refused =
        run_from_home(&home, None, &["--json", "capture", "  "], Some(&missing)).usage_refusal();
    assert_eq!(refused["code"], "empty_capture", "{refused}");
    assert!(
        !missing.exists(),
        "a refused capture must not init a corpus"
    );
}

/// Three rules the CLI used to spell itself, as `usage`, now reach it from
/// core with core's codes, still exiting 2 as usage errors: the codes
/// `blank_capture_note_and_open_status_reason_are_typed_refusals` in
/// `core/capture.rs` asserts for the same calls.
#[test]
fn blank_capture_note_and_open_status_reason_refuse_with_core_codes() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for (args, code) in [
        (vec!["--json", "capture", "   "], "empty_capture"),
        (vec!["--json", "note", &id, "  "], "empty_note"),
        (
            vec!["--json", "status", &id, "seed", "--why", "y"],
            "reason_on_open_status",
        ),
    ] {
        let refused = c.run(&args).usage_refusal();
        assert_eq!(refused["code"], code, "{args:?}: {refused}");
    }
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

/// A month file the inbox cannot read is named, under `--json` and in prose.
#[cfg(unix)]
#[test]
fn unreadable_inbox_file_error_names_the_file() {
    use std::os::unix::fs::PermissionsExt;

    let c = Corpus::new();
    c.run(&["capture", "a thought"]).assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "md"))
        .expect("a month file");
    std::fs::set_permissions(&month, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&month).is_ok() {
        // Running as root, where a mode refuses nothing.
        return;
    }
    let shown = month.display().to_string();

    let refused = c.run(&["--json", "inbox"]).refusal();
    assert_eq!(refused["code"], "io_at", "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains(&shown),
        "{refused}"
    );
    let prose = c.run(&["inbox"]).assert_fails().says(&shown);
    assert_eq!(prose.out.status.code(), Some(1));
}

/// Whether `at` is RFC 3339, which says its offset by construction.
fn is_rfc3339(at: &serde_json::Value) -> bool {
    at.as_str().is_some_and(|at| {
        time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).is_ok()
    })
}

/// A POSIX zone two hours ahead of UTC, with no daylight saving, so a stamp's
/// offset is known whatever zone the suite runs in.
const PLUS_TWO: (&str, &str) = ("TZ", "XYZ-2");

#[test]
fn inbox_at_is_rfc3339_with_offset() {
    let c = Corpus::new();

    let captured = c
        .run_with_env(&["capture", "x", "--json"], &[PLUS_TWO])
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    let at = &captured["entry"]["at"];
    assert!(is_rfc3339(at), "{captured}");
    assert!(at.as_str().unwrap().ends_with("+02:00"), "{captured}");
    c.run(&["capture", "y", "--quiet"]).assert_ok();

    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let listing: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 2, "{listing}");
    assert!(entries.iter().all(|e| is_rfc3339(&e["at"])), "{listing}");
    assert_eq!(entries[0]["at"], *at, "a stamp reads back as written");

    let dropped = c
        .run(&["drop", captured["entry"]["id"].as_str().unwrap(), "--json"])
        .assert_ok()
        .stdout();
    let dropped: serde_json::Value = serde_json::from_str(&dropped).unwrap();
    assert_eq!(dropped["at"], *at, "{dropped}");
}

/// A stamp from before 0.2.0 carries no offset. It lists as local time with
/// the offset the machine has for that instant, and settling it leaves the
/// line's stamp exactly as it was written.
#[test]
fn legacy_inbox_stamp_still_loads() {
    let c = Corpus::new();
    let inbox = c.root.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let month = inbox.join("2000-01.md");
    write(
        &month,
        "- [abcd] 2026-09-01T08:00 old thought\n\
         - [bcde] 2026-09-01T09:00 another old thought\n",
    );

    let listing = c
        .run_with_env(&["inbox", "--json"], &[PLUS_TWO])
        .assert_ok()
        .stdout();
    let listing: serde_json::Value = serde_json::from_str(&listing).unwrap();
    assert_eq!(listing[0]["id"], "abcd", "{listing}");
    assert_eq!(listing[0]["at"], "2026-09-01T08:00:00+02:00", "{listing}");
    assert_eq!(listing[1]["at"], "2026-09-01T09:00:00+02:00", "{listing}");
    let listing = c
        .run_with_env(&["inbox", "--json"], &[("TZ", "UTC0")])
        .assert_ok()
        .stdout();
    assert!(
        listing.contains("\"2026-09-01T08:00:00+00:00\""),
        "{listing}"
    );

    c.run(&["promote", "abcd"]).assert_ok().says("old-thought");
    c.run(&["drop", "bcde"]).assert_ok();

    assert_eq!(
        std::fs::read_to_string(&month).unwrap(),
        "- ~~[abcd] 2026-09-01T08:00 old thought~~ -> old-thought\n\
         - ~~[bcde] 2026-09-01T09:00 another old thought~~ dropped\n"
    );
    c.run(&["inbox", "--json"]).assert_ok().says("[]");
}

/// The ids `neb` tries, in order, for `text` captured at `stamp`: a copy of
/// the hash chain in `store::unique_entry_id`, which the binary cannot be
/// asked for.
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

/// The inbox stamp for every second from `stamp` to `seconds` after it, at
/// `stamp`'s own offset and in its own form, as `neb` would write them.
fn stamps_from(stamp: &str, seconds: i64) -> Vec<String> {
    use time::format_description::well_known::Rfc3339;
    use time::macros::format_description;
    use time::{Duration, OffsetDateTime, UtcOffset};

    let start = OffsetDateTime::parse(stamp, &Rfc3339).unwrap();
    (0..=seconds)
        .map(|s| {
            let at = start + Duration::seconds(s);
            if stamp.ends_with('Z') {
                at.to_offset(UtcOffset::UTC).format(format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
                ))
            } else {
                at.format(format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
                ))
            }
            .unwrap()
        })
        .collect()
}

#[test]
fn capture_collision_fallback_promotes_the_new_thought() {
    const TEXT: &str = "the intended new thought";
    // The binary's clock cannot be set, and its stamp, which seeds the id, is
    // to the second. So rather than hope the capture lands in the probe's
    // second, the fixture holds every id it could hash to from that second to
    // a minute after it, and the capture collides whichever it lands in.
    const WINDOW_SECS: i64 = 60;

    let c = Corpus::new();
    // A zone without daylight saving, so no offset change falls inside the window.
    c.run_with_env(&["capture", TEXT], &[PLUS_TWO]).assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let raw = std::fs::read_to_string(&month).unwrap();
    let probe = raw.split_whitespace().nth(2).unwrap().to_string();
    let window = stamps_from(&probe, WINDOW_SECS);
    let mut occupied = std::collections::HashSet::new();
    let mut fixture = String::new();
    for stamp in &window {
        for id in inbox_id_candidates(stamp, TEXT) {
            if occupied.insert(id.clone()) {
                writeln!(fixture, "- [{id}] {stamp} fixture {}", occupied.len()).unwrap();
            }
        }
    }
    std::fs::write(&month, fixture).unwrap();

    let captured = c
        .run_with_env(&["capture", TEXT], &[PLUS_TWO])
        .assert_ok()
        .stdout_trim();

    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let entries: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let new_entry = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == captured)
        .unwrap();
    let at = new_entry["at"].as_str().unwrap();
    assert!(
        window.iter().any(|stamp| stamp == at),
        "the capture at {at} fell outside the {WINDOW_SECS}s after {probe} the fixture holds"
    );
    assert!(!occupied.contains(&captured), "{captured} is already taken");
    c.run(&["promote", &captured, "--title", "Intended new thought"])
        .assert_ok();
    let node = std::fs::read_to_string(c.node_file("intended-new-thought")).unwrap();
    assert!(node.ends_with("the intended new thought\n"));
}

#[test]
fn capture_reports_id_exhaustion_without_appending() {
    let c = Corpus::new();
    c.run(&["capture", "locate the current inbox file"])
        .assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut fixture = String::new();
    for id in 0..=u16::MAX {
        writeln!(fixture, "- [{id:04x}] 2000-01-01T00:00 occupied").unwrap();
    }
    std::fs::write(&month, &fixture).unwrap();

    c.run(&["capture", "there is no id left"])
        .assert_fails()
        .says("inbox id namespace exhausted; nothing captured");
    assert_eq!(std::fs::read_to_string(month).unwrap(), fixture);
}

#[test]
fn capture_after_an_unterminated_record_round_trips_through_promotion() {
    let c = Corpus::new();
    let first = c.run(&["capture", "the first thought"]).stdout_trim();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let unterminated = std::fs::read_to_string(&month)
        .unwrap()
        .trim_end_matches('\n')
        .to_string();
    std::fs::write(&month, unterminated).unwrap();

    let second = c.run(&["capture", "the second thought"]).stdout_trim();
    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let entries: serde_json::Value = serde_json::from_str(&listing).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 2);
    assert!(listing.contains(&first));
    assert!(listing.contains(&second));

    let node = c
        .run(&["promote", &second, "--title", "Second thought"])
        .assert_ok()
        .stdout_trim();
    let body = std::fs::read_to_string(c.node_file(&node)).unwrap();
    assert!(body.ends_with("the second thought\n"));
    c.run(&["inbox", "--json"])
        .assert_ok()
        .says(&first)
        .says("the first thought");
}

#[test]
fn promoted_and_dropped_entries_leave_the_inbox_but_stay_on_disk() {
    let c = Corpus::new();
    let keep = c.run(&["capture", "worth keeping"]).stdout_trim();
    let toss = c.run(&["capture", "not worth keeping"]).stdout_trim();

    c.run(&["promote", &keep, "--title", "Worth keeping"])
        .assert_ok();
    c.run(&["drop", &toss]).assert_ok();

    let listing = c.run(&["inbox"]).assert_ok().stdout();
    assert!(
        !listing.contains(&keep) && !listing.contains(&toss),
        "both should be settled"
    );

    // Nothing is deleted: the record of what was captured, and what became of
    // it, is the part that stops you re-treading ground.
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let raw = std::fs::read_to_string(month.path()).unwrap();
    assert!(
        raw.contains("not worth keeping"),
        "dropped text must remain on disk"
    );
    assert!(raw.contains("dropped"), "drop should be recorded");
    assert!(
        raw.contains("worth-keeping"),
        "promotion should name the node it became"
    );
}

/// Capture never refuses for content, so a thought typed twice lands twice;
/// stderr says which waiting entry it repeats, and stdout is untouched.
#[test]
fn a_repeated_capture_names_the_entry_still_waiting_on_stderr() {
    let c = Corpus::new();
    let first = c.run(&["capture", "Tags beat domains"]).assert_ok();
    assert_eq!(first.stderr(), "", "a new thought is not news");
    let first = first.stdout_trim();
    let notice = format!("note: same as {first}, still waiting\n");

    let quiet = c
        .run(&["capture", "--quiet", "tags  BEAT domains"])
        .assert_ok();
    assert_eq!(quiet.stderr(), notice);
    let second = quiet.stdout_trim();
    assert_eq!(
        quiet.stdout(),
        format!("{second}\n"),
        "stdout is the id alone"
    );
    assert_ne!(second, first);

    let text = c.run(&["capture", "TAGS beat domains"]).assert_ok();
    assert_eq!(text.stderr(), notice);
    assert!(
        !text.stdout().contains("same as"),
        "the notice stays off stdout"
    );

    let keys = |run: &Run| {
        let value: serde_json::Value = serde_json::from_str(&run.stdout()).expect("stdout is JSON");
        let mut keys: Vec<String> = value["entry"]
            .as_object()
            .expect("an entry")
            .keys()
            .chain(value.as_object().expect("an object").keys())
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let repeated = c
        .run(&["capture", "--json", " tags beat\tdomains "])
        .assert_ok();
    assert_eq!(repeated.stderr(), notice);
    let fresh = c
        .run(&["capture", "--json", "an unrelated thought"])
        .assert_ok();
    assert_eq!(fresh.stderr(), "");
    assert_eq!(keys(&repeated), keys(&fresh), "the payload is unchanged");

    let inbox: serde_json::Value =
        serde_json::from_str(&c.run(&["inbox", "--json"]).assert_ok().stdout()).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 5, "every capture landed");

    // A settled entry is not waiting, so it is never the one named.
    c.run(&["drop", &first]).assert_ok();
    let after_drop = c.run(&["capture", "tags beat domains"]).assert_ok();
    assert_eq!(
        after_drop.stderr(),
        format!("note: same as {second}, still waiting\n")
    );
}

#[test]
fn promote_and_drop_on_a_settled_entry_say_how_it_was_settled() {
    let c = Corpus::new();
    let keep = c.run(&["capture", "worth keeping"]).stdout_trim();
    let toss = c.run(&["capture", "not worth keeping"]).stdout_trim();
    c.run(&["promote", &keep, "--title", "Worth keeping"])
        .assert_ok();
    c.run(&["drop", &toss]).assert_ok();

    for verb in ["promote", "drop"] {
        c.run(&[verb, &keep])
            .assert_fails()
            .says(&format!("`{keep}` was already promoted to `worth-keeping`"))
            .says("neb show worth-keeping");
        c.run(&[verb, &toss])
            .assert_fails()
            .says(&format!("`{toss}` was already dropped"))
            .says("neb inbox");
        let json = c.run(&[verb, &keep, "--json"]);
        assert_eq!(json.stdout(), "", "a refusal prints no payload");
        assert_eq!(
            json.refusal(),
            serde_json::json!({
                "error": format!("`{keep}` was already promoted to `worth-keeping`"),
                "code": "inbox_entry_settled",
                "hint": "See the node with:  neb show worth-keeping",
            })
        );
        assert_eq!(
            c.run(&[verb, &toss, "--json"]).refusal(),
            serde_json::json!({
                "error": format!("`{toss}` was already dropped"),
                "code": "inbox_entry_settled",
                "hint": "See what is still waiting with:  neb inbox",
            })
        );
        c.run(&[verb, "zzzz"])
            .assert_fails()
            .says("no open inbox entry `zzzz`");
    }
    assert!(
        !c.node_file("not-worth-keeping").exists(),
        "a refused promote writes no node"
    );
}
