//! Unit tests for `error`.

use crate::lock::LockHolder;
use crate::model::{EdgeType, FrontmatterProblem, Status};
use crate::store::Settlement;

use crate::error::{Error, ErrorClass};
use crate::fs_impl::EntryKind;
use std::collections::BTreeMap;
use std::ffi::OsString;
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

/// Each error's message holds every needle beside it.
fn assert_named(rows: Vec<(Error, &[&str])>) {
    for (error, needles) in rows {
        let said = error.to_string();
        for needle in needles {
            assert!(
                said.contains(needle),
                "{} lacks `{needle}`: {said}",
                error.code()
            );
        }
    }
}

/// An environment or machine-setting error names the value or the file
/// that decided it (STD-02 §R26). One row per variant.
#[test]
fn environment_errors_name_their_value_or_setting() {
    #[cfg(unix)]
    let not_unicode = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"/home/\xffuser".to_vec())
    };
    #[cfg(not(unix))]
    let not_unicode = OsString::from("/home/user");
    assert_named(vec![
        (Error::HomeUnset, &["HOME is not set"]),
        (
            Error::HomeNotUnicode(not_unicode),
            &["not valid UTF-8", "/home/", "user"],
        ),
        (
            Error::EmptyRootSetting(PathBuf::from("/h/.config/nebula/root")),
            &["/h/.config/nebula/root"],
        ),
        (
            Error::RelativeRootSetting {
                setting: PathBuf::from("/h/.config/nebula/root"),
                root: PathBuf::from("rel"),
            },
            &["`rel`", "/h/.config/nebula/root"],
        ),
    ]);
}

/// An I/O or on-disk error names the path it touched (STD-02 §R26). One
/// row per variant.
#[test]
fn io_errors_name_their_path() {
    let p = PathBuf::from;
    let denied = || io::Error::from(io::ErrorKind::PermissionDenied);
    let (month, entry) = (p("/c/inbox/2026-09.md"), || "ab12".to_string());
    assert_named(vec![
        (
            Error::io_at("reading", &month, denied()),
            &["reading /c/inbox/2026-09.md", "ermission denied"],
        ),
        (
            Error::MalformedFrontmatter {
                path: p("/c/nodes/a.md"),
                problem: FrontmatterProblem::Unterminated,
            },
            &["/c/nodes/a.md", "not terminated"],
        ),
        (
            Error::InboxEntryForeign {
                id: entry(),
                file: p("/other/inbox/2026-09.md"),
                inbox: p("/c/inbox"),
            },
            &["ab12", "/other/inbox/2026-09.md", "/c/inbox"],
        ),
        (
            Error::InboxEntryMissing {
                id: entry(),
                file: month.clone(),
                line: 4,
            },
            &["ab12", "/c/inbox/2026-09.md", "line 5"],
        ),
        (
            Error::InboxEntryChanged {
                id: entry(),
                file: month.clone(),
                line: 4,
            },
            &["ab12", "/c/inbox/2026-09.md", "line 5"],
        ),
        (Error::NodesSymlink(p("/c/nodes")), &["/c/nodes"]),
        (Error::InboxSymlink(p("/c/inbox")), &["/c/inbox"]),
        (
            Error::MalformedHistory {
                root: p("/c"),
                command: "log",
                pathspec: "nodes/a.md".into(),
            },
            &["git log", "`nodes/a.md`", "/c"],
        ),
        (
            Error::NoFreeKeepName {
                id: "a".into(),
                dir: p("/h/.local/state/nebula/kept"),
                attempts: 16,
            },
            &["`a`", "/h/.local/state/nebula/kept", "16 tries"],
        ),
        (Error::DirtyTree(p("/c")), &["/c has uncommitted changes"]),
        (
            Error::MigratedNodeUnreadable {
                path: p("/c/nodes/a.md"),
                source: Box::new(Error::NotAStatus("odd".into())),
            },
            &["/c/nodes/a.md", "`odd`"],
        ),
        (
            Error::NotAV1Status {
                path: p("/c/nodes/a.md"),
                status: "odd".into(),
            },
            &["/c/nodes/a.md", "`odd`"],
        ),
        (
            Error::NotAV1EdgeType {
                path: p("/c/nodes/a.md"),
                edge_type: "odd".into(),
            },
            &["/c/nodes/a.md", "`odd`"],
        ),
        (
            Error::TempCleanupFailed {
                path: p("/c/nodes/a.md"),
                tmp: p("/c/nodes/a.md.1.tmp"),
                write: io::Error::from(io::ErrorKind::StorageFull),
                cleanup: denied(),
            },
            &["/c/nodes/a.md", "/c/nodes/a.md.1.tmp", "ermission denied"],
        ),
        (
            Error::NoFreeTempName {
                path: p("/c/nodes/a.md"),
                attempts: 8,
            },
            &["/c/nodes/a.md", "8 tries"],
        ),
    ]);
}

/// An entry that is not a regular file is named, with what it is instead.
#[test]
fn a_non_regular_entry_is_named_with_what_it_is() {
    for (found, what) in [
        (EntryKind::Symlink, "a symlink"),
        (EntryKind::Directory, "a directory"),
        (EntryKind::Fifo, "a FIFO"),
        (EntryKind::Socket, "a socket"),
        (EntryKind::Device, "a device"),
        (EntryKind::Other, "a special file"),
    ] {
        let error = Error::NotRegularFile {
            path: PathBuf::from("/c/nodes/a.md"),
            found,
        };
        assert!(
            error
                .to_string()
                .starts_with(&format!("/c/nodes/a.md is {what}, not a regular file")),
            "{error}"
        );
        assert_eq!(error.class(), ErrorClass::State);
    }
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
