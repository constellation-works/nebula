//! Unit tests for `model`.

use crate::error::{Error, Result};
use crate::fs_impl::write_private_atomic;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;
use time::{Date, format_description::well_known::Iso8601};

use crate::model::*;

#[test]
fn tags_are_lowercase_kebab_case() {
    assert_eq!(normalize_tag("Physics"), "physics");
    assert_eq!(normalize_tag("Machine Learning"), "machine-learning");
    assert_eq!(normalize_tag("foo_bar--baz "), "foo-bar-baz");
    assert_eq!(normalize_tag("Ünïcode título"), "ünïcode-título");
    assert_eq!(
        normalize_tag("시간은 프레임의 수다"),
        "시간은-프레임의-수다"
    );
    assert_eq!(normalize_tag("  "), "");
    assert_eq!(
        normalize_tags(&["A".into(), "a".into(), String::new(), "B c".into()]),
        vec!["a", "b-c"]
    );
}

/// A column is a width specifier on the value itself, so `Display` has
/// to honour it; `write_str` would print the bare word and leave every
/// column after it ragged.
#[test]
fn statuses_and_edge_types_honour_width_and_fill() {
    assert_eq!(format!("{:<10}|", Status::Seed), "seed      |");
    assert_eq!(format!("{:>10}|", Status::Refuted), "   refuted|");
    assert_eq!(format!("{:.<12}|", Status::Hypothesis), "hypothesis..|");
    assert_eq!(format!("{:<14}|", EdgeType::Reopens), "reopens       |");
    assert_eq!(Status::Abandoned.to_string(), "abandoned");
}

#[test]
fn statuses_and_edge_types_round_trip_through_strings() {
    for s in [
        Status::Seed,
        Status::Hypothesis,
        Status::Refuted,
        Status::Abandoned,
    ] {
        assert_eq!(s.to_string().parse::<Status>().unwrap(), s);
    }
    for e in [
        EdgeType::DerivesFrom,
        EdgeType::Refines,
        EdgeType::Generalizes,
        EdgeType::Reopens,
        EdgeType::Contradicts,
    ] {
        assert_eq!(e.to_string().parse::<EdgeType>().unwrap(), e);
    }
    let Err(Error::NotAStatus(status)) = "graduated".parse::<Status>() else {
        panic!("expected an unknown status")
    };
    assert_eq!(status, "graduated");
    let Err(Error::NotAnEdgeType(kind)) = "supports".parse::<EdgeType>() else {
        panic!("expected an unknown edge type")
    };
    assert_eq!(kind, "supports");
}

#[test]
fn lineage_names_each_parent_once_with_every_kind() {
    let doc = parse(
        "---\nid: child\ntitle: Child\nstatus: seed\ncreated: 2026-09-26\nupdated: 2026-09-26\n\
         edges:\n\
         - type: derives-from\n  to: old\n\
         - type: refines\n  to: other\n\
         - type: contradicts\n  to: old\n\
         - type: reopens\n  to: old\n\
         - type: derives-from\n  to: old\n\
         ---\n\nbody\n",
        Path::new("nodes/child.md"),
    )
    .unwrap();
    assert_eq!(
        doc.node.lineage(),
        [
            ("old", vec![EdgeType::DerivesFrom, EdgeType::Reopens]),
            ("other", vec![EdgeType::Refines]),
        ],
        "first-declared order, `contradicts` left out, a repeated kind once"
    );
}

#[test]
fn notes_accumulate_in_order_and_leave_earlier_body_alone() {
    let first = append_note("the original capture", "2026-09-21", "first thought", None);
    assert_eq!(
        first,
        "the original capture\n\n## Notes\n\n- 2026-09-21: first thought"
    );
    let second = append_note(&first, "2026-09-21", "second thought", None);
    assert_eq!(
        second,
        "the original capture\n\n## Notes\n\n- 2026-09-21: first thought\n- 2026-09-21: second thought"
    );
    assert!(
        second.starts_with("the original capture"),
        "the capture itself is not rewritten"
    );
    assert_eq!(
        notes_from_body(&second),
        vec![
            Note {
                at: "2026-09-21".into(),
                text: "first thought".into(),
                by: HUMAN.into(),
            },
            Note {
                at: "2026-09-21".into(),
                text: "second thought".into(),
                by: HUMAN.into(),
            },
        ]
    );
}

