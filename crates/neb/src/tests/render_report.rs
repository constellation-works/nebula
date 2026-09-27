//! Unit tests for `render_report`.

use crate::output::Role;
use crate::render::table::{Cell, Column, Table, Target};
use crate::render::{Notice, bold, count, dim, paint, status_badge, status_cell};
use nebula_core::{
    Band, CommitSetting, DroppedLegacy, EdgeType, HUMAN, INBOX_DAYS, Impact, Inbox,
    MigrationReport, Near, Neighbour, NodeView, OBSERVATORY_ROOT_ENV, ObservatoryRoot,
    ObservatorySource, OpenReport, Report, Retagged, ReviewItem, ReviewReport, ReviewRule,
    Severity, TagCounts, Via,
};
use std::fmt::Write as _;
use std::path::Path;

use crate::render::report::*;
use nebula_core::TagCount;

#[test]
fn tag_list_counts_right_aligned() {
    let counts = TagCounts(vec![
        TagCount {
            tag: "physics".into(),
            count: 5,
        },
        TagCount {
            tag: "filler".into(),
            count: 150,
        },
    ]);
    let drawn = tags(&counts, Target::Terminal { colour: false });
    let lines: Vec<&str> = drawn.lines().collect();
    assert_eq!(
        lines,
        ["TAG      COUNT", "physics      5", "filler     150"]
    );
    let end = lines[0].len();
    assert!(lines.iter().all(|l| l.len() == end), "{lines:?}");
}
