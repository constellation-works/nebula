//! The commit that follows a write when `config.yaml` asks for one, the
//! corpus `.gitignore` that keeps runtime files out of it, and the git calls
//! the store makes, all through the one supervised runner.

use super::Corpus;
use crate::config;
use crate::error::{Error, Result};
use crate::fs_impl::{Links, read_regular_bytes, write_private_atomic};
use crate::git::{self, GitAt, GitOutput};
use crate::lock::LOCK_FILE;
use crate::pending::PENDING_FILE;
use serde::Serialize;
use std::path::Path;

impl Corpus {
    /// Commit the corpus after a write, when `config.yaml` asks for it and
    /// the root is inside a git work tree.
    ///
    /// Stages `nodes/`, `inbox/`, `config.yaml` and the generated `.gitignore`
    /// under the root and nothing else — not `.lock` or `.pending`, which
    /// record nothing about the corpus, and not a temporary file a killed
    /// write left in `nodes/` or `inbox/` ([`NEVER_STAGED`]) — and commits
    /// exactly those paths as `neb <verb> <ids>`. The commit names them as its pathspec, so whatever
    /// else is staged in the repository, before this runs or while it does,
    /// is neither committed nor unstaged (STD-03 §R28). The write is on disk
    /// before this runs and stays there whatever git says. Never pushes.
    ///
    /// The setting is read from disk under the caller's lock rather than from
    /// the snapshot: whether this write is recorded is a question about the
    /// configuration in force, and a writer that waited its turn opened before
    /// the writer ahead of it had finished saying what that configuration is.
    pub(crate) fn commit(&self, verb: &str, ids: &[&str]) -> Result<CommitOutcome> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        if !self.current_config()?.commit {
            return Ok(CommitOutcome::Disabled);
        }
        let at = self.git_at();
        // Fail closed (integrity, STD-02 §R31): a repository git cannot read
        // is an error here, never a reason to leave the write unrecorded
        // without a word.
        if !inside_work_tree(at)? {
            return Ok(CommitOutcome::NotARepository);
        }
        refuse_ignored_corpus(at)?;
        let paths = commit_pathspec(&self.root);
        if paths.is_empty() {
            return Ok(CommitOutcome::NothingToCommit);
        }
        let mut add = vec!["add", "-A", "--"];
        add.extend(&paths);
        add.extend(NEVER_STAGED);
        git_ok(at, &add)?;
        let changed = staged_paths(at, &paths)?;
        if changed.is_empty() {
            // The write changed nothing git can see.
            return Ok(CommitOutcome::NothingToCommit);
        }
        let message = match ids {
            [] => format!("neb {verb}"),
            ids => format!("neb {verb} {}", ids.join(" ")),
        };
        // `--only` commits the named paths and nothing else the index holds,
        // so work someone else stages meanwhile stays staged and theirs.
        let mut commit = vec!["commit", "-q", "-m", &message, "--only", "--"];
        commit.extend(&changed);
        git_ok(at, &commit)?;
        let hash = git_ok(at, &["rev-parse", "HEAD"])?.trim().to_string();
        Ok(CommitOutcome::Committed(Committed { hash, message }))
    }
}

/// A commit `neb` made after a write.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Committed {
    /// The full commit hash.
    pub hash: String,
    /// The message, `neb <verb> <ids>`.
    pub message: String,
}

/// What the commit after a write did. The write is on disk before the commit
/// runs, so it stays there whichever this is.
#[must_use = "a write left uncommitted is a fact to report, not to drop"]
#[derive(Debug, Clone)]
pub enum CommitOutcome {
    /// The corpus paths were committed, as `neb <verb> <ids>`.
    Committed(Committed),
    /// `config.yaml` does not ask for commits.
    Disabled,
    /// Commits are on, but there is no git repository at or above the root,
    /// so the write was not recorded.
    NotARepository,
    /// The write left nothing git would record.
    NothingToCommit,
}