#[test]
fn an_empty_body_still_gets_a_notes_section() {
    let body = append_note("", "2026-09-21", "alone", None);
    assert_eq!(body, "## Notes\n\n- 2026-09-21: alone");
    assert_eq!(
        notes_from_body(&body),
        vec![Note {
            at: "2026-09-21".into(),
            text: "alone".into(),
            by: HUMAN.into(),
        }]
    );
}

/// A note somebody else wrote names them in the line, and reads back as
/// theirs; the human's line is untouched and reads back as the human's.
#[test]
fn a_note_carries_its_author_in_the_line_only_when_it_is_not_the_human() {
    let mine = append_note("", "2026-09-21", "my reasoning", None);
    let ours = append_note(&mine, "2026-09-21", "its reasoning", Some("crew-alpha"));
    assert_eq!(
        ours,
        "## Notes\n\n- 2026-09-21: my reasoning\n- 2026-09-21 (crew-alpha): its reasoning"
    );
    let notes = notes_from_body(&ours);
    assert_eq!(notes[0].by, HUMAN);
    assert_eq!(notes[1].by, "crew-alpha");
    assert_eq!(notes[1].text, "its reasoning");
}

#[test]
fn an_author_label_is_free_text_but_cannot_break_a_note_line() {
    assert_eq!(author(None).unwrap(), None);
    assert_eq!(author(Some(" human ")).unwrap(), None);
    assert_eq!(author(Some("  ")).unwrap(), None);
    assert_eq!(
        author(Some("agent:session-7d2")).unwrap(),
        Some("agent:session-7d2".into())
    );
    assert!(author(Some("crew (alpha)")).is_err());
    assert!(author(Some("crew: alpha")).is_err());
    assert!(is_human(None) && is_human(Some("human")));
    assert!(!is_human(Some("agent:session-7d2")));
}

#[test]
fn a_later_heading_gets_a_fresh_notes_section_at_the_end() {
    let body = append_note("intro\n\n## Next\n\ndo x", "2026-09-21", "why", None);
    assert_eq!(
        body,
        "intro\n\n## Next\n\ndo x\n\n## Notes\n\n- 2026-09-21: why"
    );
    assert_eq!(notes_from_body(&body)[0].text, "why");
}

/// A body whose notes are followed by other prose gets a second section
/// rather than an edit to the first, and both are reasoning: the
/// projection reads every section, so the older thought is not hidden by
/// the newer one.
#[test]
fn notes_in_an_earlier_section_are_still_read_back() {
    let body =
        "the argument\n\n## Notes\n\n- 2026-09-21: the first thought\n\n## More\n\nstill to do";
    let after = append_note(body, "2026-09-22", "the second thought", None);
    assert_eq!(
        after,
        format!("{body}\n\n## Notes\n\n- 2026-09-22: the second thought"),
        "a closed notes section is left alone and a new one opens at the end"
    );

    let sections = notes_sections(&after);
    assert_eq!(sections.len(), 2, "{sections:?}");
    assert_eq!(
        sections[0].trim_end(),
        "## Notes\n\n- 2026-09-21: the first thought",
        "a section ends at the heading that follows it"
    );

    let notes = notes_from_body(&after);
    assert_eq!(
        notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
        vec!["the first thought", "the second thought"],
        "every section is read, oldest first"
    );
}

/// Note-shaped lines outside a notes section are somebody's prose, not
/// reasoning the corpus promised to keep.
#[test]
fn a_dated_line_outside_a_notes_section_is_not_a_note() {
    let body = "## Log\n\n- 2026-09-21: not a note\n\n## Notes\n\n- 2026-09-22: a note";
    assert_eq!(notes_sections(body).len(), 1);
    let notes = notes_from_body(body);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].text, "a note");
}
