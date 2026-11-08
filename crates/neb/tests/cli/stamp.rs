//! Inbox stamps: an RFC 3339 stamp reads back as written with its offset, and
//! a legacy offset-less stamp loads as local time for the instant it names.

use crate::harness::{Corpus, PLUS_TWO, write};

/// Whether `at` is RFC 3339, which says its offset by construction.
fn is_rfc3339(at: &serde_json::Value) -> bool {
    at.as_str().is_some_and(|at| {
        time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).is_ok()
    })
}

#[test]
fn inbox_at_is_rfc3339_with_offset() {
    let c = Corpus::new();

    let captured = c
        .run_with_env(&["capture", "x", "--json"], &[PLUS_TWO])
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    let id = captured["entry"]["id"].as_str().unwrap();
    let at = &captured["entry"]["at"];
    assert!(is_rfc3339(at), "{captured}");
    assert!(at.as_str().unwrap().ends_with("+02:00"), "{captured}");
    // Pinned like x: under the host zone, y can be filed in another month than
    // x, and the listing orders by month file.
    c.run_with_env(&["capture", "y", "--quiet"], &[PLUS_TWO])
        .assert_ok();

    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let listing: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 2, "{listing}");
    assert!(entries.iter().all(|e| is_rfc3339(&e["at"])), "{listing}");
    // Found by id rather than position, so the check does not depend on which
    // month file either capture landed in.
    let ours = entries
        .iter()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("the capture is listed: {listing}"));
    assert_eq!(ours["at"], *at, "a stamp reads back as written");

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