/// What a `neb` commit may contain, relative to the corpus root. Everything
/// else under the root, and everything outside it, is left alone — the write
/// lock's `.lock` included, which is why it is not listed here and never
/// will be: it is a fact about which process is writing right now, not about
/// the corpus, and it means nothing on another machine. `.gitignore` is
/// corpus setup metadata: `init` maintains it so ordinary git commands cannot
/// mistake the lock for corpus content.
pub(crate) const GITIGNORE_FILE: &str = ".gitignore";
const COMMIT_PATHS: [&str; 4] = ["nodes", "inbox", config::FILE, GITIGNORE_FILE];

/// The pathspec a `neb` commit stages and commits: the [`COMMIT_PATHS`] that
/// exist under `root`, relative to it. Only those, because `inbox/` appears
/// on the first capture and a pathspec that matches nothing is a git error.
fn commit_pathspec(root: &Path) -> Vec<&'static str> {
    COMMIT_PATHS
        .iter()
        .copied()
        .filter(|path| root.join(path).exists())
        .collect()
}

/// Refuse a corpus that the repository around it ignores: `commit` on would
/// otherwise record nothing, forever, without a word. A private repository at
/// the corpus root is the fix.
fn refuse_ignored_corpus(at: GitAt<'_>) -> Result<()> {
    let ignored = git(at, &["check-ignore", "-q", "--", "nodes"])?;
    match ignored.status.code() {
        Some(0) => Err(Error::CorpusIgnored(at.root.to_path_buf())),
        Some(1) => Ok(()),
        _ => Err(git_failed(at.root, "check-ignore", &ignored.stderr.text())),
    }
}

/// The entries of `pathspec` under which the index holds a change `HEAD`
/// does not: what a `neb` commit names, since `git commit --only` refuses an
/// entry that matches nothing git knows, such as the empty `nodes/` of a new
/// corpus. Each entry is asked about alone and nothing else is: something
/// staged elsewhere is not the corpus's to commit, so it is not something to
/// commit either. The entries are paths, never `:(exclude)` magic, which
/// asked about alone would mean everything else: a file to keep out of a
/// commit is kept out of the `git add` before it, since `--only` commits only
/// paths git already tracks.
fn staged_paths<'a>(at: GitAt<'_>, pathspec: &[&'a str]) -> Result<Vec<&'a str>> {
    let mut staged = Vec::new();
    for &path in pathspec {
        let diff = git(at, &["diff", "--cached", "--quiet", "--", path])?;
        match diff.status.code() {
            Some(0) => {}
            Some(1) => staged.push(path),
            _ => return Err(git_failed(at.root, "diff", &diff.stderr.text())),
        }
    }
    Ok(staged)
}

/// Pathspecs that keep crash debris out of a `neb` commit, appended to the
/// [`commit_pathspec`] of its `git add` and nowhere else: [`staged_paths`]
/// and `commit --only` take plain paths, and `--only` never commits a file
/// the `git add` left untracked.
///
/// A write killed before its rename leaves its temporary file,
/// `<file>.<pid>-<n>-<nanos>.tmp` (see `fs.rs`), beside the file it was
/// replacing — in `nodes/` or `inbox/`, which a commit stages whole. It is
/// debris, not corpus: it stays on disk for `check` to report and a person
/// to remove, and out of the history (STD-03 §R4). The pending-write record
/// is excluded too, although no corpus path names it, so a pathspec that
/// ever widens cannot sweep it in. `init` writes the matching ignore rules
/// ([`ignore_rules`]), so a person's own `git add -A` skips both as well.
pub(crate) const NEVER_STAGED: [&str; 2] = [":(exclude,glob)**/*.tmp", ":(exclude).pending"];

/// The rules `init` keeps last in the corpus `.gitignore`, in this order:
/// the advisory lock and the pending-write record at the root, which say
/// which process is writing and what it is part-way through, and every
/// temporary file a killed write leaves behind ([`NEVER_STAGED`]). None of
/// them is corpus content.
pub(crate) fn ignore_rules() -> [String; 3] {
    [
        format!("/{LOCK_FILE}"),
        format!("/{PENDING_FILE}"),
        "*.tmp".to_string(),
    ]
}

/// Keep the runtime files and crash debris ([`ignore_rules`]) out of the
/// corpus repository.
///
/// Existing ignore content is preserved. Re-running `init` is idempotent when
/// the final effective rules are already ours, in order; if the user later
/// adds another rule, a later `init` puts ours last again. A file that ends
/// with the first of ours — the one rule an older `init` wrote — gets only
/// the rest appended, so an upgrade does not repeat it. Git does not follow a
/// `.gitignore` symlink, so even a symlink whose target already ends in our
/// rules is replaced atomically with a regular file containing the same
/// bytes. The target itself is never changed. That copy is the one read
/// below the root that follows a symlink, and it follows it only to a regular
/// file: a FIFO or a device at `.gitignore`, or at the end of its link, is
/// [`Error::NotRegularFile`] rather than read.
pub(crate) fn ensure_lock_ignored(root: &Path) -> Result<()> {
    let path = root.join(GITIGNORE_FILE);
    let is_symlink = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(Error::io_at("inspecting", &path, error)),
    };
    let original = read_regular_bytes(&path, Links::Follow)?.unwrap_or_default();
    let contents = with_runtime_ignores(original.clone());
    if contents != original || is_symlink {
        write_private_atomic(&path, contents)?;
    }
    Ok(())
}

