//! Unit tests for `stamp`.

use crate::stamp::{format_stamp, parse_stamp, rfc3339_stamp, stamp};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

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
