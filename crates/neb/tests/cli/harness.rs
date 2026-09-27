//! The fixture every area shares: `neb` and `git` children built only
//! through the isolating builder, the `Corpus` a test runs `neb` against, the
//! `Run` it gets back with its assertions, the helpers that plant dates,
//! editors and files, and the git repository, log and snapshot fixtures the
//! commit and history areas share. See `support` for what each child is
//! isolated from.

use crate::history::git_commit_stamped;
use crate::support::{self, ChildGuard};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub(super) fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("neb")
}

/// A `neb` isolated from the host with `home` as its home: the only place a
/// test here creates one. See [`support::isolate`] for what it clears and
/// sets; a test that needs a variable back sets it after this.
pub(super) fn neb_command(home: &Path) -> Command {
    let mut cmd = Command::new(bin());
    support::isolate(&mut cmd, home);
    cmd
}

/// `git -C dir`, isolated with `home` as its home.
pub(super) fn git_command(dir: &Path, home: &Path) -> Command {
    support::git_command(dir, home)
}

/// Run `cmd` to completion under a guard and the deadline, failing the test
/// if it outlives it.
pub(super) fn output(cmd: &mut Command) -> Output {
    support::output(cmd, support::DEADLINE).unwrap_or_else(|e| panic!("{e}"))
}

/// [`output`] as a [`Run`] reported as `neb {args}`.
pub(super) fn run_command(cmd: &mut Command, args: String) -> Run {
    Run {
        args,
        out: output(cmd),
    }
}

pub(super) struct Corpus {
    pub(super) dir: tempfile::TempDir,
    pub(super) root: PathBuf,
}

