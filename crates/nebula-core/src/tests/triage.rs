//! Unit tests for `triage`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures hand-write inbox month files, not nebula state"
)]

use crate::error::{Error, Result};
use crate::graph_impl::{NEAR_DEFAULT, Neighbour};
use crate::ops_impl::{self as ops, Created, Promotion};
use crate::store::{self, Corpus, InboxEntry};
use crate::verb_impl::{WriteOptions, Written};
use serde::Serialize;
use std::collections::VecDeque;

use crate::triage_impl::*;

#[test]
fn triage_orders_mixed_stamp_forms_by_instant() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::init(&crate::Locations::default(), &dir.path().join("corpus")).unwrap();
    let inbox = dir.path().join("corpus/inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    // In file order, and so that text order differs from instant order:
    // `b` sorts before `a` as text but is an hour later. The legacy
    // stamp is local time, which is within fourteen hours of UTC
    // wherever this runs, so it falls between `b` and `d` on any host.
    std::fs::write(
        inbox.join("2026-09.md"),
        "- [000d] 2026-09-05T01:00:00+09:00 d\n\
         - [000c] 2026-09-03T08:00 c\n\
         - [000b] 2026-09-01T06:00:00-05:00 b\n\
         - [000e] someday e\n\
         - [000a] 2026-09-01T12:00:00+02:00 a\n",
    )
    .unwrap();

    let session = Triage::start(&corpus, None).unwrap();

    let order: Vec<&str> = session.queue.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(order, ["a", "b", "c", "d", "e"]);

    // An entry's age counts from the date its stamp was taken on, as its
    // own offset has it, whichever form the stamp is in.
    let legacy = store::days_since_stamp("2026-09-01T08:00");
    assert!(legacy.is_some());
    for stamp in [
        "2026-09-01T08:00:00+02:00",
        "2026-09-01T08:00:00Z",
        "2026-09-01T23:30:00-05:00",
    ] {
        assert_eq!(store::days_since_stamp(stamp), legacy, "{stamp}");
    }
    assert_eq!(store::days_since_stamp("someday"), None);
}