/// The exact bytes installed by `ensure_lock_ignored`, also used to prove
/// that an interrupted migration owns a dirty ignore file.
pub(crate) fn with_runtime_ignores(mut contents: Vec<u8>) -> Vec<u8> {
    let ours = ignore_rules();
    let rules: Vec<&[u8]> = contents
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .collect();
    // How many of ours already end the file, in order: the rest are appended.
    let present = (0..=ours.len())
        .rev()
        .find(|&n| {
            n <= rules.len()
                && rules[rules.len() - n..]
                    .iter()
                    .zip(&ours[..n])
                    .all(|(rule, our)| *rule == our.as_bytes())
        })
        .unwrap_or(0);
    if present == ours.len() {
        return contents;
    }

    if !contents.is_empty() && !contents.ends_with(b"\n") {
        contents.push(b'\n');
    }
    for rule in &ours[present..] {
        contents.extend_from_slice(rule.as_bytes());
        contents.push(b'\n');
    }
    contents
}

/// Run git at the corpus root, through the one supervised runner. Git not
/// starting, running out of time or being stopped is what this reports;
/// whether the command succeeded is the caller's to judge, since a non-zero
/// exit is an answer for some of them.
pub(crate) fn git(at: GitAt<'_>, args: &[&str]) -> Result<GitOutput> {
    git::run_git(at, args)
        .map_err(|e| e.into_error(at.root, args.first().copied().unwrap_or("git")))
}

/// Run git at the corpus root and require it to succeed; all of stdout as
/// text.
pub(super) fn git_ok(at: GitAt<'_>, args: &[&str]) -> Result<String> {
    let context = args.first().copied().unwrap_or("git");
    let out = git(at, args)?;
    if !out.status.success() {
        return Err(git_failed(at.root, context, &out.stderr.text()));
    }
    Ok(String::from_utf8_lossy(out.stdout_whole(at.root, context)?).into_owned())
}

pub(crate) fn git_failed(root: &Path, context: &str, stderr: &str) -> Error {
    Error::Git {
        root: root.to_path_buf(),
        context: context.to_string(),
        stderr: stderr.trim().to_string(),
    }
}

/// Whether the root is inside a git work tree.
///
/// `false` when there is no repository to find
/// ([`git::repository_expected`]), without running git, or when git found
/// one and says the root is not in its work tree. With a repository there, git
/// failing — not starting, or unable to read it — is [`Error::Git`], never
/// "not a work tree" (STD-02 §R29).
pub(crate) fn inside_work_tree(at: GitAt<'_>) -> Result<bool> {
    if !git::repository_expected(at) {
        return Ok(false);
    }
    let out = git(at, &["rev-parse", "--is-inside-work-tree"])?;
    if !out.status.success() {
        return Err(git_failed(at.root, "rev-parse", &out.stderr.text()));
    }
    Ok(String::from_utf8_lossy(out.stdout_whole(at.root, "rev-parse")?).trim() == "true")
}