impl Corpus {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("corpus");
        let me = Self { dir, root };
        me.run(&["init"]).assert_ok();
        me
    }

    pub(super) fn run(&self, args: &[&str]) -> Run {
        self.run_with_env(args, &[])
    }

    pub(super) fn run_with_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut cmd = self.command(args);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        run_command(&mut cmd, args.join(" "))
    }

    pub(super) fn run_with_stdin(&self, args: &[&str], input: &str) -> Run {
        let mut cmd = self.command(args);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ChildGuard::spawn(&mut cmd).unwrap_or_else(|e| panic!("{e}"));
        child
            .take_stdin()
            .write_all(input.as_bytes())
            .expect("writing stdin");
        let out = child
            .wait_with_output(support::DEADLINE)
            .unwrap_or_else(|e| panic!("{e}"));
        Run {
            args: args.join(" "),
            out,
        }
    }

    /// [`Corpus::run_with_env`] with stdout closed before `neb` writes a
    /// byte, and `input` on stdin.
    pub(super) fn run_closed_stdout(
        &self,
        args: &[&str],
        extra: &[(&str, &str)],
        input: &str,
    ) -> Run {
        let mut cmd = self.command(args);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        Run {
            args: args.join(" "),
            out: closed_stdout(&mut cmd, input),
        }
    }

    /// Start the binary without waiting for it, so two of them can be in
    /// flight at once. The returned handle is finished with
    /// [`Spawned::wait`].
    pub(super) fn spawn(&self, args: &[&str]) -> Spawned {
        self.spawn_with_env(args, &[])
    }

    /// [`Corpus::spawn`] with extra variables set, and stdin closed.
    pub(super) fn spawn_with_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Spawned {
        let mut cmd = self.command(args);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Spawned {
            args: args.join(" "),
            child: ChildGuard::spawn(&mut cmd).unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// `neb --root <root> <args>` with the fixture's directory as its home.
    pub(super) fn command(&self, args: &[&str]) -> Command {
        let mut cmd = neb_command(self.workdir());
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1");
        cmd
    }

    pub(super) fn workdir(&self) -> &Path {
        self.dir.path()
    }

    pub(super) fn node_file(&self, id: &str) -> PathBuf {
        self.root.join("nodes").join(format!("{id}.md"))
    }

    /// Create a seed by capturing then promoting, the normal path.
    pub(super) fn seed(&self, text: &str, title: &str) -> String {
        let id = self.run(&["capture", text]).stdout_trim();
        self.run(&["promote", &id, "--title", title])
            .assert_ok()
            .stdout_trim()
    }
}

/// Run against an isolated home directory so root discovery is part of the
/// fixture rather than a property of the developer's shell. The working
/// directory is the fixture's own temporary directory, which holds corpora
/// but is not one, so no corpus is found from it.
pub(super) fn run_from_home(
    home: &Path,
    root: Option<&Path>,
    args: &[&str],
    nebula_root: Option<&Path>,
) -> Run {
    run_in(home.parent().unwrap_or(home), home, root, args, nebula_root)
}

/// [`run_from_home`] from a chosen working directory, with `PWD` set to it
/// the way a shell sets it after `cd`, so the spelling of `cwd` is what the
/// binary sees.
pub(super) fn run_in(
    cwd: &Path,
    home: &Path,
    root: Option<&Path>,
    args: &[&str],
    nebula_root: Option<&Path>,
) -> Run {
    let mut cmd = neb_command(home);
    if let Some(root) = root {
        cmd.arg("--root").arg(root);
    }
    cmd.args(args)
        .current_dir(cwd)
        .env("PWD", cwd)
        .env("NO_COLOR", "1");
    if let Some(nebula_root) = nebula_root {
        cmd.env("NEBULA_ROOT", nebula_root);
    }
    run_command(&mut cmd, args.join(" "))
}

/// Run `cmd` with stdout on a pipe whose read end is already closed, as
/// `neb … | head -1` leaves it once `head` has its line, and `input` on
/// stdin. Only stdio is set here, so any builder's command can come through.
pub(super) fn closed_stdout(cmd: &mut Command, input: &str) -> Output {
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    cmd.stdin(Stdio::piped())
        .stdout(writer)
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(cmd).unwrap_or_else(|e| panic!("{e}"));
    let mut stdin = child.take_stdin();
    // A verb that never reads stdin may be gone before this lands.
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    child
        .wait_with_output(support::DEADLINE)
        .unwrap_or_else(|e| panic!("{e}"))
}

/// A `neb` still running, so a test can have two of them racing. Dropped
/// unfinished, as by a failed assertion, it is killed and reaped.
pub(super) struct Spawned {
    pub(super) args: String,
    pub(super) child: ChildGuard,
}

impl Spawned {
    /// Wait for it to finish, failing the test past the deadline.
    pub(super) fn wait(self) -> Run {
        let out = self
            .child
            .wait_with_output(support::DEADLINE)
            .unwrap_or_else(|e| panic!("{e}"));
        Run {
            args: self.args,
            out,
        }
    }
}

pub(super) struct Run {
    pub(super) args: String,
    pub(super) out: Output,
}

impl Run {
    pub(super) fn assert_ok(self) -> Self {
        assert!(
            self.out.status.success(),
            "`neb {}` failed:\n{}\n{}",
            self.args,
            String::from_utf8_lossy(&self.out.stdout),
            String::from_utf8_lossy(&self.out.stderr)
        );
        self
    }
    pub(super) fn assert_fails(self) -> Self {
        assert!(
            !self.out.status.success(),
            "`neb {}` unexpectedly succeeded",
            self.args
        );
        self
    }
    pub(super) fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.out.stdout).to_string()
    }
    pub(super) fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).to_string()
    }
    pub(super) fn stdout_trim(&self) -> String {
        self.stdout()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    }
    pub(super) fn says(self, needle: &str) -> Self {
        let all = format!("{}{}", self.stdout(), self.stderr());
        assert!(
            all.contains(needle),
            "expected `{needle}` in output of `neb {}`:\n{all}",
            self.args
        );
        self
    }
    /// The refusal `--json` reports: exit 1, and stderr exactly one line
    /// holding `{"error", "code", "hint"}` and nothing else, with a
    /// `snake_case` code. Returns the envelope.
    pub(super) fn refusal(&self) -> serde_json::Value {
        self.refusal_exiting(1)
    }
    /// [`Run::refusal`] for a usage error, which exits 2 (STD-01 §R20).
    pub(super) fn usage_refusal(&self) -> serde_json::Value {
        self.refusal_exiting(2)
    }
    pub(super) fn refusal_exiting(&self, exit: i32) -> serde_json::Value {
        let stderr = self.stderr();
        assert_eq!(
            self.out.status.code(),
            Some(exit),
            "`neb {}` should refuse with exit {exit}:\n{stderr}",
            self.args
        );
        assert!(
            stderr.ends_with('\n') && stderr.lines().count() == 1,
            "`neb {}` should write one line to stderr:\n{stderr}",
            self.args
        );
        let value: serde_json::Value = serde_json::from_str(&stderr)
            .unwrap_or_else(|e| panic!("`neb {}` stderr is not JSON ({e}):\n{stderr}", self.args));
        let envelope = value.as_object().expect("the envelope is an object");
        assert_eq!(
            envelope.keys().collect::<Vec<_>>(),
            ["code", "error", "hint"],
            "{value}"
        );
        assert!(value["error"].is_string(), "{value}");
        let code = value["code"].as_str().expect("`code` is a string");
        assert!(
            !code.is_empty()
                && code.split('_').all(|word| {
                    word.starts_with(|c: char| c.is_ascii_lowercase())
                        && word
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                }),
            "`{code}` is not snake_case: {value}"
        );
        assert!(
            value["hint"].is_string() || value["hint"].is_null(),
            "{value}"
        );
        value
    }
}

pub(super) fn write(path: &Path, s: &str) {
    std::fs::write(path, s).expect("writing fixture");
}

