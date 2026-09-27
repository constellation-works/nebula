//! Unit tests for `git`.

use crate::error::Error;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use crate::git::*;

fn captured(bytes: &[u8]) -> Captured {
    let mut out = Captured::default();
    out.push(bytes);
    out.ended = true;
    out
}

#[test]
fn output_under_the_cap_is_whole_and_unmarked() {
    let out = captured(b"  fatal: not a git repository\n");
    assert_eq!(out.whole(), Some(&b"  fatal: not a git repository\n"[..]));
    assert_eq!(out.text(), "fatal: not a git repository");
    assert!(!out.is_empty());
    assert!(captured(b"").is_empty());
}

#[test]
fn output_past_the_cap_is_kept_to_the_cap_refused_whole_and_marked() {
    let out = captured(&vec![b'x'; GIT_OUTPUT_CAP + 10]);
    assert_eq!(out.bytes.len(), GIT_OUTPUT_CAP);
    assert_eq!(out.whole(), None, "a cut stream is never parsed");
    let text = out.text();
    assert!(
        text.ends_with(" … [truncated 10 bytes]"),
        "{}",
        &text[GIT_OUTPUT_CAP..]
    );
    assert_eq!(text.len(), GIT_OUTPUT_CAP + " … [truncated 10 bytes]".len());
}

#[test]
fn a_cap_that_falls_inside_a_character_cuts_before_it() {
    // Three-byte characters from the start, so the cap lands mid-character.
    let mut bytes = "가".repeat(GIT_OUTPUT_CAP / 3 + 1).into_bytes();
    bytes.truncate(GIT_OUTPUT_CAP + 1);
    let text = captured(&bytes).text();
    let shown = text.split(" … [truncated").next().unwrap();
    assert!(shown.len() <= GIT_OUTPUT_CAP);
    assert!(shown.chars().all(|c| c == '가'));
}

#[test]
fn a_stream_that_never_ended_is_not_whole_and_says_so() {
    let mut out = Captured::default();
    out.push(b"partial");
    assert_eq!(out.whole(), None);
    assert!(
        out.text().starts_with("partial … [cut off"),
        "{}",
        out.text()
    );
}

#[test]
fn commit_gets_the_longer_deadline_and_the_override_is_per_thread() {
    // A fresh thread, so no other test's override is in force on it.
    let clean = std::thread::spawn(|| {
        assert_eq!(deadline_for(&["commit", "-q"]), GIT_COMMIT_DEADLINE);
        assert_eq!(deadline_for(&["log"]), GIT_DEADLINE);
        {
            let _short = GitDeadlineOverride::new(Duration::from_millis(5));
            assert_eq!(deadline_for(&["commit"]), Duration::from_millis(5));
            let other = std::thread::spawn(|| deadline_for(&["commit"]));
            assert_eq!(other.join().unwrap(), GIT_COMMIT_DEADLINE);
        }
        assert_eq!(deadline_for(&["commit"]), GIT_COMMIT_DEADLINE);
    });
    clean.join().unwrap();
}

/// The list is git's own: every variable this git says selects a
/// repository is scrubbed. A newer git that adds one fails here, which
/// is the prompt to add it.
#[cfg(unix)]
#[test]
fn every_repository_variable_git_names_is_scrubbed() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let at = GitAt {
        root: dir.path(),
        ceilings: Some(dir.path().as_os_str()),
    };
    let out = run_git(at, &["rev-parse", "--local-env-vars"]).expect("git runs");
    assert!(out.status.success(), "{}", out.stderr.text());
    let named = String::from_utf8(out.stdout.whole().unwrap().to_vec()).unwrap();
    let named: Vec<&str> = named.lines().collect();
    assert!(!named.is_empty());
    for name in named {
        assert!(REPOSITORY_ENV.contains(&name), "{name} is not scrubbed");
    }
}

/// `outer/.git` and a corpus two levels below it, resolved, so the walk
/// and the ceilings compare the same spelling on every platform.
fn nested_repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let outer = std::fs::canonicalize(dir.path()).unwrap().join("outer");
    let root = outer.join("a").join("corpus");
    crate::fs_impl::create_private_dir_all(&root).unwrap();
    std::fs::create_dir(outer.join(".git")).unwrap();
    (dir, outer, root)
}

#[test]
fn a_repository_is_found_at_or_above_the_root() {
    let (_dir, outer, root) = nested_repository();
    assert!(discovers(&root, &[]));
    assert!(discovers(&outer, &[]));
    // What is in `.git` is not read: a repository git cannot use is
    // still one, which is the point.
    assert!(discovers(&root, &[root.join("elsewhere")]));
}

#[test]
fn a_ceiling_stops_the_walk_before_it_but_never_at_the_root() {
    let (_dir, outer, root) = nested_repository();
    let a = outer.join("a");
    assert!(!discovers(&root, std::slice::from_ref(&a)));
    assert!(!discovers(&root, std::slice::from_ref(&outer)));
    // The root itself is always looked in, as git looks in its own
    // working directory whatever the ceilings say.
    assert!(discovers(&outer, std::slice::from_ref(&outer)));
}

#[test]
fn a_gitdir_file_names_a_repository_and_any_other_file_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let dot_git = root.join(".git");
    crate::fs_impl::write_private_atomic(&dot_git, b"not a pointer\n").unwrap();
    assert!(!names_a_repository(&dot_git));
    crate::fs_impl::write_private_atomic(&dot_git, b"gitdir: ../elsewhere/.git\n").unwrap();
    assert!(names_a_repository(&dot_git));
    assert!(discovers(&root, &[]));
}

#[test]
fn ceilings_are_read_as_git_reads_them() {
    let (_dir, outer, _root) = nested_repository();
    let missing = outer.join("missing");
    let joined = |parts: &[&Path]| {
        let mut value = std::ffi::OsString::new();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                value.push(":");
            }
            value.push(part);
        }
        value
    };
    // Relative and unresolvable entries are dropped; the rest resolve.
    assert_eq!(
        ceiling_directories(Some(
            joined(&[Path::new("relative"), &missing, &outer.join("a").join("..")]).as_os_str()
        )),
        std::slice::from_ref(&outer)
    );
    // After an empty entry, entries are taken as written.
    assert_eq!(
        ceiling_directories(Some(joined(&[Path::new(""), &missing]).as_os_str())),
        [missing]
    );
    assert!(ceiling_directories(None).is_empty());
}

#[test]
fn the_scrub_list_names_each_variable_once() {
    let mut names = REPOSITORY_ENV.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), REPOSITORY_ENV.len());
    assert!(names.iter().all(|name| name.starts_with("GIT_")));
}
