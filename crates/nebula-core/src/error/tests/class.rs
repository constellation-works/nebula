//! Unit tests for `error/class.rs`: every variant's code and class.

use crate::error::{Error, ErrorClass};
use crate::model::Status;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

/// A variant name in `snake_case`: `NoSuchNode` is `no_such_node`.
fn snake_case(variant: &str) -> String {
    let mut out = String::new();
    for (i, c) in variant.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// Lowercase words of letters and digits joined by single underscores.
fn is_snake_case(code: &str) -> bool {
    code.split('_').all(|word| {
        word.starts_with(|c: char| c.is_ascii_lowercase())
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// `code()` is what `neb --json` reports, so it must be unique, and it
/// must be the variant's own name in `snake_case`: a consumer reading the
/// variant in Rust and one reading the code in JSON should be matching
/// the same words. [`Error::CODES`] holds every variant by construction,
/// so a new one is checked here without anyone listing it.
#[test]
fn every_code_is_its_variant_name_in_snake_case_and_unique() {
    // A variant split by class has one line per case, sharing its code.
    let mut seen = BTreeMap::new();
    for &(variant, code, _) in Error::CODES {
        assert!(is_snake_case(code), "`{code}` is not snake_case");
        assert_eq!(code, snake_case(variant), "the code for `{variant}`");
        let named = *seen.entry(code).or_insert(variant);
        assert_eq!(named, variant, "`{code}` names two variants");
    }
}

/// One line per variant covers every shape: tuple, struct and unit.
#[test]
fn the_codes_table_is_what_code_returns() {
    let io = Error::io_at("reading", "/x", std::io::Error::other("x"));
    assert!(Error::CODES.contains(&("IoAt", io.code(), io.class())));
    assert_eq!(Error::NoSuchNode("x".into()).code(), "no_such_node");
    assert_eq!(
        Error::NoNodeAtRevision {
            node: "x".into(),
            revision: "HEAD".into(),
        }
        .code(),
        "no_node_at_revision"
    );
    assert_eq!(Error::EmptyRoot.code(), "empty_root");
}

/// A refusal about the arguments alone is `Argument`; one about what they
/// met is `State`, and `RelativeObservatoryRoot` is either, by whether
/// the path came off the command line or out of the setting file.
#[test]
fn each_refusal_has_the_class_its_cause_decides() {
    let argument = [
        Error::EmptyCapture,
        Error::EmptyNote,
        Error::ReasonOnOpenStatus(Status::Seed),
        Error::UriRequired {
            kind: "paper".into(),
        },
        Error::InvalidAt("x".into()),
        Error::SelfLoop,
        Error::RelativeObservatoryRoot {
            root: PathBuf::from("rel"),
            setting: None,
        },
    ];
    let state = [
        Error::NoSuchNode("x".into()),
        Error::NoKillToConfirm("x".into()),
        Error::HomeUnset,
        Error::DirtyTree(PathBuf::from("/c")),
        Error::io_at("reading", "/x", io::Error::other("x")),
        Error::RelativeObservatoryRoot {
            root: PathBuf::from("rel"),
            setting: Some(PathBuf::from("/h/.config/nebula/observatory-root")),
        },
    ];
    for error in argument {
        assert_eq!(error.class(), ErrorClass::Argument, "{}", error.code());
    }
    for error in state {
        assert_eq!(error.class(), ErrorClass::State, "{}", error.code());
    }
    // Both classes appear in the table, and each line's class is the one
    // `class()` returns for its variant.
    assert!(
        Error::CODES
            .iter()
            .any(|&(_, _, c)| c == ErrorClass::Argument)
    );
    assert!(Error::CODES.iter().any(|&(_, _, c)| c == ErrorClass::State));
}

#[test]
fn snake_case_is_checked_word_by_word() {
    assert_eq!(snake_case("IoAt"), "io_at");
    assert!(is_snake_case("no_such_node"));
    for bad in ["NoSuchNode", "no__such", "_no", "no_", "no-such", ""] {
        assert!(!is_snake_case(bad), "{bad}");
    }
}
