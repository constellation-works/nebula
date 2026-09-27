//! Unit tests for `id`.

use crate::id::{CAPTURE_ID_WORDS, capture_ids, is_path_safe_id, is_slug, slugify};
use std::path::Path;

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
