//! Unit tests for `error`, one file per source file (STD-02 §R19). This
//! file holds the tests for `error/mod.rs`: what each message names.

mod class;

use crate::model::FrontmatterProblem;

use crate::error::{Error, ErrorClass};
use crate::fs_impl::EntryKind;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

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
