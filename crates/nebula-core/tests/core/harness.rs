//! The fixtures every area shares: this process's resolved locations, a
//! fresh corpus, seeded nodes, planted edges, the live inbox entry for a
//! capture, git and its commits, files that are not regular, and a snapshot
//! of every file under a root.

use crate::support;
use nebula_core::{CommitOutcome, Committed, Corpus, Error, InboxEntry, Locations, NewNode, ops};
use std::path::Path;

/// The resolved environment the existing tests run under: this process's,
/// which `support` isolated before any test thread started. The new
/// resolver tests build theirs by hand instead.
pub(super) fn process_locations() -> Locations {
    Locations::from_reader(|name| std::env::var_os(name), std::env::current_dir().ok())
}

/// A hand edit of one node's frontmatter: the integration suite cannot call
/// `Corpus::save`, so it plants the edge in the file the node was written as.
pub(super) fn plant_edges(corpus: &Corpus, id: &str, edges_yaml: &str) {
    let path = corpus.node_path(id).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    let (front, rest) = raw.split_once("\n---").expect("frontmatter");
    assert!(
        !front.contains("\nedges:"),
        "the fixture node already has edges"
    );
    std::fs::write(&path, format!("{front}\nedges:\n{edges_yaml}\n---{rest}")).unwrap();
}

pub(super) fn live_inbox(corpus: &Corpus, id: &str) -> InboxEntry {
    corpus
        .inbox()
        .unwrap()
        .0
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("no live inbox entry {id}"))
}

pub(super) fn corpus() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().expect("tempdir");
    let corpus = Corpus::init(&process_locations(), &dir.path().join("corpus")).expect("init");
    (dir, corpus)
}

pub(super) fn seed(corpus: &Corpus, title: &str, parents: &[&str]) -> String {
    ops::new_node(
        corpus,
        &NewNode {
            title: title.to_string(),
            parents: parents.iter().map(ToString::to_string).collect(),
            ..NewNode::default()
        },
    )
    .expect("new node")
    .doc
    .node
    .id
}

/// Every entry under `root`, not following symlinks: a file's bytes, a
/// directory as `dir`, a symlink as `-> target`. What a test compares before
/// and after a call that must write nothing.
pub(super) fn every_file(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            let content = if kind.is_symlink() {
                format!("-> {}", std::fs::read_link(&path).unwrap().display()).into_bytes()
            } else if kind.is_dir() {
                pending.push(path.clone());
                b"dir".to_vec()
            } else {
                std::fs::read(&path).unwrap()
            };
            out.insert(path, content);
        }
    }
    out
}

/// Run git in `dir`, asserting it succeeded; stdout as text.
pub(super) fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = support::output(
        support::git_command(dir, support::home()).args(args),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository at `dir` with an identity, so `neb`'s commits do not depend
/// on the developer's global git configuration.
pub(super) fn git_init(dir: &std::path::Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "neb-test"]);
    git(dir, &["config", "user.email", "neb-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// The paths a commit touched, relative to the repository's top level.
pub(super) fn committed_paths(dir: &std::path::Path, rev: &str) -> Vec<String> {
    git(dir, &["show", "--name-only", "--format=", rev])
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

pub(super) fn head(dir: &std::path::Path) -> String {
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

/// The commit a write made, failing the test on any other outcome.
pub(super) fn committed(outcome: Result<CommitOutcome, Error>) -> Committed {
    match outcome {
        Ok(CommitOutcome::Committed(done)) => done,
        other => panic!("expected a commit, got {other:?}"),
    }
}

/// Whether `result` is the refusal of `path` as `found`.
pub(super) fn not_regular<T>(
    result: &nebula_core::Result<T>,
    path: &Path,
    found: nebula_core::fs::EntryKind,
) -> bool {
    matches!(result, Err(Error::NotRegularFile { path: p, found: f }) if p == path && *f == found)
}

/// Make a FIFO at `path` with `mkfifo`, through the isolating builder.
#[cfg(unix)]
pub(super) fn mkfifo(path: &Path) {
    let output = support::output(
        support::command("mkfifo", support::home()).arg(path),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(output.status.success(), "mkfifo {}", path.display());
}

/// Run `f` on a thread and return what it returned, failing the test if it
/// has not returned within two seconds: a read that blocks on a FIFO or
/// never finishes on a device fails here instead of hanging the suite.
pub(super) fn within_two_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .expect("the call did not return within 2 s")
}