#[cfg(unix)]
pub(super) fn editor_script(c: &Corpus, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = c.workdir().join(name);
    write(&path, script);
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

/// `YYYY-MM-DD` for `days` ago, computed the same way `stamp::days_since`
/// computes "now", so fixtures land unambiguously on one side of a threshold
/// regardless of what day the suite actually runs.
pub(super) fn date_days_ago(days: i64) -> String {
    use time::{OffsetDateTime, macros::format_description};
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = now.date() - time::Duration::days(days);
    date.format(format_description!("[year]-[month]-[day]"))
        .expect("formatting a date")
}

/// A stamp in the legacy `YYYY-MM-DDTHH:MM` form, so the tests that back-date
/// captures keep covering legacy stamps. Only the date feeds the fourteen-day
/// rule.
pub(super) fn stamp_days_ago(days: i64) -> String {
    format!("{}T00:00", date_days_ago(days))
}

/// Back-date a node's `updated` field in place, to land it on a chosen side
/// of a staleness threshold without waiting for real time to pass.
pub(super) fn set_updated(path: &Path, date: &str) {
    let raw = std::fs::read_to_string(path).unwrap();
    let needle = "\nupdated: ";
    let start = raw.find(needle).unwrap() + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, date);
    write(path, &raw);
}

/// Back-date a node's `created` field without changing when it was last touched.
pub(super) fn set_created(path: &Path, date: &str) {
    let raw = std::fs::read_to_string(path).unwrap();
    let needle = "\ncreated: ";
    let start = raw.find(needle).unwrap() + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, date);
    write(path, &raw);
}

/// Back-date one capture's timestamp in place, found by its own text rather
/// than by position, since `Corpus::seed` leaves earlier settled captures in
/// the same monthly inbox file ahead of whichever one a test cares about.
/// The whole stamp is replaced, whichever form either one is in.
pub(super) fn set_inbox_stamp_for(root: &Path, capture_text: &str, stamp: &str) {
    let inbox_dir = root.join("inbox");
    for entry in std::fs::read_dir(&inbox_dir).unwrap() {
        let path = entry.unwrap().path();
        let mut raw = std::fs::read_to_string(&path).unwrap();
        let Some(text_at) = raw.find(capture_text) else {
            continue;
        };
        let line_start = raw[..text_at].rfind('\n').map_or(0, |i| i + 1);
        let stamp_start = raw[line_start..].find("] ").unwrap() + line_start + 2;
        let stamp_end = stamp_start + raw[stamp_start..].find(' ').unwrap();
        raw.replace_range(stamp_start..stamp_end, stamp);
        std::fs::write(&path, raw).unwrap();
        return;
    }
    panic!("no inbox entry contains `{capture_text}`");
}

/// Every file under `nodes/` and `inbox/`, by path, for a before/after diff.
pub(super) fn snapshot_corpus_files(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    let mut map = std::collections::BTreeMap::new();
    for sub in ["nodes", "inbox"] {
        let dir = root.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let content = std::fs::read_to_string(&path).unwrap();
            map.insert(path, content);
        }
    }
    map
}

/// Run git in `dir`, asserting it succeeded; stdout as text.
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let out = output(git_command(dir, support::home()).args(args));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository at `dir` with a local identity, so `neb`'s commits do not
/// depend on the developer's global git configuration.
pub(super) fn git_init(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "neb-test"]);
    git(dir, &["config", "user.email", "neb-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Commit the current index at a fixed date, independent of machine config
/// and wall-clock time, so date-based history assertions are deterministic.
pub(super) fn git_commit_at(dir: &Path, date: &str, message: &str) {
    git_commit_stamped(dir, &format!("{date}T12:00:00Z"), message);
}

/// `git status --porcelain`, ignoring the lock left by legacy-corpus fixtures
/// that deliberately exercise behavior without re-running `neb init`.
pub(super) fn dirt(dir: &Path) -> String {
    git(dir, &["status", "--porcelain"])
        .lines()
        .filter(|line| line.trim_end() != "?? .lock")
        .fold(String::new(), |mut out, line| {
            out.push_str(line);
            out.push('\n');
            out
        })
}

/// Commit messages, newest first.
pub(super) fn log(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--format=%s"])
        .lines()
        .map(String::from)
        .collect()
}

/// The paths the newest commit touched, relative to the repository's top
/// level, sorted.
pub(super) fn head_paths(dir: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git(dir, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    paths.sort();
    paths
}

pub(super) fn assert_only_corpus_paths_in_log(dir: &Path) {
    for line in git(dir, &["log", "--name-only", "--format="])
        .lines()
        .filter(|line| !line.is_empty())
    {
        assert!(
            matches!(line, ".gitignore" | "config.yaml")
                || line.starts_with("nodes/")
                || line.starts_with("inbox/"),
            "a commit touched {line}"
        );
    }
}

/// The recommended setup: a repository at the corpus root, with a remote it
/// must never push to.
pub(super) fn corpus_repo() -> (Corpus, PathBuf) {
    let c = Corpus::new();
    git_init(&c.root);
    let remote = c.workdir().join("remote.git");
    git(c.workdir(), &["init", "-q", "--bare", "remote.git"]);
    git(
        &c.root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    (c, remote)
}

/// Make the repository at `repo` one git cannot read, as a bad disk or a
/// half-finished edit of `.git` does: git no longer recognises it at all.
pub(super) fn break_head(repo: &Path) {
    write(&repo.join(".git").join("HEAD"), "garbage\n");
}
