//! A triage session over the inbox: oldest first through promote, drop,
//! skip and quit.

use crate::harness::corpus;
use crate::near::lexical_fixture;
use nebula_core::triage::{Action, Step, Tally};
use nebula_core::{Corpus, Error, Settlement, Triage, ops};

/// Rewrite one capture's stamp in place, found by its text, so a test can
/// put captures out of file order without waiting for real time to pass.
fn restamp(dir: &tempfile::TempDir, text: &str, stamp: &str) {
    let inbox = dir.path().join("corpus/inbox");
    for file in std::fs::read_dir(&inbox).unwrap() {
        let path = file.unwrap().path();
        let raw = std::fs::read_to_string(&path).unwrap();
        let Some(line) = raw.lines().find(|l| l.ends_with(text)) else {
            continue;
        };
        let (head, _) = line.split_once("] ").unwrap();
        let rewritten = format!("{head}] {stamp} {text}");
        std::fs::write(&path, raw.replace(line, &rewritten)).unwrap();
        return;
    }
    panic!("no capture `{text}`");
}

/// Every inbox file's text, settled lines included.
fn inbox_text(dir: &tempfile::TempDir) -> String {
    std::fs::read_dir(dir.path().join("corpus/inbox"))
        .unwrap()
        .map(|f| std::fs::read_to_string(f.unwrap().path()).unwrap())
        .collect()
}

/// The id of the entry triage is on now.
fn on(session: &mut Triage, corpus: &Corpus) -> Option<String> {
    session.current(corpus).unwrap().map(|w| w.entry.id.clone())
}

#[test]
fn triage_walks_the_inbox_oldest_first_through_promote_and_drop() {
    let (dir, corpus) = corpus();
    lexical_fixture(&corpus);
    // Captured in one order, stamped in another: triage follows the stamps.
    let newest = ops::capture(&corpus, "buy more coffee").unwrap();
    let oldest = ops::capture(&corpus, "a taxonomy for tags").unwrap();
    let middle = ops::capture(&corpus, "ranking decay again").unwrap();
    restamp(&dir, "buy more coffee", "2026-01-03T09:00");
    restamp(&dir, "a taxonomy for tags", "2026-01-01T09:00");
    restamp(&dir, "ranking decay again", "2026-01-02T09:00");

    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(session.total(), 3);
    let first = session.current(&corpus).unwrap().unwrap().clone();
    assert_eq!(first.entry.id, oldest.id);
    assert_eq!((first.position, first.total), (1, 3));
    assert!(first.days.is_some_and(|d| d > 0), "{:?}", first.days);
    assert_eq!(
        first.near.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ["a-single-global-taxonomy", "tags-beat-domains"],
        "the candidates are `near` over the captured text"
    );

    // `p`: a root, whatever the candidates were.
    let Step::Promoted { entry, created } = session.apply(&corpus, Action::Promote).unwrap() else {
        panic!("expected a promotion")
    };
    assert_eq!(entry.id, oldest.id);
    assert!(created.doc.node.edges.is_empty(), "nothing links unasked");
    assert!(created.near.is_empty(), "the candidates were shown already");
    assert_eq!(
        corpus.load(&created.doc.node.id).unwrap().body.trim(),
        entry.text
    );

    // A number: promoted under that candidate, and only that one.
    let second = session.current(&corpus).unwrap().unwrap().clone();
    assert_eq!(second.entry.id, middle.id);
    assert_eq!(second.near[0].id, "ranking-decay-half-life");
    let Step::Promoted { created, .. } = session.apply(&corpus, Action::PromoteUnder(1)).unwrap()
    else {
        panic!("expected a promotion")
    };
    let on_disk = corpus.load(&created.doc.node.id).unwrap();
    assert_eq!(
        on_disk.node.parents().collect::<Vec<_>>(),
        ["ranking-decay-half-life"]
    );

    // `d`: struck through, exactly as `drop` does.
    assert_eq!(on(&mut session, &corpus), Some(newest.id.clone()));
    let Step::Dropped { entry } = session.apply(&corpus, Action::Drop).unwrap() else {
        panic!("expected a drop")
    };
    assert_eq!(entry.id, newest.id);

    assert_eq!(on(&mut session, &corpus), None);
    assert!(corpus.inbox().unwrap().0.is_empty());
    let raw = inbox_text(&dir);
    assert!(raw.contains("~~ dropped"), "{raw}");
    assert!(
        raw.contains(&format!("~~ -> {}", created.doc.node.id)),
        "{raw}"
    );
    assert_eq!(
        session.tally(),
        Tally {
            promoted: 2,
            dropped: 1,
            skipped: 0,
            untouched: 0,
        }
    );
}

