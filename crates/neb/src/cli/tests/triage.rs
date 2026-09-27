//! Unit tests for `cli/triage.rs`.

use crate::cli::emit::CommitOpts;
use crate::cli::failure::KeyError;
use crate::cli::triage::{Key, parse_key, triage};
use crate::output;
use nebula_core::triage::Action;
use nebula_core::{Corpus, ops};
use std::io::Write;

/// Each key is one action, `t` is the only one that takes text, and
/// anything else is refused by name rather than guessed at.
#[test]
fn triage_keys_parse_to_their_actions() {
    for (line, key) in [
        ("p\n", Key::Act(Action::Promote)),
        ("  2 ", Key::Act(Action::PromoteUnder(2))),
        ("d", Key::Act(Action::Drop)),
        ("s", Key::Act(Action::Skip)),
        ("q", Key::Act(Action::Quit)),
        ("t", Key::AskTitle),
        (
            "t  A title  here ",
            Key::Act(Action::Title("A title  here".into())),
        ),
        ("?", Key::Help),
        ("h", Key::Help),
        ("   \n", Key::Nothing),
    ] {
        assert_eq!(parse_key(line).unwrap(), key, "{line:?}");
    }
    for line in [
        "x",
        "pp",
        "p now",
        "d 2",
        "+1",
        "-1",
        "1.5",
        "99999999999999999999999",
    ] {
        let Err(KeyError::Unknown(said)) = parse_key(line) else {
            panic!("{line:?} should be refused")
        };
        assert_eq!(said, line.trim());
    }
}

/// On a terminal the keys are listed and prompted for, and a refusal is
/// reported and the same entry asked about again rather than ending the
/// session, which is what lets a person correct a slip.
#[test]
fn interactive_triage_prompts_and_asks_again_after_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(
        &nebula_core::Locations::default(),
        &dir.path().join("corpus"),
    )
    .unwrap();
    let entry = ops::capture(&corpus, "a thought worth keeping").unwrap();
    let mut input = std::io::Cursor::new("x\n7\nt\n!!!\np\nt Worth keeping\np\n");
    let mut out = Vec::new();
    let commits = CommitOpts {
        skip: false,
        json: false,
    };
    let done = triage(&corpus, None, &mut input, &mut out, true, commits);
    assert!(done.is_ok(), "refusals do not end a terminal session");
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("p promote as a root"), "{out}");
    assert!(out.contains("title> "), "{out}");
    assert_eq!(
        out.matches("\n[1/1]").count(),
        1,
        "the entry is shown once, then asked about again:\n{out}"
    );
    assert!(
        out.contains(&format!("promoted {} -> worth-keeping", entry.id)),
        "{out}"
    );
    assert!(corpus.inbox().unwrap().0.is_empty());
}

/// A screen that goes away once the first decision is reported.
struct GoesAfterFirstStep(Vec<u8>);

impl Write for GoesAfterFirstStep {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl output::Closable for GoesAfterFirstStep {
    fn is_closed(&self) -> bool {
        String::from_utf8_lossy(&self.0).contains("dropped ")
    }
}

/// When the screen closes mid-session, the session ends as `q` ends it:
/// the decision already made stays, and the keys typed after it are
/// never applied to entries nobody saw.
#[test]
fn triage_ends_as_quit_does_when_its_screen_closes() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(
        &nebula_core::Locations::default(),
        &dir.path().join("corpus"),
    )
    .unwrap();
    let first = ops::capture(&corpus, "the first thought").unwrap();
    let second = ops::capture(&corpus, "the second thought").unwrap();
    let mut input = std::io::Cursor::new("d\nd\n");
    let mut out = GoesAfterFirstStep(Vec::new());
    let commits = CommitOpts {
        skip: true,
        json: false,
    };
    let done = triage(&corpus, None, &mut input, &mut out, false, commits);
    assert!(done.is_ok(), "a closed screen is no refusal");
    let waiting: Vec<_> = corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(waiting, [second.id], "only {} was dropped", first.id);
}
