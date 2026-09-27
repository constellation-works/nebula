//! Dates and inbox stamps: today's date, the stamp a capture is taken at,
//! and how a stamp in either of the forms an inbox line holds is read back.

use time::{
    Date, OffsetDateTime, PrimitiveDateTime, UtcOffset,
    format_description::well_known::{Iso8601, Rfc3339},
    macros::format_description,
};

/// Today, local, as `YYYY-MM-DD`.
pub(crate) fn today() -> String {
    now()
        .format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

/// Now, as an inbox stamp: RFC 3339 to the second, with the local offset,
/// or in UTC marked `Z` when the local offset cannot be read. The first seven
/// characters are the month, which names the inbox file.
pub(crate) fn stamp() -> String {
    let now = OffsetDateTime::now_utc();
    format_stamp(now, UtcOffset::local_offset_at(now).ok())
}

/// `instant` as an inbox stamp at `offset`, or in UTC marked `Z` when there
/// is none.
///
/// `Z` and `+00:00` are the same instant, and the difference is kept on
/// purpose: `+00:00` is a local offset of zero that was read, `Z` is a
/// fallback, so a stamp never passes a guess off as local time
/// (STD-01 §R11).
pub(crate) fn format_stamp(instant: OffsetDateTime, offset: Option<UtcOffset>) -> String {
    match offset {
        Some(offset) => instant.to_offset(offset).format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
        )),
        None => instant
            .to_offset(UtcOffset::UTC)
            .format(format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z")),
    }
    .unwrap_or_default()
}

/// When an inbox stamp says a capture happened, in either form it may be in.
///
/// RFC 3339 is read as written. The legacy `YYYY-MM-DDTHH:MM` form, which
/// carries no offset, is local time with the offset this machine has for
/// that instant, or UTC when there is none: the reading recorded in
/// `docs/design/lineage-graph/4_decisions.md`.
pub(crate) fn parse_stamp(stamp: &str) -> Option<OffsetDateTime> {
    if let Ok(at) = OffsetDateTime::parse(stamp, &Rfc3339) {
        return Some(at);
    }
    let (wall, offset) = legacy_stamp(stamp)?;
    Some(wall.assume_offset(offset.unwrap_or(UtcOffset::UTC)))
}

/// A stamp as RFC 3339: itself when it already is, the legacy form read as
/// [`parse_stamp`] reads it, and `None` when it is neither.
pub(crate) fn rfc3339_stamp(stamp: &str) -> Option<String> {
    if OffsetDateTime::parse(stamp, &Rfc3339).is_ok() {
        return Some(stamp.to_string());
    }
    let (wall, offset) = legacy_stamp(stamp)?;
    Some(format_stamp(
        wall.assume_offset(offset.unwrap_or(UtcOffset::UTC)),
        offset,
    ))
}

/// A legacy `YYYY-MM-DDTHH:MM` stamp's wall-clock time, and the local
/// offset for it, if this machine can say.
///
/// The offset is looked up twice because it depends on the instant, which
/// depends on the offset: the second lookup settles a wall-clock time within
/// a few hours of a daylight-saving change.
fn legacy_stamp(stamp: &str) -> Option<(PrimitiveDateTime, Option<UtcOffset>)> {
    let wall = PrimitiveDateTime::parse(
        stamp,
        format_description!("[year]-[month]-[day]T[hour]:[minute]"),
    )
    .ok()?;
    let offset = UtcOffset::local_offset_at(wall.assume_utc())
        .ok()
        .map(|guess| UtcOffset::local_offset_at(wall.assume_offset(guess)).unwrap_or(guess));
    Some((wall, offset))
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

/// Whole days between a `YYYY-MM-DD` string and today, if it parses.
pub(crate) fn days_since(date: &str) -> Option<i64> {
    let d = Date::parse(date, &Iso8601::DATE).ok()?;
    Some((now().date() - d).whole_days())
}

/// Whole days between the date an inbox stamp was taken on, as its own
/// offset has it, and today. Either stamp form; `None` when it parses as
/// neither.
pub(crate) fn days_since_stamp(stamp: &str) -> Option<i64> {
    Some((now().date() - parse_stamp(stamp)?.date()).whole_days())
}