#[test]
fn triage_skip_and_quit_write_nothing_and_leave_entries_waiting() {
    let (dir, corpus) = corpus();
    let a = ops::capture(&corpus, "first thought").unwrap();
    let b = ops::capture(&corpus, "second thought").unwrap();
    let c = ops::capture(&corpus, "third thought").unwrap();
    let before = inbox_text(&dir);

    let mut session = Triage::start(&corpus, None).unwrap();
    let Step::Skipped { entry } = session.apply(&corpus, Action::Skip).unwrap() else {
        panic!("expected a skip")
    };
    assert_eq!(entry.id, a.id);
    assert_eq!(on(&mut session, &corpus), Some(b.id.clone()));
    assert!(matches!(
        session.apply(&corpus, Action::Quit).unwrap(),
        Step::Quit
    ));
    assert_eq!(on(&mut session, &corpus), None, "quit ends the session");
    assert!(
        matches!(session.apply(&corpus, Action::Drop).unwrap(), Step::Quit),
        "after quit there is nothing left to act on"
    );

    let after = inbox_text(&dir);
    assert_eq!(before, after, "skip and quit write nothing");
    let waiting: Vec<_> = corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(waiting, [a.id, b.id, c.id]);
    assert_eq!(
        session.tally(),
        Tally {
            promoted: 0,
            dropped: 0,
            skipped: 1,
            untouched: 2,
        }
    );
}

#[test]
fn triage_refuses_a_candidate_it_did_not_show_and_stays_on_the_entry() {
    let (_dir, corpus) = corpus();
    lexical_fixture(&corpus);
    let entry = ops::capture(&corpus, "a taxonomy for tags").unwrap();
    let mut session = Triage::start(&corpus, None).unwrap();
    let shown = session.current(&corpus).unwrap().unwrap().near.len();
    assert_eq!(shown, 2);

    for number in [0, shown + 1] {
        let refused = session
            .apply(&corpus, Action::PromoteUnder(number))
            .unwrap_err();
        assert!(
            matches!(refused, Error::NoSuchCandidate { number: n, shown: 2 } if n == number),
            "{refused:?}"
        );
    }
    assert_eq!(on(&mut session, &corpus), Some(entry.id.clone()));
    assert_eq!(corpus.load_all().unwrap().len(), 4, "nothing was written");
    assert_eq!(corpus.inbox().unwrap().0.len(), 1);
}

#[test]
fn triage_titles_the_promotion_and_a_refusal_keeps_the_entry_to_retry() {
    let (_dir, corpus) = corpus();
    let entry = ops::capture(&corpus, "the captured words").unwrap();
    let mut session = Triage::start(&corpus, Some("agent-x".into())).unwrap();

    // A title that makes no id is `promote`'s own refusal, and nothing moves.
    assert!(matches!(
        session.apply(&corpus, Action::Title("  !!!  ".into())).unwrap(),
        Step::Titled { title: Some(t) } if t == "!!!"
    ));
    let refused = session.apply(&corpus, Action::Promote).unwrap_err();
    assert!(matches!(refused, Error::UnusableTitle(_)), "{refused:?}");
    let waiting = session.current(&corpus).unwrap().unwrap();
    assert_eq!(waiting.entry.id, entry.id);
    assert_eq!(waiting.title.as_deref(), Some("!!!"), "the title is kept");

    // A blank title goes back to the captured text; a real one is used.
    assert!(matches!(
        session.apply(&corpus, Action::Title("   ".into())).unwrap(),
        Step::Titled { title: None }
    ));
    session
        .apply(&corpus, Action::Title("A better title".into()))
        .unwrap();
    let Step::Promoted { created, .. } = session.apply(&corpus, Action::Promote).unwrap() else {
        panic!("expected a promotion")
    };
    let node = corpus.load(&created.doc.node.id).unwrap().node;
    assert_eq!(node.id, "a-better-title");
    assert_eq!(node.title, "A better title");
    assert_eq!(
        node.title_by.as_deref(),
        Some("agent-x"),
        "`by` is attributed as `promote --by` would"
    );
    assert_eq!(on(&mut session, &corpus), None);
}

#[test]
fn triage_moves_past_an_entry_settled_elsewhere_and_says_so() {
    let (_dir, corpus) = corpus();
    let gone = ops::capture(&corpus, "settled by someone else").unwrap();
    let next = ops::capture(&corpus, "still here").unwrap();
    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(on(&mut session, &corpus), Some(gone.id.clone()));

    ops::drop(&corpus, &gone.id).unwrap();
    let refused = session.apply(&corpus, Action::Drop).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::InboxEntrySettled { id, settlement: Settlement::Dropped } if *id == gone.id
        ),
        "{refused:?}"
    );
    assert_eq!(on(&mut session, &corpus), Some(next.id));
    assert_eq!(session.tally().dropped, 0, "it was not this session's drop");
}

#[test]
fn triage_of_an_empty_inbox_has_nothing_to_decide() {
    let (_dir, corpus) = corpus();
    let mut session = Triage::start(&corpus, None).unwrap();
    assert_eq!(session.total(), 0);
    assert!(session.current(&corpus).unwrap().is_none());
    assert_eq!(session.tally(), Tally::default());
}
