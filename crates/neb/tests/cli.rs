#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests over a real corpus in a temporary directory.
//!
//! These drive the built binary rather than library functions, because the
//! things most likely to break are the transition guards and the exit codes,
//! and both live at the edge.
//!
//! Paths are used exactly as the temporary directory reports them and are never
//! canonicalized or compared against a resolved form. On macOS the temporary
//! directory sits under a symlink, and a checker that resolved paths would pass
//! on Linux and fail here.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures plant and corrupt corpus files directly; only the binary under test goes \
              through nebula_core's durable write helper"
)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

// The isolation every test here runs under, this process's and each child's.
#[path = "../../nebula-core/tests/support/mod.rs"]
mod support;

use support::ChildGuard;

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("neb")
}

// ---------------------------------------------- the child-command builder --

/// A `neb` isolated from the host with `home` as its home: the only place a
/// test here creates one. See [`support::isolate`] for what it clears and
/// sets; a test that needs a variable back sets it after this.
fn neb_command(home: &Path) -> Command {
    let mut cmd = Command::new(bin());
    support::isolate(&mut cmd, home);
    cmd
}

/// `git -C dir`, isolated with `home` as its home.
fn git_command(dir: &Path, home: &Path) -> Command {
    support::git_command(dir, home)
}

/// Run `cmd` to completion under a guard and the deadline, failing the test
/// if it outlives it.
fn output(cmd: &mut Command) -> Output {
    support::output(cmd, support::DEADLINE).unwrap_or_else(|e| panic!("{e}"))
}

/// [`output`] as a [`Run`] reported as `neb {args}`.
fn run_command(cmd: &mut Command, args: String) -> Run {
    Run {
        args,
        out: output(cmd),
    }
}

struct Corpus {
    dir: tempfile::TempDir,
    root: PathBuf,
}

impl Corpus {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("corpus");
        let me = Self { dir, root };
        me.run(&["init"]).assert_ok();
        me
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_with_env(args, &[])
    }

    fn run_with_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        let mut cmd = self.command(args);
        for (key, value) in extra {
            cmd.env(key, value);
        }
        run_command(&mut cmd, args.join(" "))
    }

    fn run_with_stdin(&self, args: &[&str], input: &str) -> Run {
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
    fn run_closed_stdout(&self, args: &[&str], extra: &[(&str, &str)], input: &str) -> Run {
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
    fn spawn(&self, args: &[&str]) -> Spawned {
        self.spawn_with_env(args, &[])
    }

    /// [`Corpus::spawn`] with extra variables set, and stdin closed.
    fn spawn_with_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Spawned {
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
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = neb_command(self.workdir());
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1");
        cmd
    }

    fn workdir(&self) -> &Path {
        self.dir.path()
    }

    fn node_file(&self, id: &str) -> PathBuf {
        self.root.join("nodes").join(format!("{id}.md"))
    }

    /// Create a seed by capturing then promoting, the normal path.
    fn seed(&self, text: &str, title: &str) -> String {
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
fn run_from_home(
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
fn run_in(
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
fn closed_stdout(cmd: &mut Command, input: &str) -> Output {
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
struct Spawned {
    args: String,
    child: ChildGuard,
}

impl Spawned {
    /// Wait for it to finish, failing the test past the deadline.
    fn wait(self) -> Run {
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

struct Run {
    args: String,
    out: Output,
}

impl Run {
    fn assert_ok(self) -> Self {
        assert!(
            self.out.status.success(),
            "`neb {}` failed:\n{}\n{}",
            self.args,
            String::from_utf8_lossy(&self.out.stdout),
            String::from_utf8_lossy(&self.out.stderr)
        );
        self
    }
    fn assert_fails(self) -> Self {
        assert!(
            !self.out.status.success(),
            "`neb {}` unexpectedly succeeded",
            self.args
        );
        self
    }
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.out.stdout).to_string()
    }
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.out.stderr).to_string()
    }
    fn stdout_trim(&self) -> String {
        self.stdout()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    }
    fn says(self, needle: &str) -> Self {
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
    fn refusal(&self) -> serde_json::Value {
        self.refusal_exiting(1)
    }
    /// [`Run::refusal`] for a usage error, which exits 2 (STD-01 §R20).
    fn usage_refusal(&self) -> serde_json::Value {
        self.refusal_exiting(2)
    }
    fn refusal_exiting(&self, exit: i32) -> serde_json::Value {
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

fn write(path: &Path, s: &str) {
    std::fs::write(path, s).expect("writing fixture");
}

#[cfg(unix)]
fn editor_script(c: &Corpus, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = c.workdir().join(name);
    write(&path, script);
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

/// `YYYY-MM-DD` for `days` ago, computed the same way `store::days_since`
/// computes "now", so fixtures land unambiguously on one side of a threshold
/// regardless of what day the suite actually runs.
fn date_days_ago(days: i64) -> String {
    use time::{OffsetDateTime, macros::format_description};
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = now.date() - time::Duration::days(days);
    date.format(format_description!("[year]-[month]-[day]"))
        .expect("formatting a date")
}

/// A stamp in the legacy `YYYY-MM-DDTHH:MM` form, so the tests that back-date
/// captures keep covering legacy stamps. Only the date feeds the fourteen-day
/// rule.
fn stamp_days_ago(days: i64) -> String {
    format!("{}T00:00", date_days_ago(days))
}

/// Back-date a node's `updated` field in place, to land it on a chosen side
/// of a staleness threshold without waiting for real time to pass.
fn set_updated(path: &Path, date: &str) {
    let raw = std::fs::read_to_string(path).unwrap();
    let needle = "\nupdated: ";
    let start = raw.find(needle).unwrap() + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, date);
    write(path, &raw);
}

/// Back-date a node's `created` field without changing when it was last touched.
fn set_created(path: &Path, date: &str) {
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
fn set_inbox_stamp_for(root: &Path, capture_text: &str, stamp: &str) {
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

// -------------------------------------------------------------- containment --

/// Whether process `pid` still exists, reaped or not: a zombie still has its
/// `/proc` entry and still answers `kill -0`.
fn process_exists(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new(&format!("/proc/{pid}")).exists()
    } else {
        output(support::command("kill", support::home()).args(["-0", &pid.to_string()]))
            .status
            .success()
    }
}

/// Every name the builder must not let a child inherit, spelled out here
/// rather than read from the builder, plus whatever the installed git says
/// points it at one repository.
#[test]
fn the_child_command_builder_isolates_home_and_git_env() {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};

    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path();
    let local = output(git_command(home, support::home()).args(["rev-parse", "--local-env-vars"]));
    assert!(local.status.success(), "{local:?}");
    let local = String::from_utf8(local.stdout).unwrap();
    assert!(local.lines().any(|name| name == "GIT_DIR"), "{local}");
    let removed = [
        "NEBULA_ROOT",
        "OBSERVATORY_ROOT",
        "VISUAL",
        "EDITOR",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "ORBIT_RUN_ID",
        "ORBIT_TASK_ID",
        "NEBULA_READ_ONLY",
        "NO_COLOR",
        "CLICOLOR_FORCE",
        "TERM",
        "COLUMNS",
    ]
    .into_iter()
    .chain(local.lines());

    for cmd in [neb_command(home), git_command(home, home)] {
        let what = support::describe(&cmd);
        let envs: HashMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
        let set = |name: &str| envs.get(OsStr::new(name)).copied().flatten();
        for name in removed.clone() {
            assert_eq!(
                envs.get(OsStr::new(name)),
                Some(&None),
                "`{what}` would inherit {name}"
            );
        }
        assert_eq!(set("HOME"), Some(home.as_os_str()), "{what}");
        assert_eq!(set("GIT_CONFIG_NOSYSTEM"), Some(OsStr::new("1")), "{what}");
        let global = PathBuf::from(set("GIT_CONFIG_GLOBAL").expect("GIT_CONFIG_GLOBAL"));
        let global = std::fs::read_to_string(&global).expect("the test gitconfig exists");
        assert!(
            global.contains("email = neb-test@example.invalid"),
            "{global}"
        );
        assert!(global.contains("gpgsign = false"), "{global}");
        let ceilings: Vec<PathBuf> =
            std::env::split_paths(set("GIT_CEILING_DIRECTORIES").expect("ceilings")).collect();
        assert!(
            ceilings
                .iter()
                .any(|dir| Some(dir.as_path()) == home.parent()),
            "`{what}` may climb above its fixture: {ceilings:?}"
        );
    }

    // The same isolation holds in this process, which is what the git that
    // production code starts in-process inherits.
    assert_eq!(
        std::env::var_os("HOME"),
        Some(OsString::from(support::home()))
    );
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "NEBULA_ROOT",
        "OBSERVATORY_ROOT",
    ] {
        assert_eq!(std::env::var_os(name), None, "{name}");
    }
}

/// The check that every child comes from the builder reads this file, and
/// it catches a command created anywhere else.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    let strays = support::commands_outside(include_str!("cli.rs"), &["neb_command"]);
    assert!(
        strays.is_empty(),
        "crates/neb/tests/cli.rs creates a child outside `neb_command`/`git_command`:\n{}",
        strays.join("\n")
    );

    let stray = format!(
        "fn neb_command() {{\n    {}bin());\n}}\n\n#[test]\nfn a_test() {{\n    let git = std::process::{}\"git\");\n}}\n",
        concat!("Command", "::new("),
        concat!("Command", "::new("),
    );
    assert_eq!(
        support::commands_outside(&stray, &["neb_command"]),
        [format!(
            "7: in `a_test`: let git = std::process::{}\"git\");",
            concat!("Command", "::new(")
        )]
    );
}

/// A failed assertion between spawn and wait drops the guard, and that must
/// not leave the child running (STD-03 §R18).
#[test]
fn a_guarded_child_is_killed_and_reaped_when_dropped() {
    let c = Corpus::new();
    let mut cmd = c.command(&["capture", "-"]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = ChildGuard::spawn(&mut cmd).unwrap();
    // Held open, so `capture -` blocks reading it.
    let stdin = child.take_stdin();
    let pid = child.id();
    assert!(process_exists(pid), "the child is running");

    drop(child);
    assert!(!process_exists(pid), "process {pid} outlived its guard");
    drop(stdin);
}

/// A child that never finishes fails the wait at its deadline, naming the
/// command, rather than hanging the suite (STD-03 §R17).
#[test]
fn a_guarded_wait_fails_at_its_deadline_instead_of_hanging() {
    use std::time::{Duration, Instant};

    let c = Corpus::new();
    let mut cmd = c.command(&["capture", "-"]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(&mut cmd).unwrap();
    let _stdin = child.take_stdin();
    let pid = child.id();

    let deadline = Duration::from_millis(500);
    let started = Instant::now();
    let err = child.wait_with_output(deadline).unwrap_err();
    let waited = started.elapsed();
    assert!(
        waited < deadline + Duration::from_secs(2),
        "waited {waited:?} for a {deadline:?} deadline"
    );
    assert!(err.contains("neb --root"), "{err}");
    assert!(err.contains("capture -"), "{err}");
    assert!(err.contains("still running"), "{err}");
    assert!(!process_exists(pid), "process {pid} was not reaped");
}

// ------------------------------------------------------------------ capture --

#[test]
fn capture_then_promote_leaves_the_original_text_in_the_node() {
    let c = Corpus::new();
    let entry = c
        .run(&[
            "capture",
            "ranking decay looks like a half-life, not a cliff",
        ])
        .assert_ok();
    let id = entry.stdout_trim();
    c.run(&["inbox"]).assert_ok().says(&id).says("half-life");

    let node = c
        .run(&["promote", &id, "--title", "Ranking decay half-life"])
        .assert_ok();
    let node_id = node.stdout_trim();
    assert_eq!(node_id, "ranking-decay-half-life");

    let body = std::fs::read_to_string(c.node_file(&node_id)).unwrap();
    assert!(
        body.contains("half-life, not a cliff"),
        "capture text should survive promotion"
    );
}

/// The lines of the one inbox month file a fresh corpus has captured into.
fn inbox_lines(c: &Corpus) -> Vec<String> {
    let files: Vec<PathBuf> = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let [month] = files.as_slice() else {
        panic!("expected one inbox month file, found {files:?}");
    };
    std::fs::read_to_string(month)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn capture_dash_reads_stdin_and_joins_its_lines() {
    let c = Corpus::new();

    let id = c
        .run_with_stdin(&["capture", "-", "--quiet"], "a\nb\n")
        .assert_ok()
        .stdout_trim();

    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 1, "one line per entry: {lines:?}");
    assert!(lines[0].starts_with(&format!("- [{id}] ")), "{lines:?}");
    assert!(lines[0].ends_with(" a b"), "{lines:?}");

    let captured = c
        .run_with_stdin(&["capture", "-", "--json"], "one\r\n\r\n  two\r\n")
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(captured["entry"]["text"], "one two");
    assert_eq!(inbox_lines(&c).len(), 2);
}

/// `capture -` reads at most 64 KiB, and one byte more is refused before the
/// corpus is opened for writing: the lock held here would otherwise turn it
/// into a five-second wait and a `locked` refusal, and nothing is written.
#[test]
fn oversized_stdin_capture_is_refused_before_locking() {
    let c = Corpus::new();
    c.run(&["capture", "-q", "already waiting"]).assert_ok();
    let before = inbox_lines(&c);
    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    let over = "a".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT + 1);
    let run = c.run_with_stdin(&["--json", "capture", "-"], &over);
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "input_too_large", "{refusal}");
    let error = refusal["error"].as_str().unwrap();
    assert!(error.contains("65536 bytes"), "names the limit: {error}");
    assert!(
        refusal["hint"].as_str().unwrap().contains("64 KiB"),
        "{refusal}"
    );

    drop(held);
    assert_eq!(inbox_lines(&c), before, "nothing was captured");
}

/// `--body -` reads at most 1 MiB, refused the same way before any lock.
#[test]
fn oversized_stdin_body_is_refused_before_locking() {
    let c = Corpus::new();
    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    let over = "b".repeat(nebula_core::ops::BODY_INPUT_LIMIT + 1);
    let run = c.run_with_stdin(&["--json", "new", "Too long", "--body", "-"], &over);
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "input_too_large", "{refusal}");
    assert!(
        refusal["error"].as_str().unwrap().contains("1048576 bytes"),
        "{refusal}"
    );

    drop(held);
    assert!(!c.node_file("too-long").exists(), "no node was written");
}

/// Exactly the limit is not over it: taken whole, for a capture and a body.
#[test]
fn stdin_at_the_limit_is_accepted() {
    let c = Corpus::new();
    let thought = "a".repeat(nebula_core::ops::CAPTURE_INPUT_LIMIT);
    let id = c
        .run_with_stdin(&["capture", "-q", "-"], &thought)
        .assert_ok()
        .stdout_trim();
    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 1, "{} lines", lines.len());
    assert!(lines[0].starts_with(&format!("- [{id}] ")));
    assert!(
        lines[0].ends_with(&format!(" {thought}")),
        "the capture is whole"
    );

    let body = "b".repeat(nebula_core::ops::BODY_INPUT_LIMIT);
    c.run_with_stdin(&["new", "At the limit", "--body", "-"], &body)
        .assert_ok();
    let node = std::fs::read_to_string(c.node_file("at-the-limit")).unwrap();
    assert!(node.ends_with(&format!("{body}\n")), "the body is whole");
}

#[test]
fn multiline_capture_text_is_joined_onto_one_line() {
    let c = Corpus::new();

    let captured = c
        .run(&["capture", "first line\nsecond line", "--quiet", "--json"])
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(captured["entry"]["text"], "first line second line");

    // A lone `-` is stdin; a dash among other words is part of the thought.
    c.run(&["capture", "--quiet", "--", "left", "-", "right"])
        .assert_ok();

    let lines = inbox_lines(&c);
    assert_eq!(lines.len(), 2, "one line per entry: {lines:?}");
    assert!(lines[0].ends_with(" first line second line"), "{lines:?}");
    assert!(lines[1].ends_with(" left - right"), "{lines:?}");
    c.run(&["inbox"])
        .assert_ok()
        .says("first line second line")
        .says("left - right");
}

#[test]
fn empty_capture_is_refused_without_writing() {
    let c = Corpus::new();

    for input in ["", "\n", " \r\n\t\n "] {
        c.run_with_stdin(&["capture", "-"], input)
            .assert_fails()
            .says("nothing to capture");
    }
    c.run(&["capture", " \n \r\n "])
        .assert_fails()
        .says("nothing to capture");

    assert_eq!(
        std::fs::read_dir(c.root.join("inbox")).unwrap().count(),
        0,
        "a refused capture must not create an inbox record"
    );
    c.run(&["inbox", "--json"]).assert_ok().says("[]");
}

/// Capture creates a missing corpus, but not for a thought that is not there:
/// empty standard input is refused before anything is written.
#[test]
fn empty_stdin_capture_creates_no_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = Corpus {
        root: dir.path().join("fresh"),
        dir,
    };

    fresh
        .run_with_stdin(&["capture", "-"], "\n\n")
        .assert_fails()
        .says("nothing to capture");

    assert!(
        !fresh.root.exists(),
        "a refused capture must not init a corpus"
    );
}

/// Blank text to a root that does not exist yet is refused with core's code
/// for it, and the root is still not there: the rule runs before a corpus
/// can be created for it (STD-02 §R24, §R34).
#[test]
fn blank_capture_to_a_missing_root_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let missing = dir.path().join("not-yet");

    let refused =
        run_from_home(&home, None, &["--json", "capture", "  "], Some(&missing)).usage_refusal();
    assert_eq!(refused["code"], "empty_capture", "{refused}");
    assert!(
        !missing.exists(),
        "a refused capture must not init a corpus"
    );
}

/// Three rules the CLI used to spell itself, as `usage`, now reach it from
/// core with core's codes, still exiting 2 as usage errors: the codes
/// `blank_capture_note_and_open_status_reason_are_typed_refusals` in
/// `core.rs` asserts for the same calls.
#[test]
fn blank_capture_note_and_open_status_reason_refuse_with_core_codes() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for (args, code) in [
        (vec!["--json", "capture", "   "], "empty_capture"),
        (vec!["--json", "note", &id, "  "], "empty_note"),
        (
            vec!["--json", "status", &id, "seed", "--why", "y"],
            "reason_on_open_status",
        ),
    ] {
        let refused = c.run(&args).usage_refusal();
        assert_eq!(refused["code"], code, "{args:?}: {refused}");
    }
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

/// A month file the inbox cannot read is named, under `--json` and in prose.
#[cfg(unix)]
#[test]
fn unreadable_inbox_file_error_names_the_file() {
    use std::os::unix::fs::PermissionsExt;

    let c = Corpus::new();
    c.run(&["capture", "a thought"]).assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "md"))
        .expect("a month file");
    std::fs::set_permissions(&month, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&month).is_ok() {
        // Running as root, where a mode refuses nothing.
        return;
    }
    let shown = month.display().to_string();

    let refused = c.run(&["--json", "inbox"]).refusal();
    assert_eq!(refused["code"], "io_at", "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains(&shown),
        "{refused}"
    );
    let prose = c.run(&["inbox"]).assert_fails().says(&shown);
    assert_eq!(prose.out.status.code(), Some(1));
}

/// Whether `at` is RFC 3339, which says its offset by construction.
fn is_rfc3339(at: &serde_json::Value) -> bool {
    at.as_str().is_some_and(|at| {
        time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).is_ok()
    })
}

/// A POSIX zone two hours ahead of UTC, with no daylight saving, so a stamp's
/// offset is known whatever zone the suite runs in.
const PLUS_TWO: (&str, &str) = ("TZ", "XYZ-2");

#[test]
fn inbox_at_is_rfc3339_with_offset() {
    let c = Corpus::new();

    let captured = c
        .run_with_env(&["capture", "x", "--json"], &[PLUS_TWO])
        .assert_ok()
        .stdout();
    let captured: serde_json::Value = serde_json::from_str(&captured).unwrap();
    let at = &captured["entry"]["at"];
    assert!(is_rfc3339(at), "{captured}");
    assert!(at.as_str().unwrap().ends_with("+02:00"), "{captured}");
    c.run(&["capture", "y", "--quiet"]).assert_ok();

    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let listing: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 2, "{listing}");
    assert!(entries.iter().all(|e| is_rfc3339(&e["at"])), "{listing}");
    assert_eq!(entries[0]["at"], *at, "a stamp reads back as written");

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

/// The ids `neb` tries, in order, for `text` captured at `stamp`: a copy of
/// the hash chain in `store::unique_entry_id`, which the binary cannot be
/// asked for.
fn inbox_id_candidates(stamp: &str, text: &str) -> Vec<String> {
    fn fnv(s: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    let mut h = fnv(&format!("{stamp}{text}"));
    let mut ids = Vec::new();
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if !ids.contains(&id) {
            ids.push(id);
        }
        h = fnv(&format!("{h}"));
    }
    ids
}

/// The inbox stamp for every second from `stamp` to `seconds` after it, at
/// `stamp`'s own offset and in its own form, as `neb` would write them.
fn stamps_from(stamp: &str, seconds: i64) -> Vec<String> {
    use time::format_description::well_known::Rfc3339;
    use time::macros::format_description;
    use time::{Duration, OffsetDateTime, UtcOffset};

    let start = OffsetDateTime::parse(stamp, &Rfc3339).unwrap();
    (0..=seconds)
        .map(|s| {
            let at = start + Duration::seconds(s);
            if stamp.ends_with('Z') {
                at.to_offset(UtcOffset::UTC).format(format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
                ))
            } else {
                at.format(format_description!(
                    "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
                ))
            }
            .unwrap()
        })
        .collect()
}

#[test]
fn capture_collision_fallback_promotes_the_new_thought() {
    const TEXT: &str = "the intended new thought";
    // The binary's clock cannot be set, and its stamp, which seeds the id, is
    // to the second. So rather than hope the capture lands in the probe's
    // second, the fixture holds every id it could hash to from that second to
    // a minute after it, and the capture collides whichever it lands in.
    const WINDOW_SECS: i64 = 60;

    let c = Corpus::new();
    // A zone without daylight saving, so no offset change falls inside the window.
    c.run_with_env(&["capture", TEXT], &[PLUS_TWO]).assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let raw = std::fs::read_to_string(&month).unwrap();
    let probe = raw.split_whitespace().nth(2).unwrap().to_string();
    let window = stamps_from(&probe, WINDOW_SECS);
    let mut occupied = std::collections::HashSet::new();
    let mut fixture = String::new();
    for stamp in &window {
        for id in inbox_id_candidates(stamp, TEXT) {
            if occupied.insert(id.clone()) {
                writeln!(fixture, "- [{id}] {stamp} fixture {}", occupied.len()).unwrap();
            }
        }
    }
    std::fs::write(&month, fixture).unwrap();

    let captured = c
        .run_with_env(&["capture", TEXT], &[PLUS_TWO])
        .assert_ok()
        .stdout_trim();

    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let entries: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let new_entry = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == captured)
        .unwrap();
    let at = new_entry["at"].as_str().unwrap();
    assert!(
        window.iter().any(|stamp| stamp == at),
        "the capture at {at} fell outside the {WINDOW_SECS}s after {probe} the fixture holds"
    );
    assert!(!occupied.contains(&captured), "{captured} is already taken");
    c.run(&["promote", &captured, "--title", "Intended new thought"])
        .assert_ok();
    let node = std::fs::read_to_string(c.node_file("intended-new-thought")).unwrap();
    assert!(node.ends_with("the intended new thought\n"));
}

#[test]
fn capture_reports_id_exhaustion_without_appending() {
    let c = Corpus::new();
    c.run(&["capture", "locate the current inbox file"])
        .assert_ok();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut fixture = String::new();
    for id in 0..=u16::MAX {
        writeln!(fixture, "- [{id:04x}] 2000-01-01T00:00 occupied").unwrap();
    }
    std::fs::write(&month, &fixture).unwrap();

    c.run(&["capture", "there is no id left"])
        .assert_fails()
        .says("inbox id namespace exhausted; nothing captured");
    assert_eq!(std::fs::read_to_string(month).unwrap(), fixture);
}

#[test]
fn capture_after_an_unterminated_record_round_trips_through_promotion() {
    let c = Corpus::new();
    let first = c.run(&["capture", "the first thought"]).stdout_trim();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let unterminated = std::fs::read_to_string(&month)
        .unwrap()
        .trim_end_matches('\n')
        .to_string();
    std::fs::write(&month, unterminated).unwrap();

    let second = c.run(&["capture", "the second thought"]).stdout_trim();
    let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
    let entries: serde_json::Value = serde_json::from_str(&listing).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 2);
    assert!(listing.contains(&first));
    assert!(listing.contains(&second));

    let node = c
        .run(&["promote", &second, "--title", "Second thought"])
        .assert_ok()
        .stdout_trim();
    let body = std::fs::read_to_string(c.node_file(&node)).unwrap();
    assert!(body.ends_with("the second thought\n"));
    c.run(&["inbox", "--json"])
        .assert_ok()
        .says(&first)
        .says("the first thought");
}

#[test]
fn promoted_and_dropped_entries_leave_the_inbox_but_stay_on_disk() {
    let c = Corpus::new();
    let keep = c.run(&["capture", "worth keeping"]).stdout_trim();
    let toss = c.run(&["capture", "not worth keeping"]).stdout_trim();

    c.run(&["promote", &keep, "--title", "Worth keeping"])
        .assert_ok();
    c.run(&["drop", &toss]).assert_ok();

    let listing = c.run(&["inbox"]).assert_ok().stdout();
    assert!(
        !listing.contains(&keep) && !listing.contains(&toss),
        "both should be settled"
    );

    // Nothing is deleted: the record of what was captured, and what became of
    // it, is the part that stops you re-treading ground.
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let raw = std::fs::read_to_string(month.path()).unwrap();
    assert!(
        raw.contains("not worth keeping"),
        "dropped text must remain on disk"
    );
    assert!(raw.contains("dropped"), "drop should be recorded");
    assert!(
        raw.contains("worth-keeping"),
        "promotion should name the node it became"
    );
}

/// Capture never refuses for content, so a thought typed twice lands twice;
/// stderr says which waiting entry it repeats, and stdout is untouched.
#[test]
fn a_repeated_capture_names_the_entry_still_waiting_on_stderr() {
    let c = Corpus::new();
    let first = c.run(&["capture", "Tags beat domains"]).assert_ok();
    assert_eq!(first.stderr(), "", "a new thought is not news");
    let first = first.stdout_trim();
    let notice = format!("note: same as {first}, still waiting\n");

    let quiet = c
        .run(&["capture", "--quiet", "tags  BEAT domains"])
        .assert_ok();
    assert_eq!(quiet.stderr(), notice);
    let second = quiet.stdout_trim();
    assert_eq!(
        quiet.stdout(),
        format!("{second}\n"),
        "stdout is the id alone"
    );
    assert_ne!(second, first);

    let text = c.run(&["capture", "TAGS beat domains"]).assert_ok();
    assert_eq!(text.stderr(), notice);
    assert!(
        !text.stdout().contains("same as"),
        "the notice stays off stdout"
    );

    let keys = |run: &Run| {
        let value: serde_json::Value = serde_json::from_str(&run.stdout()).expect("stdout is JSON");
        let mut keys: Vec<String> = value["entry"]
            .as_object()
            .expect("an entry")
            .keys()
            .chain(value.as_object().expect("an object").keys())
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let repeated = c
        .run(&["capture", "--json", " tags beat\tdomains "])
        .assert_ok();
    assert_eq!(repeated.stderr(), notice);
    let fresh = c
        .run(&["capture", "--json", "an unrelated thought"])
        .assert_ok();
    assert_eq!(fresh.stderr(), "");
    assert_eq!(keys(&repeated), keys(&fresh), "the payload is unchanged");

    let inbox: serde_json::Value =
        serde_json::from_str(&c.run(&["inbox", "--json"]).assert_ok().stdout()).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 5, "every capture landed");

    // A settled entry is not waiting, so it is never the one named.
    c.run(&["drop", &first]).assert_ok();
    let after_drop = c.run(&["capture", "tags beat domains"]).assert_ok();
    assert_eq!(
        after_drop.stderr(),
        format!("note: same as {second}, still waiting\n")
    );
}

#[test]
fn promote_and_drop_on_a_settled_entry_say_how_it_was_settled() {
    let c = Corpus::new();
    let keep = c.run(&["capture", "worth keeping"]).stdout_trim();
    let toss = c.run(&["capture", "not worth keeping"]).stdout_trim();
    c.run(&["promote", &keep, "--title", "Worth keeping"])
        .assert_ok();
    c.run(&["drop", &toss]).assert_ok();

    for verb in ["promote", "drop"] {
        c.run(&[verb, &keep])
            .assert_fails()
            .says(&format!("`{keep}` was already promoted to `worth-keeping`"))
            .says("neb show worth-keeping");
        c.run(&[verb, &toss])
            .assert_fails()
            .says(&format!("`{toss}` was already dropped"))
            .says("neb inbox");
        let json = c.run(&[verb, &keep, "--json"]);
        assert_eq!(json.stdout(), "", "a refusal prints no payload");
        assert_eq!(
            json.refusal(),
            serde_json::json!({
                "error": format!("`{keep}` was already promoted to `worth-keeping`"),
                "code": "inbox_entry_settled",
                "hint": "See the node with:  neb show worth-keeping",
            })
        );
        assert_eq!(
            c.run(&[verb, &toss, "--json"]).refusal(),
            serde_json::json!({
                "error": format!("`{toss}` was already dropped"),
                "code": "inbox_entry_settled",
                "hint": "See what is still waiting with:  neb inbox",
            })
        );
        c.run(&[verb, "zzzz"])
            .assert_fails()
            .says("no open inbox entry `zzzz`");
    }
    assert!(
        !c.node_file("not-worth-keeping").exists(),
        "a refused promote writes no node"
    );
}

#[test]
fn capture_works_before_a_corpus_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");
    let out = output(
        neb_command(dir.path())
            .arg("--root")
            .arg(&root)
            .args(["capture", "the thought arrives before the setup"]),
    );
    assert!(
        out.status.success(),
        "capture must never be blocked by missing setup"
    );
    assert!(root.join("inbox").is_dir());
}

const CREATED_NOTICE: &str = "note: created a new corpus at ";

/// A mistyped `--root` or `$NEBULA_ROOT` must not split the corpus silently.
/// Capture still creates rather than refuses; it says where, on stderr, once.
#[test]
fn capture_names_the_corpus_it_creates_on_stderr_only_the_first_time() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let flagged = dir.path().join("typo-flag");
    let environment = dir.path().join("typo-env");

    for (root, nebula_root) in [
        (Some(flagged.as_path()), None),
        (None, Some(environment.as_path())),
    ] {
        let target = root.or(nebula_root).unwrap();
        let first = run_from_home(&home, root, &["capture", "x"], nebula_root).assert_ok();
        assert_eq!(
            first.stderr(),
            format!("{CREATED_NOTICE}{}\n", target.display()),
            "exactly one line naming the new corpus"
        );
        let id = first.stdout_trim();
        assert_eq!(
            first.stdout(),
            format!("{id}\n"),
            "stdout is the entry id alone"
        );
        assert!(target.join("inbox").is_dir());

        let again = run_from_home(&home, root, &["capture", "y"], nebula_root).assert_ok();
        assert_eq!(again.stderr(), "", "an existing corpus is not news");
    }
}

#[test]
fn capture_json_names_the_corpus_it_creates_on_stderr_and_keeps_the_payload() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("typo");

    let first = run_from_home(&home, Some(&root), &["capture", "--json", "x"], None).assert_ok();
    assert_eq!(
        first.stderr(),
        format!("{CREATED_NOTICE}{}\n", root.display())
    );
    let again = run_from_home(&home, Some(&root), &["capture", "--json", "y"], None).assert_ok();
    assert_eq!(again.stderr(), "");

    let keys = |run: &Run| {
        let value: serde_json::Value = serde_json::from_str(&run.stdout()).expect("stdout is JSON");
        let mut keys: Vec<String> = value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(
        keys(&first),
        keys(&again),
        "creating the corpus does not change the payload"
    );
}

/// A relative root is shown absolute, so the notice says where it landed.
#[test]
fn capture_shows_a_relative_root_it_creates_as_an_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let out = output(
        neb_command(&dir.path().join("home"))
            .current_dir(dir.path())
            .args(["--root", "typo", "capture", "x"])
            .env("NO_COLOR", "1"),
    );
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    let shown = stderr
        .strip_suffix('\n')
        .and_then(|line| line.strip_prefix(CREATED_NOTICE))
        .unwrap_or_else(|| panic!("expected one notice line, got:\n{stderr}"));
    // Compared by what is there, not by spelling: the child's working
    // directory may read back resolved on macOS, where temp sits under a
    // symlink.
    let shown = Path::new(shown);
    assert!(shown.is_absolute(), "{} is not absolute", shown.display());
    assert!(shown.ends_with("typo"));
    assert!(shown.join("inbox").is_dir());
    assert!(dir.path().join("typo").join("inbox").is_dir());
}

#[cfg(unix)]
#[test]
fn capture_that_cannot_create_its_corpus_names_the_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let parent = dir.path().join("read-only");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let root = parent.join("corpus");

    let text = run_from_home(&home, Some(&root), &["capture", "x"], None);
    let json = run_from_home(&home, Some(&root), &["capture", "--json", "x"], None);
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    for run in [text, json] {
        let run = run
            .assert_fails()
            .says(&format!("creating {}", root.display()))
            .says("Permission denied");
        assert!(
            !run.stderr().contains(CREATED_NOTICE),
            "nothing was created, so nothing is announced"
        );
        assert_eq!(run.stdout(), "");
    }
    assert!(!root.exists());
}

#[test]
fn root_discovery_prefers_flag_then_environment_then_config_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");
    let environment = dir.path().join("environment");
    let explicit = dir.path().join("explicit");

    let init = run_from_home(&home, Some(&configured), &["init", "--set-root"], None).assert_ok();
    let config_path = home.join(".config/nebula/root");
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", configured.display())
    );
    assert!(
        init.stdout()
            .contains(&format!("wrote {}", config_path.display()))
    );

    run_from_home(&home, Some(&environment), &["init"], None).assert_ok();
    run_from_home(&home, Some(&explicit), &["init"], None).assert_ok();

    run_from_home(&home, Some(&explicit), &["check"], Some(&environment))
        .assert_ok()
        .says("0 nodes");
    run_from_home(&home, None, &["check"], Some(&environment))
        .assert_ok()
        .says("0 nodes");
    run_from_home(&home, None, &["check"], None)
        .assert_ok()
        .says("0 nodes");

    let default_home = dir.path().join("default-home");
    let default = default_home.join(".nebula");
    run_from_home(&default_home, Some(&default), &["init"], None).assert_ok();
    run_from_home(&default_home, None, &["check"], None)
        .assert_ok()
        .says("0 nodes");
}

/// Seed a corpus at `root` with one capture whose text names it, so a later
/// `inbox` shows which corpus a command resolved to.
fn corpus_marked(home: &Path, root: &Path, mark: &str) {
    run_from_home(home, Some(root), &["capture", "--quiet", mark], None).assert_ok();
}

/// `inbox` from `cwd`, asserting it read the corpus marked `expected` and
/// none of the `others`.
fn inbox_reads(run: Run, expected: &str, others: &[&str]) {
    let run = run.assert_ok().says(expected);
    for other in others.iter().filter(|other| **other != expected) {
        assert!(
            !run.stdout().contains(other),
            "`neb {}` read `{other}` instead of `{expected}`:\n{}",
            run.args,
            run.stdout()
        );
    }
}

/// Standing inside a corpus, or anywhere under it, is a choice of corpus.
/// Capture writes there and announces nothing, because nothing was created.
#[test]
fn a_corpus_is_found_from_its_root_and_every_directory_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("corpus");
    run_from_home(&home, Some(&root), &["init"], None).assert_ok();
    let deep = root.join("notes").join("deep");
    std::fs::create_dir_all(&deep).unwrap();

    for (cwd, text) in [
        (root.clone(), "from the root"),
        (root.join("nodes"), "from nodes"),
        (deep.clone(), "from a directory of my own"),
    ] {
        let run = run_in(&cwd, &home, None, &["capture", "--quiet", text], None).assert_ok();
        assert_eq!(run.stderr(), "", "an existing corpus is not news");
        run_in(&cwd, &home, None, &["check"], None)
            .assert_ok()
            .says("0 nodes");
    }

    run_from_home(&home, Some(&root), &["inbox"], None)
        .assert_ok()
        .says("from the root")
        .says("from nodes")
        .says("from a directory of my own");
    assert!(
        !home.join(".nebula").exists(),
        "discovery must not fall through to the default corpus"
    );
}

/// `--root` > `$NEBULA_ROOT` > the working directory's corpus >
/// `~/.config/nebula/root` > `~/.nebula`, one step at a time.
#[test]
fn the_working_directory_corpus_sits_between_the_environment_and_the_machine_settings() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let explicit = dir.path().join("explicit");
    let environment = dir.path().join("environment");
    let local = dir.path().join("local");
    let configured = dir.path().join("configured");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    run_from_home(&home, Some(&configured), &["init", "--set-root"], None).assert_ok();
    for (root, mark) in [
        (&explicit, "mark-explicit"),
        (&environment, "mark-environment"),
        (&local, "mark-local"),
        (&configured, "mark-configured"),
    ] {
        corpus_marked(&home, root, mark);
    }
    let marks = [
        "mark-explicit",
        "mark-environment",
        "mark-local",
        "mark-configured",
        "mark-default",
    ];
    let inside = local.join("nodes");

    let flag = run_in(
        &inside,
        &home,
        Some(&explicit),
        &["inbox"],
        Some(&environment),
    );
    inbox_reads(flag, "mark-explicit", &marks);
    let env = run_in(&inside, &home, None, &["inbox"], Some(&environment));
    inbox_reads(env, "mark-environment", &marks);
    let cwd = run_in(&inside, &home, None, &["inbox"], None);
    inbox_reads(cwd, "mark-local", &marks);
    let config = run_in(&outside, &home, None, &["inbox"], None);
    inbox_reads(config, "mark-configured", &marks);

    // With no configured root, the working directory still beats the
    // default, and outside any corpus the default is what is left.
    let bare_home = dir.path().join("bare-home");
    corpus_marked(&bare_home, &bare_home.join(".nebula"), "mark-default");
    let cwd = run_in(&inside, &bare_home, None, &["inbox"], None);
    inbox_reads(cwd, "mark-local", &marks);
    let default = run_in(&outside, &bare_home, None, &["inbox"], None);
    inbox_reads(default, "mark-default", &marks);
}

/// The nearest corpus wins, so a corpus kept inside another is found from
/// inside itself and the outer one from everywhere else in it.
#[test]
fn the_nearest_of_two_nested_corpora_wins() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let outer = dir.path().join("outer");
    let inner = outer.join("projects").join("inner");
    corpus_marked(&home, &outer, "mark-outer");
    corpus_marked(&home, &inner, "mark-inner");
    let marks = ["mark-outer", "mark-inner"];

    let deep = run_in(&inner.join("nodes"), &home, None, &["inbox"], None);
    inbox_reads(deep, "mark-inner", &marks);
    let between = run_in(&outer.join("projects"), &home, None, &["inbox"], None);
    inbox_reads(between, "mark-outer", &marks);
}

/// Discovery only ever finds a corpus that exists, so it cannot steer
/// capture into creating one. A directory with only half the marker, or a
/// `config.yaml` that names no corpus, is walked past untouched, and capture
/// creates the corpus it would have created anyway, saying so.
#[test]
fn capture_never_adopts_a_directory_that_only_looks_like_a_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let lookalike = dir.path().join("lookalike");
    let config_only = dir.path().join("config-only");
    let nodes_only = dir.path().join("nodes-only");
    std::fs::create_dir_all(lookalike.join("nodes")).unwrap();
    write(&lookalike.join("config.yaml"), "name: some other tool\n");
    std::fs::create_dir_all(&config_only).unwrap();
    write(
        &config_only.join("config.yaml"),
        "schema_version: 2\ncorpus_id: neb-000001\n",
    );
    std::fs::create_dir_all(nodes_only.join("nodes")).unwrap();

    let default = home.join(".nebula");
    // Distinct texts, so no capture is reported as a duplicate of another.
    for (i, (cwd, text)) in [
        (lookalike.join("nodes"), "from the lookalike"),
        (config_only.clone(), "from the config-only directory"),
        (nodes_only, "from the nodes-only directory"),
    ]
    .iter()
    .enumerate()
    {
        let run = run_in(cwd, &home, None, &["capture", "--quiet", text], None).assert_ok();
        let expected = if i == 0 {
            format!("{CREATED_NOTICE}{}\n", default.display())
        } else {
            String::new()
        };
        assert_eq!(run.stderr(), expected, "only the default is ever created");
    }
    assert!(default.join("inbox").is_dir(), "the captures landed there");
    assert!(!lookalike.join("inbox").exists() && !config_only.join("inbox").exists());
    assert!(!config_only.join("nodes").exists());
    assert_eq!(
        std::fs::read_to_string(lookalike.join("config.yaml")).unwrap(),
        "name: some other tool\n"
    );
}

/// Paths are used as given. A corpus reached through a symlink resolves to
/// the spelling the shell used, as `$PWD` carries it, and a `$PWD` that no
/// longer names the working directory is ignored rather than trusted.
#[cfg(unix)]
#[test]
fn a_corpus_found_through_a_symlink_keeps_the_spelling_of_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let real = dir.path().join("real");
    let alias = dir.path().join("alias");
    run_from_home(&home, Some(&real.join("corpus")), &["init"], None).assert_ok();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let spelled = alias.join("corpus");

    let run = run_in(
        &spelled.join("nodes"),
        &home,
        None,
        &["init", "--json"],
        None,
    )
    .assert_ok();
    let value: serde_json::Value = serde_json::from_str(&run.stdout()).expect("stdout is JSON");
    assert_eq!(value["root"], spelled.to_str().unwrap());

    // A stale `$PWD`: the OS's own answer stands, which is still this corpus.
    let out = output(
        neb_command(&home)
            .args(["init", "--json"])
            .current_dir(spelled.join("nodes"))
            .env("PWD", dir.path())
            .env("NO_COLOR", "1"),
    );
    let run = Run {
        args: "init --json under a stale PWD".into(),
        out,
    }
    .assert_ok();
    let value: serde_json::Value = serde_json::from_str(&run.stdout()).expect("stdout is JSON");
    let root = PathBuf::from(value["root"].as_str().unwrap());
    assert!(root.ends_with("real/corpus"), "{}", root.display());
}

/// An exported-but-blank `NEBULA_ROOT` must be treated as unset, not as the
/// current directory: every later `join` would otherwise silently target
/// whatever directory the shell happened to be in.
#[test]
fn empty_nebula_root_env_falls_through_to_configured_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");

    run_from_home(&home, Some(&configured), &["init", "--set-root"], None).assert_ok();

    let out = output(
        neb_command(&home)
            .args(["check"])
            .env("NO_COLOR", "1")
            .env("NEBULA_ROOT", ""),
    );
    Run {
        args: "check".into(),
        out,
    }
    .assert_ok()
    .says("0 nodes");

    let default_home = dir.path().join("default-home");
    let default = default_home.join(".nebula");
    run_from_home(&default_home, Some(&default), &["init"], None).assert_ok();

    let out = output(
        neb_command(&default_home)
            .args(["check"])
            .env("NO_COLOR", "1")
            .env("NEBULA_ROOT", ""),
    );
    Run {
        args: "check".into(),
        out,
    }
    .assert_ok()
    .says("0 nodes");
}

/// An empty machine root setting is refused naming the file, so the reader
/// knows which file to fix.
#[test]
fn empty_root_setting_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let setting = home.join(".config/nebula/root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, "  \n").unwrap();

    let refused = run_from_home(&home, None, &["--json", "list"], None).refusal();
    assert_eq!(refused["code"], "empty_root_setting", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains(&setting.display().to_string()),
        "{refused}"
    );
}

/// A `HOME` that is set to bytes that are not UTF-8 is refused as that, not
/// as a `HOME` that is not set; an unset one still says so.
#[cfg(unix)]
#[test]
fn non_utf8_home_is_not_reported_as_unset() {
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let run = |cmd: &mut Command| Run {
        args: "--json list".into(),
        out: output(
            cmd.args(["--json", "list"])
                .current_dir(dir.path())
                .env("PWD", dir.path()),
        ),
    };

    let refused =
        run(neb_command(&home).env("HOME", std::ffi::OsStr::from_bytes(b"\xff"))).refusal();
    assert_ne!(refused["code"], "corpus", "{refused}");
    assert_eq!(refused["code"], "home_not_unicode", "{refused}");
    let said = refused["error"].as_str().unwrap();
    assert!(!said.contains("not set"), "{said}");
    assert!(said.contains("not valid UTF-8"), "{said}");

    let unset = run(neb_command(&home).env_remove("HOME")).refusal();
    assert_eq!(unset["code"], "home_unset", "{unset}");
    assert!(
        unset["error"].as_str().unwrap().contains("HOME is not set"),
        "{unset}"
    );
}

#[test]
fn empty_root_flag_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = output(
        neb_command(dir.path())
            .args(["--root", "", "inbox"])
            .env("NO_COLOR", "1"),
    );
    Run {
        args: "--root \"\" inbox".into(),
        out,
    }
    .assert_fails()
    .says("--root");
}

/// `resolve_root` itself refuses an empty explicit root with a typed error,
/// so a caller that reaches it without going through clap's own parsing
/// (the desktop app, the agent skill) is covered the same way the CLI is.
#[test]
fn resolve_root_refuses_an_empty_explicit_path() {
    let err = nebula_core::Corpus::resolve_root(Some(PathBuf::new()))
        .expect_err("an empty explicit root must be refused");
    assert!(matches!(err, nebula_core::Error::EmptyRoot), "{err}");
    assert_eq!(err.to_string(), "an explicit corpus root cannot be empty");
}

/// A relative `~/.config/nebula/root` would name a different corpus from
/// every working directory, and `capture` would create each one. It is
/// refused at load, by name, before anything is created.
#[test]
fn relative_machine_root_is_refused_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let setting = home.join(".config/nebula/root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, "relcorpus\n").unwrap();

    for cwd in ["w1", "w2"].map(|name| dir.path().join(name)) {
        std::fs::create_dir_all(&cwd).unwrap();
        let refused = run_in(&cwd, &home, None, &["capture", "x"], None);
        assert_eq!(refused.out.status.code(), Some(1), "{}", refused.stderr());
        assert!(
            refused.stderr().contains(&setting.display().to_string())
                && refused.stderr().contains("relcorpus"),
            "the refusal names the setting file and its value:\n{}",
            refused.stderr()
        );
        assert!(
            !cwd.join("relcorpus").exists(),
            "a relative setting must not create a corpus under {}",
            cwd.display()
        );

        let json = run_in(&cwd, &home, None, &["--json", "capture", "x"], None);
        let envelope = json.refusal();
        assert_eq!(envelope["code"], "relative_root_setting", "{envelope}");
        let error = envelope["error"].as_str().unwrap();
        assert!(
            error.contains(&setting.display().to_string()) && error.contains("relcorpus"),
            "{envelope}"
        );
        assert!(!cwd.join("relcorpus").exists());
    }
}

/// `--root` and `init`'s path name the corpus twice, so two different
/// directories are refused before either is created: exit 2, naming both
/// (STD-01 §R28). The same directory given both ways is one corpus.
#[test]
fn init_refuses_conflicting_root_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));

    let run = run_from_home(&home, Some(&a), &["init", b.to_str().unwrap()], None);
    assert_eq!(run.out.status.code(), Some(2), "{}", run.stderr());
    assert_eq!(run.stdout(), "");
    for named in [&a, &b] {
        assert!(
            run.stderr().contains(named.to_str().unwrap()),
            "{}",
            run.stderr()
        );
    }
    assert!(!a.exists() && !b.exists(), "neither directory is created");
    let refused = run_from_home(
        &home,
        Some(&a),
        &["--json", "init", b.to_str().unwrap()],
        None,
    )
    .usage_refusal();
    assert_eq!(refused["code"], "root_and_path_differ");
    assert!(!a.exists() && !b.exists());

    run_from_home(&home, Some(&a), &["init", a.to_str().unwrap()], None)
        .assert_ok()
        .says(&format!("corpus ready at {}", a.display()));
    assert!(a.join("nodes").is_dir());
    assert!(!b.exists());
}

#[test]
fn plain_init_never_changes_the_machine_root_setting() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let scratch = dir.path().join("scratch");
    let config_path = home.join(".config/nebula/root");

    let out = run_from_home(&home, Some(&scratch), &["init"], None).assert_ok();
    assert!(
        !config_path.exists(),
        "plain init must leave an absent root setting absent"
    );
    assert!(
        out.stderr().contains("--set-root")
            && out.stderr().contains(&scratch.display().to_string()),
        "plain init should name the opt-in command on stderr:\n{}",
        out.stderr()
    );
    assert!(!out.stdout().contains("--set-root"), "{}", out.stdout());

    let configured = dir.path().join("configured");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, format!("{}\n", configured.display())).unwrap();
    let other = dir.path().join("other");
    run_from_home(&home, Some(&other), &["init"], None).assert_ok();
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", configured.display()),
        "plain init must leave an existing root setting untouched"
    );
}

#[test]
fn set_root_requires_force_to_replace_a_different_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let config_path = home.join(".config/nebula/root");

    run_from_home(&home, Some(&first), &["init", "--set-root"], None).assert_ok();
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", first.display())
    );

    let refused = run_from_home(&home, Some(&second), &["init", "--set-root"], None).assert_fails();
    assert!(
        refused.stderr().contains("Pass --force to replace it."),
        "expected typed conflict in:\n{}",
        refused.stderr()
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", first.display()),
        "a refused replacement must preserve the setting"
    );
    assert!(
        !second.exists(),
        "a root-setting conflict must be refused before creating the target corpus"
    );

    run_from_home(
        &home,
        Some(&second),
        &["init", "--set-root", "--force"],
        None,
    )
    .assert_ok();
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", second.display())
    );
}

/// The remedy for a relative setting runs even though every other command
/// refuses on it: `--set-root --force` replaces it.
#[test]
fn set_root_force_replaces_a_relative_machine_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let target = dir.path().join("corpus");
    let setting = home.join(".config/nebula/root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    std::fs::write(&setting, "relcorpus\n").unwrap();

    run_from_home(
        &home,
        None,
        &[
            "init",
            &target.display().to_string(),
            "--set-root",
            "--force",
        ],
        None,
    )
    .assert_ok();
    assert_eq!(
        std::fs::read_to_string(&setting).unwrap(),
        format!("{}\n", target.display())
    );
    run_from_home(&home, None, &["check"], None)
        .assert_ok()
        .says("0 nodes");
}

/// A setting that fails the rule names no corpus, so replacing it redirects
/// nothing and needs no `--force`, which the refusal's hint leaves out. A
/// valid setting naming another corpus still does.
#[test]
fn set_root_replaces_an_invalid_machine_root_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let setting = home.join(".config/nebula/root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();

    for (invalid, name) in [("relcorpus\n", "first"), ("\n", "second")] {
        std::fs::write(&setting, invalid).unwrap();
        let refused = run_from_home(&home, None, &["list"], None).assert_fails();
        assert!(
            refused.stderr().contains("neb init <DIR> --set-root")
                && !refused.stderr().contains("--force"),
            "{}",
            refused.stderr()
        );
        let target = dir.path().join(name);
        run_from_home(&home, Some(&target), &["init", "--set-root"], None).assert_ok();
        assert_eq!(
            std::fs::read_to_string(&setting).unwrap(),
            format!("{}\n", target.display())
        );
    }

    let other = dir.path().join("other");
    run_from_home(&home, Some(&other), &["init", "--set-root"], None)
        .assert_fails()
        .says("Pass --force to replace it.");
}

/// Two `init --set-root` at once could both find the setting free and both
/// write it. The check and the write sit under `~/.config/nebula/.lock`, and
/// a writer that cannot get it refuses before it creates anything.
///
/// The lock is held here in the test process, so the spawned `neb` meets a
/// real cross-process `flock`, as the corpus-lock tests do; the guard
/// releases it on every exit from this test.
#[test]
fn set_root_waits_for_the_machine_setting_lock() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let target = dir.path().join("corpus");
    let settings = home.join(".config").join("nebula");
    std::fs::create_dir_all(&settings).unwrap();
    let held = nebula_core::CorpusLock::acquire(&settings).expect("holding the settings lock");

    // It waits the full five seconds before giving up.
    let refused = run_from_home(
        &home,
        None,
        &[
            "--json",
            "init",
            &target.display().to_string(),
            "--set-root",
        ],
        None,
    );
    assert_eq!(refused.refusal()["code"], "locked");
    assert!(!target.exists(), "the corpus was created without the lock");
    assert!(
        !settings.join("root").exists(),
        "the setting was written without the lock"
    );

    drop(held);
    run_from_home(
        &home,
        None,
        &["init", &target.display().to_string(), "--set-root"],
        None,
    )
    .assert_ok();
    assert_eq!(
        std::fs::read_to_string(settings.join("root")).unwrap(),
        format!("{}\n", target.display())
    );
}

/// `neb` run through `sh` with a chosen umask, so the test process's own
/// umask, which every other test shares, is never touched (STD-03 §R20).
///
/// `sh` comes from the isolating builder, and `exec` hands its environment
/// to `neb` unchanged, so `neb` gets the same `HOME`, cleared variables and
/// git isolation as one the builder made directly.
#[cfg(unix)]
fn run_under_umask(umask: &str, home: &Path, nebula_root: &Path, args: &[&str]) -> Run {
    let mut cmd = support::command("sh", home);
    cmd.arg("-c")
        .arg(format!("umask {umask} && exec \"$0\" \"$@\""))
        .arg(bin())
        .args(args)
        .current_dir(home.parent().unwrap_or(home))
        .env("NEBULA_ROOT", nebula_root)
        .env("NO_COLOR", "1");
    run_command(&mut cmd, format!("(umask {umask}) {}", args.join(" ")))
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// Every file nebula writes is `0600` and every directory it creates is
/// `0700`, whatever the umask would have given them (STD-05 §R8). `002`
/// would otherwise make the corpus group-writable.
#[cfg(unix)]
#[test]
fn state_is_owner_only_under_a_permissive_umask() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = home.join("notes").join("corpus");
    let root_arg = root.display().to_string();
    let observatory = dir.path().join("observatory").display().to_string();
    let neb = |args: &[&str]| run_under_umask("0002", &home, &root, args).assert_ok();

    neb(&["init", &root_arg, "--set-root"]);
    let kept = neb(&["capture", "a thought to promote"]).stdout_trim();
    let dropped = neb(&["capture", "a thought to drop"]).stdout_trim();
    let parent = neb(&["new", "A parent node"]).stdout_trim();
    let child = neb(&["promote", &kept, "--title", "A child node"]).stdout_trim();
    neb(&["link", &child, "derives-from", &parent]);
    neb(&["note", &parent, "reasoning, written down"]);
    neb(&["drop", &dropped]);
    neb(&["config", "observatory-root", &observatory]);

    let mut seen = Vec::new();
    let mut pending = vec![home.clone()];
    while let Some(path) = pending.pop() {
        let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
        if kind.is_dir() {
            assert_eq!(mode(&path), 0o700, "{}", path.display());
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            assert!(kind.is_file(), "{} is not a regular file", path.display());
            assert_eq!(mode(&path), 0o600, "{}", path.display());
        }
        seen.push(path.strip_prefix(&home).unwrap().to_path_buf());
    }
    // The walk reached everything the verbs above write, so the modes
    // asserted on are the ones that matter.
    for expected in [
        ".config/nebula",
        ".config/nebula/root",
        ".config/nebula/observatory-root",
        ".config/nebula/.lock",
        "notes",
        "notes/corpus",
        "notes/corpus/config.yaml",
        "notes/corpus/.gitignore",
        "notes/corpus/.lock",
        "notes/corpus/nodes",
        "notes/corpus/inbox",
    ] {
        assert!(
            seen.contains(&PathBuf::from(expected)),
            "{expected} was not written: {seen:?}"
        );
    }
    for id in [&parent, &child] {
        let node = PathBuf::from(format!("notes/corpus/nodes/{id}.md"));
        assert!(seen.contains(&node), "{} missing: {seen:?}", node.display());
    }
    assert!(
        seen.iter()
            .any(|path| path.starts_with("notes/corpus/inbox") && path.extension().is_some()),
        "no inbox month was written: {seen:?}"
    );
}

/// A file its owner narrowed stays narrow: a rewrite replaces it with a file
/// created `0600`, never with one the umask widened.
#[cfg(unix)]
#[test]
fn a_rewrite_never_widens_a_node_or_inbox_file() {
    use std::os::unix::fs::PermissionsExt;

    let c = Corpus::new();
    let id = c.seed("a node kept private", "Private idea");
    let pending = c.run(&["capture", "a capture to drop"]).stdout_trim();
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "md"))
        .expect("an inbox month");
    let node = c.node_file(&id);
    for path in [&node, &month] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    let neb = |args: &[&str]| run_under_umask("0022", c.workdir(), &c.root, args).assert_ok();
    neb(&["note", &id, "still private"]);
    neb(&["tag", &id, "--add", "alpha"]);
    neb(&["drop", &pending]);

    assert!(std::fs::read_to_string(&node).unwrap().contains("alpha"));
    assert!(std::fs::read_to_string(&month).unwrap().contains("dropped"));
    assert_eq!(mode(&node), 0o600, "the node was widened");
    assert_eq!(mode(&month), 0o600, "the inbox month was widened");
}

#[test]
fn set_root_refuses_a_relative_path_before_mutating_anything() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let from = dir.path().join("from");
    std::fs::create_dir_all(&from).unwrap();

    let out = output(
        neb_command(&home)
            .args(["--root", "relative", "init", "--set-root"])
            .current_dir(&from)
            .env("NO_COLOR", "1"),
    );
    Run {
        args: "--root relative init --set-root".into(),
        out,
    }
    .assert_fails()
    .says("must be an absolute path, not `relative`")
    .says("neb init <DIR> --set-root");

    assert!(
        !from.join("relative").exists(),
        "a refused relative root must not initialize a corpus"
    );
    assert!(
        !home.join(".config/nebula/root").exists(),
        "a refused relative root must not create the machine setting"
    );
}

#[cfg(unix)]
#[test]
fn set_root_preserves_an_absolute_symlink_spelling_across_working_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let real_parent = dir.path().join("real-parent");
    let alias_parent = dir.path().join("alias-parent");
    let from = dir.path().join("from");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&real_parent).unwrap();
    std::fs::create_dir_all(&from).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    symlink(&real_parent, &alias_parent).unwrap();
    let root = alias_parent.join("corpus");

    let init = output(
        neb_command(&home)
            .arg("--root")
            .arg(&root)
            .args(["init", "--set-root"])
            .current_dir(&from)
            .env("NO_COLOR", "1"),
    );
    Run {
        args: format!("--root {} init --set-root", root.display()),
        out: init,
    }
    .assert_ok();

    assert_eq!(
        std::fs::read_to_string(home.join(".config/nebula/root")).unwrap(),
        format!("{}\n", root.display()),
        "the configured root must preserve the caller's symlink spelling"
    );

    let inbox = output(
        neb_command(&home)
            .arg("inbox")
            .current_dir(&elsewhere)
            .env("NO_COLOR", "1"),
    );
    Run {
        args: "inbox from a different working directory".into(),
        out: inbox,
    }
    .assert_ok();
}

#[test]
fn repeated_init_and_set_root_preserve_the_existing_corpus_byte_for_byte() {
    let c = Corpus::new();
    let observatory = c.workdir().join("observatory");
    c.run(&["config", "observatory-root", observatory.to_str().unwrap()])
        .assert_ok();
    c.run(&["config", "commit", "on"]).assert_ok();
    c.seed("an idea that must survive init", "Durable idea");
    c.run(&["capture", "an inbox thought that must survive init"])
        .assert_ok();

    let config_before = std::fs::read(c.root.join("config.yaml")).unwrap();
    let content_before = snapshot_corpus_files(&c.root);

    c.run(&["init"]).assert_ok();
    assert_eq!(
        config_before,
        std::fs::read(c.root.join("config.yaml")).unwrap()
    );
    assert_eq!(content_before, snapshot_corpus_files(&c.root));

    // The documented positional recipe must be just as safe, while still
    // creating the machine-local root setting when it is absent.
    run_from_home(
        c.workdir(),
        None,
        &["init", c.root.to_str().unwrap(), "--set-root"],
        None,
    )
    .assert_ok();
    assert_eq!(
        config_before,
        std::fs::read(c.root.join("config.yaml")).unwrap()
    );
    assert_eq!(content_before, snapshot_corpus_files(&c.root));
    assert_eq!(
        std::fs::read_to_string(c.workdir().join(".config/nebula/root")).unwrap(),
        format!("{}\n", c.root.display())
    );
}

#[test]
fn repeated_init_adds_the_lock_ignore_without_replacing_existing_rules() {
    let c = Corpus::new();
    write(&c.root.join(".gitignore"), "private-notes/\r\n");

    c.run(&["init"]).assert_ok();
    assert_eq!(
        std::fs::read(c.root.join(".gitignore")).unwrap(),
        b"private-notes/\r\n/.lock\n/.pending\n*.tmp\n"
    );

    c.run(&["init"]).assert_ok();
    assert_eq!(
        std::fs::read(c.root.join(".gitignore")).unwrap(),
        b"private-notes/\r\n/.lock\n/.pending\n*.tmp\n",
        "the setup repair is idempotent"
    );
}

/// Crash debris is ignored as well as the lock, after whatever rules the
/// file already holds, and a corpus an older `init` set up gets only the
/// rules it lacks.
#[test]
fn init_ignores_temp_debris_without_disturbing_existing_rules() {
    let c = Corpus::new();
    git_init(&c.root);
    let ignore = c.root.join(".gitignore");
    let rules = std::fs::read_to_string(&ignore).unwrap();
    assert!(rules.lines().any(|line| line == "*.tmp"), "{rules}");
    for debris in [
        "nodes/x.md.a-0-1.tmp",
        "inbox/2026-09.md.a-0-1.tmp",
        ".pending",
    ] {
        assert_eq!(
            git(&c.root, &["check-ignore", debris]),
            format!("{debris}\n"),
            "{debris} is not ignored"
        );
    }
    let node =
        output(git_command(&c.root, support::home()).args(["check-ignore", "-q", "nodes/x.md"]));
    assert_eq!(node.status.code(), Some(1), "a node is never ignored");

    // The file an older `init` wrote ends with the lock rule alone.
    write(&ignore, "private-notes/\n/.lock\n");
    c.run(&["init"]).assert_ok();
    assert_eq!(
        std::fs::read(&ignore).unwrap(),
        b"private-notes/\n/.lock\n/.pending\n*.tmp\n"
    );
    c.run(&["init"]).assert_ok();
    assert_eq!(
        std::fs::read(&ignore).unwrap(),
        b"private-notes/\n/.lock\n/.pending\n*.tmp\n",
        "idempotent"
    );
}

#[cfg(unix)]
#[test]
fn init_replaces_a_gitignore_symlink_even_when_its_target_already_has_the_rule() {
    use std::os::unix::fs::symlink;

    let c = Corpus::new();
    git_init(&c.root);
    let ignore = c.root.join(".gitignore");
    let target = c.workdir().join("external.gitignore");
    let target_bytes = b"private-notes/\n/.lock\n/.pending\n*.tmp\n";
    write(&target, std::str::from_utf8(target_bytes).unwrap());
    std::fs::remove_file(&ignore).unwrap();
    symlink(&target, &ignore).unwrap();

    c.run(&["init"]).assert_ok();

    assert_eq!(std::fs::read(&target).unwrap(), target_bytes);
    assert!(
        !std::fs::symlink_metadata(&ignore)
            .unwrap()
            .file_type()
            .is_symlink(),
        "git only honors a regular .gitignore"
    );
    assert_eq!(std::fs::read(&ignore).unwrap(), target_bytes);
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(
        !git(&c.root, &["status", "--porcelain"]).contains(".lock"),
        "the runtime lock must be absent from actual git status"
    );

    c.run(&["config", "commit", "on"])
        .assert_ok()
        .says("committed ");
    git(&c.root, &["add", "-A"]);
    assert!(
        !git(&c.root, &["diff", "--cached", "--name-only"]).contains(".lock"),
        "git add -A must not stage the runtime lock"
    );
    c.run(&["new", "Still commits", "--id", "still-commits"])
        .assert_ok()
        .says("committed ");
    assert_eq!(log(&c.root)[0], "neb new still-commits");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

#[cfg(unix)]
#[test]
fn init_copies_a_gitignore_symlink_target_without_mutating_it() {
    use std::os::unix::fs::symlink;

    let c = Corpus::new();
    let ignore = c.root.join(".gitignore");
    let target = c.workdir().join("external.gitignore");
    let target_bytes = b"private-notes/\r\n";
    write(&target, std::str::from_utf8(target_bytes).unwrap());
    std::fs::remove_file(&ignore).unwrap();
    symlink(&target, &ignore).unwrap();

    c.run(&["init"]).assert_ok();

    assert_eq!(std::fs::read(&target).unwrap(), target_bytes);
    assert_eq!(
        std::fs::read(&ignore).unwrap(),
        b"private-notes/\r\n/.lock\n/.pending\n*.tmp\n"
    );
    assert!(
        !std::fs::symlink_metadata(&ignore)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let local_before = std::fs::read(&ignore).unwrap();
    c.run(&["init"]).assert_ok();
    assert_eq!(std::fs::read(&ignore).unwrap(), local_before);
    assert_eq!(std::fs::read(&target).unwrap(), target_bytes);
}

#[cfg(unix)]
#[test]
fn init_replaces_a_dangling_gitignore_symlink_with_an_effective_local_file() {
    use std::os::unix::fs::symlink;

    let c = Corpus::new();
    let ignore = c.root.join(".gitignore");
    let missing = c.workdir().join("missing.gitignore");
    std::fs::remove_file(&ignore).unwrap();
    symlink(&missing, &ignore).unwrap();

    c.run(&["init"]).assert_ok();

    assert!(!missing.exists(), "init must not create the symlink target");
    assert_eq!(
        std::fs::read(&ignore).unwrap(),
        b"/.lock\n/.pending\n*.tmp\n"
    );
    assert!(
        !std::fs::symlink_metadata(&ignore)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    git_init(&c.root);
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
}

#[test]
fn init_refuses_invalid_existing_configs_without_mutation() {
    for config in [
        "schema_version: 1\ncorpus_id: neb-old\n",
        "schema_version: [not, a, number]\ncorpus_id: neb-broken\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let root = dir.path().join("corpus");
        std::fs::create_dir_all(root.join("nodes")).unwrap();
        std::fs::create_dir_all(root.join("inbox")).unwrap();
        write(&root.join("config.yaml"), config);
        write(
            &root.join("nodes/keep.md"),
            "node bytes that must survive\n",
        );
        write(
            &root.join("inbox/keep.md"),
            "inbox bytes that must survive\n",
        );
        let config_before = std::fs::read(root.join("config.yaml")).unwrap();
        let content_before = snapshot_corpus_files(&root);

        run_from_home(&home, Some(&root), &["init"], None).assert_fails();

        assert_eq!(
            config_before,
            std::fs::read(root.join("config.yaml")).unwrap()
        );
        assert_eq!(content_before, snapshot_corpus_files(&root));
    }
}

#[test]
fn init_warns_before_creating_default_root_that_shadows_configured_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");
    let config_path = home.join(".config/nebula/root");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, format!("{}\n", configured.display())).unwrap();

    let default = home.join(".nebula");
    let out = run_from_home(&home, Some(&default), &["init"], None).assert_ok();
    assert!(
        out.stderr().contains("warning: creating ~/.nebula"),
        "expected warning in:\n{}",
        out.stderr()
    );
    assert!(out.stderr().contains(&configured.display().to_string()));
}

#[test]
fn init_warns_when_a_third_directory_would_orphan_the_configured_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("corpus");
    let config_path = home.join(".config/nebula/root");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, format!("{}\n", configured.display())).unwrap();

    let other = dir.path().join("corpus2");
    let out = run_from_home(&home, Some(&other), &["init"], None).assert_ok();
    assert!(
        out.stderr().contains(&config_path.display().to_string()),
        "expected warning naming {} in:\n{}",
        config_path.display(),
        out.stderr()
    );
    assert!(
        out.stderr().contains(&configured.display().to_string()),
        "expected warning naming the still-configured root in:\n{}",
        out.stderr()
    );
    assert!(
        out.stderr().contains(&other.display().to_string()),
        "expected warning naming the new corpus in:\n{}",
        out.stderr()
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        format!("{}\n", configured.display()),
        "the config root must not be silently rewritten"
    );
}

/// The shadowing warning's remedy is the documented command, with the target
/// absolute and quoted as a shell word, never a hand edit of the setting file
/// and never the relative spelling that would resolve per directory.
#[test]
fn init_shadowing_warning_names_the_set_root_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("corpus");
    let config_path = home.join(".config/nebula/root");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, format!("{}\n", configured.display())).unwrap();
    let setting = std::fs::read(&config_path).unwrap();
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = std::fs::canonicalize(&cwd).unwrap();

    let run = run_in(&cwd, &home, None, &["init", "rel"], None).assert_ok();
    let stderr = run.stderr();
    let Some(warning) = stderr.lines().find(|l| l.starts_with("warning: ")) else {
        panic!("no warning in:\n{stderr}");
    };
    let absolute = cwd.join("rel");
    assert!(
        warning.contains(&format!(
            "run `neb init {} --set-root --force`",
            absolute.display()
        )),
        "{warning}"
    );
    assert!(
        warning.contains(&config_path.display().to_string()),
        "{warning}"
    );
    assert!(
        warning.contains(&configured.display().to_string()),
        "{warning}"
    );
    assert!(!warning.contains("echo"), "{warning}");
    assert!(
        !warning.contains(" rel ") && !warning.contains("`rel") && !warning.contains(" rel;"),
        "the relative spelling: {warning}"
    );
    assert_eq!(std::fs::read(&config_path).unwrap(), setting, "unchanged");

    // A target that needs quoting is quoted, so the command runs as printed.
    let run = run_in(&cwd, &home, None, &["init", "my corpus"], None).assert_ok();
    let spaced = cwd.join("my corpus").display().to_string();
    assert!(
        run.stderr()
            .contains(&format!("run `neb init '{spaced}' --set-root --force`"))
            || run
                .stderr()
                .contains(&format!("run `neb init \"{spaced}\" --set-root --force`")),
        "{}",
        run.stderr()
    );
    assert_eq!(std::fs::read(&config_path).unwrap(), setting, "unchanged");
}

#[test]
fn zsh_completions_include_the_cli_commands() {
    let dir = tempfile::tempdir().unwrap();
    run_from_home(
        &dir.path().join("home"),
        None,
        &["completions", "zsh"],
        None,
    )
    .assert_ok()
    .says("#compdef neb")
    .says("capture");
}

/// The script is the same for every corpus and never JSON, so `--root` and
/// `--json` would be ignored: each is refused as a usage error, on either
/// side of the verb, with nothing on stdout (STD-01 §R28, §R36). The ambient
/// `$NEBULA_ROOT` is not a flag the caller typed, and is not read.
#[test]
fn completions_refuses_json_and_root() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let nowhere = dir.path().join("nowhere");
    let after = ["completions", "bash", "--root", nowhere.to_str().unwrap()];
    for (root, args) in [
        (None, ["--json", "completions", "bash"].as_slice()),
        (None, &["completions", "bash", "--json"]),
        (Some(nowhere.as_path()), &["completions", "bash"]),
        (None, &after),
    ] {
        let run = run_from_home(&home, root, args, None);
        assert_eq!(
            run.out.status.code(),
            Some(2),
            "{}: {}",
            run.args,
            run.stderr()
        );
        assert_eq!(run.stdout(), "", "{}", run.args);
        assert!(
            run.stderr().contains("cannot be used with"),
            "{}",
            run.stderr()
        );
    }
    assert!(!nowhere.exists());

    run_from_home(&home, None, &["completions", "bash"], Some(&nowhere))
        .assert_ok()
        .says("neb");
    assert!(!nowhere.exists());
}

// -------------------------------------------------------------------- help --

/// The command names listed under `heading:` in a rendered help page, up to
/// the next blank-line-separated heading.
fn listed_commands(help: &str, headings: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    let mut listing = false;
    for line in help.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            listing = headings.contains(&line.trim_end_matches(':'));
            continue;
        }
        if listing
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && let Some(name) = line.split_whitespace().next()
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// Every help page the built binary renders: the top level, each command
/// it lists, each `config` setting, and the hidden `open`.
fn every_help_page(c: &Corpus) -> Vec<(String, String)> {
    let top = c.run(&["--help"]).assert_ok().stdout();
    let sections = [
        "Corpus",
        "Inbox",
        "Nodes",
        "References",
        "Query",
        "Maintenance",
    ];
    let mut paths: Vec<Vec<String>> = listed_commands(&top, &sections)
        .into_iter()
        .map(|name| vec![name])
        .collect();
    let config = c.run(&["config", "--help"]).assert_ok().stdout();
    paths.extend(
        listed_commands(&config, &["Commands"])
            .into_iter()
            .map(|name| vec!["config".to_owned(), name]),
    );
    paths.push(vec!["open".to_owned()]);
    assert!(paths.len() >= 30, "{paths:?}");
    let mut pages = vec![("neb".to_owned(), top)];
    for path in paths {
        let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
        args.push("--help");
        pages.push((
            format!("neb {}", path.join(" ")),
            c.run(&args).assert_ok().stdout(),
        ));
    }
    pages
}

/// Every word shaped like an Observatory record id: Q, H, T or R and three
/// digits, standing alone.
fn record_ids(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| {
            w.len() == 4
                && w.starts_with(['Q', 'H', 'T', 'R'])
                && w[1..].bytes().all(|b| b.is_ascii_digit())
        })
        .map(str::to_owned)
        .collect()
}

/// A command whose use is not obvious ends its help with examples (STD-01
/// §R22), written with placeholders.
#[test]
fn help_has_examples_for_non_obvious_commands() {
    let c = Corpus::new();
    for verb in [
        "promote", "triage", "link", "cite", "handoff", "near", "trace",
    ] {
        let help = c.run(&[verb, "--help"]).assert_ok().stdout();
        let Some((_, examples)) = help.split_once("\nExamples:\n") else {
            panic!("`neb {verb} --help` has no Examples:\n{help}");
        };
        assert!(
            examples
                .lines()
                .filter(|l| !l.trim().is_empty())
                .all(|l| l.contains(&format!("neb {verb}"))),
            "each example runs `neb {verb}`:\n{examples}"
        );
    }
}

/// No help page and no refusal names a real record: the examples use
/// placeholders (STD-01 §R23).
#[test]
fn no_real_record_ids_in_help_or_refusals() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for (page, help) in every_help_page(&c) {
        assert_eq!(record_ids(&help), Vec::<String>::new(), "`{page} --help`");
    }
    for args in [
        vec!["cite", &id, "--uri", "/abs/x", "--note", "n"],
        vec!["handoff", &id, "nonsense"],
    ] {
        let run = c.run(&args).assert_fails();
        assert_eq!(
            record_ids(&run.stderr()),
            Vec::<String>::new(),
            "{}",
            run.stderr()
        );
        let mut with_json = vec!["--json"];
        with_json.extend_from_slice(&args);
        let refused = c.run(&with_json).usage_refusal().to_string();
        assert_eq!(record_ids(&refused), Vec::<String>::new(), "{refused}");
    }
    assert_eq!(
        record_ids("Q002, H012 and `T123`"),
        ["Q002", "H012", "T123"]
    );
    assert!(record_ids("Q<nnn> Q0021 XQ002 q002").is_empty());
}

// -------------------------------------------------------------------- near --

/// Nodes whose vocabulary overlaps in known ways, so a query has one right
/// answer and one wrong one.
fn lexical_fixture(c: &Corpus) {
    c.run(&[
        "new",
        "Tags beat domains",
        "--tag",
        "design",
        "--tag",
        "corpus",
        "--kill",
        "a corpus of 50+ nodes needs a query that tags cannot answer",
    ])
    .assert_ok();
    c.run(&["new", "A single global taxonomy", "--tag", "design"])
        .assert_ok();
    c.run(&["new", "Ranking decay half-life", "--tag", "search"])
        .assert_ok();
    c.run(&["new", "Proper time is a count", "--tag", "physics"])
        .assert_ok();
}

/// The ids in `near --json`'s envelope, in order.
fn ids(json: &str) -> Vec<String> {
    let out: serde_json::Value = serde_json::from_str(json).unwrap();
    out["items"]
        .as_array()
        .unwrap_or_else(|| panic!("an envelope's items: {json}"))
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn near_ranks_existing_nodes_against_free_text() {
    let c = Corpus::new();
    lexical_fixture(&c);

    let json = c
        .run(&[
            "near", "--json", "tags", "and", "domains", "beat", "a", "taxonomy",
        ])
        .assert_ok()
        .stdout();
    assert_eq!(
        ids(&json),
        ["tags-beat-domains", "a-single-global-taxonomy"],
        "{json}"
    );
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["total"], 2, "{json}");
    assert_eq!(out["truncated"], false, "{json}");
    let out = &out["items"];
    let first = &out[0];
    assert_eq!(first["title"], "Tags beat domains");
    assert_eq!(first["status"], "hypothesis");
    assert_eq!(first["tags"], serde_json::json!(["design", "corpus"]));
    let score = first["score"].as_f64().unwrap();
    assert!(score > 0.0 && score <= 1.0, "{json}");
    assert!(
        score >= out[1]["score"].as_f64().unwrap(),
        "best first: {json}"
    );
    // The raw score stays in JSON, beside the band a human is shown; free
    // text is no node, so nothing is linked to it.
    for n in out.as_array().unwrap() {
        assert!(
            ["strong", "some", "weak"].contains(&n["band"].as_str().unwrap()),
            "{json}"
        );
        assert!(n["linked"].is_null(), "present, and null: {json}");
    }
    assert_eq!(first["band"], "strong", "{json}");

    // Text mode, piped: one line per neighbour, band first, the best on
    // top, and no bare number to misread.
    let text = c
        .run(&["near", "tags and domains beat a taxonomy"])
        .assert_ok()
        .stdout();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("\ttags-beat-domains\t"), "{text}");
    assert!(lines[1].contains("\ta-single-global-taxonomy\t"), "{text}");
    assert!(
        lines[0].starts_with("strong\t"),
        "band leads the line: {text}"
    );
    let band = out[1]["band"].as_str().unwrap();
    assert!(lines[1].starts_with(band), "the JSON's band: {text}");
    assert!(!text.contains("0."), "no raw score in text: {text}");

    // A limit caps the answer.
    let json = c
        .run(&["near", "--json", "--limit", "1", "tags taxonomy ranking"])
        .assert_ok()
        .stdout();
    assert_eq!(ids(&json).len(), 1, "{json}");
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["truncated"], true, "{json}");
}

#[test]
fn near_takes_a_node_id_and_never_returns_that_node() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let json = c
        .run(&["near", "--json", "tags-beat-domains"])
        .assert_ok()
        .stdout();
    let found = ids(&json);
    assert!(!found.contains(&"tags-beat-domains".to_string()), "{json}");
    assert_eq!(
        found.first().map(String::as_str),
        Some("a-single-global-taxonomy"),
        "{json}"
    );
}

/// Asked about a node, `near` says which neighbours are already linked to
/// it and by what, in text and in JSON, and still writes nothing.
#[test]
fn near_marks_candidates_already_linked_to_the_node() {
    let c = Corpus::new();
    lexical_fixture(&c);
    c.run(&[
        "new",
        "Tags beat domains in corpus design",
        "--parent",
        "tags-beat-domains",
    ])
    .assert_ok();
    c.run(&[
        "link",
        "tags-beat-domains",
        "contradicts",
        "a-single-global-taxonomy",
    ])
    .assert_ok();
    let before = snapshot_corpus_files(&c.root);

    let text = c.run(&["near", "tags-beat-domains"]).assert_ok().stdout();
    let line = |id: &str| {
        text.lines()
            .find(|l| l.contains(id))
            .unwrap_or_else(|| panic!("{id} in: {text}"))
            .to_string()
    };
    assert!(
        line("tags-beat-domains-in-corpus-design").ends_with("\tchild (derives-from)"),
        "{text}"
    );
    assert!(
        line("a-single-global-taxonomy").ends_with("\tcontradicts"),
        "one mark for a contradiction stored on both ends: {text}"
    );

    let text = c
        .run(&["near", "tags-beat-domains-in-corpus-design"])
        .assert_ok()
        .stdout();
    let parent = text
        .lines()
        .find(|l| l.contains("\ttags-beat-domains\t"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(parent.ends_with("\tparent (derives-from)"), "{text}");
    let unlinked = text
        .lines()
        .find(|l| l.contains("a-single-global-taxonomy"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(unlinked.ends_with("\t-"), "nothing linked is `-`: {text}");

    let json = c
        .run(&["near", "--json", "tags-beat-domains-in-corpus-design"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    let by_id = |id: &str| {
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == id)
            .unwrap_or_else(|| panic!("{id} in: {json}"))
            .clone()
    };
    assert_eq!(
        by_id("tags-beat-domains")["linked"],
        serde_json::json!([{
            "from": "tags-beat-domains-in-corpus-design",
            "type": "derives-from",
            "to": "tags-beat-domains",
        }]),
        "{json}"
    );
    assert!(
        by_id("a-single-global-taxonomy")["linked"].is_null(),
        "{json}"
    );

    // A suggestion only: marking a link is a read, and nothing changed.
    assert_eq!(before, snapshot_corpus_files(&c.root));
}

#[test]
fn near_says_so_when_nothing_matches() {
    let c = Corpus::new();
    lexical_fixture(&c);
    c.run(&["near", "quantum gravity"])
        .assert_ok()
        .says("nothing near");
    let json = c
        .run(&["near", "--json", "quantum", "gravity"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        out,
        serde_json::json!({"items": [], "total": 0, "truncated": false})
    );
    c.run(&["near", "   "])
        .assert_fails()
        .says("nothing to look near");
}

/// `near` always has a limit, so `--json` is always the envelope: the
/// neighbours kept, how many shared a word with the query, and whether `-k`
/// cut any. A cut is named on stderr in every mode.
#[test]
fn near_json_reports_total_and_truncated() {
    let c = Corpus::new();
    for i in 0..150 {
        let id = format!("lantern-{i:03}");
        write(
            &c.node_file(&id),
            &format!(
                "---\nid: {id}\ntitle: Lantern number {i}\nstatus: seed\n\
                 created: 2026-09-01\nupdated: 2026-09-01\n---\n"
            ),
        );
    }
    c.run(&["new", "Unrelated idea"]).assert_ok();

    let cut = c.run(&["near", "--json", "lantern"]).assert_ok();
    let out: serde_json::Value = serde_json::from_str(&cut.stdout()).unwrap();
    assert_eq!(out["items"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(out["total"], 150, "{out}");
    assert_eq!(out["truncated"], true, "{out}");
    assert!(
        cut.stderr().contains("3 of 150 shown; raise -k for more"),
        "{}",
        cut.stderr()
    );
    let text = c.run(&["near", "lantern"]).assert_ok();
    assert!(text.stderr().contains("3 of 150"), "{}", text.stderr());
    assert!(!text.stdout().contains("3 of 150"), "{}", text.stdout());

    let whole = c
        .run(&["near", "--json", "lantern", "-k", "500"])
        .assert_ok();
    let out: serde_json::Value = serde_json::from_str(&whole.stdout()).unwrap();
    assert_eq!(out["truncated"], false, "{out}");
    assert_eq!(out["total"], 150, "{out}");
    assert_eq!(out["items"].as_array().unwrap().len(), 150, "{out}");
    assert_eq!(whole.stderr(), "", "no notice for a whole answer");
}

/// `--limit` after the query is a flag; `--quiet` is not a near flag, and
/// `near` writes nothing so `--no-commit` is not one either: both are refused
/// rather than folded into the text.
#[test]
fn near_trailing_limit_is_a_flag_and_quiet_and_no_commit_are_refused() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let trailing = c
        .run(&["near", "--json", "one global taxonomy", "--limit", "1"])
        .assert_ok()
        .stdout();
    let leading = c
        .run(&["near", "--json", "--limit", "1", "one global taxonomy"])
        .assert_ok()
        .stdout();
    assert_eq!(
        trailing, leading,
        "trailing --limit must not join the query"
    );
    for flag in ["--quiet", "--no-commit"] {
        c.run(&["near", "one global taxonomy", flag])
            .assert_fails()
            .says(flag);
    }
}

/// `-k 0` would answer "nothing near" for a query with a strong match: a
/// false empty answer. It is a usage error under either spelling.
#[test]
fn near_k_zero_is_a_usage_error() {
    let c = Corpus::new();
    lexical_fixture(&c);
    for flag in ["-k", "--limit"] {
        for json in [false, true] {
            let json = if json { ["--json"].as_slice() } else { &[] };
            let zero = c.run(&[json, &["near", flag, "0", "one global taxonomy"]].concat());
            assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
            assert_eq!(zero.stdout(), "", "`neb {}`", zero.args);
            // clap names the flag by its long form, whichever was typed.
            assert!(
                zero.stderr().contains("'--limit <K>'") && zero.stderr().contains("at least 1"),
                "`neb {}`: {}",
                zero.args,
                zero.stderr()
            );
        }
        c.run(&["near", flag, "1", "one global taxonomy"])
            .assert_ok();
    }
}

#[test]
fn capture_prints_the_nearest_nodes_after_the_id_unless_quiet() {
    let c = Corpus::new();
    lexical_fixture(&c);

    let out = c
        .run(&["capture", "a global taxonomy for tags"])
        .assert_ok()
        .stdout();
    let mut lines = out.lines();
    let id = lines.next().unwrap();
    assert_eq!(id.len(), 4, "the entry id alone on the first line: {out}");
    assert_eq!(lines.next(), Some("near:"), "{out}");
    let rest: Vec<&str> = lines.collect();
    assert_eq!(rest.len(), 2, "two nodes share a word, so two lines: {out}");
    assert!(rest[0].contains("a-single-global-taxonomy"), "{out}");
    assert!(rest[1].contains("tags-beat-domains"), "{out}");

    // The capture landed, and nothing else changed: no node, no edge.
    c.run(&["inbox"])
        .assert_ok()
        .says("a global taxonomy for tags");
    let graph = c.run(&["graph", "--json"]).assert_ok().stdout();
    let out: serde_json::Value = serde_json::from_str(&graph).unwrap();
    assert_eq!(out["nodes"].as_array().unwrap().len(), 4, "{graph}");
    assert!(out["edges"].as_array().unwrap().is_empty(), "{graph}");

    // --quiet: the id and nothing else.
    let out = c
        .run(&["capture", "--quiet", "a taxonomy again"])
        .assert_ok()
        .stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
    let out = c.run(&["capture", "-q", "tags again"]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");

    // Nothing near: the id alone, with no empty heading under it.
    let out = c.run(&["capture", "quantum gravity"]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
}

/// `--quiet` and `--no-commit` after the thought are flags, not more text.
#[test]
fn capture_trailing_quiet_and_no_commit_are_flags() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let out = c
        .run(&["capture", "an idea", "--quiet"])
        .assert_ok()
        .stdout();
    assert_eq!(
        out.lines().count(),
        1,
        "trailing --quiet still quiets: {out}"
    );
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    let inbox: serde_json::Value = serde_json::from_str(&json).unwrap();
    let texts: Vec<&str> = inbox
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    assert!(texts.contains(&"an idea"), "{json}");
    assert!(
        texts.iter().all(|t| !t.contains("--quiet")),
        "the flag must not land in the inbox: {json}"
    );

    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = log(&c.root).len();
    c.run(&["capture", "kept out of git", "--no-commit"])
        .assert_ok();
    assert_eq!(log(&c.root).len(), before, "trailing --no-commit skips git");
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    assert!(json.contains("kept out of git"), "{json}");
    assert!(!json.contains("--no-commit"), "{json}");
}

#[test]
fn capture_prints_its_id_before_a_node_that_will_not_parse_can_get_in_the_way() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\nverdict: supports"),
    );

    // The write lands and the id is printed; the suggestions are what fail,
    // after it, and they are a side channel: said on stderr, never the
    // capture's exit status. Nothing about the capture depended on `nodes/`
    // parsing, and it must not read as a lost thought.
    let run = c.run(&["capture", "another idea"]);
    let out = run.stdout();
    assert_eq!(out.lines().count(), 1, "the id, alone: {out}");
    assert_eq!(out.trim().len(), 4, "{out}");
    run.assert_ok()
        .says("suggestions unavailable")
        .says("unknown field `verdict`");
    c.run(&["inbox"]).assert_ok().says("another idea");

    // --quiet never reads `nodes/`, so it does not even see the problem.
    c.run(&["capture", "-q", "quietly"]).assert_ok();
}

/// A malformed node file under `nodes/`, with commits on.
fn corpus_with_a_broken_node() -> Corpus {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    c.seed("an idea", "An idea");
    write(&c.node_file("broken"), "this node lost its frontmatter\n");
    c
}

/// Suggestions are a declared side channel (STD-02 §R31): a `nodes/` that
/// cannot be read for them costs the suggestions, said on stderr with the
/// file, and never the capture. It is committed and exits 0, so nobody
/// retries it and captures the thought twice.
#[test]
fn capture_with_an_unreadable_node_file_still_commits_and_exits_zero() {
    let c = corpus_with_a_broken_node();

    let run = c.run(&["capture", "x"]).assert_ok();
    let id = run.stdout_trim();
    assert_eq!(id.len(), 4, "the id is on stdout: {}", run.stdout());
    let stderr = run.stderr();
    assert!(stderr.contains("suggestions unavailable"), "{stderr}");
    assert!(stderr.contains("broken.md"), "names the file: {stderr}");
    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
}

/// The same under `--json`: the entry is on stdout, so an agent knows the
/// capture landed.
#[test]
fn json_capture_with_an_unreadable_node_file_reports_the_entry() {
    let c = corpus_with_a_broken_node();

    let run = c.run(&["--json", "capture", "json thought"]).assert_ok();
    let out: serde_json::Value = serde_json::from_str(&run.stdout())
        .unwrap_or_else(|e| panic!("stdout is the entry ({e}): {}", run.stdout()));
    assert_eq!(out["entry"]["text"], "json thought", "{out}");
    assert_eq!(out["near"], serde_json::json!([]), "{out}");
    assert!(run.stderr().contains("broken.md"), "{}", run.stderr());
    let id = out["entry"]["id"].as_str().unwrap();
    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
}

#[test]
fn every_graph_reader_names_a_node_with_structurally_invalid_frontmatter() {
    for (contents, message) in [
        (
            "this node lost its opening delimiter\n",
            "missing YAML frontmatter",
        ),
        (
            "---\nid: broken\ntitle: Broken\nstatus: seed\ncreated: 2026-09-01\nupdated: 2026-09-01\n\nthe closing delimiter was lost\n",
            "frontmatter is not terminated",
        ),
    ] {
        let c = Corpus::new();
        let healthy = c.seed("a healthy node", "Healthy");
        let broken = c.node_file("broken");
        write(&broken, contents);
        let path = broken.display().to_string();

        for args in [
            vec!["check"],
            vec!["list"],
            vec!["show", healthy.as_str()],
            vec!["graph", "--json"],
        ] {
            c.run(&args).assert_fails().says(message).says(&path);
        }
    }
}

/// A directory where a node file belongs is refused by name as what it is,
/// by every reader, rather than surfacing as the OS's `Is a directory`.
#[test]
fn every_corpus_reader_names_a_non_regular_node_as_what_it_is() {
    let c = Corpus::new();
    let healthy = c.seed("a healthy node", "Healthy");
    let broken = c.node_file("broken");
    std::fs::create_dir(&broken).unwrap();
    let path = broken.display().to_string();

    for args in [
        vec!["check"],
        vec!["list"],
        vec!["show", healthy.as_str()],
        vec!["graph", "--mermaid"],
        vec!["trace", healthy.as_str()],
        vec!["review"],
        vec!["review", "--short"],
        vec!["near", "thing"],
        vec!["tag", "list"],
    ] {
        c.run(&args)
            .assert_fails()
            .says(&format!("{path} is a directory, not a regular file"));
    }
    assert_eq!(
        c.run(&["--json", "list"]).refusal()["code"],
        "unreadable_nodes"
    );
}

#[test]
fn opening_a_non_regular_config_names_it_as_what_it_is() {
    let c = Corpus::new();
    let config = c.root.join("config.yaml");
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    let path = config.display().to_string();

    c.run(&["list"])
        .assert_fails()
        .says(&format!("{path} is a directory, not a regular file"));
}

#[cfg(unix)]
#[test]
fn a_node_write_failure_names_the_destination_and_preserves_the_os_cause() {
    use std::os::unix::fs::PermissionsExt;

    let c = Corpus::new();
    let nodes = c.root.join("nodes");
    let mut permissions = std::fs::metadata(&nodes).unwrap().permissions();
    permissions.set_mode(0o500);
    std::fs::set_permissions(&nodes, permissions).unwrap();

    let destination = c.node_file("cannot-land");
    let run = c.run(&["new", "Cannot land", "--id", "cannot-land"]);

    let mut permissions = std::fs::metadata(&nodes).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&nodes, permissions).unwrap();

    run.assert_fails()
        .says("writing")
        .says(&destination.display().to_string())
        .says("Permission denied");
}

#[test]
fn capture_json_carries_the_entry_and_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let json = c
        .run(&["capture", "--json", "a taxonomy of tags"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["entry"]["text"], "a taxonomy of tags", "{json}");
    assert_eq!(out["entry"]["id"].as_str().unwrap().len(), 4, "{json}");
    assert!(out["entry"]["at"].is_string(), "{json}");
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(near[0]["score"].is_number(), "{json}");
    assert!(near[0]["band"].is_string(), "{json}");

    let json = c
        .run(&[
            "capture",
            "--json",
            "--quiet",
            "a taxonomy of tags, quietly",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        out["near"],
        serde_json::json!([]),
        "quiet empties it: {json}"
    );
    assert!(out["entry"]["id"].is_string(), "{json}");
}

#[test]
fn promote_without_a_parent_suggests_and_proceeds_as_a_root() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "search ranking decays with age"])
        .assert_ok()
        .stdout_trim();

    let out = c.run(&["promote", &entry]).assert_ok().stdout();
    let mut lines = out.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("search-ranking-decays-with-age "),
        "id and path first: {out}"
    );
    assert_eq!(lines.next(), Some("near:"), "{out}");
    let rest: Vec<&str> = lines.collect();
    assert_eq!(rest, [rest[0]], "one node shares a word: {out}");
    assert!(rest[0].contains("ranking-decay-half-life"), "{out}");

    // Promoted as a root: no parent, no edge, whatever was suggested.
    let raw = std::fs::read_to_string(c.node_file("search-ranking-decays-with-age")).unwrap();
    assert!(!raw.contains("edges:"), "suggesting never links:\n{raw}");
    let trace = c
        .run(&["trace", "--json", "search-ranking-decays-with-age"])
        .assert_ok()
        .stdout();
    let walk: serde_json::Value = serde_json::from_str(&trace).unwrap();
    assert_eq!(walk.as_array().unwrap().len(), 1, "{trace}");
    assert!(walk[0]["parents"].as_array().unwrap().is_empty(), "{trace}");

    // --quiet: the id and path alone.
    let entry = c
        .run(&["capture", "-q", "ranking decay, quietly"])
        .assert_ok()
        .stdout_trim();
    let out = c.run(&["promote", "--quiet", &entry]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");

    // A parent named is a decision made: nothing to suggest.
    let entry = c
        .run(&["capture", "-q", "ranking decay, parented"])
        .assert_ok()
        .stdout_trim();
    let out = c
        .run(&["promote", &entry, "--parent", "ranking-decay-half-life"])
        .assert_ok()
        .stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
}

/// A capture promoted without `--title` or `--id` is titled with the whole
/// sentence but takes a short id. A different thought colliding with it
/// falls back to a longer id from its own text; the same thought again is
/// refused, as it always was.
#[test]
fn promote_without_title_or_id_mints_a_short_id_from_a_long_capture() {
    let c = Corpus::new();
    let text = "gravity might be a scarcity gradient in some shared resource";
    let short = "gravity-scarcity-gradient-shared-resource";
    let full = "gravity-might-be-a-scarcity-gradient-in-some-shared-resource";

    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, short);
    let raw = std::fs::read_to_string(c.node_file(short)).unwrap();
    assert!(raw.contains(&format!("id: {short}\n")), "{raw}");
    assert!(raw.contains(&format!("title: {text}\n")), "{raw}");

    // The same sentence again: already a node, so refused by that node's
    // id rather than duplicated under a fallback, and the capture waits.
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    c.run(&["promote", "-q", &entry])
        .assert_fails()
        .says(&format!("node `{short}` already exists"));
    assert!(c.run(&["inbox"]).assert_ok().stdout().contains(&entry));
    assert!(!c.node_file(full).exists(), "no duplicate node");

    // A different thought sharing the first five significant words falls
    // back to one more word, and the node holding the short id is untouched.
    let entry = c
        .run(&["capture", "-q", &format!("{text} pool")])
        .assert_ok()
        .stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, format!("{short}-pool"));
    assert_eq!(
        std::fs::read_to_string(c.node_file(short)).unwrap(),
        raw,
        "the node holding the short id is untouched"
    );

    // `--title` and `--id` decide the id exactly as they always have.
    let entry = c
        .run(&["capture", "-q", "a thought"])
        .assert_ok()
        .stdout_trim();
    let id = c
        .run(&[
            "promote",
            "-q",
            &entry,
            "--title",
            "Gravity might be a scarcity gradient in a shared pool",
        ])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "gravity-might-be-a-scarcity-gradient-in-a-shared-pool");
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c
        .run(&["promote", "-q", &entry, "--id", "scarcity-gravity"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "scarcity-gravity");
}

#[test]
fn promote_json_is_the_created_node_with_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "decay of a taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&["promote", "--json", &entry, "--title", "Taxonomies decay"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["doc"]["node"]["id"], "taxonomies-decay", "{json}");
    assert_eq!(out["doc"]["body"], "decay of a taxonomy", "{json}");
    assert!(
        out["path"]
            .as_str()
            .unwrap()
            .ends_with("taxonomies-decay.md"),
        "{json}"
    );
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(
        near.iter().all(|n| n["id"] != "taxonomies-decay"),
        "a promotion is not its own neighbour: {json}"
    );
    assert_eq!(out["doc"]["node"]["edges"], serde_json::json!([]), "{json}");

    // With a parent, `near` is empty: present, never omitted.
    let entry = c
        .run(&["capture", "-q", "another taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&[
            "promote",
            "--json",
            &entry,
            "--parent",
            "a-single-global-taxonomy",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["near"], serde_json::json!([]), "{json}");
}

// ------------------------------------------------------------------ triage --

/// The node id `triage` printed for an entry it promoted.
fn promoted_as(out: &str, entry: &str) -> String {
    let prefix = format!("promoted {entry} -> ");
    out.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("`{entry}` was not promoted:\n{out}"))
        .to_string()
}

/// Where each needle first appears in `out`, so an order can be asserted.
fn position(out: &str, needle: &str) -> usize {
    out.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in:\n{out}"))
}

#[test]
fn triage_decides_each_entry_oldest_first_as_the_single_verbs_would() {
    let (c, _remote) = corpus_repo();
    lexical_fixture(&c);
    // Captured newest first, so file order and age order disagree.
    let later = c.run(&["capture", "-q", "later maybe"]).stdout_trim();
    let coffee = c.run(&["capture", "-q", "buy more coffee"]).stdout_trim();
    let tags = c
        .run(&["capture", "-q", "a taxonomy for tags"])
        .stdout_trim();
    let decay = c
        .run(&["capture", "-q", "ranking decay again"])
        .stdout_trim();
    set_inbox_stamp_for(&c.root, "ranking decay again", &stamp_days_ago(4));
    set_inbox_stamp_for(&c.root, "a taxonomy for tags", &stamp_days_ago(3));
    set_inbox_stamp_for(&c.root, "buy more coffee", &stamp_days_ago(2));
    set_inbox_stamp_for(&c.root, "later maybe", &stamp_days_ago(1));
    c.run(&["config", "commit", "on"]).assert_ok();

    // Root, the first candidate, drop, skip.
    let run = c.run_with_stdin(&["triage"], "p\n1\nd\ns\n").assert_ok();
    let out = run.stdout();
    assert!(
        position(&out, "ranking decay again") < position(&out, "a taxonomy for tags")
            && position(&out, "a taxonomy for tags") < position(&out, "buy more coffee")
            && position(&out, "buy more coffee") < position(&out, "later maybe"),
        "oldest first:\n{out}"
    );
    assert!(out.contains("[1/4]") && out.contains("[4/4]"), "{out}");
    assert!(out.contains("· 4 days"), "each entry shows its age:\n{out}");
    assert!(
        out.lines().any(|l| {
            let mut words = l.split_whitespace();
            words.next() == Some("1")
                && words
                    .next()
                    .is_some_and(|band| ["strong", "some", "weak"].contains(&band))
        }) && out.contains("a-single-global-taxonomy"),
        "the candidates are numbered, each with its band:\n{out}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("> ")) && !out.contains("q quit"),
        "no prompt or key legend when the keys are scripted:\n{out}"
    );

    let root = promoted_as(&out, &decay);
    let raw = std::fs::read_to_string(c.node_file(&root)).unwrap();
    assert!(!raw.contains("edges:"), "`p` promotes as a root:\n{raw}");
    let child = promoted_as(&out, &tags);
    let raw = std::fs::read_to_string(c.node_file(&child)).unwrap();
    assert!(
        raw.contains("- type: derives-from\n  to: a-single-global-taxonomy\n"),
        "`1` promotes under the first candidate, and only it:\n{raw}"
    );
    assert_eq!(raw.matches("- type:").count(), 1, "{raw}");
    assert!(out.contains(&format!("dropped {coffee}")), "{out}");
    assert!(out.contains(&format!("skipped {later}")), "{out}");
    assert!(
        out.contains("promoted 2, dropped 1, skipped 1; 1 still waiting"),
        "{out}"
    );

    // One commit per write, named as the single verb names its own, and
    // said on stderr: the session's screen is stdout.
    assert_eq!(run.stderr().matches("committed ").count(), 3, "{out}");
    assert!(!out.contains("committed "), "{out}");
    assert_eq!(
        log(&c.root)[..3],
        [
            format!("neb drop {coffee}"),
            format!("neb promote {tags} {child}"),
            format!("neb promote {decay} {root}"),
        ]
    );
    assert_only_corpus_paths_in_log(&c.root);
    let waiting = c.run(&["inbox"]).assert_ok().stdout();
    assert!(waiting.contains(&later), "{waiting}");
    for settled in [&decay, &tags, &coffee] {
        assert!(!waiting.contains(settled.as_str()), "{waiting}");
    }

    // `--no-commit` waives it for the whole session, as for one verb.
    let entry = c.run(&["capture", "-q", "one more"]).stdout_trim();
    let commits = log(&c.root).len();
    c.run_with_stdin(&["triage", "--no-commit"], "s\nd\n")
        .assert_ok();
    assert!(!c.run(&["inbox"]).stdout().contains(&entry));
    assert_eq!(log(&c.root).len(), commits);
}

#[test]
fn triage_skip_quit_and_end_of_input_leave_the_rest_waiting() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    let second = c.run(&["capture", "-q", "second thought"]).stdout_trim();
    let before = c.run(&["inbox"]).assert_ok().stdout();

    let out = c
        .run_with_stdin(&["triage"], "s\nq\nd\n")
        .assert_ok()
        .stdout();
    assert!(out.contains(&format!("skipped {first}")), "{out}");
    assert!(
        out.contains("promoted 0, dropped 0, skipped 1; 2 still waiting"),
        "`q` stops before the `d` after it:\n{out}"
    );
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A blank line decides nothing, and end of input stops as `q` does.
    let out = c.run_with_stdin(&["triage"], "\n").assert_ok().stdout();
    assert!(out.contains(&second) || out.contains(&first), "{out}");
    assert!(out.contains("skipped 0; 2 still waiting"), "{out}");
    assert_eq!(c.run(&["inbox"]).stdout(), before);
    let out = c.run_with_stdin(&["triage"], "").assert_ok().stdout();
    assert!(out.contains("skipped 0; 2 still waiting"), "{out}");
}

/// End of input stops as `q` does, except while a title waits to be used:
/// then the title would be lost without a word, so it is a refusal naming it
/// and the single verb that does the same without triage (STD-01 §R27).
#[test]
fn triage_eof_with_a_pending_title_is_an_error() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    c.run(&["capture", "-q", "second thought"]).assert_ok();
    let before = c.run(&["inbox"]).assert_ok().stdout();

    // `t` alone asks for the title on the next line, which never comes.
    let run = c.run_with_stdin(&["triage"], "t\n");
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.starts_with(&format!(
            "error: input ended before the title for `{first}` was given; nothing was promoted"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("neb promote {first} --title \"...\"")),
        "{stderr}"
    );
    assert!(!run.stdout().contains("still waiting"), "{}", run.stdout());
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A title set and never used is named, quoted for the shell.
    let run = c.run_with_stdin(&["triage"], "t A lost title\n");
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.contains(&format!("the title `A lost title` was used for `{first}`"))
            && stderr.contains(&format!("neb promote {first} --title 'A lost title'")),
        "{stderr}"
    );
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A title that was used is not pending, so end of input is `q` again.
    let run = c
        .run_with_stdin(&["triage"], "t A kept title\np\n")
        .assert_ok();
    assert!(run.stdout().contains("promoted 1"), "{}", run.stdout());
}

#[test]
fn triage_titles_a_promotion_and_attributes_it_with_by() {
    let c = Corpus::new();
    let a = c.run(&["capture", "-q", "the first words"]).stdout_trim();
    let b = c.run(&["capture", "-q", "the second words"]).stdout_trim();

    // `t` alone asks for the title on the next line; `t <title>` inlines it.
    let out = c
        .run_with_stdin(
            &["triage", "--by", "agent-x"],
            "t\nMy own title\np\nt Inline title\np\n",
        )
        .assert_ok()
        .stdout();
    assert_eq!(promoted_as(&out, &a), "my-own-title");
    assert_eq!(promoted_as(&out, &b), "inline-title");
    let raw = std::fs::read_to_string(c.node_file("my-own-title")).unwrap();
    assert!(raw.contains("title: My own title\n"), "{raw}");
    assert!(raw.contains("title_by: agent-x\n"), "{raw}");
    assert!(
        raw.contains("the first words"),
        "the capture is the body:\n{raw}"
    );
}

#[test]
fn triage_refuses_json_and_names_the_scriptable_verbs() {
    let c = Corpus::new();
    let entry = c.run(&["capture", "-q", "a thought"]).stdout_trim();
    for args in [["--json", "triage"], ["triage", "--json"]] {
        let run = c
            .run_with_stdin(&args, "d\n")
            .assert_fails()
            .says("interactive and has no JSON form")
            .says("neb inbox --json")
            .says("neb promote <entry>")
            .says("neb drop <entry>");
        assert!(run.stdout().is_empty(), "{}", run.stdout());
    }
    assert!(c.run(&["inbox"]).stdout().contains(&entry));
}

#[test]
fn a_piped_triage_refusal_ends_the_session_non_zero() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    let second = c.run(&["capture", "-q", "second thought"]).stdout_trim();

    // An unknown key: the lines after it were written for an entry that did
    // not move, so none of them runs.
    let run = c
        .run_with_stdin(&["triage"], "x\nd\n")
        .assert_fails()
        .says("`x` is not a triage key");
    assert!(!run.stdout().contains("dropped"), "{}", run.stdout());

    // A candidate the entry was not shown is refused, not guessed at.
    c.run_with_stdin(&["triage"], "3\nd\n")
        .assert_fails()
        .says("there is no candidate 3; this entry has no candidates");

    // A refusal after a decision keeps the decision.
    c.run_with_stdin(&["triage"], "d\nt !!!\np\ns\n")
        .assert_fails()
        .says(&format!("dropped {first}"))
        .says("does not reduce to a usable id");
    let waiting = c.run(&["inbox"]).stdout();
    assert!(!waiting.contains(&first), "{waiting}");
    assert!(waiting.contains(&second), "{waiting}");
}

#[test]
fn triage_on_an_empty_inbox_says_so() {
    let c = Corpus::new();
    c.run_with_stdin(&["triage"], "p\n")
        .assert_ok()
        .says("inbox is empty");
}

// ------------------------------------------------------------------- graph --

#[test]
fn graph_exports_the_whole_corpus_as_json() {
    let c = Corpus::new();
    let base = c.seed("base", "Base");
    let child = c.seed("child", "Child");
    c.run(&["link", &child, "derives-from", &base]).assert_ok();
    let rival = c.seed("rival", "Rival");
    c.run(&["link", &base, "contradicts", &rival]).assert_ok();

    let json = c.run(&["graph", "--json"]).assert_ok().stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    let nodes = out["nodes"].as_array().unwrap();
    let edges = out["edges"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{json}");
    assert!(
        nodes
            .iter()
            .any(|n| n["id"] == base && n["status"] == "seed")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["from"] == child && e["type"] == "derives-from" && e["to"] == base),
        "{json}"
    );
    // `contradicts` is written on both ends, so it is exported from both.
    assert_eq!(
        edges.iter().filter(|e| e["type"] == "contradicts").count(),
        2,
        "{json}"
    );

    let run = c.run(&["graph", "--from", "base", "--json"]);
    assert_eq!(run.out.status.code(), Some(2));
    run.assert_fails()
        .says("the argument '--from <ID>' cannot be used with '--json'");

    // `--json` is global, so it may come before the verb too.
    let before = c.run(&["--json", "graph"]).assert_ok().stdout();
    assert_eq!(before, json);
}

/// There is no default text form, so `graph` without a format is refused as
/// a usage error, on stderr like every other (STD-01 §R19, §R20).
#[test]
fn graph_without_a_format_is_a_usage_error_on_stderr() {
    let c = Corpus::new();
    c.seed("base", "Base");
    let run = c.run(&["graph"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    let stderr = run.stderr();
    assert_eq!(
        stderr,
        "error: neb graph needs an output format; pass --json or --mermaid\n"
    );
    for format in ["--json", "--mermaid"] {
        assert!(stderr.contains(format), "{stderr}");
    }
}

/// `--json` is a global flag, so `graph`'s formats conflict with it on either
/// side of the verb: the same usage error, exit 2, and nothing on stdout.
#[test]
fn graph_formats_refuse_json_before_or_after_the_verb() {
    let c = Corpus::new();
    let base = c.seed("base", "Base");

    for (before, after, says) in [
        (
            vec!["--json", "graph", "--mermaid"],
            vec!["graph", "--mermaid", "--json"],
            "the argument '--mermaid' cannot be used with '--json'",
        ),
        (
            vec!["--json", "graph", "--mermaid", "--from", &base],
            vec!["graph", "--mermaid", "--from", &base, "--json"],
            "the argument '--mermaid' cannot be used with '--json'",
        ),
        (
            vec!["--json", "graph", "--from", &base, "--mermaid"],
            vec!["graph", "--from", &base, "--mermaid", "--json"],
            "the argument '--from <ID>' cannot be used with '--json'",
        ),
    ] {
        let first = c.run(&before);
        let second = c.run(&after);
        for run in [&first, &second] {
            assert_eq!(run.out.status.code(), Some(2), "{}", run.args);
            assert_eq!(run.stdout(), "", "{}", run.args);
            assert!(
                run.stderr().starts_with(&format!("error: {says}\n")),
                "{}: {}",
                run.args,
                run.stderr()
            );
            assert!(
                run.stderr().contains("\nUsage: neb graph "),
                "{}",
                run.stderr()
            );
        }
        assert_eq!(first.stderr(), second.stderr(), "{}", first.args);
    }

    // A refused `--from` is a usage error before the verb too, never a
    // lookup of the node under `--json`.
    let run = c.run(&["--json", "graph", "--mermaid", "--from", "nope"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert!(!run.stderr().contains("NoSuchNode"), "{}", run.stderr());

    // `--no-commit` before the verb is not global, so it is no conflict
    // either: `graph` never commits, and that is what refuses it.
    let run = c.run(&["--no-commit", "graph", "--mermaid"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    assert!(
        run.stderr().contains("`graph` never commits"),
        "{}",
        run.stderr()
    );
}

#[test]
fn graph_exports_mermaid_and_limits_it_to_lineage() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "A \"quoted\" [ancestor] & <source>",
        "--id",
        "ancestor",
    ])
    .assert_ok();
    c.run(&[
        "new", "Focus", "--id", "focus", "--parent", "ancestor", "--kill", "evidence",
    ])
    .assert_ok();
    c.run(&[
        "new",
        "Descendant",
        "--id",
        "descendant",
        "--parent",
        "focus",
        "--kill",
        "evidence",
    ])
    .assert_ok();
    c.run(&[
        "status",
        "descendant",
        "refuted",
        "--why",
        "evidence arrived",
    ])
    .assert_ok();
    c.run(&["new", "Sibling", "--id", "sibling", "--parent", "ancestor"])
        .assert_ok();
    c.run(&["status", "sibling", "abandoned"]).assert_ok();
    c.run(&["new", "Unrelated", "--id", "unrelated"])
        .assert_ok();
    c.run(&["link", "ancestor", "contradicts", "focus"])
        .assert_ok();

    let whole = c.run(&["graph", "--mermaid"]).assert_ok().stdout();
    assert!(whole.starts_with("graph BT\n"), "{whole}");
    assert!(
        whole.contains(
            "ancestor[\"A &quot;quoted&quot; &#91;ancestor&#93; &amp; &lt;source&gt;\"]:::seed"
        ),
        "{whole}"
    );
    for status in ["seed", "hypothesis", "refuted", "abandoned"] {
        assert!(whole.contains(&format!("  classDef {status} ")), "{whole}");
    }
    assert_eq!(whole.matches("|contradicts|").count(), 1, "{whole}");
    assert_eq!(
        whole.lines().filter(|line| line.contains(":::")).count(),
        5,
        "{whole}"
    );
    assert_eq!(
        whole.lines().filter(|line| line.contains("| ")).count(),
        4,
        "{whole}"
    );

    let lineage = c
        .run(&["graph", "--mermaid", "--from", "focus"])
        .assert_ok()
        .stdout();
    for id in ["ancestor", "focus", "descendant"] {
        assert!(lineage.contains(&format!("  {id}[")), "{lineage}");
    }
    assert!(!lineage.contains("  sibling["), "{lineage}");
    assert!(!lineage.contains("  unrelated["), "{lineage}");
    assert_eq!(
        lineage.lines().filter(|line| line.contains(":::")).count(),
        3,
        "{lineage}"
    );
    assert_eq!(
        lineage.lines().filter(|line| line.contains("| ")).count(),
        3,
        "{lineage}"
    );
}

/// An unknown `--from` is core's `NoSuchNode`, with its message and hint,
/// rather than a code the CLI made up.
#[test]
fn mermaid_from_an_unknown_node_is_no_such_node() {
    let c = Corpus::new();
    c.run(&["new", "Something", "--id", "something"])
        .assert_ok();
    let run = c
        .run(&["graph", "--mermaid", "--from", "nope"])
        .assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(run.stdout(), "");
    run.says("no node `nope`").says("neb list");
}

#[test]
fn orbit_provenance_is_recorded_only_when_supplied() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "Orbit-produced idea",
        "--task",
        "ORB-12345",
        "--run",
        "jrun-20260907-0001",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("orbit-produced-idea")).unwrap();
    assert!(raw.contains("origin:\n  task: ORB-12345\n  run: jrun-20260907-0001"));

    c.run(&["new", "Unattributed idea"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file("unattributed-idea")).unwrap();
    assert!(!raw.contains("origin:"));

    let cited = c.seed("context", "Context");
    c.run(&[
        "cite",
        &cited,
        "--uri",
        "https://example.com",
        "--task",
        "ORB-12345",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&cited)).unwrap();
    assert!(raw.contains("references:"));
    assert!(raw.contains("task: ORB-12345"));

    let entry = c
        .run(&["capture", "promoted with provenance"])
        .stdout_trim();
    c.run(&[
        "promote",
        &entry,
        "--title",
        "Promoted with provenance",
        "--run",
        "jrun-20260907-0002",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("promoted-with-provenance")).unwrap();
    assert!(raw.contains("run: jrun-20260907-0002"));
}

// ------------------------------------------------------------------ --id --

#[test]
fn unicode_titles_derive_stable_ids_that_are_accepted_explicitly() {
    let c = Corpus::new();
    let accented = c
        .run(&["new", "Ünïcode título → ok"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(accented, "ünïcode-título-ok");

    let korean = c
        .run(&["new", "시간은 프레임의 수다"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(korean, "시간은-프레임의-수다");

    let explicit = Corpus::new();
    explicit
        .run(&["new", "Explicit Unicode id", "--id", "ünïcode-título-ok"])
        .assert_ok();
    assert!(explicit.node_file("ünïcode-título-ok").exists());
}

#[test]
fn new_and_promote_accept_an_explicit_id_overriding_the_slug() {
    let c = Corpus::new();
    let node = c
        .run(&[
            "new",
            "A title that would slugify to something else entirely",
            "--id",
            "short-id",
        ])
        .assert_ok()
        .stdout_trim();
    assert_eq!(node, "short-id");
    let raw = std::fs::read_to_string(c.node_file("short-id")).unwrap();
    assert!(raw.contains("id: short-id"));
    assert!(raw.contains("title: A title that would slugify to something else entirely"));

    let entry = c
        .run(&["capture", "promoted under a chosen id"])
        .stdout_trim();
    let node = c
        .run(&[
            "promote",
            &entry,
            "--title",
            "Promoted under a chosen id",
            "--id",
            "chosen-id",
        ])
        .assert_ok()
        .stdout_trim();
    assert_eq!(node, "chosen-id");
    assert!(c.node_file("chosen-id").exists());
}

#[test]
fn an_explicit_id_that_breaks_the_slug_rules_is_a_typed_refusal() {
    let c = Corpus::new();
    c.run(&["new", "Some idea", "--id", "Not-Lowercase"])
        .assert_fails()
        .says("not a valid id");
    c.run(&["new", "Some idea", "--id", "trailing-dash-"])
        .assert_fails()
        .says("not a valid id");
    c.run(&["new", "Some idea", "--id", "double--dash"])
        .assert_fails()
        .says("not a valid id");
    assert_eq!(
        std::fs::read_dir(c.root.join("nodes")).unwrap().count(),
        0,
        "a refused id should not leave a node behind"
    );
}

#[test]
fn an_explicit_id_that_collides_with_an_existing_node_is_a_typed_refusal() {
    let c = Corpus::new();
    c.run(&["new", "First idea", "--id", "taken"]).assert_ok();
    c.run(&["new", "Second idea", "--id", "taken"])
        .assert_fails()
        .says("already exists");
}

#[test]
fn a_node_can_descend_from_two_parents_and_trace_shows_the_diamond() {
    let c = Corpus::new();
    let a = c.seed("gravity might be about scarcity", "Gravity as scarcity");
    let b = c.seed(
        "a moving source should drag the field",
        "Moving source drag",
    );

    c.run(&[
        "new",
        "Retardation in the wake",
        "--parent",
        &a,
        "--parent",
        &b,
    ])
    .assert_ok();

    let tree = c
        .run(&["trace", "retardation-in-the-wake"])
        .assert_ok()
        .stdout();
    assert!(
        tree.contains(&a) && tree.contains(&b),
        "both parents belong in the trace:\n{tree}"
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_shared_ancestor_is_reached_by_both_branches_and_expanded_once() {
    let c = Corpus::new();
    let root_id = c.seed("space might have a density of something", "Density hunch");
    c.run(&["new", "Left branch", "--parent", &root_id])
        .assert_ok();
    c.run(&["new", "Right branch", "--parent", &root_id])
        .assert_ok();
    c.run(&[
        "new",
        "Synthesis",
        "--parent",
        "left-branch",
        "--parent",
        "right-branch",
    ])
    .assert_ok();

    // Piped, the walk is one line per node, so the shared ancestor is one
    // line; the tree a terminal gets points to it from both branches
    // (`render::tree`'s unit tests).
    let lines = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        traced_ids(&lines),
        ["synthesis", "left-branch", root_id.as_str(), "right-branch"],
        "the shared ancestor is reached once:\n{lines}"
    );
}

/// The ids of piped `trace` output, one per line, from its second field.
fn traced_ids(lines: &str) -> Vec<&str> {
    lines
        .lines()
        .map(|l| l.split('\t').nth(1).expect("an id field"))
        .collect()
}

#[test]
fn trace_names_edge_kinds_and_draws_parallel_edges_as_one_line() {
    let c = Corpus::new();
    let root = c.seed("gravity might be about scarcity", "Gravity as scarcity");
    c.run(&["new", "Left branch", "--parent", &root])
        .assert_ok();
    c.run(&["new", "Right branch", "--parent", &root])
        .assert_ok();
    c.run(&[
        "new",
        "Synthesis",
        "--parent",
        "left-branch",
        "--parent",
        "right-branch",
    ])
    .assert_ok();
    // A parallel edge: synthesis now both derives from and reopens `left-branch`.
    c.run(&["link", "synthesis", "reopens", "left-branch"])
        .assert_ok();

    // Piped, each line is `depth, id, status, title, kinds`.
    let steps = |lines: &str| -> Vec<(String, String, String)> {
        lines
            .lines()
            .map(|l| {
                let fields: Vec<&str> = l.split('\t').collect();
                assert_eq!(fields.len(), 5, "{l:?}");
                (fields[0].into(), fields[1].into(), fields[4].into())
            })
            .collect()
    };
    let step = |depth: &str, id: &str, kinds: &str| (depth.into(), id.into(), kinds.into());

    let up = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        steps(&up),
        [
            step("0", "synthesis", "-"),
            step("1", "left-branch", "derives-from,reopens"),
            step("2", &root, "derives-from"),
            step("1", "right-branch", "derives-from"),
        ],
        "every step names its edge, and the parallel pair is one step:\n{up}"
    );
    assert!(!up.contains("shown above"), "{up}");

    let down = c.run(&["trace", "--down", &root]).assert_ok().stdout();
    assert_eq!(
        steps(&down),
        [
            step("0", &root, "-"),
            step("1", "left-branch", "derives-from"),
            step("2", "synthesis", "derives-from,reopens"),
            step("1", "right-branch", "derives-from"),
        ],
        "descent names the same edges, and reaches the parallel pair once:\n{down}"
    );

    let json = c
        .run(&["trace", "--json", "synthesis"])
        .assert_ok()
        .stdout();
    let walk: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(walk[0]["via"].is_null(), "the start has no step: {json}");
    assert_eq!(
        walk[0]["parents"],
        serde_json::json!(["left-branch", "right-branch"]),
        "a parent is named once however many edges reach it: {json}"
    );
    assert_eq!(walk[1]["id"], "left-branch");
    assert_eq!(
        walk[1]["via"],
        serde_json::json!({ "from": "synthesis", "kinds": ["derives-from", "reopens"] }),
        "{json}"
    );
    assert_eq!(walk.as_array().unwrap().len(), 4, "{json}");
}

/// `root <- branch <- cut <- deep`, with `cut` also descending from `root`
/// directly: walking down, `cut` is two steps out through `branch` and one
/// step out on its own.
fn shortcut(c: &Corpus) {
    c.run(&["new", "Root"]).assert_ok();
    c.run(&["new", "Branch", "--parent", "root"]).assert_ok();
    c.run(&["new", "Cut", "--parent", "branch", "--parent", "root"])
        .assert_ok();
    c.run(&["new", "Deep", "--parent", "cut"]).assert_ok();
}

/// `--depth` bounds the walk and the JSON alike, stderr says how many nodes
/// it left out, and a node within reach along any path is kept.
#[test]
fn trace_depth_bounds_the_tree_and_the_json_alike() {
    let c = Corpus::new();
    shortcut(&c);
    let walked = |args: &[&str]| -> Vec<String> {
        let json: serde_json::Value =
            serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap();
        assert_eq!(json["total"], 4, "the whole walk: {json}");
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };

    let one = c
        .run(&["trace", "root", "--down", "--depth", "1"])
        .assert_ok();
    assert_eq!(traced_ids(&one.stdout()), ["root", "branch", "cut"]);
    assert_eq!(
        one.stderr(),
        "1 more node beyond --depth 1; raise --depth for more\n"
    );
    assert_eq!(
        walked(&["trace", "--json", "root", "--down", "--depth", "1"]),
        ["root", "branch", "cut"]
    );

    // Through `branch`, `deep` is three steps out; through the shortcut, two.
    let two = c
        .run(&["trace", "root", "--down", "--depth", "2"])
        .assert_ok();
    assert!(
        traced_ids(&two.stdout()).contains(&"deep"),
        "{}",
        two.stdout()
    );
    assert_eq!(two.stderr(), "", "nothing was left out");
    assert_eq!(
        walked(&["trace", "--json", "root", "--down", "--depth", "2"]).len(),
        4
    );

    for depth in ["1", "2", "3"] {
        let drawn = c
            .run(&["trace", "root", "--down", "--depth", depth])
            .assert_ok()
            .stdout();
        let listed = walked(&["trace", "--json", "root", "--down", "--depth", depth]);
        assert_eq!(
            traced_ids(&drawn),
            listed,
            "--depth {depth}: the lines and the JSON disagree:\n{drawn}"
        );
        assert!(!drawn.contains("--depth"), "{drawn}");
    }

    let up = c.run(&["trace", "deep", "--depth", "1"]).assert_ok();
    assert_eq!(traced_ids(&up.stdout()), ["deep", "cut"]);
    assert_eq!(
        up.stderr(),
        "2 more nodes beyond --depth 1; raise --depth for more\n"
    );
}

/// `--depth 0` would print the node alone, although the help says 1 is its
/// parents: it is a usage error, and the least depth still walks.
#[test]
fn trace_depth_zero_is_a_usage_error() {
    let c = Corpus::new();
    shortcut(&c);
    for json in [false, true] {
        let json = if json { ["--json"].as_slice() } else { &[] };
        let zero = c.run(&[json, &["trace", "root", "--depth", "0"]].concat());
        assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
        assert_eq!(zero.stdout(), "");
        assert!(
            zero.stderr().contains("--depth") && zero.stderr().contains("at least 1"),
            "{}",
            zero.stderr()
        );
    }
    let one = c
        .run(&["trace", "root", "--down", "--depth", "1"])
        .assert_ok();
    assert_eq!(traced_ids(&one.stdout()), ["root", "branch", "cut"]);
}

/// Without `--depth` the walk is whole, and a bound the corpus never reaches
/// prints exactly the same lines and no notice.
#[test]
fn trace_without_depth_is_unchanged_and_an_unreached_depth_matches_it() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [["trace", "root", "--down"].as_slice(), &["trace", "deep"]] {
        let whole = c.run(args).assert_ok();
        assert_eq!(whole.stderr(), "");
        let ids = traced_ids(&whole.stdout()).join(" ");
        assert!(ids.contains("root") && ids.contains("deep"), "{ids}");
        let bounded = c.run(&[args, &["--depth", "10"]].concat()).assert_ok();
        assert_eq!(bounded.stdout(), whole.stdout());
        assert_eq!(bounded.stderr(), "");
    }
}

/// Piped, `trace` draws no tree: one line per node with the same fields on
/// every line, and none of the box-drawing glyphs (STD-01 §R9).
#[test]
fn piped_trace_has_no_box_glyphs() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [
        ["trace", "deep"].as_slice(),
        &["trace", "root", "--down"],
        &["trace", "root", "--down", "--depth", "1"],
    ] {
        let out = c.run(args).assert_ok().stdout();
        assert!(!out.contains(['└', '├', '│', '─']), "{out}");
        assert!(!out.contains('\x1b'), "{out}");
        let fields: Vec<usize> = out.lines().map(|l| l.split('\t').count()).collect();
        assert!(!fields.is_empty(), "{out}");
        assert!(fields.iter().all(|n| *n == 5), "{out}");
    }
}

/// The lines `trace` prints are the payload `--json` returns: the same
/// nodes, whole or bounded (STD-01 §R6).
#[test]
fn trace_human_matches_payload() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [
        ["trace", "deep"].as_slice(),
        &["trace", "root", "--down"],
        &["trace", "root", "--down", "--depth", "1"],
        &["trace", "cut", "--down", "--depth", "1"],
    ] {
        let printed = c.run(args).assert_ok().stdout();
        let json: serde_json::Value = serde_json::from_str(
            &c.run(&[["--json"].as_slice(), args].concat())
                .assert_ok()
                .stdout(),
        )
        .unwrap();
        // Bounded, the walk is the capped-list envelope (STD-01 §R34).
        let walk = if json.is_object() {
            &json["items"]
        } else {
            &json
        };
        let mut from_payload: Vec<&str> = walk
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        let mut from_lines = traced_ids(&printed);
        from_payload.sort_unstable();
        from_lines.sort_unstable();
        assert_eq!(from_lines, from_payload, "{args:?}:\n{printed}");
    }
}

/// `trace --depth` answers in the envelope, with the size of the whole walk
/// as `total`; without the flag it is the bare array.
#[test]
fn trace_depth_json_envelope() {
    let c = Corpus::new();
    shortcut(&c);
    let cut = json_of(&c, &["trace", "--json", "root", "--down", "--depth", "1"]);
    assert_envelope(&cut, 3, 4, true);
    assert!(cut["items"][0]["handed_off_to"].is_null(), "{cut}");
    assert_envelope(
        &json_of(&c, &["trace", "--json", "root", "--down", "--depth", "10"]),
        4,
        4,
        false,
    );
    assert_envelope(
        &json_of(&c, &["trace", "--json", "deep", "--depth", "1"]),
        2,
        4,
        true,
    );
    let bare = json_of(&c, &["trace", "--json", "root", "--down"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

#[test]
fn genealogy_cycles_are_refused_at_the_point_of_linking() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    c.run(&["new", "Second", "--parent", &a]).assert_ok();
    c.run(&["link", &a, "derives-from", "second"])
        .assert_fails()
        .says("its own ancestor");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// A repeated edge is refused by name, both ends and the kind, so the
/// refusal says which edge; from `link` it points at the node that has it.
#[test]
fn duplicate_edge_refusal_names_the_edge() {
    let c = Corpus::new();
    let a = c.seed("alpha", "Alpha");
    let b = c.seed("beta", "Beta");
    c.run(&["link", &b, "derives-from", &a]).assert_ok();
    let run = c.run(&["link", &b, "derives-from", &a]);
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(
        run.stderr(),
        format!(
            "error: the edge `{b}` derives-from `{a}` already exists\n\n\
             See its edges with:  neb show {b}\n"
        )
    );
    let refused = c.run(&["--json", "link", &b, "derives-from", &a]).refusal();
    assert_eq!(refused["code"], "duplicate_edge");
    assert_eq!(
        refused["hint"],
        format!("See its edges with:  neb show {b}")
    );

    // From `new` the repeat is a flag given twice, and nothing is written.
    let run = c.run(&["new", "X", "--parent", &a, "--parent", &a]);
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.starts_with(&format!(
            "error: the edge `x` derives-from `{a}` already exists\n"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("`--parent {a}` is given twice")),
        "{stderr}"
    );
    assert!(!c.node_file("x").exists());
}

#[test]
fn contradicts_is_recorded_on_both_nodes() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["link", &a, "contradicts", &b]).assert_ok();
    c.run(&["show", &b])
        .assert_ok()
        .says("contradicts")
        .says(&a);
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn impact_reports_descendants_and_contradictions() {
    let c = Corpus::new();
    let base = c.seed("foundation", "Foundation");
    c.run(&["new", "Child", "--parent", &base]).assert_ok();
    c.run(&["new", "Grandchild", "--parent", "child"])
        .assert_ok();
    let rival = c.seed("the other way round", "Rival");
    c.run(&["link", &base, "contradicts", &rival]).assert_ok();
    let unrelated = c.seed("nothing to do with it", "Unrelated");

    let out = c.run(&["impact", &base]).assert_ok().stdout();
    assert!(out.contains("child") && out.contains("grandchild"), "{out}");
    assert!(out.contains(&rival), "{out}");
    assert!(!out.contains(&unrelated), "{out}");

    let json = c.run(&["--json", "impact", &base]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["id"] == "grandchild" && i["via"] == "descends"),
        "{json}"
    );
    assert!(
        items
            .iter()
            .any(|i| i["id"] == rival && i["via"] == "contradicts"),
        "{json}"
    );
    c.run(&["impact", &unrelated])
        .assert_ok()
        .says("nothing descends from or contradicts");
}

// -------------------------------------------------------------- discipline --

#[test]
fn a_hypothesis_must_name_what_would_kill_it() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("kill condition");
    c.run(&[
        "sharpen",
        &id,
        "--kill",
        "if the effect vanishes under control",
    ])
    .assert_ok();
    c.run(&["show", &id])
        .assert_ok()
        .says("hypothesis")
        .says("vanishes under control");
}

#[test]
fn refuting_needs_a_reason_and_writes_the_closed_block() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["status", &id, "refuted"])
        .assert_fails()
        .says("refuted needs --why");
    c.run(&["status", &id, "refuted", "--why", "  "])
        .assert_fails()
        .says("refuted needs --why");
    c.run(&["status", &id, "refuted", "--why", "X happened"])
        .assert_ok();

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("closed:\n  why: X happened\n  at: "), "{raw}");
    c.run(&["show", &id]).assert_ok().says("X happened");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // The reason is the invariant, not the flag: strip it by hand and
    // `check` catches the gap as rule 5.
    write(
        &c.node_file(&id),
        &raw.replace("  why: X happened\n", "  why: ''\n"),
    );
    c.run(&["check"])
        .assert_fails()
        .says("[5]")
        .says("closed.why is empty");

    // A seed cannot be refuted: there is no kill condition to have fired.
    let seed = c.seed("another idea", "Another idea");
    c.run(&["status", &seed, "refuted", "--why", "no"])
        .assert_fails()
        .says("kill condition");
    // And --why means nothing on an open status.
    c.run(&["status", &seed, "hypothesis", "--why", "no"])
        .assert_fails()
        .says("a reason only applies to refuted or abandoned")
        .says("Drop --why");
}

#[test]
fn abandoning_takes_an_optional_reason_and_reviving_clears_it() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&["status", &bare, "abandoned"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&bare)).unwrap();
    assert!(!raw.contains("closed:"), "{raw}");

    let reasoned = c.seed("another idea", "Another idea");
    c.run(&["status", &reasoned, "abandoned", "--why", "lost interest"])
        .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&reasoned)).unwrap();
    assert!(raw.contains("why: lost interest"), "{raw}");

    // Abandoned is not a verdict, so it can come back, and the closed block
    // goes with it. Its kill is not part of a recorded firing either, so
    // sharpening is allowed.
    c.run(&["sharpen", &bare, "--kill", "if Y"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&bare)).unwrap();
    assert!(raw.contains("kill: if Y"), "{raw}");
    assert!(raw.contains("status: abandoned"), "{raw}");
    c.run(&["status", &reasoned, "seed"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&reasoned)).unwrap();
    assert!(!raw.contains("closed:"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_node_with_a_kill_condition_cannot_go_back_to_seed() {
    let c = Corpus::new();
    c.run(&["new", "X", "--id", "x", "--kill", "k"]).assert_ok();

    // hypothesis -> seed would keep the kill, since nothing is deleted, and
    // `check` would then blame a hand edit for what the tool did.
    let before = std::fs::read_to_string(c.node_file("x")).unwrap();
    c.run(&["status", "x", "seed"])
        .assert_fails()
        .says("`x` names a kill condition, so it cannot go back to seed")
        .says("neb status x hypothesis");
    let after = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert_eq!(before, after, "a refused move leaves the node untouched");

    // abandoned -> seed is refused the same way.
    c.run(&["status", "x", "abandoned"]).assert_ok();
    let before = std::fs::read_to_string(c.node_file("x")).unwrap();
    c.run(&["status", "x", "seed"])
        .assert_fails()
        .says("cannot go back to seed")
        .says("neb status x hypothesis");
    let after = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert_eq!(before, after, "a refused move leaves the node untouched");

    // The remedy works, keeps the kill, and leaves the corpus check-clean.
    c.run(&["status", "x", "hypothesis"])
        .assert_ok()
        .says("abandoned -> hypothesis");
    let raw = std::fs::read_to_string(c.node_file("x")).unwrap();
    assert!(raw.contains("status: hypothesis"), "{raw}");
    assert!(raw.contains("kill: k"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

#[test]
fn a_refuted_idea_cannot_quietly_come_back() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    c.run(&["status", &id, "refuted", "--why", "X happened"])
        .assert_ok();

    // Reviving it takes a new node with a `reopens` edge, so the fact that it
    // was once ruled out stays visible in the graph. Not even abandoning it
    // is allowed: refuted is final.
    let refused = c
        .run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("cannot simply reopen");
    let hint = reopen_hint(&refused.stderr(), &id);
    c.run(&["status", &id, "abandoned"])
        .assert_fails()
        .says("cannot simply reopen");

    // Even refuted -> refuted is refused: a verdict is part of the record,
    // and a second `--why` would silently overwrite the first one's reason
    // and date rather than leaving the recorded verdict alone.
    let before_reverdict = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["status", &id, "refuted", "--why", "second verdict"])
        .assert_fails()
        .says("cannot simply reopen");
    let after_reverdict = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(
        before_reverdict, after_reverdict,
        "a refused refuted -> refuted leaves the node untouched"
    );
    assert!(
        after_reverdict.contains("why: X happened"),
        "{after_reverdict}"
    );

    // The kill is part of the verdict: rewriting it would leave closed.why
    // describing a falsifier the node no longer names. `--confirm` is the
    // same write path with different flags.
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["sharpen", &id, "--kill", "a different falsifier"])
        .assert_fails()
        .says(&format!("`{id}` is refuted"));
    c.run(&["sharpen", &id, "--confirm"])
        .assert_fails()
        .says(&format!("`{id}` is refuted"));
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(before, after, "a refused sharpen leaves the node untouched");
    assert!(after.contains("kill: if X"), "{after}");
    assert!(after.contains("why: X happened"), "{after}");
    c.run(&["check"]).assert_ok().says("0 errors");

    // The hint runs as printed once the placeholder title is filled in: no
    // id to copy out of one command's output into the next.
    let mut revive: Vec<&str> = hint.iter().map(String::as_str).collect();
    revive[1] = "Second attempt";
    revive.extend(["--kill", "if Y"]);
    c.run(&revive).assert_ok().says("second-attempt");
    let edges = show_edges(&c, "second-attempt");
    assert_eq!(edges, [("reopens".to_string(), id.clone())], "{edges:?}");
    c.run(&["show", "second-attempt"])
        .assert_ok()
        .says("hypothesis");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// The command the refused-reopen hint offers, as `neb` arguments, with the
/// title placeholder second. Asserts that the hint is one
/// command, not a chain, and that it names the refuted node.
fn reopen_hint(stderr: &str, id: &str) -> Vec<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("neb "))
        .unwrap_or_else(|| panic!("no command in the hint:\n{stderr}"));
    assert!(!line.contains("&&") && !line.contains('<'), "{line}");
    let words = shlex::split(line).expect("the hint parses as shell words");
    assert_eq!(words, ["neb", "new", "...", "--reopens", id], "{line}");
    words[1..].to_vec()
}

/// A node's edges as `(type, to)` pairs, read through `show --json`.
fn show_edges(c: &Corpus, id: &str) -> Vec<(String, String)> {
    let shown: serde_json::Value =
        serde_json::from_str(&c.run(&["show", id, "--json"]).assert_ok().stdout()).unwrap();
    shown["node"]["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("no edges in {shown}"))
        .iter()
        .map(|e| {
            (
                e["type"].as_str().unwrap().to_string(),
                e["to"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn new_writes_reopens_and_contradicts_edges_with_the_node() {
    let c = Corpus::new();
    let dead = c.seed("an idea", "An idea");
    c.run(&["sharpen", &dead, "--kill", "if X"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "X happened"])
        .assert_ok();
    let rival = c.seed("a rival", "A rival");
    let files = || std::fs::read_dir(c.root.join("nodes")).unwrap().count();

    // Every refusal comes before anything is written.
    for (args, says) in [
        (vec!["--reopens", "absent"], "no node `absent`"),
        (vec!["--contradicts", "absent"], "no node `absent`"),
        (
            vec!["--parent", &dead, "--reopens", &dead],
            "named as both a parent and the node this reopens",
        ),
        (
            vec!["--contradicts", &rival, "--contradicts", &rival],
            &format!("the edge `take-two` contradicts `{rival}` already exists"),
        ),
    ] {
        let mut argv = vec!["new", "Take two"];
        argv.extend(args);
        c.run(&argv).assert_fails().says(says);
        assert_eq!(files(), 2, "`neb {}` wrote nothing", argv.join(" "));
    }
    c.run(&["new", "Take two", "--parent", &dead, "--reopens", &dead])
        .assert_fails()
        .says(&format!("drop `--parent {dead}`"));

    c.run(&[
        "new",
        "Take two",
        "--reopens",
        &dead,
        "--contradicts",
        &rival,
    ])
    .assert_ok();
    assert_eq!(
        show_edges(&c, "take-two"),
        [
            ("reopens".to_string(), dead.clone()),
            ("contradicts".to_string(), rival.clone())
        ]
    );
    // `contradicts` is a claim about both nodes, as `link` records it.
    assert_eq!(
        show_edges(&c, &rival),
        [("contradicts".to_string(), "take-two".to_string())]
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn new_refuses_a_reopens_edge_that_closes_a_loop() {
    let c = Corpus::new();
    let dead = c.seed("an idea", "An idea");
    // A hand edit left a dangling edge to a node that does not exist yet.
    let path = c.node_file(&dead);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen(
            "status: seed\n",
            "status: seed\nedges:\n- type: derives-from\n  to: take-two\n",
            1,
        ),
    );
    c.run(&["new", "Take two", "--reopens", &dead])
        .assert_fails()
        .says("its own ancestor");
    assert!(!c.node_file("take-two").exists());
}

#[test]
fn a_reference_cannot_smuggle_in_a_verdict() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace(
        "status: seed",
        "status: seed\nreferences:\n- id: r1\n  kind: paper\n  uri: http://example.com\n  added: 2026-09-06\n  verdict: supports",
    );
    write(&c.node_file(&id), &raw);
    // The separation between context and evidence is the discipline the whole
    // system exists to impose, so it fails to parse rather than merely warning.
    c.run(&["check"])
        .assert_fails()
        .says("unknown field `verdict`");
}

#[test]
fn cite_accepts_the_documented_kinds_and_refuses_other_values() {
    const ACCEPTED: [&str; 10] = [
        "paper",
        "study",
        "article",
        "note",
        "discussion",
        "book",
        "dataset",
        "thread",
        "observatory",
        "other",
    ];
    const ACCEPTED_MESSAGE: &str = "accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other";

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for kind in ACCEPTED {
        let uri = if kind == "observatory" {
            "Q002"
        } else {
            "https://example.org"
        };
        c.run(&[
            "cite", "--kind", kind, "--uri", uri, "--note", "context", &id,
        ])
        .assert_ok();
    }
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    for kind in ACCEPTED {
        assert!(
            raw.contains(&format!("  kind: {kind}")),
            "{kind} missing from:\n{raw}"
        );
    }

    // Case is not a second kind: it is lowercased, as tags are, and only
    // then checked against the vocabulary.
    for (given, uri) in [
        ("Paper", "https://example.org/case"),
        ("Observatory", "q003"),
    ] {
        c.run(&[
            "cite", "--kind", given, "--uri", uri, "--note", "context", &id,
        ])
        .assert_ok();
    }
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        raw.contains("  kind: paper\n  uri: https://example.org/case\n"),
        "{raw}"
    );
    assert!(raw.contains("  kind: observatory\n  uri: Q003\n"), "{raw}");
    assert!(
        !raw.contains("Paper") && !raw.contains("Observatory"),
        "{raw}"
    );

    let before = raw;
    for kind in ["bogus", "VERDICT", "Bogus", "", "not a kind"] {
        c.run(&[
            "cite",
            "--kind",
            kind,
            "--uri",
            "https://example.org/unexpected",
            "--note",
            "context",
            &id,
        ])
        .assert_fails()
        .says(ACCEPTED_MESSAGE);
    }
    assert_eq!(
        before,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "refused kinds must not change the node"
    );
}

#[test]
fn check_warns_about_a_legacy_unexpected_reference_kind_without_rejecting_the_file() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "paper",
        "--uri",
        "https://example.org",
        "--note",
        "context",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("  kind: paper\n", "  kind: bogus\n"),
    );

    let shown = c.run(&["show", "--json", &id]).assert_ok().stdout();
    let node: serde_json::Value = serde_json::from_str(&shown).expect("show --json is valid JSON");
    assert_eq!(node["node"]["references"][0]["kind"], "bogus");
    c.run(&["check"])
        .assert_ok()
        .says("[16]")
        .says("reference `r1` has unexpected kind `bogus`")
        .says("accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other")
        .says("0 errors, 1 warning");
}

#[test]
fn a_reference_with_no_note_warns_and_a_noted_one_does_not() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &bare,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "",
    ])
    .assert_ok();
    c.run(&["check"])
        .assert_ok()
        .says("[10]")
        .says("reference `r1` has no note saying why it is here")
        .says(&bare)
        .says("0 errors, 1 warning");

    let omitted = Corpus::new();
    let omitted_id = omitted.seed("another idea", "Another idea");
    omitted
        .run(&[
            "cite",
            &omitted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
        ])
        .assert_ok();
    omitted
        .run(&["check"])
        .assert_ok()
        .says("[10]")
        .says("0 errors, 1 warning");

    let noted = Corpus::new();
    let noted_id = noted.seed("a third idea", "A third idea");
    noted
        .run(&[
            "cite",
            &noted_id,
            "--uri",
            "https://example.org",
            "--kind",
            "paper",
            "--note",
            "explains the mechanism",
        ])
        .assert_ok();
    noted
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

#[test]
fn a_discussion_reference_may_omit_its_uri_but_other_kinds_may_not() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");

    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussion",
        "--note",
        "came from the ideation session",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("kind: discussion"), "{raw}");
    assert!(
        !raw.contains("uri:"),
        "URI-less discussion should omit uri:\n{raw}"
    );
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    c.run(&["cite", &id, "--kind", "paper", "--note", "missing URI"])
        .assert_fails()
        .says("a reference of kind `paper` needs a URI")
        .says("--uri");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussions",
        "--note",
        "missing URI",
    ])
    .assert_fails()
    .says("a reference of kind `discussions` needs a URI");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "paper",
        "--uri",
        "",
        "--note",
        "empty URI",
    ])
    .assert_fails()
    .says("a reference of kind `paper` needs a URI");

    // The kind's case is normalised before it decides whether a URI is
    // needed, so `Discussion` is a discussion.
    c.run(&["cite", &id, "--kind", "Discussion"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(!raw.contains("Discussion"), "{raw}");
    c.run(&["check"])
        .assert_ok()
        .says("reference `r2` has no note saying why it is here")
        .says("0 errors, 1 warning");

    let invalid = Corpus::new();
    let invalid_id = invalid.seed("another idea", "Another idea");
    invalid
        .run(&[
            "cite",
            &invalid_id,
            "--kind",
            "paper",
            "--uri",
            "https://example.org",
            "--note",
            "context",
        ])
        .assert_ok();
    let raw = std::fs::read_to_string(invalid.node_file(&invalid_id)).unwrap();
    write(
        &invalid.node_file(&invalid_id),
        &raw.replace("  uri: https://example.org\n", "  uri: ''\n"),
    );
    invalid
        .run(&["check"])
        .assert_fails()
        .says("kind `paper` but no URI")
        .says("1 error, 0 warnings");
}

/// A citation with no URI, or a blank one, has its own code. Core's message
/// names no flag, since the desktop has none; the CLI's hint names `--uri`.
#[test]
fn cite_without_uri_has_a_specific_code() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for args in [
        vec!["--json", "cite", &id, "--kind", "paper"],
        vec!["--json", "cite", &id, "--kind", "paper", "--uri", "  "],
    ] {
        let refused = c.run(&args).usage_refusal();
        assert_eq!(refused["code"], "uri_required", "{args:?}: {refused}");
        let said = refused["error"].as_str().unwrap();
        assert!(!said.contains("--"), "a flag in core's message: {said}");
        assert!(
            refused["hint"].as_str().unwrap().contains("--uri"),
            "{refused}"
        );
    }
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

#[test]
fn a_local_uri_must_resolve_and_external_urls_never_trip_it() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    // Rule 8 at the point of action: `cite` refuses a path to nothing.
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_fails()
        .says("does not resolve");

    std::fs::create_dir_all(c.root.join("nodes").join("notes")).unwrap();
    write(
        &c.root.join("nodes").join("notes").join("missing.md"),
        "here now",
    );
    c.run(&["cite", &id, "--uri", "./notes/missing.md", "--note", "n"])
        .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // And in `check`, for the file that went missing later: an error, since
    // a citation to nothing is a broken record rather than an untidy one.
    std::fs::remove_file(c.root.join("nodes").join("notes").join("missing.md")).unwrap();
    c.run(&["check"])
        .assert_fails()
        .says("[8]")
        .says("points at a path that does not resolve: ./notes/missing.md")
        .says("1 error,");

    // Schemes and URLs are never resolved, so none of these trip rule 8.
    let online = Corpus::new();
    let online_id = online.seed("an online idea", "An online idea");
    for uri in [
        "https://example.org/paper",
        "doi:10.1000/x",
        "orbit:DANI-10345",
        "[[almanac/some-page]]",
    ] {
        online
            .run(&["cite", &online_id, "--uri", uri, "--note", "n"])
            .assert_ok();
    }
    online
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

/// Rule 8 at `cite`: a path that is absolute here names nothing on the other
/// machines the corpus is synced to, so it is refused even when it exists,
/// and the refusal says what to write instead. The relative spelling of the
/// same file is accepted.
#[test]
fn cite_refuses_an_absolute_local_path_even_when_it_exists() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let beside = c.root.join("studies").join("x.md");
    std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
    write(&beside, "a study");
    let absolute = beside.to_str().unwrap().to_string();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    for uri in [
        absolute.clone(),
        format!("file://{absolute}"),
        format!("FILE://{absolute}"),
        format!("file:{absolute}"),
        "/nonexistent/x.md".to_string(),
        "C:/studies/x.md".to_string(),
    ] {
        c.run(&["cite", &id, "--kind", "study", "--uri", &uri, "--note", "n"])
            .assert_fails()
            .says(&format!(
                "`{uri}` is an absolute local path; local references are relative to nodes/"
            ))
            .says("Cite it by a path relative to nodes/")
            .says(&format!(
                "neb cite {id} --kind observatory --uri <record-id>"
            ));
    }
    assert_eq!(
        before,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "a refused citation must not change the node"
    );

    c.run(&[
        "cite",
        &id,
        "--kind",
        "study",
        "--uri",
        "../studies/x.md",
        "--note",
        "n",
    ])
    .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// Rule 8 in `check`: an absolute path already in the corpus — hand written,
/// or carried over from a v1 `evidence` source — still loads, and is a
/// warning whether or not it resolves on this machine, since resolving here
/// is what cannot be judged. URLs and scheme handles never trip it.
#[test]
fn check_warns_about_an_absolute_local_path_already_in_the_corpus() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for uri in [
        "https://example.org/a",
        "http://example.org/b",
        "mailto:someone@example.org",
        "orbit:ORB-13049",
        "neb:another-idea",
    ] {
        c.run(&["cite", &id, "--uri", uri, "--note", "n"])
            .assert_ok();
    }
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    let existing = c.root.join("config.yaml");
    let existing = existing.to_str().unwrap();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace(
            "  uri: https://example.org/a\n",
            &format!("  uri: {existing}\n"),
        )
        .replace(
            "  uri: http://example.org/b\n",
            "  uri: file:///nonexistent/x.md\n",
        ),
    );

    let shown = c.run(&["show", "--json", &id]).assert_ok().stdout();
    let node: serde_json::Value = serde_json::from_str(&shown).expect("show --json is valid JSON");
    assert_eq!(node["node"]["references"][0]["uri"], existing);
    c.run(&["check"])
        .assert_ok()
        .says("[8]")
        .says(&format!(
            "reference `r1` uses an absolute local path, which resolves on this machine only: \
             {existing}"
        ))
        .says(
            "reference `r2` uses an absolute local path, which resolves on this machine only: \
             file:///nonexistent/x.md",
        )
        .says("0 errors, 2 warnings");

    let out = c.run(&["--json", "check"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json is valid JSON");
    let findings = v["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 2, "{findings:?}");
    for f in findings {
        assert_eq!(f["rule"], 8);
        assert_eq!(f["level"], "warn");
        assert_eq!(f["node"], id);
    }
}

/// Rule 17: a temporary file an interrupted write left behind is named, with
/// the command that removes it, and left exactly where it is.
#[test]
fn check_warns_about_stale_temp_files() {
    let c = Corpus::new();
    let debris = plant_debris(&c);

    let text = c
        .run(&["check"])
        .assert_ok()
        .says("0 errors, 2 warnings")
        .stdout();
    for name in [
        "nodes/x.md.1f2e-0-18d8.tmp",
        "inbox/2026-09.md.1f2e-0-18d8.tmp",
    ] {
        text.lines()
            .find(|line| line.contains(name))
            .filter(|line| line.contains("[17]") && line.contains("rm '"))
            .unwrap_or_else(|| panic!("no rule-17 warning for {name}:\n{text}"));
    }

    let json = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let findings = report["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{findings:?}");
    for (finding, path) in findings.iter().zip(&debris) {
        assert_eq!(finding["rule"], 17);
        assert_eq!(finding["level"], "warn");
        let message = finding["message"].as_str().unwrap();
        assert!(
            message.contains(&format!("rm '{}'", path.display())),
            "{message}"
        );
    }
    for path in &debris {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "half of a write that never landed\n",
            "check never deletes or rewrites debris"
        );
    }
}

#[test]
fn check_json_exposes_rule_and_level() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\ntags:\n- Sims\n- sim"),
    );

    let out = c.run(&["--json", "check"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("check --json is valid JSON");
    let findings = v["findings"].as_array().expect("findings array");

    let rule10 = findings
        .iter()
        .find(|f| f["rule"] == 10)
        .expect("rule 10 finding present");
    assert_eq!(rule10["level"], "warn");
    assert_eq!(rule10["node"], id);

    let rule11 = findings
        .iter()
        .find(|f| f["rule"] == 11)
        .expect("rule 11 finding present");
    assert_eq!(rule11["level"], "warn");
    assert!(rule11["node"].is_null(), "tag drift is a corpus finding");
}

fn corpus_with_two_bad_nodes() -> (Corpus, String, String) {
    let c = Corpus::new();
    let first = c.seed("first thought", "First thought");
    let second = c.seed("second thought", "Second thought");
    let raw = std::fs::read_to_string(c.node_file(&first)).unwrap();
    write(
        &c.node_file(&first),
        &raw.replacen(
            "status: seed\n",
            &format!("status: seed\nedges:\n- type: contradicts\n  to: {second}\n"),
            1,
        ),
    );
    write(&c.node_file("broken"), "---\nid: broken\n");
    write(&c.node_file("zzz-bad"), "no frontmatter\n");
    (c, first, second)
}

#[test]
fn check_reports_every_unparsable_node_and_checks_the_rest() {
    let (c, _, _) = corpus_with_two_bad_nodes();
    let run = c.run(&["--json", "check"]).assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    assert_eq!(report["nodes"], 2);
    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 2);
    for name in ["broken.md", "zzz-bad.md"] {
        let item = unreadable
            .iter()
            .find(|item| item["path"].as_str().unwrap().ends_with(name))
            .unwrap();
        assert_eq!(item["code"], "malformed_frontmatter", "{item}");
        assert!(item["message"].is_string(), "{item}");
    }
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == 4),
        "{report}"
    );
    let human = c.run(&["check"]).assert_fails();
    let text = human.stdout();
    assert!(text.find("broken.md") < text.find("contradicts"), "{text}");
    assert!(human.stderr().contains("2 unreadable files"));
}

#[test]
fn check_reports_an_unknown_field_as_an_unreadable_node() {
    let c = Corpus::new();
    let first = c.seed("first thought", "First thought");
    c.seed("second thought", "Second thought");
    let raw = std::fs::read_to_string(c.node_file(&first)).unwrap();
    write(
        &c.node_file(&first),
        &raw.replace("status: seed", "status: seed\nfuture_field: unknown"),
    );
    let run = c.run(&["--json", "check"]).assert_fails();
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    assert_eq!(report["nodes"], 1);
    let unreadable = report["unreadable"].as_array().unwrap();
    assert_eq!(unreadable.len(), 1);
    assert!(
        unreadable[0]["path"]
            .as_str()
            .unwrap()
            .ends_with(&format!("{first}.md"))
    );
    assert!(
        unreadable[0]["message"]
            .as_str()
            .unwrap()
            .contains("future_field")
    );
}

#[test]
fn strict_verbs_name_every_unreadable_node() {
    let (c, first, _) = corpus_with_two_bad_nodes();
    for args in [vec!["list"], vec!["show", &first], vec!["trace", &first]] {
        let mut full = vec!["--json"];
        full.extend(args);
        let refusal = c.run(&full).refusal();
        assert_eq!(refusal["code"], "unreadable_nodes");
        let error = refusal["error"].as_str().unwrap();
        assert!(error.contains("2 unreadable"), "{error}");
        assert!(
            error.contains("broken.md") && error.contains("zzz-bad.md"),
            "{error}"
        );
        assert!(refusal["hint"].as_str().unwrap().contains("neb check"));
    }
}

#[test]
fn check_reports_a_broken_observatory_setting_as_a_finding() {
    let c = Corpus::new();
    let id = c.seed("a thought", "A thought");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replacen(
            "status: seed\n",
            "status: seed\nedges:\n- type: derives-from\n  to: ghost\n",
            1,
        ),
    );
    let setting = c.workdir().join(".config/nebula/observatory-root");
    std::fs::create_dir_all(setting.parent().unwrap()).unwrap();
    write(&setting, "relative-observatory\n");
    let run = c.run(&["--json", "check"]).assert_fails();
    let report: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    let findings = report["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["level"] == "error"
            && f["message"]
                .as_str()
                .unwrap()
                .contains(&setting.display().to_string())),
        "{report}"
    );
    assert!(findings.iter().any(|f| f["rule"] == 3), "{report}");
    assert!(!run.stdout().contains("no observatory root is set"));
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::print_stderr,
    reason = "report why the unreadable-directory test is skipped as root"
)]
fn an_unreadable_observatory_directory_is_not_reported_as_unresolved() {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root can read mode-000 directories");
        return;
    }
    let c = Corpus::new();
    let id = c.seed("a thought", "A thought");
    let obs = observatory(c.workdir());
    let questions = obs.join("questions");
    let env = [("OBSERVATORY_ROOT", obs.to_str().unwrap())];
    c.run_with_env(
        &["cite", &id, "--uri", "Q002", "--kind", "observatory"],
        &env,
    )
    .assert_ok();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let mode = std::fs::metadata(&questions).unwrap().permissions();
    std::fs::set_permissions(&questions, std::fs::Permissions::from_mode(0o000)).unwrap();
    let checked = c.run_with_env(&["--json", "check"], &env).assert_fails();
    let refused = c
        .run_with_env(&["--json", "handoff", &id, "Q002"], &env)
        .refusal();
    std::fs::set_permissions(&questions, mode).unwrap();
    let report: serde_json::Value = serde_json::from_str(&checked.stdout()).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |f| f["message"].as_str().unwrap().contains("could not read")
                    && f["message"].as_str().unwrap().contains("questions")
            ),
        "{report}"
    );
    assert!(!checked.stdout().contains("does not resolve"));
    assert_eq!(refused["code"], "io_at");
    assert!(refused["error"].as_str().unwrap().contains("questions"));
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

#[test]
fn dangling_edges_are_caught() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let mut raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    raw = raw.replace(
        "status: seed",
        "status: seed\nedges:\n- type: derives-from\n  to: ghost",
    );
    write(&c.node_file(&id), &raw);
    c.run(&["check"]).assert_fails().says("missing node");
}

/// `status` always clears `closed` the moment a node leaves refuted or
/// abandoned, so a `closed` block on a seed or hypothesis is not a state any
/// verb produces — only a hand edit that reopened the node outside `status`
/// leaves one behind.
#[test]
fn a_closed_block_on_an_open_node_is_a_rule_12_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace(
            "status: hypothesis",
            "status: hypothesis\nclosed:\n  why: it was abandoned once\n  at: 2026-09-01",
        ),
    );
    c.run(&["check"])
        .assert_fails()
        .says("[12]")
        .says("status is `hypothesis` but a `closed` block is still set")
        .says("1 error,");
}

/// `new --kill` and `sharpen` always move status to `hypothesis` together
/// with writing `kill`, so a `seed` carrying one was set by hand without the
/// guard. Not wrong by itself, so it is a warning rather than an error.
#[test]
fn a_seed_with_a_kill_condition_is_a_rule_13_warning() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    write(
        &c.node_file(&id),
        &raw.replace("status: seed", "status: seed\nkill: if X"),
    );
    c.run(&["check"])
        .assert_ok()
        .says("[13]")
        .says("status is seed but a kill condition is set")
        .says("0 errors, 1 warning");
}

/// `ops.rs` only ever stamps `created`/`updated` from `store::today()`, so
/// either field failing to parse, or `updated` landing before `created`, is
/// a hand edit — and `review`/`open` then silently treat the node as never
/// stale, since `days_since` returns `None` for a date it cannot parse.
#[test]
fn an_unparsable_created_or_updated_date_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), "not-a-date");
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("created `not-a-date` is not a YYYY-MM-DD date")
        .says("1 error,");
}

/// Calendar-looking strings must still name real Gregorian dates. Otherwise
/// hand edits evade rule 14 while `review` and `open` silently ignore them.
#[test]
fn impossible_calendar_dates_are_rule_14_errors() {
    for (field, date) in [
        ("created", "2026-99-99"),
        ("updated", "2026-04-31"),
        ("created", "2026-02-29"),
    ] {
        let c = Corpus::new();
        let id = c.seed("an idea", "An idea");
        match field {
            "created" => set_created(&c.node_file(&id), date),
            "updated" => set_updated(&c.node_file(&id), date),
            _ => unreachable!("test fields are explicit"),
        }
        c.run(&["check"])
            .assert_fails()
            .says("[14]")
            .says(&format!("{field} `{date}` is not a YYYY-MM-DD date"))
            .says("1 error,");
    }

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "2026-02-29");
    write(&c.node_file(&id), &raw);
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("reference `r1` has an added date `2026-02-29` that does not parse")
        .says("1 error,");
}

#[test]
fn leap_day_dates_are_valid_for_nodes_and_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), "2024-02-29");
    set_updated(&c.node_file(&id), "2024-02-29");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "2024-02-29");
    write(&c.node_file(&id), &raw);
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn updated_earlier_than_created_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    set_created(&c.node_file(&id), &date_days_ago(1));
    set_updated(&c.node_file(&id), &date_days_ago(2));
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("updated")
        .says("earlier than created")
        .says("1 error,");
}

/// A reference's `added` date is stamped the same way and can go wrong the
/// same way, so rule 14 covers it too.
#[test]
fn an_unparsable_reference_added_date_is_a_rule_14_error() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.org",
        "--kind",
        "paper",
        "--note",
        "n",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let needle = "\n  added: ";
    let start = raw.find(needle).expect("added: line") + needle.len();
    let end = start + 10;
    let mut raw = raw;
    raw.replace_range(start..end, "not-a-date");
    write(&c.node_file(&id), &raw);
    c.run(&["check"])
        .assert_fails()
        .says("[14]")
        .says("reference `r1` has an added date `not-a-date` that does not parse")
        .says("1 error,");
}

// --------------------------------------------------------- --json refusals --

/// Under `--json` a refusal is data: one envelope on stderr, stdout left to
/// the payload, and the exit code it always had. Without `--json` nothing
/// changed, byte for byte.
#[test]
fn a_json_refusal_is_one_envelope_on_stderr_and_the_prose_is_unchanged() {
    let c = Corpus::new();
    let run = c.run(&["show", "nope", "--json"]);
    assert_eq!(
        run.refusal(),
        serde_json::json!({
            "error": "no node `nope`",
            "code": "no_such_node",
            "hint": "List what exists with:  neb list",
        })
    );
    assert!(run.stdout().is_empty(), "{}", run.stdout());

    let run = c.run(&["show", "nope"]).assert_fails();
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(
        run.stderr(),
        "error: no node `nope`\n\nList what exists with:  neb list\n"
    );
    assert!(run.stdout().is_empty(), "{}", run.stdout());
}

/// Each point-of-action guard reports its core variant as `code`.
#[test]
fn json_refusals_name_the_link_or_cite_that_was_refused() {
    let c = Corpus::new();
    let a = c.seed("an idea", "An idea");
    let b = c
        .run(&["new", "A child", "--parent", &a])
        .assert_ok()
        .stdout_trim();

    let self_loop = c
        .run(&["--json", "link", &a, "refines", &a])
        .usage_refusal();
    assert_eq!(self_loop["code"], "self_loop");
    assert_eq!(self_loop["error"], "a node cannot link to itself");
    assert_eq!(self_loop["hint"], serde_json::Value::Null);

    let cycle = c.run(&["--json", "link", &a, "derives-from", &b]).refusal();
    assert_eq!(cycle["code"], "cycle");
    assert_eq!(
        cycle["error"],
        format!("that edge would make `{a}` its own ancestor")
    );

    let unknown = c
        .run(&[
            "--json",
            "cite",
            &b,
            "--kind",
            "bogus",
            "--uri",
            "https://example.org",
            "--note",
            "n",
        ])
        .usage_refusal();
    assert_eq!(unknown["code"], "unknown_reference_kind");
    assert_eq!(
        unknown["error"],
        "`bogus` is not an accepted reference kind; accepted kinds: paper, study, article, note, discussion, book, dataset, thread, observatory, other"
    );

    let unresolved = c
        .run(&[
            "--json",
            "cite",
            &b,
            "--uri",
            "./notes/missing.md",
            "--note",
            "n",
        ])
        .refusal();
    assert_eq!(unresolved["code"], "unresolved_uri");
    let message = unresolved["error"].as_str().unwrap();
    assert!(
        message.starts_with("`./notes/missing.md` does not resolve from")
            && message.ends_with("local references are relative to nodes/"),
        "{message}"
    );
}

/// The guards whose hint names the node report a clean message and hint
/// under `--json`, and keep their prose exactly without it.
#[test]
fn json_refusals_about_a_node_split_message_and_hint() {
    // (arguments, exit, code, message, hint, the prose without --json)
    type Guard<'a> = (&'a [&'a str], i32, &'a str, String, String, String);
    let c = Corpus::new();
    let a = c.seed("an idea", "An idea");
    let b = c.seed("a child", "A child");
    c.run(&["sharpen", &b, "--kill", "if X"]).assert_ok();
    let dead = c.seed("a dead idea", "A dead idea");
    c.run(&["sharpen", &dead, "--kill", "if Y"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "Y happened"])
        .assert_ok();
    let guards: [Guard; 3] = [
        (
            &["status", &a, "hypothesis"],
            1,
            "needs_kill",
            "`hypothesis` needs a kill condition first".to_string(),
            format!("neb sharpen {a} --kill \"...\""),
            format!(
                "error: `hypothesis` needs a kill condition first:\n\n  neb sharpen {a} --kill \"...\"\n"
            ),
        ),
        (
            &["status", &b, "refuted"],
            2,
            "refuted_needs_why",
            "refuted needs --why: say how the kill condition fired".to_string(),
            format!("neb status {b} refuted --why \"...\""),
            format!(
                "error: refuted needs --why: say how the kill condition fired\n\n  neb status {b} refuted --why \"...\"\n"
            ),
        ),
        (
            &["status", &dead, "seed"],
            1,
            "refuted_cannot_reopen",
            format!("`{dead}` is refuted and cannot simply reopen"),
            // The hint is the one command to run, and nothing else.
            format!("neb new \"...\" --reopens {dead}"),
            format!(
                "error: `{dead}` is refuted and cannot simply reopen.\n\nRevive it as a new node that reopens it:\n  neb new \"...\" --reopens {dead}\n"
            ),
        ),
    ];
    for (args, exit, code, message, hint, prose) in guards {
        let mut with_json = vec!["--json"];
        with_json.extend_from_slice(args);
        let refused = c.run(&with_json).refusal_exiting(exit);
        assert_eq!(
            refused,
            serde_json::json!({"error": message, "code": code, "hint": hint}),
            "neb {}",
            args.join(" ")
        );
        let run = c.run(args).assert_fails();
        assert_eq!(run.out.status.code(), Some(exit));
        assert_eq!(run.stderr(), prose);
    }
}

/// A schema this build does not read is one kind either way; the hint says
/// which way to go.
#[test]
fn json_refusals_for_a_schema_too_old_or_too_new_carry_the_direction_in_the_hint() {
    let old = v1_corpus();
    let refused = old.run(&["list", "--json"]).refusal();
    assert_eq!(refused["code"], "schema_mismatch");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .ends_with("is schema_version 1, and this build understands 2"),
        "{refused}"
    );
    assert_eq!(
        refused["hint"],
        "Bring the corpus forward with:  neb migrate"
    );

    let new = Corpus::new();
    let config = new.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &raw.replacen("schema_version: 2", "schema_version: 3", 1),
    );
    let refused = new.run(&["list", "--json"]).refusal();
    assert_eq!(refused["code"], "schema_mismatch");
    assert_eq!(
        refused["hint"],
        "This corpus was written by a newer nebula. Upgrade this build."
    );
}

/// A corpus with no `config.yaml` is refused by every verb, the reads and
/// capture alike, and none of them puts a config back: a synthesized one
/// would stamp this build's schema over files that may predate it.
#[test]
fn list_on_corpus_without_config_refuses_and_leaves_no_file() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let config = c.root.join("config.yaml");
    std::fs::remove_file(&config).unwrap();
    let before = snapshot_corpus_files(&c.root);

    for args in [
        &["--json", "list"][..],
        &["--json", "show", &id],
        &["--json", "check"],
        &["--json", "capture", "x"],
    ] {
        let refused = c.run(args).refusal();
        assert_eq!(refused["code"], "missing_config", "{args:?}: {refused}");
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains(config.to_str().unwrap()),
            "{args:?}: {refused}"
        );
        assert!(
            refused["hint"].as_str().unwrap().contains("neb migrate"),
            "{args:?}: {refused}"
        );
        assert!(!config.exists(), "`neb {}` wrote a config", args.join(" "));
    }
    assert_eq!(before, snapshot_corpus_files(&c.root), "a refusal wrote");
}

/// A refusal is written by the output layer to stderr: under `--json`, one
/// JSON object there and nothing on stdout.
#[test]
fn json_refusal_goes_to_stderr_through_the_output_layer() {
    let c = Corpus::new();
    let run = c.run(&["--json", "show", "nope"]);
    assert_eq!(run.refusal()["code"], "no_such_node");
    assert_eq!(run.stdout(), "");
}

/// A refused commit comes after the write, so under `--json` stdout still
/// holds the write's payload and stderr the refusal: two streams, each one
/// JSON document.
#[test]
fn json_commit_refusals_leave_the_payload_on_stdout() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    break_head(&c.root);

    let run = c.run(&["new", "An idea", "--json"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "git");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .starts_with("git rev-parse failed in"),
        "{refused}"
    );
    let created: serde_json::Value =
        serde_json::from_str(&run.stdout()).expect("the write's payload is still JSON");
    assert_eq!(created["doc"]["node"]["id"], "an-idea");

    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    git_init(&outer);
    write(&outer.join(".gitignore"), "corpus/\n");
    let run = c.run(&["config", "commit", "on", "--json"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "corpus_ignored");
    assert!(
        refused["hint"]
            .as_str()
            .unwrap()
            .contains("neb config commit off"),
        "{refused}"
    );
    serde_json::from_str::<serde_json::Value>(&run.stdout())
        .expect("the setting's payload is still JSON");
}

/// The CLI's own refusals use the same envelope, a usage error among them
/// exiting 2; only clap's usage errors, raised before `neb` knows it was
/// asked for JSON, stay prose, with the same exit 2.
#[test]
fn json_covers_the_clis_own_refusals_but_not_clap_usage_errors() {
    let c = Corpus::new();
    let empty = c.run(&["capture", "  ", "--json"]).usage_refusal();
    assert_eq!(
        empty,
        serde_json::json!({"error": "nothing to capture", "code": "empty_capture", "hint": null})
    );

    let id = c.seed("an idea", "An idea");
    let editor = c.run(&["edit", &id, "--json"]).refusal();
    assert_eq!(editor["code"], "editor_not_configured");

    let run = c.run(&["show", "--json"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert!(run.stderr().starts_with("error:"), "{}", run.stderr());
    assert!(
        serde_json::from_str::<serde_json::Value>(&run.stderr()).is_err(),
        "{}",
        run.stderr()
    );
}

/// `args` refused as a usage error in both modes: exit 2 and nothing on
/// stdout, `error:` prose on stderr without `--json` and the envelope with
/// `code` under it. Returns the envelope.
fn assert_usage_error(c: &Corpus, args: &[&str], code: &str) -> serde_json::Value {
    let run = c.run(args);
    assert_eq!(run.out.status.code(), Some(2), "neb {}", run.args);
    assert_eq!(run.stdout(), "", "neb {}", run.args);
    assert!(
        run.stderr().starts_with("error: "),
        "neb {}: {}",
        run.args,
        run.stderr()
    );
    let mut with_json = vec!["--json"];
    with_json.extend_from_slice(args);
    let run = c.run(&with_json);
    let refused = run.usage_refusal();
    assert_eq!(run.stdout(), "", "neb {}", run.args);
    assert_eq!(refused["code"], code, "neb {}", run.args);
    refused
}

/// Each refusal the CLI makes of arguments that parsed but ask for nothing
/// is a usage error, exit 2, like clap's own (STD-01 §R20).
#[test]
fn usage_refusals_exit_two() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = snapshot_corpus_files(&c.root);
    for (args, message) in [
        (
            vec!["tag", &id],
            "nothing to do; pass --add <tag> or --remove <tag>",
        ),
        (vec!["near", "  "], "nothing to look near"),
    ] {
        let refused = assert_usage_error(&c, &args, "usage");
        assert_eq!(refused["error"], message, "neb {}", args.join(" "));
    }
    // Core's own argument rules, with core's codes, exit 2 as well.
    for (args, code, message) in [
        (
            vec!["status", &id, "seed", "--why", "x"],
            "reason_on_open_status",
            "a reason only applies to refuted or abandoned, not `seed`",
        ),
        (vec!["capture", "  "], "empty_capture", "nothing to capture"),
        (
            vec!["note", &id, "  "],
            "empty_note",
            "a note cannot be empty",
        ),
    ] {
        let refused = assert_usage_error(&c, &args, code);
        assert_eq!(refused["error"], message, "neb {}", args.join(" "));
    }
    // `triage` has no JSON form: `--json` is a flag the verb does not take.
    let run = c.run_with_stdin(&["--json", "triage"], "d\n");
    assert_eq!(run.usage_refusal()["code"], "interactive");
    assert_eq!(run.stdout(), "");
    // `sharpen`'s `--kill`-or-`--confirm` is clap's to refuse, first.
    let run = c.run(&["sharpen", &id]);
    assert_eq!(run.out.status.code(), Some(2));
    assert_eq!(run.stdout(), "");
    assert_eq!(
        snapshot_corpus_files(&c.root),
        before,
        "nothing was written"
    );
}

/// A core refusal of an argument no corpus could accept, whatever it holds,
/// is a usage error too, decided per variant in one place.
#[test]
fn core_argument_shape_refusals_have_a_decided_exit_code() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let hypothesis = c.seed("a hypothesis", "A hypothesis");
    c.run(&["sharpen", &hypothesis, "--kill", "if X"])
        .assert_ok();
    let dead = c.seed("a dead idea", "A dead idea");
    c.run(&["sharpen", &dead, "--kill", "if Y"]).assert_ok();
    c.run(&["status", &dead, "refuted", "--why", "Y happened"])
        .assert_ok();
    let before = snapshot_corpus_files(&c.root);
    for (args, code) in [
        (vec!["handoff", &id, "nonsense"], "invalid_observatory_id"),
        (
            vec![
                "cite",
                &id,
                "--kind",
                "bogus",
                "--uri",
                "https://example.org",
                "--note",
                "n",
            ],
            "unknown_reference_kind",
        ),
        (vec!["new", "X", "--id", "Bad Id"], "invalid_id"),
        (vec!["new", "X", "--kill", "  "], "empty_kill"),
        (
            vec!["new", "X", "--parent", &dead, "--reopens", &dead],
            "parent_and_reopens",
        ),
        (vec!["status", &hypothesis, "refuted"], "refuted_needs_why"),
        (vec!["new", "   "], "unusable_title"),
        (
            vec!["cite", &id, "--uri", "/abs/x", "--note", "n"],
            "absolute_uri",
        ),
    ] {
        assert_usage_error(&c, &args, code);
    }
    assert_eq!(
        snapshot_corpus_files(&c.root),
        before,
        "nothing was written"
    );
    // What the corpus holds decides a failure, which stays exit 1.
    c.run(&["--json", "show", "nope"]).refusal();
}

// ------------------------------------------------------------------ triage --

#[test]
fn review_short_finds_the_hypothesis_with_no_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));
    c.run(&["review", "--short"])
        .assert_ok()
        .says(&id)
        .says("hypothesis with no references");

    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "n"])
        .assert_ok();
    let after = c.run(&["review", "--short"]).assert_ok().stdout();
    assert!(
        !after.contains("no references"),
        "a reference closes the gap:\n{after}"
    );
}

#[test]
fn review_short_json_items_have_exactly_id_and_why_keys() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));

    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        !items.is_empty(),
        "expected at least one short-review item:\n{json}"
    );
    for item in &items {
        let keys: std::collections::BTreeSet<&str> = item
            .as_object()
            .expect("review --short --json item is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from(["id", "why"]),
            "review --short --json item key set drifted from {{id, why}}: {item}"
        );
    }
}

#[test]
fn review_and_its_short_form_apply_the_no_references_grace_at_fourteen_days() {
    let c = Corpus::new();
    let grace = c.seed("still in grace", "Still in grace");
    c.run(&["sharpen", &grace, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&grace), &date_days_ago(13));

    let due = c.seed("now due", "Now due");
    c.run(&["sharpen", &due, "--kill", "if Y"]).assert_ok();
    c.run(&["note", &due, "reasoning is not a reference"])
        .assert_ok();
    set_created(&c.node_file(&due), &date_days_ago(14));

    for args in [["review", "--short"].as_slice(), &["review"]] {
        let out = c.run(args).assert_ok().stdout();
        assert!(
            !out.contains(&grace),
            "{args:?} raised a 13-day-old node:\n{out}"
        );
        assert!(
            out.contains(&due),
            "{args:?} missed a 14-day-old node:\n{out}"
        );
    }
}

#[test]
fn review_short_finds_seeds_untouched_for_ninety_days_and_filters_by_tag() {
    let c = Corpus::new();
    let old = c.seed("an old seed", "An old seed");
    c.run(&["tag", &old, "--add", "physics"]).assert_ok();
    set_updated(&c.node_file(&old), &date_days_ago(90));
    let fresh = c.seed("a fresh seed", "A fresh seed");
    set_updated(&c.node_file(&fresh), &date_days_ago(89));

    let out = c.run(&["review", "--short"]).assert_ok().stdout();
    assert!(
        out.contains(&old) && out.contains("seed untouched for ninety days"),
        "{out}"
    );
    assert!(!out.contains(&fresh), "{out}");

    let out = c
        .run(&["review", "--short", "--tag", "physics"])
        .assert_ok()
        .stdout();
    assert!(out.contains(&old), "{out}");
    let out = c
        .run(&["review", "--short", "--tag", "orrery"])
        .assert_ok()
        .stdout();
    assert!(!out.contains(&old), "{out}");
}

#[test]
fn review_short_finds_inbox_captures_waiting_over_fourteen_days() {
    let c = Corpus::new();
    c.run(&["capture", "an old capture"]);

    let inbox_file = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut raw = std::fs::read_to_string(&inbox_file).unwrap();
    let stamp_start = raw.find("] ").unwrap() + 2;
    let stamp_end = stamp_start + raw[stamp_start..].find(' ').unwrap();
    raw.replace_range(stamp_start..stamp_end, "2020-01-01T00:00");
    std::fs::write(inbox_file, raw).unwrap();

    c.run(&["review", "--short"])
        .assert_ok()
        .says("1 capture waiting over fourteen days; promote or drop them");

    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items.iter().any(|item| item["id"] == "inbox"),
        "inbox finding missing: {json}"
    );
}

#[test]
fn review_short_on_an_empty_corpus_says_nothing_needs_attention() {
    let c = Corpus::new();
    c.run(&["review", "--short"])
        .assert_ok()
        .says("nothing needs attention");
    let json = c.run(&["review", "--short", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(items.is_empty(), "{json}");
}

#[test]
fn review_short_refuses_the_full_reports_since_and_out_and_tag_needs_it() {
    let c = Corpus::new();
    let out_path = c.workdir().join("short.md");
    for args in [
        vec!["review", "--short", "--since", "7"],
        vec!["review", "--short", "--out", out_path.to_str().unwrap()],
    ] {
        c.run(&args).assert_fails().says("cannot be used with");
    }
    assert!(!out_path.exists(), "a refused --out must not write a file");
    c.run(&["review", "--tag", "physics"])
        .assert_fails()
        .says("--short");
}

/// A negative `--since` would call a seed made today untouched for -1 days
/// and propose abandoning it. It is a usage error; zero days is a threshold.
#[test]
fn review_since_negative_is_a_usage_error() {
    let c = Corpus::new();
    c.seed("made today", "Made today");
    for since in ["--since=-1", "--since=-9223372036854775808"] {
        let refused = c.run(&["review", since]);
        assert_eq!(refused.out.status.code(), Some(2), "{}", refused.stderr());
        assert_eq!(refused.stdout(), "");
        assert!(
            refused.stderr().contains("--since") && refused.stderr().contains("at least 0"),
            "{}",
            refused.stderr()
        );
    }
    c.run(&["review", "--since", "0"])
        .assert_ok()
        .says("## Seeds untouched for 0 days");
}

/// `open` is kept for one release so routines that call it keep working: the
/// same stdout as `review --short`, in text and JSON, plus one deprecation
/// line on stderr, and no row in `--help`.
#[test]
fn deprecated_open_matches_review_short_and_warns_once_on_stderr() {
    let c = Corpus::new();
    let bare = c.seed("an idea", "An idea");
    c.run(&["sharpen", &bare, "--kill", "if X"]).assert_ok();
    c.run(&["tag", &bare, "--add", "physics"]).assert_ok();
    set_created(&c.node_file(&bare), &date_days_ago(14));
    let old = c.seed("an old seed", "An old seed");
    set_updated(&c.node_file(&old), &date_days_ago(90));

    for extra in [
        [].as_slice(),
        &["--json"],
        &["--tag", "physics"],
        &["--json", "--tag", "physics"],
    ] {
        let short = c
            .run(&[["review", "--short"].as_slice(), extra].concat())
            .assert_ok();
        let open = c.run(&[["open"].as_slice(), extra].concat()).assert_ok();
        assert!(
            short.stdout().contains(&bare),
            "{extra:?}: {}",
            short.stdout()
        );
        assert_eq!(open.stdout(), short.stdout(), "{extra:?}");
        assert_eq!(
            short.stderr(),
            "",
            "{extra:?}: review --short is not deprecated"
        );
        assert_eq!(
            open.stderr().lines().collect::<Vec<_>>(),
            ["warning: `neb open` is deprecated; use `neb review --short`"],
            "{extra:?}"
        );
    }

    let help = c.run(&["--help"]).assert_ok().stdout();
    assert!(!help.contains("\n  open "), "open is hidden:\n{help}");
    assert!(help.contains("\n  review "), "{help}");
}

/// Help reads the same on every machine: clap is built without `wrap_help`
/// and `color`, so neither the width nor the terminal changes a byte
/// (STD-01 §R17).
#[test]
fn help_does_not_depend_on_columns() {
    let c = Corpus::new();
    for args in [["list", "--help"].as_slice(), &["--help"]] {
        let narrow = c.run_with_env(args, &[("COLUMNS", "50")]).assert_ok();
        let wide = c.run_with_env(args, &[("COLUMNS", "200")]).assert_ok();
        let plain = c.run_with_env(args, &[("NO_COLOR", "")]).assert_ok();
        assert_eq!(narrow.stdout(), wide.stdout(), "{args:?}");
        assert_eq!(plain.stdout(), wide.stdout(), "{args:?}");
        assert!(!wide.stdout().contains('\x1b'), "{args:?}");
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One contract matrix is easier to audit than split verb lists.
fn every_documented_json_verb_emits_machine_readable_json() {
    let init_dir = tempfile::tempdir().unwrap();
    let init_home = init_dir.path().join("home");
    let init_root = init_dir.path().join("corpus");
    let initialized = run_from_home(&init_home, Some(&init_root), &["init", "--json"], None)
        .assert_ok()
        .stdout();
    let initialized: serde_json::Value =
        serde_json::from_str(&initialized).expect("init --json is valid JSON");
    assert_eq!(initialized["root"], init_root.display().to_string());

    let c = Corpus::new();
    let json = |args: &[&str]| {
        let out = c.run(args).assert_ok().stdout();
        serde_json::from_str::<serde_json::Value>(&out)
            .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
    };

    json(&["migrate", "--json"]);
    json(&["config", "observatory-root", "--json"]);
    json(&["config", "commit", "--json"]);

    let captured = json(&["capture", "--json", "an idea to promote"]);
    let promote_entry = captured["entry"]["id"].as_str().unwrap();
    json(&["inbox", "--json"]);
    let promoted = json(&[
        "promote",
        "--json",
        promote_entry,
        "--title",
        "Promoted idea",
    ]);
    assert_eq!(promoted["doc"]["node"]["id"], "promoted-idea");
    assert!(promoted["path"].is_string());

    let captured = json(&["capture", "--json", "an idea to drop"]);
    let drop_entry = captured["entry"]["id"].as_str().unwrap();
    let dropped = json(&["drop", "--json", drop_entry]);
    assert_eq!(dropped["id"], drop_entry);
    assert_eq!(dropped["text"], "an idea to drop");

    let created = json(&["new", "--json", "Direct idea", "--tag", "design"]);
    assert_eq!(created["doc"]["node"]["id"], "direct-idea");
    assert!(created["path"].is_string());

    json(&["new", "--json", "Idea to sharpen"]);
    let sharpened = json(&[
        "sharpen",
        "--json",
        "idea-to-sharpen",
        "--kill",
        "the evidence changes",
        "--by",
        "agent:test",
    ]);
    assert_eq!(sharpened["node"]["status"], "hypothesis");
    assert_eq!(sharpened["node"]["kill"], "the evidence changes");
    assert_eq!(sharpened["node"]["kill_by"], "agent:test");
    let confirmed = json(&["sharpen", "--json", "idea-to-sharpen", "--confirm"]);
    assert_eq!(confirmed["node"]["kill"], "the evidence changes");
    assert_eq!(confirmed["node"]["kill_by"], "human");

    json(&["new", "--json", "Idea to close"]);
    let changed = json(&[
        "status",
        "--json",
        "idea-to-close",
        "abandoned",
        "--why",
        "superseded",
    ]);
    assert_eq!(changed["from"], "seed");
    assert_eq!(changed["doc"]["node"]["status"], "abandoned");

    json(&["new", "--json", "Parent idea"]);
    json(&["new", "--json", "Child idea"]);
    let linked = json(&[
        "link",
        "--json",
        "child-idea",
        "derives-from",
        "parent-idea",
    ]);
    assert_eq!(linked.as_array().unwrap().len(), 1);
    assert_eq!(linked[0]["node"]["edges"][0]["to"], "parent-idea");

    let tagged = json(&["tag", "--json", "direct-idea", "--add", "corpus"]);
    assert_eq!(
        tagged["node"]["tags"],
        serde_json::json!(["design", "corpus"])
    );
    json(&["tag", "list", "--json"]);

    let noted = json(&["note", "--json", "direct-idea", "supporting detail"]);
    assert_eq!(noted["notes"][0]["text"], "supporting detail");
    let cited = json(&[
        "cite",
        "--json",
        "direct-idea",
        "--kind",
        "article",
        "--uri",
        "https://example.org/evidence",
        "--note",
        "supporting evidence",
    ]);
    assert_eq!(cited["reference"], "r1");
    assert_eq!(cited["doc"]["node"]["references"][0]["kind"], "article");

    let shown = json(&["show", "--json", "direct-idea"]);
    assert_eq!(shown["node"]["id"], "direct-idea");
    assert!(json(&["list", "--json"]).is_array());
    let near = json(&["near", "--json", "direct design"]);
    assert!(near["items"].is_array() && near["total"].is_u64(), "{near}");
    assert!(near["truncated"].is_boolean(), "{near}");
    assert!(json(&["trace", "--json", "child-idea"]).is_array());
    json(&["impact", "--json", "parent-idea"]);
    json(&["graph", "--json"]);
    assert!(json(&["inbox", "--json"]).is_array());
    assert!(json(&["review", "--json"]).is_array());
    assert!(json(&["review", "--short", "--json"]).is_array());

    let checked = json(&["check", "--json"]);
    assert!(checked["nodes"].as_u64().unwrap() >= 6);
}

/// A JSON object's keys, sorted as `serde_json` keeps them.
fn keys(v: &serde_json::Value) -> Vec<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("an object: {v}"))
        .keys()
        .cloned()
        .collect()
}

/// Every key of a node, whichever verb serialised it (STD-01 §R10, §R11).
const NODE_KEYS: [&str; 13] = [
    "closed",
    "created",
    "edges",
    "id",
    "kill",
    "kill_by",
    "origin",
    "references",
    "status",
    "tags",
    "title",
    "title_by",
    "updated",
];

/// An absent value is `null` and an empty collection `[]` on every node a
/// `--json` verb returns, and a write, a `show` and a `list` agree on the
/// key set.
#[test]
fn json_node_payload_has_every_field_with_null_for_absent() {
    let c = Corpus::new();
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap()
    };
    let created = json(&["new", "--json", "X"])["doc"]["node"].clone();
    let shown = json(&["show", "--json", "x"])["node"].clone();
    let listed = json(&["list", "--json"])[0].clone();
    for node in [&created, &shown, &listed] {
        assert_eq!(keys(node), NODE_KEYS, "{node}");
        for list in ["tags", "edges", "references"] {
            assert_eq!(node[list], serde_json::json!([]), "`{list}`: {node}");
        }
        for absent in ["kill", "kill_by", "closed", "origin"] {
            assert!(node[absent].is_null(), "`{absent}`: {node}");
        }
        assert_eq!(node["title_by"], "human", "{node}");
    }
}

/// Every write verb's node is the shape `show` gives, authors stated: a
/// field written without `--by` says `human` rather than leaving it out.
#[test]
fn json_write_payloads_match_the_read_shape() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap().to_owned();
    let json = |args: &[&str]| -> serde_json::Value {
        let out = c
            .run_with_env(args, &[("OBSERVATORY_ROOT", &obs)])
            .assert_ok()
            .stdout();
        serde_json::from_str(&out)
            .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
    };
    // The node a write returned, against what `show` says of it now.
    let same_as_show = |verb: &str, node: &serde_json::Value| {
        let id = node["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{verb}: {node}"));
        let shown = json(&["show", "--json", id])["node"].clone();
        assert_eq!(keys(node), keys(&shown), "`{verb}` vs `show {id}`: {node}");
        assert_eq!(keys(node), NODE_KEYS, "`{verb}`: {node}");
        assert_eq!(node["title_by"], "human", "`{verb}`: {node}");
        for list in ["edges", "references"] {
            for item in node[list].as_array().unwrap() {
                assert_eq!(item["by"], "human", "`{verb}` {list}: {node}");
            }
        }
    };

    json(&["new", "--json", "Parent"]);
    let created = json(&["new", "--json", "Child", "--parent", "parent"]);
    assert_eq!(created["doc"]["node"]["edges"][0]["by"], "human");
    same_as_show("new", &created["doc"]["node"]);

    let entry = json(&["capture", "--json", "-q", "a promoted thought"])["entry"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let promoted = json(&["promote", "--json", &entry, "--parent", "parent"]);
    assert_eq!(promoted["doc"]["node"]["edges"][0]["by"], "human");
    same_as_show("promote", &promoted["doc"]["node"]);

    let changed = json(&["status", "--json", "child", "abandoned", "--why", "moot"]);
    same_as_show("status", &changed["doc"]["node"]);

    json(&["new", "--json", "Linked"]);
    let linked = json(&["link", "--json", "linked", "derives-from", "parent"]);
    assert_eq!(linked[0]["node"]["edges"][0]["by"], "human");
    same_as_show("link", &linked[0]["node"]);

    let tagged = json(&["tag", "--json", "linked", "--add", "t"]);
    same_as_show("tag", &tagged["node"]);

    let sharpened = json(&["sharpen", "--json", "parent", "--kill", "if not"]);
    assert_eq!(sharpened["node"]["kill_by"], "human");
    same_as_show("sharpen", &sharpened["node"]);

    let cited = json(&[
        "cite",
        "--json",
        "parent",
        "--uri",
        "https://example.org",
        "--note",
        "why",
    ]);
    assert_eq!(cited["doc"]["node"]["references"][0]["by"], "human");
    same_as_show("cite", &cited["doc"]["node"]);

    let handed = json(&["handoff", "--json", "linked", "H012", "--note", "n"]);
    assert_eq!(handed["doc"]["node"]["references"][0]["by"], "human");
    same_as_show("handoff", &handed["doc"]["node"]);
}

/// Optionals nested inside a node, and beside it in a payload, are `null`
/// or `[]` rather than left out.
#[test]
fn json_nested_optionals_are_null_not_omitted() {
    let c = Corpus::new();
    let json = |args: &[&str]| -> serde_json::Value {
        serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap()
    };

    json(&["new", "--json", "Plain"]);
    let cited = json(&["cite", "--json", "plain", "--uri", "https://example.org"]);
    let reference = &cited["doc"]["node"]["references"][0];
    assert_eq!(
        keys(reference),
        [
            "added", "by", "id", "kind", "note", "origin", "title", "uri"
        ],
        "{reference}"
    );
    for absent in ["title", "note", "origin"] {
        assert!(reference[absent].is_null(), "`{absent}`: {reference}");
    }

    let tasked = json(&["new", "--json", "Tasked", "--task", "T"]);
    assert_eq!(
        tasked["doc"]["node"]["origin"],
        serde_json::json!({
            "task": "T",
            "workspace": null,
            "run": null,
            "artifact": null,
            "agent": null,
            "at": null,
        })
    );

    let shown = json(&["show", "--json", "tasked"]);
    assert_eq!(
        keys(&shown),
        ["body", "handed_off_to", "node", "notes", "observatory"],
        "{shown}"
    );
    assert_eq!(shown["notes"], serde_json::json!([]), "{shown}");
    assert_eq!(shown["observatory"], serde_json::json!([]), "{shown}");
    assert!(shown["handed_off_to"].is_null(), "{shown}");

    // No observatory root on this machine, so the record cannot resolve.
    c.run(&[
        "cite",
        "plain",
        "--kind",
        "observatory",
        "--uri",
        "H012",
        "--note",
        "n",
    ])
    .assert_ok();
    let shown = json(&["show", "--json", "plain"]);
    assert_eq!(
        shown["observatory"],
        serde_json::json!([{"reference": "r2", "record": "H012", "path": null}]),
        "{shown}"
    );

    let walk = json(&["trace", "--json", "tasked"]);
    assert!(walk[0]["handed_off_to"].is_null(), "{walk}");
    assert!(walk[0].get("handed_off_to").is_some(), "{walk}");

    let captured = json(&["capture", "--json", "-q", "plain again"]);
    assert_eq!(captured["near"], serde_json::json!([]), "{captured}");
    let entry = captured["entry"]["id"].as_str().unwrap();
    let promoted = json(&["promote", "--json", entry, "-q"]);
    assert_eq!(promoted["near"], serde_json::json!([]), "{promoted}");
}

#[test]
fn an_empty_corpus_is_valid() {
    let c = Corpus::new();
    c.run(&["check"]).assert_ok().says("0 nodes, 0 errors");
    c.run(&["list"]).assert_ok().says("no nodes match");
}

#[test]
fn round_tripping_a_node_preserves_prose_and_fields() {
    let c = Corpus::new();
    let id = c.seed("the original thought", "The original thought");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["cite", &id, "--uri", "http://example.com", "--note", "why"])
        .assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        after.contains("the original thought"),
        "prose survives a write:\n{after}"
    );
    assert!(
        before.contains("id: the-original-thought") && after.contains("id: the-original-thought")
    );
}

// ------------------------------------------------------------- body/edit --

#[test]
fn new_reads_body_from_stdin_and_promote_appends_body_after_capture() {
    let c = Corpus::new();
    let id = c
        .run_with_stdin(
            &["new", "A body from stdin", "--body", "-"],
            "first\n\nsecond\n",
        )
        .assert_ok()
        .stdout_trim();
    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.ends_with("first\n\nsecond\n"), "{node}");

    let entry = c.run(&["capture", "the captured first line"]).stdout_trim();
    let promoted = c
        .run(&[
            "promote",
            &entry,
            "--title",
            "Promoted body",
            "--body",
            "the added argument",
        ])
        .assert_ok()
        .stdout_trim();
    let node = std::fs::read_to_string(c.node_file(&promoted)).unwrap();
    let captured = node.find("the captured first line").unwrap();
    let added = node.find("the added argument").unwrap();
    assert!(captured < added, "captured text stays first:\n{node}");
    assert!(
        node.contains("the captured first line\n\nthe added argument"),
        "body paragraphs are separated:\n{node}"
    );
}

#[cfg(unix)]
#[test]
fn edit_exposes_only_body_saves_it_stamps_updated_and_commits() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Editable", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    set_updated(&c.node_file(&id), &date_days_ago(7));
    let before_commits = log(&c.root).len();
    let script = editor_script(
        &c,
        "rewrite-body.sh",
        "#!/bin/sh\nif grep -q '^---$' \"$1\"; then exit 70; fi\nprintf 'the rewritten body\\n' > \"$1\"\n",
    );

    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_ok();

    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.ends_with("the rewritten body\n"), "{node}");
    assert!(
        node.contains(&format!("updated: {}", date_days_ago(0))),
        "updated is stamped:\n{node}"
    );
    assert_eq!(log(&c.root).len(), before_commits + 1);
    assert_eq!(log(&c.root)[0], format!("neb edit {id}"));
}

/// An editor that leaves the body as it was changes nothing, so nothing is
/// written: `updated` stays, no commit lands, and stderr says so (STD-01
/// §R30). `--json` still answers with the node.
#[cfg(unix)]
#[test]
fn edit_with_an_unchanged_body_writes_nothing() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Left alone", "--body", "the body"])
        .assert_ok()
        .stdout_trim();
    set_updated(&c.node_file(&id), &date_days_ago(7));
    git(&c.root, &["commit", "-qam", "back-dated"]);
    let before = std::fs::read(c.node_file(&id)).unwrap();
    let commits = log(&c.root);

    let run = c
        .run_with_env(&["edit", &id], &[("EDITOR", "true")])
        .assert_ok();
    assert_eq!(run.stdout(), "");
    assert!(run.stderr().contains("no change"), "{}", run.stderr());
    assert!(run.stderr().contains(&id), "{}", run.stderr());
    let run = c
        .run_with_env(&["--json", "edit", &id], &[("EDITOR", "true")])
        .assert_ok();
    let v: serde_json::Value = serde_json::from_str(&run.stdout()).unwrap();
    assert_eq!(v["node"]["id"], id.as_str());
    assert!(run.stderr().contains("no change"), "{}", run.stderr());

    assert_eq!(std::fs::read(c.node_file(&id)).unwrap(), before);
    assert_eq!(log(&c.root), commits, "no commit");
}

/// `edit --by` was validated and then dropped, since a body has no author
/// field, so it is gone rather than advertised (STD-01 §R29, §R36).
#[test]
fn edit_by_is_not_accepted() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read(c.node_file(&id)).unwrap();
    let run = c.run_with_env(&["edit", &id, "--by", "x"], &[("EDITOR", "true")]);
    assert_eq!(run.out.status.code(), Some(2), "{}", run.stderr());
    assert!(
        run.stderr().contains("unexpected argument '--by'"),
        "{}",
        run.stderr()
    );
    assert_eq!(std::fs::read(c.node_file(&id)).unwrap(), before);
    let help = c.run(&["edit", "--help"]).assert_ok().stdout();
    assert!(!help.contains("--by"), "{help}");
}

#[cfg(unix)]
#[test]
fn edit_passes_editor_arguments_before_the_temp_file() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Editor arguments", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let script = editor_script(
        &c,
        "argument-editor.sh",
        "#!/bin/sh\n[ \"$1\" = \"--wait\" ] || exit 71\n[ -f \"$2\" ] || exit 72\nprintf 'the editor received its arguments\\n' > \"$2\"\n",
    );
    let editor = format!("{} --wait", script.display());

    c.run_with_env(&["edit", &id], &[("EDITOR", &editor)])
        .assert_ok();

    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        node.ends_with("the editor received its arguments\n"),
        "{node}"
    );
}

#[cfg(unix)]
#[test]
fn edit_honours_quoted_editor_path_with_spaces_and_arguments() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Quoted editor path", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let script = editor_script(
        &c,
        "editor with spaces.sh",
        "#!/bin/sh\n[ \"$1\" = \"-w\" ] || exit 71\n[ -f \"$2\" ] || exit 72\nprintf 'the quoted editor ran\\n' > \"$2\"\n",
    );
    let editor = format!("\"{}\" -w", script.display());

    c.run_with_env(&["edit", &id], &[("EDITOR", &editor)])
        .assert_ok();

    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.ends_with("the quoted editor ran\n"), "{node}");
}

#[cfg(unix)]
#[test]
fn edit_refuses_when_the_editor_exits_unsuccessfully() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Unsuccessful editor", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let script = editor_script(&c, "fail-editor.sh", "#!/bin/sh\nexit 42\n");

    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("exited unsuccessfully; the node was not changed");

    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "an editor failure must not change the node"
    );
}

#[cfg(unix)]
#[test]
fn edit_refuses_to_remove_notes_and_leaves_the_node_unchanged() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Protected notes", "--body", "the argument"])
        .assert_ok()
        .stdout_trim();
    c.run(&["note", &id, "an append-only note"]).assert_ok();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let script = editor_script(
        &c,
        "remove-notes.sh",
        "#!/bin/sh\nprintf 'the argument, rewritten without notes\\n' > \"$1\"\n",
    );

    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("existing ## Notes section was removed, reordered, or changed");

    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(after, before, "a refused edit must not touch the node");

    let today = date_days_ago(0);
    let reordered = format!(
        "#!/bin/sh\nprintf '## Notes\\n\\n- {today}: an append-only note\\n\\nthe argument\\n' > \"$1\"\n"
    );
    let script = editor_script(&c, "reorder-notes.sh", &reordered);
    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("existing ## Notes section was removed, reordered, or changed");
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(after, before, "a reordered notes section must not be saved");
}

/// A node whose notes are followed by other prose ends up with two `## Notes`
/// sections, because `note` never rewrites body text it did not write. The
/// older section is reasoning too: an edit may not touch it, and `show` may
/// not hide it behind the newer one.
#[cfg(unix)]
#[test]
fn edit_protects_notes_that_a_later_section_no_longer_closes() {
    let c = Corpus::new();
    let body = "the argument\n\n## Notes\n\n- 2026-09-21: the first reasoning\n\n## More\n\nstill to work out";
    let id = c
        .run(&["new", "Two notes sections", "--body", body])
        .assert_ok()
        .stdout_trim();
    c.run(&["note", &id, "the second reasoning"]).assert_ok();

    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(
        before.matches("## Notes").count(),
        2,
        "a closed notes section gets a sibling, not an edit:\n{before}"
    );

    // The editor rewrites the *earlier* dated note and leaves the final
    // notes section exactly as it found it.
    let script = editor_script(
        &c,
        "rewrite-earlier-note.sh",
        "#!/bin/sh\nsed 's/the first reasoning/rewritten reasoning/' \"$1\" > \"$1.new\" && mv \"$1.new\" \"$1\"\n",
    );
    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("existing ## Notes section was removed, reordered, or changed");
    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "a refused edit must not touch the node"
    );

    // Dropping the earlier section outright is the same refusal.
    let script = editor_script(
        &c,
        "drop-earlier-note.sh",
        "#!/bin/sh\ngrep -v 'the first reasoning' \"$1\" > \"$1.new\" && mv \"$1.new\" \"$1\"\n",
    );
    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("existing ## Notes section was removed, reordered, or changed");
    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "a refused edit must not touch the node"
    );

    // Prose that is not a note is still the editor's to rewrite.
    let script = editor_script(
        &c,
        "rewrite-prose.sh",
        "#!/bin/sh\nsed 's/still to work out/worked out after all/' \"$1\" > \"$1.new\" && mv \"$1.new\" \"$1\"\n",
    );
    c.run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(after.contains("worked out after all"), "{after}");
    assert!(
        after.contains("- 2026-09-21: the first reasoning"),
        "{after}"
    );

    // Both sections' reasoning is exposed, oldest first.
    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["notes"].as_array().map(Vec::len), Some(2), "{out}");
    assert_eq!(v["notes"][0]["text"], "the first reasoning");
    assert_eq!(v["notes"][0]["at"], "2026-09-21");
    assert_eq!(v["notes"][1]["text"], "the second reasoning");
}

#[test]
fn edit_without_visual_or_editor_is_a_named_refusal() {
    let c = Corpus::new();
    let id = c.run(&["new", "No editor"]).assert_ok().stdout_trim();
    c.run(&["edit", &id])
        .assert_fails()
        .says("neither $VISUAL nor $EDITOR names an editor");
}

/// An `$EDITOR` that says it has started, then waits for the test to let it
/// finish before it writes `text` over the body. The wait is bounded, so an
/// editor nobody releases exits non-zero rather than hanging the suite.
#[cfg(unix)]
struct HeldEditor {
    script: PathBuf,
    started: PathBuf,
    release: PathBuf,
}

#[cfg(unix)]
impl HeldEditor {
    fn new(c: &Corpus, name: &str, text: &str) -> Self {
        let dir = c.workdir();
        let started = dir.join(format!("{name}.started"));
        let release = dir.join(format!("{name}.release"));
        let typed = dir.join(format!("{name}.typed"));
        write(&typed, text);
        let script = editor_script(
            c,
            &format!("{name}.sh"),
            &format!(
                "#!/bin/sh\n: > '{started}'\ni=0\n\
                 while [ ! -e '{release}' ]; do\n\
                 i=$((i + 1)); [ \"$i\" -gt 200 ] && exit 75; sleep 0.05\n\
                 done\ncat '{typed}' > \"$1\"\n",
                started = started.display(),
                release = release.display(),
                typed = typed.display(),
            ),
        );
        Self {
            script,
            started,
            release,
        }
    }

    /// `neb <args>` with this editor, started and not yet released.
    fn open(&self, c: &Corpus, args: &[&str]) -> Spawned {
        let mut edit = c.spawn_with_env(args, &[("EDITOR", self.script.to_str().unwrap())]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !self.started.exists() {
            assert!(
                edit.child.try_wait().is_none(),
                "`neb {}` exited before its editor started",
                edit.args
            );
            assert!(
                std::time::Instant::now() < deadline,
                "the editor never started"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        edit
    }

    fn release(&self) {
        write(&self.release, "");
    }
}

/// Every edit kept after a refused save, under the fixture's home.
fn kept_edits(c: &Corpus) -> Vec<PathBuf> {
    let dir = c.workdir().join(".local/state/nebula/edits");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut kept: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
    kept.sort();
    kept
}

/// The one kept edit a refusal names: outside the corpus, owner-only, and
/// holding exactly `text`.
#[cfg(unix)]
fn assert_kept(c: &Corpus, named_in: &str, text: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let kept = kept_edits(c);
    let [path] = kept.as_slice() else {
        panic!("expected one kept edit, found {kept:?}");
    };
    assert!(
        named_in.contains(&path.display().to_string()),
        "the refusal names {}:\n{named_in}",
        path.display()
    );
    assert!(!path.starts_with(&c.root), "kept outside the corpus");
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "{}", path.display());
    path.clone()
}

/// The editor holds no lock (STD-03 §R1): a capture made while a body is
/// open in `$EDITOR` lands at once, and the edit still saves afterwards.
#[cfg(unix)]
#[test]
fn a_capture_lands_while_neb_edit_is_open() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Open in an editor", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let editor = HeldEditor::new(&c, "held", "the edited body\n");
    let edit = editor.open(&c, &["edit", &id]);

    let started = std::time::Instant::now();
    c.run(&["capture", "x"]).assert_ok();
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_secs(1),
        "the capture waited {took:?} on the editor"
    );

    editor.release();
    edit.wait().assert_ok();
    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.ends_with("the edited body\n"), "{node}");
    c.run(&["inbox"]).assert_ok().says("x");
}

/// A body changed while the editor was open — here by `note` — would be
/// erased by the save, so the edit is refused as `edit_conflict`, the
/// concurrent note survives, and what the person typed is kept in a file the
/// refusal names, in the prose and in the `--json` envelope.
#[cfg(unix)]
#[test]
fn an_edit_whose_body_changed_meanwhile_is_refused_and_kept() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Edited twice", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let typed = "Hours of careful new prose\n";

    let editor = HeldEditor::new(&c, "first", typed);
    let edit = editor.open(&c, &["edit", &id]);
    c.run(&["note", &id, "a concurrent note"]).assert_ok();
    editor.release();
    let run = edit
        .wait()
        .assert_fails()
        .says("body changed while it was being edited");
    let first = assert_kept(&c, &run.stderr(), typed);
    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.contains("a concurrent note"), "{node}");
    assert!(!node.contains("careful new prose"), "{node}");

    // The same under `--json`, on a node with no notes yet, so the refusal
    // is the conflict and not the notes check.
    std::fs::remove_file(first).unwrap();
    let id = c
        .run(&["new", "Edited twice again", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let editor = HeldEditor::new(&c, "second", typed);
    let edit = editor.open(&c, &["--json", "edit", &id]);
    c.run(&["note", &id, "another concurrent note"]).assert_ok();
    editor.release();
    let run = edit.wait();
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "edit_conflict", "{refusal}");
    assert_kept(&c, refusal["error"].as_str().unwrap(), typed);
    let node = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(node.contains("another concurrent note"), "{node}");
}

/// Only the body is compared: a tag added while the editor was open is not
/// a conflict, and the edit lands on top of it.
#[cfg(unix)]
#[test]
fn an_edit_keeps_a_concurrent_frontmatter_change() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Tagged meanwhile", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let editor = HeldEditor::new(&c, "held", "the new body\n");
    let edit = editor.open(&c, &["edit", &id]);
    c.run(&["tag", &id, "--add", "x"]).assert_ok();
    editor.release();
    edit.wait().assert_ok();

    let shown = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let shown: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(shown["node"]["tags"], serde_json::json!(["x"]), "{shown}");
    assert_eq!(shown["body"].as_str().map(str::trim), Some("the new body"));
    assert!(kept_edits(&c).is_empty(), "a saved edit keeps nothing");
}

/// The notes refusal comes after the person typed, so it keeps the text too.
#[cfg(unix)]
#[test]
fn a_notes_changing_edit_is_refused_and_the_text_is_kept() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Protected notes", "--body", "the argument"])
        .assert_ok()
        .stdout_trim();
    c.run(&["note", &id, "an append-only note"]).assert_ok();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let script = editor_script(
        &c,
        "drop-notes.sh",
        "#!/bin/sh\nprintf 'Hours of careful new prose\\n' > \"$1\"\n",
    );

    let run = c
        .run_with_env(&["edit", &id], &[("EDITOR", script.to_str().unwrap())])
        .assert_fails()
        .says("existing ## Notes section was removed, reordered, or changed");
    assert_kept(&c, &run.stderr(), "Hours of careful new prose\n");
    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "a refused edit must not touch the node"
    );
}

/// The lock is taken only to save, after the editor exits. A writer holding
/// it past the bounded wait refuses the save as `locked`, and the text is
/// kept.
#[cfg(unix)]
#[test]
fn an_edit_that_waits_out_the_lock_keeps_the_text() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Locked at save", "--body", "the old body"])
        .assert_ok()
        .stdout_trim();
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let typed = "text typed while somebody held the lock\n";
    let editor = HeldEditor::new(&c, "held", typed);
    let edit = editor.open(&c, &["--json", "edit", &id]);
    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");
    editor.release();

    // It waits the full five seconds before giving up.
    let run = edit.wait();
    drop(held);
    let refusal = run.refusal();
    assert_eq!(refusal["code"], "locked", "{refusal}");
    assert_kept(&c, refusal["error"].as_str().unwrap(), typed);
    assert_eq!(std::fs::read_to_string(c.node_file(&id)).unwrap(), before);
}

// --------------------------------------------------------------------- note --

#[test]
fn note_appends_in_order_and_show_exposes_them() {
    let c = Corpus::new();
    let id = c.seed("the original thought", "The original thought");
    c.run(&["tag", &id, "--add", "physics"]).assert_ok();
    let parent = c.seed("a parent idea", "A parent idea");
    c.run(&["link", &id, "derives-from", &parent]).assert_ok();
    set_updated(&c.node_file(&id), &date_days_ago(7));

    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["note", &id, "first thought"]).assert_ok();
    c.run(&["note", &id, "second thought"]).assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();

    let today = date_days_ago(0);
    assert!(
        after.contains("the original thought"),
        "the capture is still there:\n{after}"
    );
    let first = format!("- {today}: first thought");
    let second = format!("- {today}: second thought");
    let first_at = after.find(&first).expect("first note");
    let second_at = after.find(&second).expect("second note");
    assert!(first_at < second_at, "notes accumulate in order:\n{after}");
    assert!(after.contains("## Notes"), "{after}");
    assert!(
        after.contains("status: seed") && after.contains("tags:\n- physics"),
        "status and tags stay put:\n{after}"
    );
    assert!(
        after.contains(&format!("to: {parent}")),
        "edges stay put:\n{after}"
    );
    assert!(
        after.contains(&format!("updated: {today}")),
        "updated is bumped:\n{after}"
    );
    assert!(
        before.contains(&format!("updated: {}", date_days_ago(7))),
        "the stamp really moved from a past date"
    );

    c.run(&["show", &id])
        .assert_ok()
        .says("the original thought")
        .says("## Notes")
        .says(&first)
        .says(&second);

    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["notes"][0]["text"], "first thought");
    assert_eq!(v["notes"][1]["text"], "second thought");
    assert_eq!(v["notes"][0]["at"], today);
    assert_eq!(v["node"]["status"], "seed");
    assert_eq!(v["node"]["tags"][0], "physics");

    let noted = c
        .run(&["--json", "note", &id, "third thought"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&noted).expect("note --json is valid JSON");
    assert_eq!(v["notes"].as_array().map(Vec::len), Some(3));
    assert_eq!(v["notes"][2]["text"], "third thought");

    c.run(&["note", "nope", "lost"])
        .assert_fails()
        .says("no node `nope`");
}

/// `--no-commit` after the reasoning is a flag, not more text; `--quiet` is
/// not a note flag, so it is refused rather than written into the body.
#[test]
fn note_trailing_no_commit_is_a_flag_and_quiet_is_refused() {
    let (c, _remote) = corpus_repo();
    let id = c.seed("the original thought", "The original thought");
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = log(&c.root).len();
    c.run(&["note", &id, "a thought", "--no-commit"])
        .assert_ok();
    assert_eq!(log(&c.root).len(), before, "trailing --no-commit skips git");
    let body = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let today = date_days_ago(0);
    assert!(
        body.contains(&format!("- {today}: a thought")),
        "the note should land without the flag:\n{body}"
    );
    assert!(
        !body.contains("--no-commit"),
        "the flag must not land in the body:\n{body}"
    );
    c.run(&["note", &id, "a thought", "--quiet"])
        .assert_fails()
        .says("--quiet");
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(body, after, "a refused --quiet must not write");
}

// ------------------------------------------------------------- authorship --

/// Who wrote what is stored per field, and the human is stored by omission:
/// a corpus written by hand looks exactly as it did before this existed.
#[test]
fn an_unattributed_write_is_the_humans_and_leaves_the_file_alone() {
    let c = Corpus::new();
    let parent = c.seed("a parent idea", "A parent idea");
    let id = c.seed("an idea worth keeping", "An idea worth keeping");
    c.run(&["sharpen", &id, "--kill", "if X never happens"])
        .assert_ok();
    c.run(&["link", &id, "derives-from", &parent]).assert_ok();
    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "why"])
        .assert_ok();
    c.run(&["note", &id, "my own reasoning"]).assert_ok();

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        !raw.contains("title_by") && !raw.contains("kill_by") && !raw.contains("by:"),
        "the human is stored by omission:\n{raw}"
    );
    let today = date_days_ago(0);
    assert!(
        raw.contains(&format!("- {today}: my own reasoning")),
        "the human's note line names no author:\n{raw}"
    );

    // `show --json` states the default rather than making a reader know it.
    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["title_by"], "human");
    assert_eq!(v["node"]["kill_by"], "human");
    assert_eq!(v["node"]["edges"][0]["by"], "human");
    assert_eq!(v["node"]["references"][0]["by"], "human");
    assert_eq!(v["notes"][0]["by"], "human");

    let listed = c.run(&["--json", "list"]).assert_ok().stdout();
    let nodes: Vec<serde_json::Value> = serde_json::from_str(&listed).expect("list --json");
    assert!(nodes.iter().all(|n| n["title_by"] == "human"));

    // Nothing here is the agent's, so review has nothing to confirm.
    let report = c.run(&["review"]).assert_ok().stdout();
    let section = report
        .split("## Agent-authored kills not yet confirmed by a human")
        .nth(1)
        .expect("the section is always printed");
    assert!(section.trim_start().starts_with("_none_"), "{report}");
}

/// `--by` is free text — a session id, a crew name, anything the writer
/// answers to — and lands on the field that write authored, not on the node.
/// `show` opens with a header that labels what it prints: the status and id,
/// the title with its author when that is not the human, the tags, and the
/// dates. Edges and references name an agent author the same way.
#[test]
fn show_header_labels_tags_and_dates_and_names_an_agent_author() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    c.run(&["new", "Human idea"]).assert_ok();
    c.run(&[
        "new",
        "Agent idea",
        "--tag",
        "physics",
        "--tag",
        "wake",
        "--parent",
        "human-idea",
        "--kill",
        "if Y",
        "--by",
        by,
    ])
    .assert_ok();
    c.run(&[
        "cite",
        "agent-idea",
        "--uri",
        "http://example.com",
        "--note",
        "n",
        "--by",
        by,
    ])
    .assert_ok();
    set_created(&c.node_file("agent-idea"), "2026-01-02");
    set_updated(&c.node_file("agent-idea"), "2026-02-03");

    let shown = c.run(&["show", "agent-idea"]).assert_ok().stdout();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "hypothesis agent-idea", "{shown}");
    assert_eq!(lines[1], format!("Agent idea ({by})"), "{shown}");
    assert_eq!(lines[2], "tags: physics, wake", "{shown}");
    assert_eq!(
        lines[3], "created: 2026-01-02  updated: 2026-02-03",
        "{shown}"
    );
    assert_eq!(lines[4], "", "{shown}");
    assert!(shown.contains(&format!("kill: if Y ({by})")), "{shown}");
    assert!(
        shown.contains(&format!("  derives-from   human-idea ({by})")),
        "{shown}"
    );
    assert!(
        shown.contains(&format!("http://example.com ({by})")),
        "{shown}"
    );

    // The human's own node names nobody, and has no tags to label.
    let shown = c.run(&["show", "human-idea"]).assert_ok().stdout();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "seed       human-idea", "{shown}");
    assert_eq!(lines[1], "Human idea", "{shown}");
    assert!(lines[2].starts_with("created: "), "{shown}");
    assert!(
        !shown.contains("(human)") && !shown.contains("tags:"),
        "{shown}"
    );
}

#[test]
fn by_records_the_author_of_each_field_it_wrote() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    let parent = c.seed("a parent idea", "A parent idea");

    let entry = c.run(&["capture", "the human's own words"]).stdout_trim();
    c.run(&["promote", &entry, "--by", by, "--parent", &parent])
        .assert_ok();
    let promoted = std::fs::read_to_string(c.node_file("the-human-s-own-words")).unwrap();
    assert!(
        !promoted.contains("title_by"),
        "a capture promoted as captured is titled in the human's words:\n{promoted}"
    );
    assert!(
        promoted.contains(&format!("to: {parent}\n  by: {by}")),
        "the parent edge was the agent's call:\n{promoted}"
    );

    let entry = c.run(&["capture", "another thought"]).stdout_trim();
    c.run(&[
        "promote",
        &entry,
        "--title",
        "A title the agent wrote",
        "--by",
        by,
    ])
    .assert_ok();
    let retitled = std::fs::read_to_string(c.node_file("a-title-the-agent-wrote")).unwrap();
    assert!(
        retitled.contains(&format!("title_by: {by}")),
        "a title the agent wrote is the agent's:\n{retitled}"
    );

    c.run(&["new", "A node the agent made", "--kill", "if Y", "--by", by])
        .assert_ok();
    let made = std::fs::read_to_string(c.node_file("a-node-the-agent-made")).unwrap();
    assert!(
        made.contains(&format!("title_by: {by}")) && made.contains(&format!("kill_by: {by}")),
        "{made}"
    );

    c.run(&[
        "link",
        "a-node-the-agent-made",
        "contradicts",
        &parent,
        "--by",
        by,
    ])
    .assert_ok();
    let both = std::fs::read_to_string(c.node_file(&parent)).unwrap();
    assert!(
        both.contains(&format!("by: {by}")),
        "a contradiction is recorded on both ends, by whoever claimed it:\n{both}"
    );

    c.run(&[
        "cite",
        "a-node-the-agent-made",
        "--uri",
        "https://example.org",
        "--note",
        "the agent found this",
        "--by",
        by,
    ])
    .assert_ok();
    c.run(&["note", "--by", by, "a-node-the-agent-made", "its reasoning"])
        .assert_ok();

    let raw = std::fs::read_to_string(c.node_file("a-node-the-agent-made")).unwrap();
    let today = date_days_ago(0);
    assert!(
        raw.contains(&format!("- {today} ({by}): its reasoning")),
        "a note carries its author in the line:\n{raw}"
    );

    let out = c
        .run(&["--json", "show", "a-node-the-agent-made"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["title_by"], by);
    assert_eq!(v["node"]["kill_by"], by);
    assert_eq!(v["node"]["edges"][0]["by"], by);
    assert_eq!(v["node"]["references"][0]["by"], by);
    assert_eq!(v["notes"][0]["by"], by);

    // Orbit's provenance answers a different question and is untouched by it.
    c.run(&["new", "Run provenance", "--task", "ORB-1", "--by", by])
        .assert_ok();
    let orbit = std::fs::read_to_string(c.node_file("run-provenance")).unwrap();
    assert!(
        orbit.contains(&format!("title_by: {by}")) && orbit.contains("origin:\n  task: ORB-1"),
        "{orbit}"
    );

    // A label that would make a note line ambiguous is refused outright.
    c.run(&["note", "--by", "crew (alpha)", "run-provenance", "x"])
        .assert_fails()
        .says("cannot be an author label");
}

/// A label that cannot be an author has its own code, not the catch-all.
#[test]
fn invalid_author_label_has_its_own_code() {
    let c = Corpus::new();
    let refused = c
        .run(&["--json", "new", "t", "--by", "a (b)"])
        .usage_refusal();
    assert_eq!(refused["code"], "invalid_author_label", "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains("`a (b)`"),
        "{refused}"
    );
    assert_eq!(std::fs::read_dir(c.root.join("nodes")).unwrap().count(), 0);
}

/// The point of recording authorship: a kill condition the agent proposed is
/// not yet the human's claim, and `review` says so until one is confirmed.
#[test]
fn an_agent_kill_stays_on_review_until_a_human_confirms_it() {
    let c = Corpus::new();
    let by = "agent:crew-alpha";
    let id = c.seed("a sharpenable idea", "A sharpenable idea");
    c.run(&[
        "sharpen",
        &id,
        "--kill",
        "if the corpus stays small",
        "--by",
        by,
    ])
    .assert_ok();

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).expect("review --json");
    let found = items
        .iter()
        .find(|i| i["rule"] == "unconfirmed-kill")
        .unwrap_or_else(|| panic!("no unconfirmed-kill finding in {json}"));
    assert_eq!(found["id"], id.as_str());
    assert!(found["reason"].as_str().unwrap().contains(by), "{json}");
    c.run(&["review"])
        .assert_ok()
        .says("## Agent-authored kills not yet confirmed by a human")
        .says(&id);
    c.run(&["show", &id]).assert_ok().says(&format!("({by})"));

    // Confirming changes the author and nothing else.
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    c.run(&["sharpen", &id, "--confirm"]).assert_ok();
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(
        after.contains("kill: if the corpus stays small"),
        "the kill text is untouched:\n{after}"
    );
    assert!(!after.contains("kill_by"), "{after}");
    assert_eq!(
        before.replace(&format!("kill_by: {by}\n"), ""),
        after,
        "confirming appends nothing and rewrites nothing else"
    );

    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["kill_by"], "human");
    assert_eq!(v["node"]["status"], "hypothesis");
    assert!(v["notes"].as_array().is_none_or(Vec::is_empty), "{out}");

    // A confirmed falsifier is human-owned content, not a field an agent can
    // quietly replace by sharpening the already-open hypothesis again.
    let confirmed = std::fs::read_to_string(c.node_file(&id)).unwrap();
    let refused = c
        .run(&[
            "sharpen",
            &id,
            "--kill",
            "if an agent prefers a different test",
            "--by",
            by,
        ])
        .assert_fails();
    assert!(refused.stdout().is_empty(), "{}", refused.stdout());
    assert!(
        refused
            .stderr()
            .contains("kill condition is already `if the corpus stays small`; it was not replaced"),
        "{}",
        refused.stderr()
    );
    assert!(
        !refused.stderr().contains("is now hypothesis"),
        "a refused re-sharpen must not report a status transition: {}",
        refused.stderr()
    );
    assert_eq!(
        confirmed,
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        "a refused re-sharpen leaves the confirmed falsifier untouched"
    );

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).expect("review --json");
    assert!(
        !items.iter().any(|i| i["rule"] == "unconfirmed-kill"),
        "a confirmed kill is off the list: {json}"
    );

    // Confirming what nobody wrote is a refusal, not a silent no-op.
    let seed = c.seed("an unsharpened idea", "An unsharpened idea");
    c.run(&["sharpen", &seed, "--confirm"])
        .assert_fails()
        .says("no kill condition to confirm");
    // `--confirm` is not a way to rewrite the text.
    c.run(&["sharpen", &id, "--kill", "something else", "--confirm"])
        .assert_fails()
        .says("cannot be used with");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// A v2 node as it was written before authorship existed: no `title_by`, no
/// `kill_by`, no `by` on the edge, the reference or the note.
const PRE_AUTHORSHIP_NODE: &str = r"---
id: written-by-hand
title: Written by hand
status: hypothesis
created: 2026-09-01
updated: 2026-09-01
kill: if nobody ever writes one
edges:
- type: derives-from
  to: also-by-hand
references:
- id: r1
  kind: article
  uri: https://example.org
  note: why it is here
  added: 2026-09-01
---

the argument

## Notes

- 2026-09-02: an older note
";

const PRE_AUTHORSHIP_PARENT: &str = r"---
id: also-by-hand
title: Also by hand
status: seed
created: 2026-09-01
updated: 2026-09-01
---

the older argument
";

/// A node written before authorship existed loads as the human's own, and
/// `neb migrate` has nothing to do about it either way.
#[test]
fn a_corpus_written_before_authorship_loads_unchanged() {
    let c = Corpus::new();
    write(&c.node_file("written-by-hand"), PRE_AUTHORSHIP_NODE);
    write(&c.node_file("also-by-hand"), PRE_AUTHORSHIP_PARENT);
    // One authored node beside them, so migration has both shapes to keep.
    c.run(&["new", "An authored node", "--by", "agent:crew-alpha"])
        .assert_ok();

    c.run(&["check"]).assert_ok().says("0 errors");
    let out = c
        .run(&["--json", "show", "written-by-hand"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    for field in ["title_by", "kill_by"] {
        assert_eq!(v["node"][field], "human", "{field} in {out}");
    }
    assert_eq!(v["node"]["edges"][0]["by"], "human");
    assert_eq!(v["node"]["references"][0]["by"], "human");
    assert_eq!(v["notes"][0]["by"], "human");
    let report = c.run(&["review", "--json"]).assert_ok().stdout();
    assert!(
        !report.contains("unconfirmed-kill"),
        "a kill nobody attributed is the human's own: {report}"
    );

    let before = snapshot_corpus_files(&c.root);
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(
        before,
        snapshot_corpus_files(&c.root),
        "migration is a no-op: absent authorship reads as human, and an \
         authored node keeps its labels"
    );
}

// -------------------------------------------------------------------- tags --

#[test]
fn tags_are_normalised_on_every_write_path() {
    let c = Corpus::new();
    c.run(&[
        "new",
        "Direct",
        "--tag",
        "Physics",
        "--tag",
        "Machine Learning",
    ])
    .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("direct")).unwrap();
    assert!(
        raw.contains("tags:\n- physics\n- machine-learning\n"),
        "{raw}"
    );

    let entry = c.run(&["capture", "promoted with a tag"]).stdout_trim();
    c.run(&["promote", &entry, "--title", "Promoted", "--tag", "ORRERY"])
        .assert_ok();
    let raw = std::fs::read_to_string(c.node_file("promoted")).unwrap();
    assert!(raw.contains("tags:\n- orrery\n"), "{raw}");

    c.run(&["tag", "promoted", "--add", "Physics", "--add", "physics"])
        .assert_ok()
        .says("orrery, physics");
    c.run(&["tag", "promoted", "--remove", "ORRERY"])
        .assert_ok()
        .says("physics");
    let raw = std::fs::read_to_string(c.node_file("promoted")).unwrap();
    assert!(raw.contains("tags:\n- physics\n"), "{raw}");
    c.run(&["tag", "promoted"])
        .assert_fails()
        .says("--add <tag> or --remove <tag>");

    c.run(&["tag", "list"])
        .assert_ok()
        .says("machine-learning\t1\n")
        .says("physics\t2\n");
    let json = c.run(&["--json", "tag", "list"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["tag"] == "physics" && i["count"] == 2),
        "{json}"
    );

    // `--tag` filters are AND, and normalised the same way as the writes.
    let out = c.run(&["list", "--tag", "Physics"]).assert_ok().stdout();
    assert!(out.contains("direct") && out.contains("promoted"), "{out}");
    let out = c
        .run(&["list", "--tag", "physics", "--tag", "machine-learning"])
        .assert_ok()
        .stdout();
    assert!(out.contains("direct") && !out.contains("promoted"), "{out}");
    c.run(&["list", "--tag", "nope"])
        .assert_ok()
        .says("no nodes match");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// A tag not there to remove, or already there to add, is noted on stderr
/// naming the tag and the node; when that leaves the tags as they were,
/// the node is not written and nothing is committed (STD-01 §R30). A call
/// with one real change in it writes that change and notes the rest.
#[test]
fn tag_noop_changes_nothing_and_says_so() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Tagged", "--tag", "physics"])
        .assert_ok()
        .stdout_trim();
    set_updated(&c.node_file(&id), &date_days_ago(7));
    git(&c.root, &["commit", "-qam", "back-dated"]);
    let before = std::fs::read(c.node_file(&id)).unwrap();
    let commits = log(&c.root);

    for (flag, tag, says) in [
        ("--remove", "absent", "nothing to remove"),
        ("--add", "physics", "nothing to add"),
    ] {
        let run = c.run(&["tag", &id, flag, tag]).assert_ok();
        let stderr = run.stderr();
        assert!(stderr.contains(&format!("`{tag}`")), "{stderr}");
        assert!(stderr.contains(&id) && stderr.contains(says), "{stderr}");
        assert!(stderr.contains("no change"), "{stderr}");
        assert!(!stderr.contains("committed"), "{stderr}");
        assert_eq!(std::fs::read(c.node_file(&id)).unwrap(), before, "{flag}");
        assert_eq!(log(&c.root), commits, "{flag}: no commit");
    }

    let run = c
        .run(&["tag", &id, "--add", "new", "--remove", "absent"])
        .assert_ok();
    let stderr = run.stderr();
    assert!(
        stderr.contains("`absent`") && stderr.contains(&id),
        "{stderr}"
    );
    assert!(!stderr.contains("no change"), "{stderr}");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("- new"), "{raw}");
    assert!(
        raw.contains(&format!("updated: {}", date_days_ago(0))),
        "{raw}"
    );
    assert_eq!(log(&c.root)[0], format!("neb tag {id}"));
}

#[test]
fn tag_drift_by_case_or_plural_is_a_warning() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["tag", &a, "--add", "physics", "--add", "sim"])
        .assert_ok();
    c.run(&["tag", &b, "--add", "physics", "--add", "sims"])
        .assert_ok();
    // Writes normalise case, so the case collision has to be a hand edit.
    let raw = std::fs::read_to_string(c.node_file(&b)).unwrap();
    write(&c.node_file(&b), &raw.replace("- physics", "- Physics"));
    c.run(&["check"])
        .assert_ok()
        .says("[11]")
        .says(&format!(
            "tags `Physics` (on {b}) and `physics` (on {a}) differ only by case"
        ))
        .says(&format!(
            "tags `sim` (on {a}) and `sims` (on {b}) differ only by a trailing `s`"
        ))
        .says("0 errors, 2 warnings");

    let json = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&json).expect("check --json");
    let drift: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == 11)
        .map(|f| f["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        drift,
        [
            format!("tags `Physics` (on {b}) and `physics` (on {a}) differ only by case"),
            format!("tags `sim` (on {a}) and `sims` (on {b}) differ only by a trailing `s`"),
        ],
        "{json}"
    );
}

/// Drift is noted at the write that introduces it, on stderr, and never
/// refused: there is no declared list to refuse against.
#[test]
fn a_write_introducing_a_near_variant_of_a_tag_notes_it_and_succeeds() {
    let c = Corpus::new();
    c.run(&["new", "First", "--tag", "physics"]).assert_ok();
    c.run(&["new", "Second", "--tag", "Physics"]).assert_ok();

    let run = c.run(&["new", "Third", "--tag", "physic"]).assert_ok();
    assert_eq!(run.stdout_trim(), "third");
    assert_eq!(
        run.stderr(),
        "note: tag physic is close to physics (2 nodes)\n"
    );
    let raw = std::fs::read_to_string(c.node_file("third")).unwrap();
    assert!(
        raw.contains("tags:\n- physic\n"),
        "the tag is written as given: {raw}"
    );

    // `tag --add` says the same, and `--json` keeps stdout the payload.
    let run = c
        .run(&["--json", "tag", "first", "--add", "Orrerys"])
        .assert_ok();
    assert!(
        run.stderr().is_empty(),
        "nothing close yet: {}",
        run.stderr()
    );
    let run = c
        .run(&["--json", "tag", "second", "--add", "orrery"])
        .assert_ok();
    serde_json::from_str::<serde_json::Value>(&run.stdout()).expect("stdout stays JSON");
    assert_eq!(
        run.stderr(),
        "note: tag orrery is close to orrerys (1 node)\n"
    );
    let run = c.run(&["tag", "list"]).assert_ok();
    assert!(run.stderr().is_empty(), "a read notes nothing");

    // A tag someone else already carries is not new to the corpus, so the
    // write adds nothing to what `check` already says.
    let run = c.run(&["tag", "first", "--add", "physic"]).assert_ok();
    assert!(run.stderr().is_empty(), "{}", run.stderr());
    let run = c.run(&["new", "Fourth", "--tag", "design"]).assert_ok();
    assert!(run.stderr().is_empty(), "{}", run.stderr());
}

#[test]
fn promote_notes_a_tag_close_to_one_in_use() {
    let c = Corpus::new();
    c.run(&["new", "First", "--tag", "sims"]).assert_ok();
    let entry = c.run(&["capture", "a promoted thought"]).stdout_trim();
    c.run(&["promote", &entry, "--title", "Promoted", "--tag", "SIM"])
        .assert_ok()
        .says("note: tag sim is close to sims (1 node)");
}

// ------------------------------------------------------------------ review --

#[test]
fn review_reports_stale_hypotheses_on_both_sides_of_thirty_days() {
    let c = Corpus::new();
    let stale = c.seed("an old hypothesis", "An old hypothesis");
    c.run(&["sharpen", &stale, "--kill", "if X"]).assert_ok();
    c.run(&["cite", &stale, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&stale), &date_days_ago(45));

    let fresh = c.seed("a fresh hypothesis", "A fresh hypothesis");
    c.run(&["sharpen", &fresh, "--kill", "if Y"]).assert_ok();
    c.run(&["cite", &fresh, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&fresh), &date_days_ago(15));

    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains(&format!("`{stale}`")),
        "stale hypothesis missing:\n{out}"
    );
    assert!(
        !out.contains(&format!("`{fresh}`")),
        "fresh hypothesis should not be flagged:\n{out}"
    );
}

#[test]
fn review_reports_untouched_seeds_on_both_sides_of_ninety_days() {
    let c = Corpus::new();
    let stale = c.seed("an old seed", "An old seed");
    c.run(&["cite", &stale, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&stale), &date_days_ago(120));

    let fresh = c.seed("a fresh seed", "A fresh seed");
    c.run(&["cite", &fresh, "--uri", "http://example.com", "--note", "n"])
        .assert_ok();
    set_updated(&c.node_file(&fresh), &date_days_ago(60));

    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains(&format!("`{stale}`")) && out.contains("propose: status abandoned"),
        "stale seed missing:\n{out}"
    );
    assert!(
        !out.contains(&format!("`{fresh}`")),
        "fresh seed should not be flagged:\n{out}"
    );
}

#[test]
fn review_and_its_short_form_report_only_aged_hypotheses_with_no_references() {
    let c = Corpus::new();
    let seed = c.seed("an old seed", "An old seed");
    set_created(&c.node_file(&seed), &date_days_ago(30));

    let hypothesis = c.seed("an old hypothesis", "An old hypothesis");
    c.run(&["sharpen", &hypothesis, "--kill", "if X"])
        .assert_ok();
    set_created(&c.node_file(&hypothesis), &date_days_ago(30));

    for args in [["review", "--short"].as_slice(), &["review"]] {
        let out = c.run(args).assert_ok().stdout();
        assert!(
            !out.contains(&seed),
            "{args:?} incorrectly reported an old seed with no references:\n{out}"
        );
        assert!(
            out.contains(&hypothesis),
            "{args:?} missed an old hypothesis with no references:\n{out}"
        );
    }
}

#[test]
fn review_reports_stale_inbox_entries_on_both_sides_of_fourteen_days() {
    let c = Corpus::new();
    c.run(&["capture", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(
        out.contains("1 capture waiting over fourteen days; promote or drop them"),
        "{out}"
    );

    let fresh = Corpus::new();
    fresh.run(&["capture", "a recent capture"]).assert_ok();
    set_inbox_stamp_for(&fresh.root, "a recent capture", &stamp_days_ago(10));
    let out = fresh.run(&["review"]).assert_ok().stdout();
    assert!(!out.contains("waiting over fourteen days"), "{out}");
}

#[test]
fn review_json_emits_all_four_rule_names() {
    let c = Corpus::new();

    let stale_hyp = c.seed("stale hypothesis", "Stale hypothesis");
    c.run(&["sharpen", &stale_hyp, "--kill", "if X"])
        .assert_ok();
    c.run(&[
        "cite",
        &stale_hyp,
        "--uri",
        "http://example.com",
        "--note",
        "n",
    ])
    .assert_ok();
    set_updated(&c.node_file(&stale_hyp), &date_days_ago(45));

    let stale_seed = c.seed("stale seed", "Stale seed");
    c.run(&[
        "cite",
        &stale_seed,
        "--uri",
        "http://example.com",
        "--note",
        "n",
    ])
    .assert_ok();
    set_updated(&c.node_file(&stale_seed), &date_days_ago(120));

    let bare_hypothesis = c.seed("bare hypothesis", "Bare hypothesis");
    c.run(&["sharpen", &bare_hypothesis, "--kill", "if Z"])
        .assert_ok();
    set_created(&c.node_file(&bare_hypothesis), &date_days_ago(14));

    c.run(&["capture", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&json).expect("review --json is valid JSON");
    let rules: std::collections::HashSet<&str> =
        items.iter().map(|i| i["rule"].as_str().unwrap()).collect();
    for rule in [
        "stale-hypothesis",
        "untouched-seed",
        "no-references",
        "stale-inbox",
    ] {
        assert!(rules.contains(rule), "missing rule `{rule}` in {json}");
    }
    for item in &items {
        assert!(item["id"].is_string());
        assert!(item["title"].is_string());
        assert!(item["reason"].is_string());
    }
}

/// `--out` writes the report to the file and names it on stderr, in both
/// modes, so stdout stays empty and the write says what it wrote (STD-01
/// §R30).
#[test]
fn review_out_names_the_file() {
    let c = Corpus::new();
    let bare = c.seed("a hypothesis needing a look", "A hypothesis needing a look");
    c.run(&["sharpen", &bare, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&bare), &date_days_ago(14));
    let out_path = c.workdir().join("review.md");
    let run = c
        .run(&["review", "--out", out_path.to_str().unwrap()])
        .assert_ok();
    assert_eq!(
        run.stdout(),
        "",
        "stdout must stay empty when writing to a file"
    );
    assert_eq!(run.stderr(), format!("wrote {}\n", out_path.display()));
    let report = std::fs::read_to_string(&out_path).unwrap();
    assert!(report.contains(&format!("`{bare}`")));
    assert!(report.contains("## Nodes with no references"));

    let json_path = c.workdir().join("review.json");
    let run = c
        .run(&["--json", "review", "--out", json_path.to_str().unwrap()])
        .assert_ok();
    assert_eq!(run.stdout(), "");
    assert_eq!(run.stderr(), format!("wrote {}\n", json_path.display()));
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert!(
        report.as_array().is_some_and(|items| !items.is_empty()),
        "{report}"
    );
}

/// A report that cannot be written is refused naming the path it was for.
#[test]
fn report_out_write_failure_names_the_path() {
    let c = Corpus::new();
    let out_path = c.workdir().join("no-such-dir").join("review.md");
    let refused = c
        .run(&["--json", "review", "--out", out_path.to_str().unwrap()])
        .refusal();
    assert_eq!(refused["code"], "io_at", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains(&out_path.display().to_string()),
        "{refused}"
    );
    assert!(!out_path.exists());
}

/// Piped, a listing is one tab-separated line per node with no header, and
/// every line has every field: an untagged node's tags are `-`, so `cut -f`
/// never shifts (STD-01 §R9).
#[test]
fn piped_list_is_tab_separated() {
    let c = Corpus::new();
    c.run(&["new", "A seed", "--tag", "physics", "--tag", "design"])
        .assert_ok();
    c.run(&["new", "A hypothesis", "--kill", "if X"])
        .assert_ok();
    c.run(&["new", "Untagged idea"]).assert_ok();
    let out = c.run(&["list"]).assert_ok().stdout();
    let rows: Vec<Vec<&str>> = out.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 3, "one line per node, no header: {out}");
    assert!(rows.iter().all(|r| r.len() == 4), "{out}");
    assert!(!out.contains("STATUS") && !out.contains('\x1b'), "{out}");
    let row = |id: &str| {
        rows.iter()
            .find(|r| r[1] == id)
            .unwrap_or_else(|| panic!("{id} in: {out}"))
    };
    assert_eq!(
        row("a-seed")[..],
        ["seed", "a-seed", "physics,design", "A seed"]
    );
    assert_eq!(
        row("a-hypothesis")[..3],
        ["hypothesis", "a-hypothesis", "-"]
    );
    assert_eq!(
        row("untagged-idea")[..],
        ["seed", "untagged-idea", "-", "Untagged idea"]
    );
}

/// Every other list-shaped verb pipes as `list` does: one tab-separated line
/// per record, the same number of fields on every line, and no header.
#[test]
fn piped_list_shaped_outputs_are_tab_separated() {
    let c = Corpus::new();
    // A hypothesis fourteen days old with no references, so `review --short`
    // has a finding, beside nodes with and without tags.
    let hyp = c.seed("gravity carries information", "Gravity carries information");
    c.run(&["sharpen", &hyp, "--kill", "if X"]).assert_ok();
    c.run(&["tag", &hyp, "--add", "physics"]).assert_ok();
    set_created(&c.node_file(&hyp), &date_days_ago(14));
    c.run(&[
        "new",
        "Gravity is information",
        "--tag",
        "physics",
        "--tag",
        "zz",
    ])
    .assert_ok();
    c.run(&["new", "Untagged gravity idea", "--parent", &hyp])
        .assert_ok();
    c.run(&["capture", "--quiet", "a thought about gravity"])
        .assert_ok();
    c.run(&["capture", "--quiet", "another thought"])
        .assert_ok();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "first");
    c.run(&["note", &hyp, "a second thought"]).assert_ok();
    git_commit_at(&c.root, "2020-01-02", "second");

    for (args, header, fields, records) in [
        (vec!["inbox"], "ID", 3, 2),
        (vec!["near", "gravity information"], "BAND", 5, 3),
        (vec!["near", hyp.as_str()], "BAND", 5, 2),
        (vec!["tag", "list"], "TAG", 2, 2),
        (vec!["review", "--short"], "ID", 2, 1),
        (vec!["log", hyp.as_str()], "HASH", 3, 2),
    ] {
        let out = c.run(&args).assert_ok().stdout();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), records, "{args:?}: {out}");
        for line in &lines {
            let cut: Vec<&str> = line.split('\t').collect();
            assert_eq!(cut.len(), fields, "{args:?}: {out}");
            assert!(cut.iter().all(|f| !f.is_empty()), "{args:?}: {out}");
            assert_ne!(cut[0], header, "no header: {out}");
        }
        assert!(!out.contains("\x1b["), "{args:?}: {out}");
    }
}

/// `--limit` cuts `list` to its first N matches and says on stderr how many
/// it left out, in every mode; without it every match is listed, as before,
/// and the count is stderr's too. `--json` gets the cut list in the envelope
/// that says so.
#[test]
fn list_limit_bounds_the_listing_and_defaults_to_all() {
    let c = Corpus::new();
    for title in ["One", "Two", "Three"] {
        c.run(&["new", title, "--tag", "t"]).assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();

    let all = c.run(&["list"]).assert_ok();
    assert_eq!(all.stdout().lines().count(), 4, "{}", all.stdout());
    assert_eq!(all.stderr(), "4 of 4 nodes\n");
    let tagged = c.run(&["list", "--tag", "t"]).assert_ok();
    assert_eq!(tagged.stdout().lines().count(), 3, "{}", tagged.stdout());
    assert_eq!(tagged.stderr(), "3 of 4 nodes\n");

    let cut = c.run(&["list", "--tag", "t", "--limit", "2"]).assert_ok();
    assert_eq!(cut.stdout().lines().count(), 2, "{}", cut.stdout());
    assert!(
        cut.stdout().lines().all(|l| l.starts_with("seed")),
        "{}",
        cut.stdout()
    );
    assert_eq!(
        cut.stderr(),
        "2 of 3 matching nodes shown, of 4 in all; raise --limit for more\n"
    );
    let roomy = c.run(&["list", "--tag", "t", "--limit", "3"]).assert_ok();
    assert_eq!(
        (roomy.stdout(), roomy.stderr()),
        (tagged.stdout(), tagged.stderr()),
        "a limit nobody reaches changes nothing"
    );
    let unfiltered = c.run(&["list", "--limit", "1"]).assert_ok();
    assert_eq!(unfiltered.stdout().lines().count(), 1);
    assert_eq!(
        unfiltered.stderr(),
        "1 of 4 nodes shown; raise --limit for more\n"
    );

    let json = c.run(&["list", "--json", "--limit", "1"]).assert_ok();
    let listed: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["total"], 4, "{listed}");
    assert_eq!(
        json.stderr(),
        "1 of 4 nodes shown; raise --limit for more\n"
    );

    let single = Corpus::new();
    single.run(&["new", "Alone"]).assert_ok();
    let one = single.run(&["list"]).assert_ok();
    assert_eq!(one.stdout().lines().count(), 1, "{}", one.stdout());
    assert_eq!(one.stderr(), "1 of 1 node\n");
}

/// Piped, `list` is one line per node and nothing else, so `wc -l` counts
/// nodes; the count of the corpus is on stderr.
#[test]
fn piped_list_has_one_line_per_record() {
    let c = Corpus::new();
    for n in 0..150 {
        c.run(&["new", &format!("Filler {n}"), "--tag", "t", "--no-commit"])
            .assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();
    let listed = c.run(&["list", "--tag", "t"]).assert_ok();
    let out = listed.stdout();
    assert_eq!(out.lines().count(), 150, "{out}");
    assert!(out.lines().all(|l| l.starts_with("seed\t")), "{out}");
    assert_eq!(listed.stderr(), "150 of 151 nodes\n");
}

/// `--limit 0` would print an empty listing, or "nothing needs attention",
/// over a corpus with plenty in it. On every verb that takes it, it is a usage
/// error with nothing on stdout, and the least limit still answers.
#[test]
fn limit_zero_is_a_usage_error() {
    let c = Corpus::new();
    let seed = c.seed("an old seed", "An old seed");
    set_updated(&c.node_file(&seed), &date_days_ago(100));
    c.run(&["capture", "-q", "waiting"]).assert_ok();
    for verb in [
        ["list"].as_slice(),
        &["inbox"],
        &["review"],
        &["review", "--short"],
    ] {
        for json in [false, true] {
            let json = if json { ["--json"].as_slice() } else { &[] };
            let zero = c.run(&[json, verb, &["--limit", "0"]].concat());
            assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
            assert_eq!(zero.stdout(), "", "`neb {}`", zero.args);
            assert!(
                zero.stderr().contains("--limit") && zero.stderr().contains("at least 1"),
                "`neb {}`: {}",
                zero.args,
                zero.stderr()
            );
        }
        let one = c.run(&[verb, &["--limit", "1"]].concat()).assert_ok();
        assert_ne!(one.stdout(), "", "`neb {}`", one.args);
    }
}

/// `--limit` cuts the inbox to its oldest entries. How many wait, and how
/// many the cut left out, are on stderr.
#[test]
fn inbox_limit_shows_the_oldest_and_counts_the_rest() {
    let c = Corpus::new();
    for text in ["first thought", "second thought", "third thought"] {
        c.run(&["capture", "--quiet", text]).assert_ok();
    }
    let all = c.run(&["inbox"]).assert_ok();
    assert_eq!(all.stdout().lines().count(), 3, "{}", all.stdout());
    assert_eq!(all.stderr(), "3 waiting. Promote or drop each one.\n");

    let cut = c.run(&["inbox", "--limit", "2"]).assert_ok();
    let listed = cut.stdout();
    assert_eq!(listed.lines().count(), 2, "{listed}");
    assert!(
        listed.contains("first thought") && listed.contains("second thought"),
        "{listed}"
    );
    assert!(!listed.contains("third thought"), "{listed}");
    assert_eq!(
        cut.stderr(),
        "2 of 3 waiting shown; raise --limit for more. Promote or drop each one.\n"
    );

    let json = c.run(&["inbox", "--json", "--limit", "1"]).assert_ok();
    let listed: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["items"][0]["text"], "first thought", "{listed}");
    assert_eq!(listed["total"], 3, "{listed}");
    assert!(
        json.stderr()
            .starts_with("1 of 3 waiting shown; raise --limit"),
        "{}",
        json.stderr()
    );
    let uncut = c.run(&["inbox", "--json"]).assert_ok();
    assert_eq!(uncut.stderr(), "", "a plain count is for a person");
}

/// `review --limit` keeps the first N findings under each heading, so a
/// crowded section cannot push a short one out, and says what it cut: in the
/// markdown, which is the file `--out` writes, and on stderr. With `--short`
/// it keeps the first N lines, and says so on stderr alone.
#[test]
fn review_limit_cuts_each_section_and_the_short_form() {
    let c = Corpus::new();
    for title in ["Cold one", "Cold two", "Cold three"] {
        c.run(&["new", title]).assert_ok();
    }
    for id in ["cold-one", "cold-two", "cold-three"] {
        set_updated(&c.node_file(id), &date_days_ago(100));
    }
    c.run(&["capture", "--quiet", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    let whole = c.run(&["review"]).assert_ok();
    assert_eq!(whole.stdout().matches("`cold-").count(), 3);
    assert!(!whole.stdout().contains("--limit"), "{}", whole.stdout());
    assert_eq!(whole.stderr(), "");

    let cut = c.run(&["review", "--limit", "1"]).assert_ok();
    let report = cut.stdout();
    assert_eq!(report.matches("`cold-").count(), 1, "{report}");
    assert!(
        report.contains("- _… and 2 more; raise --limit for more_"),
        "{report}"
    );
    assert!(
        report.contains("1 capture waiting over fourteen days"),
        "the inbox section keeps its one finding:\n{report}"
    );
    assert_eq!(
        cut.stderr(),
        "2 findings not shown; raise --limit for more\n"
    );
    let json = c.run(&["review", "--json", "--limit", "1"]).assert_ok();
    let items: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(items["items"].as_array().unwrap().len(), 2, "{items}");
    assert_eq!(items["total"], 4, "every rule's findings: {items}");
    assert_eq!(json.stderr(), cut.stderr(), "said in every mode");

    let short = c.run(&["review", "--short"]).assert_ok();
    assert_eq!(short.stdout().lines().count(), 4, "{}", short.stdout());
    assert_eq!(short.stderr(), "");
    let short_cut = c.run(&["review", "--short", "--limit", "2"]).assert_ok();
    assert_eq!(
        short_cut.stdout().lines().count(),
        2,
        "{}",
        short_cut.stdout()
    );
    assert_eq!(short_cut.stderr(), "2 of 4 shown; raise --limit for more\n");
    let json = c
        .run(&["review", "--short", "--json", "--limit", "2"])
        .assert_ok();
    let items: serde_json::Value = serde_json::from_str(&json.stdout()).unwrap();
    assert_eq!(items["items"].as_array().unwrap().len(), 2, "{items}");
    assert_eq!(items["total"], 4, "{items}");
    assert_eq!(json.stderr(), short_cut.stderr(), "said in every mode");
}

/// Run a `--json` command and parse what it printed.
fn json_of(c: &Corpus, args: &[&str]) -> serde_json::Value {
    let out = c.run(args).assert_ok().stdout();
    serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("`neb {}` did not emit JSON: {e}\n{out}", args.join(" ")))
}

/// The STD-01 §R34 envelope: `items` of length `kept`, `total` and
/// `truncated` as given, and no other key.
fn assert_envelope(v: &serde_json::Value, kept: usize, total: usize, truncated: bool) {
    assert_eq!(keys(v), ["items", "total", "truncated"], "{v}");
    assert_eq!(v["items"].as_array().unwrap().len(), kept, "{v}");
    assert_eq!(v["total"], total, "{v}");
    assert_eq!(v["truncated"], truncated, "{v}");
}

/// `list --limit` answers in the envelope, cut or not, with the number the
/// filter matched as `total`; without the flag it is the bare array.
#[test]
fn list_limit_json_envelope() {
    let c = Corpus::new();
    for title in ["One", "Two", "Three"] {
        c.run(&["new", title, "--tag", "t"]).assert_ok();
    }
    c.run(&["new", "Untagged"]).assert_ok();

    assert_envelope(
        &json_of(&c, &["list", "--json", "--limit", "2"]),
        2,
        4,
        true,
    );
    let tagged = json_of(&c, &["list", "--json", "--tag", "t", "--limit", "2"]);
    assert_envelope(&tagged, 2, 3, true);
    assert_eq!(keys(&tagged["items"][0]), NODE_KEYS, "{tagged}");
    assert_envelope(
        &json_of(&c, &["list", "--json", "--limit", "500"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["list", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

/// `inbox --limit` answers in the envelope, cut or not; without the flag it
/// is the bare array.
#[test]
fn inbox_limit_json_envelope() {
    let c = Corpus::new();
    for text in ["first thought", "second thought", "third thought"] {
        c.run(&["capture", "--quiet", text]).assert_ok();
    }
    let cut = json_of(&c, &["inbox", "--json", "--limit", "2"]);
    assert_envelope(&cut, 2, 3, true);
    assert_eq!(cut["items"][0]["text"], "first thought", "{cut}");
    assert_envelope(
        &json_of(&c, &["inbox", "--json", "--limit", "5"]),
        3,
        3,
        false,
    );
    let bare = json_of(&c, &["inbox", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(3), "{bare}");
}

/// `review --limit`'s `total` counts every rule's findings before the cut
/// kept each rule's first N; `review --short --limit` counts its lines. Both
/// are envelopes with the flag, cut or not, and bare arrays without it.
#[test]
fn review_limit_json_envelope() {
    let c = Corpus::new();
    for title in ["Cold one", "Cold two", "Cold three"] {
        c.run(&["new", title]).assert_ok();
    }
    for id in ["cold-one", "cold-two", "cold-three"] {
        set_updated(&c.node_file(id), &date_days_ago(100));
    }
    c.run(&["capture", "--quiet", "an old capture"]).assert_ok();
    set_inbox_stamp_for(&c.root, "an old capture", &stamp_days_ago(20));

    // Three cold seeds under one rule and the stale capture under another.
    let cut = json_of(&c, &["review", "--json", "--limit", "1"]);
    assert_envelope(&cut, 2, 4, true);
    assert_envelope(
        &json_of(&c, &["review", "--json", "--limit", "10"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["review", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");

    assert_envelope(
        &json_of(&c, &["review", "--short", "--json", "--limit", "1"]),
        1,
        4,
        true,
    );
    assert_envelope(
        &json_of(&c, &["review", "--short", "--json", "--limit", "10"]),
        4,
        4,
        false,
    );
    let bare = json_of(&c, &["review", "--short", "--json"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

/// Counts agree with their nouns wherever the reports print one.
#[test]
fn reports_count_in_the_singular_for_one() {
    let c = Corpus::new();
    c.run(&["new", "Alone"]).assert_ok();
    let checked = c.run(&["check"]).assert_ok();
    assert_eq!(checked.stdout(), "", "no findings, so no payload");
    assert_eq!(
        checked.stderr(),
        "1 node, 0 errors, 0 warnings, 0 unreadable files\n"
    );
    let review = c.run(&["review", "--since", "1"]).assert_ok().stdout();
    assert!(
        review.contains("## Hypotheses untouched for 1 day\n"),
        "{review}"
    );
    assert!(
        review.contains("## Seeds untouched for 1 day\n"),
        "{review}"
    );
}

/// A query that finds nothing exits 0 with nothing on stdout and one line
/// on stderr saying so, in every mode; `--json` still prints its empty
/// document (STD-01 §R16).
#[test]
fn empty_results_leave_stdout_empty_and_explain_on_stderr() {
    let empty = Corpus::new();
    let c = Corpus::new();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "initial repository");
    let root = c.seed("a root thought", "Root thought");
    c.run(&["new", "Leaf", "--parent", &root]).assert_ok();

    for (corpus, args, said) in [
        (&c, vec!["list", "--status", "refuted"], "no nodes match"),
        (&c, vec!["near", "xyzzy"], "nothing near"),
        (&c, vec!["impact", "leaf"], "nothing descends from"),
        (
            &c,
            vec!["review", "--short", "--tag", "none"],
            "nothing needs attention",
        ),
        (&empty, vec!["tag", "list"], "no tags"),
        (&empty, vec!["inbox"], "inbox is empty"),
        (&c, vec!["log", "leaf"], "no commits touched this node"),
    ] {
        let human = corpus.run(&args).assert_ok();
        assert_eq!(human.stdout(), "", "`neb {}`", human.args);
        assert_eq!(human.stderr().lines().count(), 1, "`neb {}`", human.args);
        assert!(human.stderr().contains(said), "{}", human.stderr());

        let json = corpus
            .run(&[["--json"].as_slice(), &args].concat())
            .assert_ok();
        let doc: serde_json::Value = serde_json::from_str(&json.stdout())
            .unwrap_or_else(|e| panic!("`neb {}`: {e}\n{}", json.args, json.stdout()));
        // `near` is always the capped-list envelope (STD-01 §R34).
        let records = if doc.is_object() { &doc["items"] } else { &doc };
        assert_eq!(records, &serde_json::json!([]), "`neb {}`", json.args);
        assert_eq!(json.stderr(), human.stderr(), "`neb {}`", json.args);
    }
}

/// A notice that `--limit` or `--depth` cut the result is stderr's in every
/// mode, and stdout never mentions the bound (STD-01 §R12, §R34).
#[test]
fn limit_notices_go_to_stderr_in_every_mode() {
    let c = Corpus::new();
    let root = c.seed("a root thought", "Root");
    for title in ["One", "Two"] {
        c.run(&["new", title, "--parent", &root]).assert_ok();
        set_updated(&c.node_file(&title.to_lowercase()), &date_days_ago(100));
    }
    c.run(&["new", "Three", "--parent", "one"]).assert_ok();
    for text in ["first", "second", "third"] {
        c.run(&["capture", "-q", text]).assert_ok();
    }
    for args in [
        vec!["list", "--limit", "1"],
        vec!["inbox", "--limit", "1"],
        vec!["review", "--short", "--limit", "1"],
        vec!["trace", &root, "--down", "--depth", "1"],
    ] {
        for json in [false, true] {
            let args = if json {
                [["--json"].as_slice(), &args].concat()
            } else {
                args.clone()
            };
            let run = c.run(&args).assert_ok();
            let out = run.stdout();
            assert!(
                !out.contains("--limit") && !out.contains("--depth"),
                "`neb {}`:\n{out}",
                run.args
            );
            let err = run.stderr();
            assert_eq!(err.lines().count(), 1, "`neb {}`:\n{err}", run.args);
            assert!(
                err.contains("raise --limit for more") || err.contains("raise --depth for more"),
                "`neb {}`:\n{err}",
                run.args
            );
        }
    }
}

#[test]
fn an_empty_corpus_produces_an_empty_review() {
    let c = Corpus::new();
    let out = c.run(&["review"]).assert_ok().stdout();
    assert!(out.contains("## Hypotheses untouched for 30 days"));
    assert!(out.contains("## Seeds untouched for 90 days"));
    assert!(out.contains("## Nodes with no references"));
    assert!(out.contains("## Agent-authored kills not yet confirmed by a human"));
    assert!(out.contains("## Inbox entries waiting 14 days or more"));
    assert_eq!(
        out.matches("_none_").count(),
        5,
        "every section is empty:\n{out}"
    );

    let json = c.run(&["review", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(items.is_empty(), "{json}");
}

/// Every file under `nodes/` and `inbox/`, by path, for a before/after diff.
fn snapshot_corpus_files(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
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

#[test]
fn review_never_touches_nodes_or_inbox_on_disk() {
    let c = Corpus::new();
    let hyp = c.seed("an idea", "An idea");
    c.run(&["sharpen", &hyp, "--kill", "if X"]).assert_ok();
    c.run(&["capture", "a thought"]).assert_ok();

    let before = snapshot_corpus_files(&c.root);
    c.run(&["review"]).assert_ok();
    c.run(&["review", "--json"]).assert_ok();
    let out_path = c.workdir().join("review.md");
    c.run(&["review", "--out", out_path.to_str().unwrap()])
        .assert_ok();
    let after = snapshot_corpus_files(&c.root);

    assert_eq!(before, after, "review must never mutate nodes/ or inbox/");
}

// -------------------------------------------------------------- observatory --

/// An Observatory checkout in research layout v2: records are files under
/// `questions/`, `hypotheses/` and `theories/`, and directories under
/// `research/`. Only what a test cites is created, so an id that should not
/// resolve genuinely does not.
fn observatory(at: &Path) -> PathBuf {
    let root = at.join("observatory");
    for dir in ["questions", "hypotheses", "theories", "research"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    write(
        &root
            .join("questions")
            .join("Q002-is-proper-time-a-count-of-snapshots-along-a-worldline.md"),
        "# Q002\n",
    );
    write(
        &root.join("theories").join("T003-ppn-reduction.md"),
        "# T003\n",
    );
    std::fs::create_dir_all(root.join("research").join("R012-arc")).unwrap();
    root
}

/// The portable citation: the corpus stores `Q002` and nothing about this
/// machine, and the path is reconstructed from the configured root.
#[test]
fn an_observatory_reference_stores_the_bare_record_id_and_show_resolves_it() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let id = c.seed(
        "proper time is a count of snapshots",
        "Proper time is a count",
    );
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();

    // Lower case on the way in: the id is the same record either way.
    c.run(&[
        "cite",
        &id,
        "--kind",
        "observatory",
        "--uri",
        "q002",
        "--note",
        "the question this seed became",
    ])
    .assert_ok();

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("kind: observatory"), "{raw}");
    assert!(raw.contains("uri: Q002"), "the id is stored bare:\n{raw}");
    assert!(
        !raw.contains(obs.to_str().unwrap()),
        "no machine path may reach the corpus:\n{raw}"
    );

    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    let resolved = obs
        .join("questions")
        .join("Q002-is-proper-time-a-count-of-snapshots-along-a-worldline.md");
    c.run(&["show", &id])
        .assert_ok()
        .says("observatory")
        .says("Q002")
        .says(resolved.to_str().unwrap());

    let out = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("show --json is valid JSON");
    assert_eq!(v["node"]["references"][0]["uri"], "Q002");
    assert_eq!(v["observatory"][0]["reference"], "r1");
    assert_eq!(v["observatory"][0]["record"], "Q002");
    assert_eq!(v["observatory"][0]["path"], resolved.to_str().unwrap());

    // Every letter of the scheme, including a research record, which is a
    // directory rather than a file.
    for record in ["T003", "R012"] {
        c.run(&[
            "cite",
            &id,
            "--kind",
            "observatory",
            "--uri",
            record,
            "--note",
            "n",
        ])
        .assert_ok();
    }
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// A record the checkout does not carry is a warning: the citation is still
/// the truth, and this machine is merely behind.
#[test]
fn an_unresolved_observatory_record_warns_and_never_errors() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let id = c.seed("an idea", "An idea");
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    c.run(&[
        "cite",
        &id,
        "--kind",
        "observatory",
        "--uri",
        "Q404",
        "--note",
        "a question that is not written yet",
    ])
    .assert_ok()
    .says("does not resolve under");

    c.run(&["check"])
        .assert_ok()
        .says("warn")
        .says("[9]")
        .says("Observatory record `Q404`")
        .says("does not resolve under")
        .says("0 errors, 1 warning");

    c.run(&["show", &id])
        .assert_ok()
        .says("does not resolve; check the observatory root");
}

/// With no root set there is nothing to resolve against, which is a fact
/// about the machine rather than about the corpus: still a warning, and one
/// that says how to fix it.
#[test]
fn an_observatory_reference_with_no_root_warns_and_the_env_supplies_one() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "observatory",
        "--uri",
        "Q002",
        "--note",
        "why",
    ])
    .assert_ok()
    .says("No observatory root set");

    c.run(&["check"])
        .assert_ok()
        .says("warn")
        .says("[9]")
        .says("no observatory root is set")
        .says("0 errors, 1 warning");
    c.run(&["show", &id])
        .assert_ok()
        .says("does not resolve; check the observatory root");

    // $OBSERVATORY_ROOT outranks every stored setting, so a machine that
    // exports one needs no other.
    c.run_with_env(&["check"], &[("OBSERVATORY_ROOT", obs.to_str().unwrap())])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

/// The Observatory checkout's path belongs to the machine: `config` writes it
/// to `~/.config/nebula/observatory-root`, leaves the corpus's `config.yaml`
/// byte for byte, and reports which setting is in force, with the
/// environment outranking the machine's file.
#[test]
fn the_observatory_root_is_a_machine_setting_that_never_reaches_the_corpus() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let elsewhere = c.workdir().join("elsewhere");
    let config = std::fs::read(c.root.join("config.yaml")).unwrap();
    let machine = c
        .workdir()
        .join(".config")
        .join("nebula")
        .join("observatory-root");

    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says("no observatory root");

    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok()
        .says(obs.to_str().unwrap())
        .says("this machine");
    assert_eq!(
        config,
        std::fs::read(c.root.join("config.yaml")).unwrap(),
        "setting the observatory root rewrote the corpus's config.yaml"
    );
    assert_eq!(
        std::fs::read_to_string(&machine).unwrap(),
        format!("{}\n", obs.display())
    );

    let out = c
        .run(&["--json", "config", "observatory-root"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("config --json is valid JSON");
    assert_eq!(v["root"], obs.to_str().unwrap());
    assert_eq!(v["source"], "machine");
    assert!(v["legacy"].is_null(), "{v}");

    // The environment outranks the machine's file, and says so.
    c.run_with_env(
        &["config", "observatory-root"],
        &[("OBSERVATORY_ROOT", elsewhere.to_str().unwrap())],
    )
    .assert_ok()
    .says(elsewhere.to_str().unwrap())
    .says("$OBSERVATORY_ROOT");
    let out = c
        .run_with_env(
            &["--json", "config", "observatory-root"],
            &[("OBSERVATORY_ROOT", elsewhere.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["root"], elsewhere.to_str().unwrap());
    assert_eq!(v["source"], "env");

    // Saving a setting the environment outranks is not silently moot.
    c.run_with_env(
        &["config", "observatory-root", obs.to_str().unwrap()],
        &[("OBSERVATORY_ROOT", elsewhere.to_str().unwrap())],
    )
    .assert_ok()
    .says("outranks it");

    // Read from whatever directory a command runs in, so it must be
    // absolute; the refusal writes nothing.
    c.run(&["config", "observatory-root", "observatory"])
        .assert_fails()
        .says("must be an absolute path")
        .says("`observatory`")
        .says("pass the checkout's absolute path");
    let refused = c
        .run(&["--json", "config", "observatory-root", "observatory"])
        .usage_refusal();
    assert_eq!(refused["code"], "relative_observatory_root");
    assert_eq!(
        refused["error"],
        "the observatory root must be an absolute path, not `observatory`"
    );
    assert!(
        refused["hint"]
            .as_str()
            .is_some_and(|h| h.contains("absolute path")),
        "{refused}"
    );
    assert_eq!(
        std::fs::read_to_string(&machine).unwrap(),
        format!("{}\n", obs.display())
    );

    // A broken machine file is refused rather than skipped, and names
    // itself; the environment, which outranks it, still works.
    write(&machine, "\n");
    c.run(&["config", "observatory-root"])
        .assert_fails()
        .says("must be an absolute path")
        .says(machine.to_str().unwrap())
        .says("Set it again with an absolute path");
    let refused = c.run(&["--json", "config", "observatory-root"]).refusal();
    assert_eq!(refused["code"], "relative_observatory_root");
    assert!(
        refused["hint"]
            .as_str()
            .is_some_and(|h| h.contains(machine.to_str().unwrap())),
        "{refused}"
    );
    c.run_with_env(
        &["config", "observatory-root"],
        &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
    )
    .assert_ok()
    .says(obs.to_str().unwrap());
}

/// Give the corpus the `observatory_root` key an older `neb` wrote into
/// `config.yaml`: another machine's absolute path, as the real synced corpus
/// carried. Written by hand, since nothing in this build writes it any more.
fn with_legacy_observatory_root(c: &Corpus) -> String {
    let foreign = "/Users/someone-else/workspace/observatory".to_string();
    let config = c.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(&config, &format!("{raw}observatory_root: {foreign}\n"));
    foreign
}

/// The corpus that prompted the move: synced from another machine with that
/// machine's path in `config.yaml`. The key still answers when nothing else
/// does, so no existing corpus breaks, and `check` says it is machine-specific;
/// this machine's setting outranks it, the environment outranks both, and
/// `--drop-legacy` removes it once it is no longer needed.
#[test]
fn a_foreign_legacy_observatory_root_yields_to_this_machines_setting() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let foreign = with_legacy_observatory_root(&c);
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "observatory",
        "--uri",
        "Q002",
        "--note",
        "why",
    ])
    .assert_ok();
    let resolved = obs
        .join("questions")
        .join("Q002-is-proper-time-a-count-of-snapshots-along-a-worldline.md");

    // Nothing on this machine, so the legacy key answers — and cannot
    // resolve anything here.
    let out = c
        .run(&["--json", "config", "observatory-root"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["root"], foreign.as_str());
    assert_eq!(v["source"], "config");
    assert_eq!(v["legacy"], foreign.as_str());
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says("(config.yaml, legacy)")
        .says("--drop-legacy");
    c.run(&["check"])
        .assert_ok()
        .says(&format!("does not resolve under {foreign}"))
        .says(&format!("config.yaml sets observatory_root to {foreign}"))
        .says("machine-specific")
        .says("0 errors, 2 warnings");

    // This machine's setting outranks the key, which `check` still names.
    let bogus = c.workdir().join("not-observatory");
    c.run(&["config", "observatory-root", bogus.to_str().unwrap()])
        .assert_ok()
        .says("config.yaml still carries a legacy observatory_root");
    c.run(&["check"])
        .assert_ok()
        .says(&format!("does not resolve under {}", bogus.display()))
        .says("ignored here in favour of this machine's setting");

    // The environment outranks the machine's setting.
    c.run_with_env(&["check"], &[("OBSERVATORY_ROOT", obs.to_str().unwrap())])
        .assert_ok()
        .says("ignored here in favour of $OBSERVATORY_ROOT")
        .says("0 errors, 1 warning");

    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    c.run(&["check"])
        .assert_ok()
        .says("ignored here in favour of this machine's setting")
        .says("0 errors, 1 warning");
    c.run(&["show", &id])
        .assert_ok()
        .says(resolved.to_str().unwrap());
    let out = c
        .run(&["--json", "config", "observatory-root"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["root"], obs.to_str().unwrap());
    assert_eq!(v["source"], "machine");
    assert_eq!(v["legacy"], foreign.as_str());
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(raw.contains(&foreign), "{raw}");
    assert!(!raw.contains(obs.to_str().unwrap()), "{raw}");

    // Once every machine has its own, the key goes, and the rest of the
    // file stays machine-written and whole.
    c.run(&["config", "observatory-root", "--drop-legacy"])
        .assert_ok()
        .says(obs.to_str().unwrap());
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(
        raw.starts_with("# nebula corpus configuration. Not edited by hand.\n"),
        "{raw}"
    );
    assert!(raw.contains("schema_version: 2"), "{raw}");
    assert!(raw.contains("corpus_id:"), "{raw}");
    assert!(!raw.contains("observatory_root"), "{raw}");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// `--drop-legacy` says what it did to `config.yaml`, in every mode: without
/// the key there is nothing to remove, so the file is left byte for byte and
/// nothing is committed; with it, stderr names the key's path and the file
/// (STD-01 §R30).
#[test]
fn drop_legacy_without_a_key_says_so() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let config = c.root.join("config.yaml");
    let before = std::fs::read(&config).unwrap();
    let commits = log(&c.root);

    for args in [
        ["config", "observatory-root", "--drop-legacy"].as_slice(),
        &["--json", "config", "observatory-root", "--drop-legacy"],
    ] {
        let run = c.run(args).assert_ok();
        assert!(
            run.stderr().contains(&format!(
                "no legacy observatory_root key in {}; nothing removed",
                config.display()
            )),
            "{}",
            run.stderr()
        );
        assert_eq!(std::fs::read(&config).unwrap(), before, "{args:?}");
        assert_eq!(log(&c.root), commits, "{args:?}: no commit");
    }

    let foreign = with_legacy_observatory_root(&c);
    git(&c.root, &["commit", "-qam", "an older neb"]);
    let run = c
        .run(&["config", "observatory-root", "--drop-legacy"])
        .assert_ok();
    assert!(
        run.stderr().contains(&format!(
            "removed the legacy observatory_root ({foreign}) from {}",
            config.display()
        )),
        "{}",
        run.stderr()
    );
    assert_eq!(std::fs::read(&config).unwrap(), before);
    assert_eq!(log(&c.root)[0], "neb config observatory-root");
}

/// The legacy key still answers when nothing else does, and every verb that
/// resolves a record through it says so on stderr, in every mode, naming the
/// key and its replacement (STD-01 §R35). What it resolves, on stdout, is
/// what the machine setting pointing at the same checkout gives.
#[test]
fn legacy_observatory_root_warns_on_use() {
    let dir = tempfile::tempdir().unwrap();
    let obs = observatory(dir.path());
    let fixture = || {
        let c = Corpus::new();
        let id = c.seed("an idea", "An idea");
        let other = c.seed("another idea", "Another idea");
        (c, id, other)
    };
    let (legacy, id, other) = fixture();
    let config = legacy.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &format!("{raw}observatory_root: {}\n", obs.display()),
    );
    let (machine, _, _) = fixture();
    machine
        .run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();

    for json in [false, true] {
        let mode = |args: &[&str]| {
            let mut argv: Vec<String> = args.iter().map(ToString::to_string).collect();
            if json {
                argv.insert(0, "--json".into());
            }
            argv
        };
        let mut verbs = vec![mode(&["show", &id])];
        if !json {
            // `cite --json` never reads the root: its payload has no path.
            verbs.push(mode(&[
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                "Q002",
                "--note",
                "n",
            ]));
        }
        let record = if json { "R012" } else { "T003" };
        let target = if json { id.as_str() } else { other.as_str() };
        verbs.push(mode(&["handoff", target, record, "--note", "n"]));
        for args in verbs {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let warned = legacy.run(&args).assert_ok();
            let stderr = warned.stderr();
            let warning = stderr
                .lines()
                .find(|l| l.contains("observatory_root"))
                .unwrap_or_else(|| panic!("neb {}: no warning in:\n{stderr}", warned.args));
            assert!(
                warning.starts_with("warning: ")
                    && warning.contains(&config.display().to_string())
                    && warning.contains("neb config observatory-root <DIR>"),
                "{warning}"
            );
            let plain = machine.run(&args).assert_ok();
            assert!(
                !plain.stderr().contains("observatory_root"),
                "{}",
                plain.stderr()
            );
            assert_eq!(warned.stdout(), plain.stdout(), "neb {}", warned.args);
        }
    }
}

/// `migrate` rewrites `config.yaml` whole, so a legacy key the file carries
/// has to survive it — dropping it is `--drop-legacy`'s decision, not a
/// side effect — and a corpus already at v2 still changes nothing.
#[test]
fn migrate_keeps_a_legacy_observatory_root() {
    let c = Corpus::new();
    let foreign = with_legacy_observatory_root(&c);
    let before = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    c.run(&["migrate"]).assert_ok().says("nothing changed");

    assert_eq!(
        before,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says(&foreign);
}

/// A path or a slug stored as an observatory record would never resolve, and
/// the mistake is obvious now and cryptic in a year.
#[test]
fn an_observatory_uri_that_is_not_a_record_id_is_refused_at_cite() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    for uri in [
        "/Users/someone/observatory/questions/Q002-a-question.md",
        "Q002-is-proper-time-a-count",
        "X002",
        "questions/Q002",
    ] {
        c.run(&[
            "cite",
            &id,
            "--kind",
            "observatory",
            "--uri",
            uri,
            "--note",
            "n",
        ])
        .assert_fails()
        .says("is not an Observatory record id");
    }
    c.run(&["cite", &id, "--kind", "observatory", "--note", "n"])
        .assert_fails()
        .says("a reference of kind `observatory` needs a URI");
}

/// [`observatory`], plus the hypothesis record `H012` a hand-off goes to.
fn observatory_with_h012(at: &Path) -> (PathBuf, PathBuf) {
    let root = observatory(at);
    let record = root.join("hypotheses").join("H012-scarcity-wake.md");
    write(&record, "# H012\n");
    (root, record)
}

/// The one-step hand-off: one `observatory` reference and the node closed as
/// abandoned for it, in a single write that `check` passes. `show` and
/// `trace` say where the idea went.
#[test]
fn handoff_cites_the_record_and_closes_the_node_in_one_write() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    let parent = c.seed("gravity as scarcity", "Gravity as scarcity");
    let id = c
        .run(&["new", "Scarcity wake", "--parent", &parent])
        .assert_ok()
        .stdout_trim();

    c.run(&[
        "handoff",
        &id,
        "h012",
        "--note",
        "the hypothesis this became",
    ])
    .assert_ok()
    .says(&format!("{id} seed -> abandoned, handed off to H012 (r1)"))
    .says(record.to_str().unwrap());

    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(raw.matches("\n- id: r").count(), 1, "one reference:\n{raw}");
    assert!(raw.contains("kind: observatory"), "{raw}");
    assert!(raw.contains("uri: H012"), "{raw}");
    assert!(raw.contains("note: the hypothesis this became"), "{raw}");
    assert!(raw.contains("status: abandoned"), "{raw}");
    assert!(raw.contains("why: handed off to H012"), "{raw}");
    assert!(
        !raw.contains(obs.to_str().unwrap()),
        "no machine path may reach the corpus:\n{raw}"
    );
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");

    // `show` puts the record's location under the reason it closed.
    let shown = c.run(&["show", &id]).assert_ok().stdout();
    let location = format!(
        "closed: handed off to H012 {}\n        {}\n",
        date_days_ago(0),
        record.display()
    );
    assert!(shown.contains(&location), "{shown}");
    let v: serde_json::Value =
        serde_json::from_str(&c.run(&["--json", "show", &id]).assert_ok().stdout()).unwrap();
    assert_eq!(v["handed_off_to"], "H012");
    assert_eq!(v["observatory"][0]["path"], record.to_str().unwrap());

    // `trace` names the hand-off in its payload; the tree a terminal gets
    // shows it on the node's own line (`render::tree`'s unit tests).
    let walk: serde_json::Value =
        serde_json::from_str(&c.run(&["--json", "trace", &id]).assert_ok().stdout()).unwrap();
    assert_eq!(walk[0]["handed_off_to"], "H012");
    assert!(
        walk[1]["handed_off_to"].is_null(),
        "null for a node never handed off: {walk}"
    );
}

/// `--json` is the core `HandedOff`: the node as written, the new reference,
/// the record as stored, and the status it left.
#[test]
fn handoff_json_is_the_node_the_reference_the_record_and_the_old_status() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    let id = c
        .run(&[
            "new",
            "Scarcity wake",
            "--kill",
            "if the wake is frame-independent",
        ])
        .assert_ok()
        .stdout_trim();
    let out = c
        .run_with_env(
            &[
                "--json", "handoff", &id, "H012", "--note", "n", "--by", "agent:x",
            ],
            &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("handoff --json is JSON");
    assert_eq!(v["reference"], "r1");
    assert_eq!(v["record"], "H012");
    assert_eq!(v["from"], "hypothesis");
    assert_eq!(v["doc"]["node"]["status"], "abandoned");
    assert_eq!(v["doc"]["node"]["closed"]["why"], "handed off to H012");
    assert_eq!(v["doc"]["node"]["references"][0]["kind"], "observatory");
    assert_eq!(v["doc"]["node"]["references"][0]["uri"], "H012");
    assert_eq!(v["doc"]["node"]["references"][0]["by"], "agent:x");
    assert!(
        !out.contains("committed") && !out.contains("resolve"),
        "the payload is all stdout holds:\n{out}"
    );
}

/// `cite --json` carries where an `observatory` reference's record is, as
/// its prose does (STD-01 §R6): `path` is the resolved file under the root,
/// `null` with no root, and the whole `observatory` is `null` for any other
/// kind.
#[test]
fn cite_json_carries_observatory_resolution() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    let id = c.seed("an idea", "An idea");
    let cite = |root: Option<&Path>| {
        let mut env = Vec::new();
        if let Some(root) = root {
            env.push(("OBSERVATORY_ROOT", root.to_str().unwrap()));
        }
        let out = c
            .run_with_env(
                &[
                    "--json",
                    "cite",
                    &id,
                    "--kind",
                    "observatory",
                    "--uri",
                    "H012",
                    "--note",
                    "n",
                ],
                &env,
            )
            .assert_ok()
            .stdout();
        serde_json::from_str::<serde_json::Value>(&out).unwrap()
    };

    let v = cite(Some(&obs));
    assert_eq!(v["observatory"]["reference"], v["reference"]);
    assert_eq!(v["observatory"]["record"], "H012");
    assert_eq!(v["observatory"]["path"], record.to_str().unwrap());
    let v = cite(None);
    assert_eq!(v["observatory"]["record"], "H012");
    assert!(v["observatory"]["path"].is_null(), "{v}");

    let out = c
        .run(&[
            "--json",
            "cite",
            &id,
            "--kind",
            "paper",
            "--uri",
            "https://example.org",
            "--note",
            "n",
        ])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.as_object().unwrap().contains_key("observatory"), "{v}");
    assert!(v["observatory"].is_null(), "{v}");
}

/// `handoff --json` carries where the record is, as its prose does
/// (STD-01 §R6), and `path` is `null` with no root.
#[test]
fn handoff_json_carries_observatory_resolution() {
    let c = Corpus::new();
    let (obs, record) = observatory_with_h012(c.workdir());
    let located = c.seed("an idea", "An idea");
    let out = c
        .run_with_env(
            &["--json", "handoff", &located, "H012", "--note", "n"],
            &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["observatory"]["reference"], "r1");
    assert_eq!(v["observatory"]["record"], "H012");
    assert_eq!(v["observatory"]["path"], record.to_str().unwrap());

    let unlocated = c.seed("another idea", "Another idea");
    let out = c
        .run(&["--json", "handoff", &unlocated, "H012", "--note", "n"])
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["observatory"]["record"], "H012");
    assert!(
        v["observatory"].as_object().unwrap().contains_key("path")
            && v["observatory"]["path"].is_null(),
        "{v}"
    );
}

/// With no observatory root on this machine, the id is accepted on its
/// shape and the hand-off warns exactly as `cite --kind observatory` does.
#[test]
fn handoff_with_no_observatory_root_accepts_the_id_and_warns_as_cite_does() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let other = c.seed("another idea", "Another idea");
    let cited = c
        .run(&[
            "cite",
            &other,
            "--kind",
            "observatory",
            "--uri",
            "H012",
            "--note",
            "n",
        ])
        .assert_ok();
    assert_eq!(
        cited.stdout(),
        format!("{other} r1\n"),
        "the reference alone"
    );
    let warning = cited.stderr();
    assert!(warning.starts_with("No observatory root set"), "{warning}");
    assert_eq!(warning.lines().count(), 1, "{warning}");

    let out = c.run(&["handoff", &id, "H012", "--note", "n"]).assert_ok();
    assert!(
        !out.stdout().contains("No observatory root"),
        "{}",
        out.stdout()
    );
    assert_eq!(out.stderr(), warning, "the same one line on stderr");
    let raw = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(raw.contains("why: handed off to H012"), "{raw}");
}

/// Hints and advice are stderr's, so stdout holds only what a verb did or
/// read (STD-01 §R12): `init`'s `--set-root` hint, `cite`'s and `handoff`'s
/// notes about the record and a missing note, and `config`'s advice.
#[test]
#[allow(clippy::too_many_lines)] // One walk through every hint is easier to audit than five.
fn init_cite_handoff_and_config_hints_are_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let scratch = dir.path().join("scratch");
    let init = run_from_home(&dir.path().join("home"), Some(&scratch), &["init"], None).assert_ok();
    assert_eq!(
        init.stdout(),
        format!("corpus ready at {}\n", scratch.display())
    );
    assert!(init.stderr().contains("--set-root"), "{}", init.stderr());

    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let other = c.seed("another idea", "Another idea");

    let cited = c
        .run(&["cite", &id, "--kind", "observatory", "--uri", "H012"])
        .assert_ok();
    assert_eq!(cited.stdout(), format!("{id} r1\n"));
    let stderr = cited.stderr();
    let said: Vec<&str> = stderr.lines().collect();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[0].starts_with("No observatory root set"), "{said:?}");
    assert!(said[1].starts_with("No note."), "{said:?}");

    let handed = c.run(&["handoff", &other, "H012"]).assert_ok();
    assert!(
        handed
            .stdout()
            .starts_with(&format!("{other} seed -> abandoned")),
        "{}",
        handed.stdout()
    );
    assert_eq!(handed.stdout().lines().count(), 1, "{}", handed.stdout());
    assert_eq!(handed.stderr(), cited.stderr(), "the same two lines");

    let (obs, record) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap();
    let env = [("OBSERVATORY_ROOT", obs)];
    let resolved = c
        .run_with_env(
            &[
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                "H012",
                "--note",
                "n",
            ],
            &env,
        )
        .assert_ok();
    assert_eq!(
        resolved.stdout(),
        format!("{id} r2\n{}\n", record.display()),
        "the record's path is the result"
    );
    assert_eq!(resolved.stderr(), "");
    let unresolved = c
        .run_with_env(
            &[
                "cite",
                &id,
                "--kind",
                "observatory",
                "--uri",
                "H999",
                "--note",
                "n",
            ],
            &env,
        )
        .assert_ok();
    assert_eq!(unresolved.stdout(), format!("{id} r3\n"));
    assert!(
        unresolved
            .stderr()
            .starts_with("`H999` does not resolve under"),
        "{}",
        unresolved.stderr()
    );

    let commit = c.run(&["config", "commit"]).assert_ok();
    assert_eq!(commit.stdout(), "off\n");
    assert!(
        commit.stderr().contains("neb config commit on"),
        "{}",
        commit.stderr()
    );
    let commit = c.run(&["--json", "config", "commit"]).assert_ok();
    assert_eq!(commit.stderr(), "", "advice is for a person");

    let unset = c.run(&["config", "observatory-root"]).assert_ok();
    assert_eq!(unset.stdout(), "");
    assert!(
        unset.stderr().starts_with("no observatory root; set one"),
        "{}",
        unset.stderr()
    );
    let saved = c
        .run_with_env(&["config", "observatory-root", obs], &env)
        .assert_ok();
    assert!(saved.stdout().starts_with(obs), "{}", saved.stdout());
    assert_eq!(saved.stdout().lines().count(), 1, "{}", saved.stdout());
    assert!(
        saved.stderr().contains("outranks it while exported"),
        "{}",
        saved.stderr()
    );
}

/// Each refusal exits non-zero, writes nothing, and under `--json` is the
/// typed envelope.
#[test]
fn handoff_refuses_a_closed_node_an_unknown_one_and_an_unresolved_record() {
    let c = Corpus::new();
    let (obs, _) = observatory_with_h012(c.workdir());
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    let snapshot = |c: &Corpus| {
        let mut files: Vec<(String, String)> = std::fs::read_dir(c.root.join("nodes"))
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                let raw = std::fs::read_to_string(&path).unwrap();
                (path.display().to_string(), raw)
            })
            .collect();
        files.sort();
        files
    };

    let dead = c
        .run(&["new", "Dead idea", "--kill", "k"])
        .assert_ok()
        .stdout_trim();
    c.run(&["status", &dead, "refuted", "--why", "it fired"])
        .assert_ok();
    let dropped = c.seed("dropped idea", "Dropped idea");
    c.run(&["status", &dropped, "abandoned", "--why", "lost interest"])
        .assert_ok();
    let open = c.seed("open idea", "Open idea");
    let before = snapshot(&c);

    c.run(&["handoff", &dead, "H012"])
        .assert_fails()
        .says(&format!("`{dead}` is already refuted"))
        .says(&format!("neb new \"...\" --reopens {dead}"));
    let refused = c.run(&["--json", "handoff", &dead, "H012"]).refusal();
    assert_eq!(refused["code"], "already_closed");

    c.run(&["handoff", &dropped, "H012"])
        .assert_fails()
        .says(&format!("`{dropped}` is already abandoned"))
        .says(&format!("neb show {dropped}"));
    let refused = c.run(&["--json", "handoff", &dropped, "H012"]).refusal();
    assert_eq!(refused["code"], "already_closed");

    c.run(&["handoff", "nope", "H012"])
        .assert_fails()
        .says("no node `nope`");
    let refused = c.run(&["--json", "handoff", "nope", "H012"]).refusal();
    assert_eq!(refused["code"], "no_such_node");

    c.run(&["handoff", &open, "H999"])
        .assert_fails()
        .says("Observatory record `H999` does not resolve under")
        .says(obs.to_str().unwrap());
    let refused = c.run(&["--json", "handoff", &open, "H999"]).refusal();
    assert_eq!(refused["code"], "unresolved_observatory_record");
    assert!(refused["hint"].is_string(), "{refused}");

    let refused = c
        .run(&["--json", "handoff", &open, "hypotheses/H012.md"])
        .usage_refusal();
    assert_eq!(refused["code"], "invalid_observatory_id");

    assert_eq!(snapshot(&c), before, "no refusal wrote anything");
}

// ----------------------------------------------------------------- migrate --

/// A corpus in v1 form: declared domains, evidence with verdicts and
/// strengths, a task link, a weighed reference, the removed edge kinds, and
/// the statuses v2 collapses. Written by hand so the fixture is exactly what
/// v0.1 wrote and nothing in the current binary can shape it.
const V1_CONFIG: &str = r"# nebula corpus configuration. `neb domain` edits this.
schema_version: 1
corpus_id: neb-abc123
domains:
- general
- physics
default_domain: physics
";

const V1_WAKE: &str = r"---
id: wake-retardation
title: Retardation in the scarcity wake
domain: physics
status: supported
created: 2026-09-07
updated: 2026-09-10
kill: If the wake timescale is frame-independent, this is dead.
tags:
- Orrery
edges:
- type: derives-from
  to: gravity-as-scarcity
- type: depends-on
  to: gravity-as-scarcity
evidence:
- id: ev1
  verdict: supports
  strength: strong
  source: sim://boosted-source/run-3
  date: 2026-09-08
  note: |-
    Boosted source shows a lag.
    Frame dependence still to be checked.
  origin:
    task: DANI-10001
- id: ev2
  verdict: undermines
  strength: anecdote
  source: https://example.org/objection
  date: 2026-09-09
references:
- id: r1
  kind: study
  uri: https://example.org/time-dilation
  title: Gravitational time dilation
  note: The constraint any wake timescale has to survive.
  added: 2026-09-07
  promoted_to: ev1
tasks:
- id: DANI-10001
  state: open
  why: run the boosted-source sim at three velocities
---

The wake lags the source.
";

const V1_GRAVITY: &str = r"---
id: gravity-as-scarcity
title: Gravity as scarcity
domain: physics
status: testing
created: 2026-09-01
updated: 2026-09-05
kill: If a dense region shows no pull at all.
evidence:
- id: ev1
  verdict: inconclusive
  strength: suggestive
  source: doi:10.1000/scarcity
  date: 2026-09-04
  note: Looked, learned nothing.
---

Space might have a density of something.
";

const V1_OLD: &str = r"---
id: old-idea
title: Old idea
domain: general
status: graduated
created: 2026-08-01
updated: 2026-08-20
kill: If nobody downstream wants it.
graduated_to: principia://theory/old-idea
---

It went downstream.
";

const V1_DEAD: &str = r"---
id: dead-idea
title: Dead idea
domain: general
status: refuted
created: 2026-08-01
updated: 2026-08-15
kill: If the effect vanishes under control.
edges:
- type: undermines
  to: old-idea
evidence:
- id: ev1
  verdict: undermines
  strength: strong
  source: https://example.org/control-run
  date: 2026-08-15
  note: It vanished under control.
---

The effect vanished.
";

fn v1_corpus() -> Corpus {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    write(&root.join("config.yaml"), V1_CONFIG);
    for (id, text) in [
        ("wake-retardation", V1_WAKE),
        ("gravity-as-scarcity", V1_GRAVITY),
        ("old-idea", V1_OLD),
        ("dead-idea", V1_DEAD),
    ] {
        write(&root.join("nodes").join(format!("{id}.md")), text);
    }
    Corpus { dir, root }
}

#[test]
fn a_v1_corpus_refuses_to_open_until_migrated() {
    let c = v1_corpus();
    c.run(&["list"])
        .assert_fails()
        .says("schema_version 1")
        .says("Bring the corpus forward with:  neb migrate");
    c.run(&["check"]).assert_fails().says("neb migrate");
}

#[test]
fn migrate_relabels_evidence_tasks_and_edges_into_references() {
    let c = v1_corpus();
    let first = c
        .run(&["migrate"])
        .assert_ok()
        .says("wake-retardation")
        .says("domain `physics` -> tag `physics`")
        .says("evidence ev1 -> reference r2")
        .says("evidence ev2 -> reference r3")
        .says("task DANI-10001 -> reference r4")
        .says("edge depends-on gravity-as-scarcity -> reference r5")
        .says("status supported -> hypothesis")
        .says("status testing -> hypothesis")
        .says("status graduated -> abandoned (graduated to principia://theory/old-idea)")
        .says("config.yaml schema_version -> 2")
        .says("4 of 4 nodes rewritten")
        .stdout();
    assert!(!first.contains("nothing changed"), "{first}");

    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 2"), "{config}");
    assert!(config.contains("corpus_id: neb-abc123"), "{config}");
    assert!(
        !config.contains("domains") && !config.contains("default_domain"),
        "{config}"
    );

    let wake = std::fs::read_to_string(c.node_file("wake-retardation")).unwrap();
    for gone in [
        "domain:",
        "evidence:",
        "tasks:",
        "promoted_to",
        "type: depends-on",
        "verdict:",
        "strength:",
    ] {
        assert!(!wake.contains(gone), "`{gone}` survived migration:\n{wake}");
    }
    assert!(wake.contains("status: hypothesis"), "{wake}");
    assert!(wake.contains("tags:\n- orrery\n- physics\n"), "{wake}");
    assert!(
        wake.contains("- type: derives-from\n  to: gravity-as-scarcity\n"),
        "{wake}"
    );
    // The verdict and strength survive as a prefix on the note, the note's
    // first line becomes the title, and the evidence provenance is kept.
    assert!(
        wake.contains(
            "- id: r2\n  kind: other\n  uri: sim://boosted-source/run-3\n  \
             title: Boosted source shows a lag.\n  note: |-\n    \
             [supports/strong] Boosted source shows a lag.\n    \
             Frame dependence still to be checked.\n  added: 2026-09-08\n  \
             origin:\n    task: DANI-10001\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r3\n  kind: other\n  uri: https://example.org/objection\n  \
             note: '[undermines/anecdote]'\n  added: 2026-09-09\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r4\n  kind: other\n  uri: orbit:DANI-10001\n  title: DANI-10001\n  \
             note: '[open] run the boosted-source sim at three velocities'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains(
            "- id: r5\n  kind: other\n  uri: neb:gravity-as-scarcity\n  \
             title: gravity-as-scarcity\n  note: '[depends-on] gravity-as-scarcity'\n"
        ),
        "{wake}"
    );
    assert!(
        wake.contains("The wake lags the source."),
        "prose survives:\n{wake}"
    );
    // The migration never masquerades as an edit.
    assert!(wake.contains("updated: 2026-09-10"), "{wake}");
    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
}

#[test]
fn migrate_maps_statuses_and_is_a_no_op_the_second_time() {
    let c = v1_corpus();
    c.run(&["migrate"]).assert_ok();

    let gravity = std::fs::read_to_string(c.node_file("gravity-as-scarcity")).unwrap();
    assert!(gravity.contains("status: hypothesis"), "{gravity}");
    assert!(
        gravity.contains("[inconclusive/suggestive] Looked, learned nothing."),
        "{gravity}"
    );
    assert!(gravity.contains("tags:\n- physics\n"), "{gravity}");

    let old = std::fs::read_to_string(c.node_file("old-idea")).unwrap();
    assert!(old.contains("status: abandoned"), "{old}");
    assert!(
        old.contains(
            "closed:\n  why: graduated to principia://theory/old-idea\n  at: 2026-08-20\n"
        ),
        "{old}"
    );
    assert!(!old.contains("graduated_to"), "{old}");

    let dead = std::fs::read_to_string(c.node_file("dead-idea")).unwrap();
    assert!(dead.contains("status: refuted"), "{dead}");
    assert!(dead.contains("closed:\n  why: refuted under v1"), "{dead}");
    assert!(
        dead.contains("[undermines/strong] It vanished under control."),
        "{dead}"
    );
    assert!(dead.contains("note: '[undermines] old-idea'"), "{dead}");

    c.run(&["check"])
        .assert_ok()
        .says("4 nodes, 0 errors, 0 warnings");
    c.run(&["show", "wake-retardation"])
        .assert_ok()
        .says("[supports/strong]");
    let listed = c.run(&["list", "--tag", "physics"]).assert_ok().stdout();
    assert!(
        listed.contains("wake-retardation") && listed.contains("gravity-as-scarcity"),
        "{listed}"
    );

    // Second run: nothing to do, nothing touched.
    let before = snapshot_corpus_files(&c.root);
    let before_config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(
        before_config,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
}

/// A v2 corpus can hold a `kind: discussion` reference with no `uri`
/// (allowed since DANI-10593). `migrate` reads every node through the v1
/// model to detect what needs rewriting, and that model must tolerate a
/// missing `uri` too, or a legal v2 node makes migration fail outright.
#[test]
fn migrate_is_a_no_op_on_a_uri_less_discussion_reference() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--kind",
        "discussion",
        "--note",
        "a chat with the human",
    ])
    .assert_ok();

    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert!(!before.contains("uri:"), "{before}");
    let updated_before = before
        .lines()
        .find(|l| l.starts_with("updated:"))
        .unwrap()
        .to_string();

    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");

    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_eq!(before, after, "migrate must not touch a v2 node");
    let updated_after = after
        .lines()
        .find(|l| l.starts_with("updated:"))
        .unwrap()
        .to_string();
    assert_eq!(updated_before, updated_after, "updated must not be bumped");
}

#[test]
fn migrate_refuses_a_future_schema_without_changing_any_corpus_file() {
    let c = v1_corpus();
    let node = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&node).unwrap();
    write(
        &node,
        &raw.replacen(
            "status: supported",
            "status: supported\nfuture_field: valuable",
            1,
        ),
    );
    let config = c.root.join("config.yaml");
    let raw = std::fs::read_to_string(&config).unwrap();
    write(
        &config,
        &raw.replacen("schema_version: 1", "schema_version: 3", 1),
    );
    git(&c.root, &["init", "-q"]);
    git(
        &c.root,
        &[
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "add",
            "-A",
        ],
    );
    git(
        &c.root,
        &[
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "future corpus",
        ],
    );

    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(&config).unwrap();
    let run = c
        .run(&["migrate"])
        .assert_fails()
        .says("schema_version 3")
        .says("This corpus was written by a newer nebula. Upgrade this build.");
    let output = format!("{}{}", run.stdout(), run.stderr());
    assert!(
        !output.contains("Bring the corpus forward with:  neb migrate"),
        "future schema must not suggest migration:\n{output}"
    );

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(config_before, std::fs::read_to_string(&config).unwrap());
    assert!(
        std::fs::read_to_string(node)
            .unwrap()
            .contains("future_field: valuable")
    );
}

/// A v2 corpus whose nodes are committed, so `migrate` gets past the
/// dirty-tree refusal and the assertions are about what it does to content.
fn committed_v2_corpus() -> Corpus {
    let c = Corpus::new();
    git_init(&c.root);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "v2 corpus"]);
    c
}

/// Give `c`'s node `id` a tag in a case the current model accepts but
/// `migrate` does not render, so the v1 pass normalises it and writes the
/// file back. Returns the bytes that are now on disk.
fn with_an_unnormalised_tag(c: &Corpus, id: &str) -> String {
    let path = c.node_file(id);
    // The first `---\n\n` closes the frontmatter, so this appends to it.
    let raw =
        std::fs::read_to_string(&path)
            .unwrap()
            .replacen("---\n\n", "tags:\n- Orrery\n---\n\n", 1);
    write(&path, &raw);
    raw
}

/// The positive control for the test below: on its own, the node that test
/// expects `migrate` to leave alone really is one `migrate` would rewrite.
/// Without this, "the earlier node was not rewritten" would pass just as
/// happily against a fixture `migrate` never had any reason to touch.
#[test]
fn migrate_rewrites_an_unnormalised_tag_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let id = c.seed("an idea", "Alpha idea");
    let before = with_an_unnormalised_tag(&c, &id);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "an unnormalised tag"]);

    c.run(&["migrate"])
        .assert_ok()
        .says("tags normalised: orrery");
    let after = std::fs::read_to_string(c.node_file(&id)).unwrap();
    assert_ne!(before, after);
    assert!(after.contains("- orrery"), "{after}");
}

/// A corpus already at schema 2 has nothing left to convert, so a node the
/// current model cannot read is a hand edit this build does not understand,
/// not old content to re-label. Reading it through the lenient v1 model
/// would drop the field and report the node as successfully migrated, which
/// is data loss wearing a success message.
///
/// The good node here sorts first and is one `migrate` genuinely would
/// rewrite — its tag needs normalising — so this also pins that the refusal
/// comes before the rewrite loop rather than during it. Refusing per node
/// would leave `alpha-idea` normalised on disk and the run half applied.
#[test]
fn migrate_refuses_an_unknown_node_field_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let alpha = c.seed("an idea", "Alpha idea");
    let zeta = c.seed("another idea", "Zeta idea");
    assert!(alpha < zeta, "{alpha} must sort before {zeta}");

    // Valid v2, but not in the form `migrate` renders. What it does to this
    // node alone is pinned by the test above.
    with_an_unnormalised_tag(&c, &alpha);

    let path = c.node_file(&zeta);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen("status: seed", "status: seed\nfuture_field: valuable", 1),
    );
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "hand edits"]);

    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    c.run(&["migrate"])
        .assert_fails()
        .says("schema_version 2")
        .says("future_field")
        .says("nothing was changed");

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(
        config_before,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
    // Byte for byte: the unknown field survives, and the node that sorts
    // ahead of it was never normalised.
    assert!(
        std::fs::read_to_string(c.node_file(&zeta))
            .unwrap()
            .contains("future_field: valuable")
    );
    assert!(
        std::fs::read_to_string(c.node_file(&alpha))
            .unwrap()
            .contains("- Orrery")
    );
    assert!(git(&c.root, &["status", "--porcelain"]).trim().is_empty());
}

#[test]
fn check_and_migrate_refuse_unknown_nested_v2_data_without_rewriting() {
    for nested in [
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n",
        "references:\n- id: r1\n  kind: study\n  added: 2026-09-22\n  origin:\n    task: FIXTURE-1\n    future_field: IRREPLACEABLE\n",
        "edges:\n- type: derives-from\n  to: parent\n  future_field: IRREPLACEABLE\n",
    ] {
        let c = committed_v2_corpus();
        let id = c.seed("an idea", "An idea");
        let path = c.node_file(&id);
        let raw = std::fs::read_to_string(&path).unwrap();
        let edited = raw.replacen("---\n\n", &format!("{nested}---\n\n"), 1);
        assert_ne!(raw, edited);
        write(&path, &edited);
        git(&c.root, &["add", "-A"]);
        git(&c.root, &["commit", "-q", "-m", "unknown nested data"]);

        let before = snapshot_corpus_files(&c.root);
        c.run(&["check"]).assert_fails().says("future_field");
        c.run(&["migrate"])
            .assert_fails()
            .says("schema_version 2")
            .says("future_field")
            .says("nothing was changed");
        assert_eq!(before, snapshot_corpus_files(&c.root));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }
}

#[test]
fn migrate_preflights_late_nested_unknown_before_normalising_earlier_node() {
    let c = committed_v2_corpus();
    let alpha = c.seed("first", "Alpha idea");
    let zeta = c.seed("last", "Zeta idea");
    assert!(alpha < zeta);
    with_an_unnormalised_tag(&c, &alpha);

    let path = c.node_file(&zeta);
    let raw = std::fs::read_to_string(&path).unwrap();
    let edited = raw.replacen(
        "---\n\n",
        "origin:\n  task: FIXTURE-1\n  future_field: IRREPLACEABLE\n---\n\n",
        1,
    );
    assert_ne!(raw, edited);
    write(&path, &edited);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "late nested unknown"]);

    let before = snapshot_corpus_files(&c.root);
    c.run(&["check"]).assert_fails().says("future_field");
    c.run(&["migrate"])
        .assert_fails()
        .says("future_field")
        .says("nothing was changed");
    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
    assert!(
        std::fs::read_to_string(c.node_file(&alpha))
            .unwrap()
            .contains("- Orrery")
    );
}

/// Invariant 7 is enforced by `deny_unknown_fields` on the reference, not by
/// a check, so a `verdict` on a v2 reference has to fail to parse. The v1
/// reference model tolerates one because a v1 corpus really did carry
/// weighed references; applied to a v2 corpus that tolerance would delete
/// the key and call it a migration.
#[test]
fn migrate_refuses_a_reference_verdict_in_a_v2_corpus() {
    let c = committed_v2_corpus();
    let id = c.seed("an idea", "An idea");
    c.run(&[
        "cite",
        &id,
        "--uri",
        "https://example.invalid/p",
        "--note",
        "a paper",
    ])
    .assert_ok();
    let path = c.node_file(&id);
    let raw = std::fs::read_to_string(&path).unwrap();
    write(
        &path,
        &raw.replacen("  kind: ", "  verdict: supports\n  kind: ", 1),
    );
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "a weighed reference"]);

    let before = snapshot_corpus_files(&c.root);
    c.run(&["migrate"])
        .assert_fails()
        .says("schema_version 2")
        .says("verdict");
    assert_eq!(before, snapshot_corpus_files(&c.root));
}

/// The strict read is scoped to corpora that declare the current schema. A
/// v1 one keeps the leniency it was written for: its retired keys are
/// exactly what migration is there to drop, so an unrecognised key in a v1
/// file must still convert rather than refuse.
#[test]
fn migrate_still_drops_an_unknown_field_from_a_v1_corpus() {
    let c = v1_corpus();
    let node = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&node).unwrap();
    write(
        &node,
        &raw.replacen(
            "status: supported",
            "status: supported\nretired_v1_key: gone",
            1,
        ),
    );

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    assert!(
        !std::fs::read_to_string(&node)
            .unwrap()
            .contains("retired_v1_key")
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn migrate_keeps_valid_v1_origin_mapping_with_retired_nested_keys() {
    let c = v1_corpus();
    let path = c.node_file("wake-retardation");
    let raw = std::fs::read_to_string(&path).unwrap();
    let edited = raw
        .replacen(
            "    task: DANI-10001\n",
            "    task: DANI-10001\n    retired_v1_origin_key: gone\n",
            1,
        )
        .replacen(
            "---\n\n",
            "origin:\n  task: DANI-10002\n  retired_v1_origin_key: gone\n---\n\n",
            1,
        );
    assert_ne!(raw, edited);
    write(&path, &edited);

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    let migrated = std::fs::read_to_string(path).unwrap();
    assert!(migrated.contains("task: DANI-10001"), "{migrated}");
    assert!(migrated.contains("task: DANI-10002"), "{migrated}");
    assert!(!migrated.contains("retired_v1_origin_key"), "{migrated}");
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn migrate_refuses_a_malformed_config_before_changing_any_corpus_file() {
    let c = v1_corpus();
    let config = c.root.join("config.yaml");
    write(&config, "schema_version: 1\ncorpus_id: [not, a, string]\n");
    let before = snapshot_corpus_files(&c.root);
    let config_before = std::fs::read_to_string(&config).unwrap();

    c.run(&["migrate"])
        .assert_fails()
        .says("parsing")
        .says("config.yaml");

    assert_eq!(before, snapshot_corpus_files(&c.root));
    assert_eq!(config_before, std::fs::read_to_string(&config).unwrap());
}

/// Every entry under `root` but the write lock, with a file's bytes, a
/// directory as `dir` and a symlink as its target. What a refused migration
/// is compared on: the lock is the one thing a run may leave behind.
fn every_file_but_lock(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == root.join(".lock") {
                continue;
            }
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

/// A v1 node `migrate` can convert, sorting first.
const V1_FIRST: &str = "---\nid: a-first\ntitle: First\ndomain: Physics\nstatus: testing\nkill: If it never shows up.\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe first.\n";

/// A v1 node it cannot, sorting after it: `bogus` was never a v1 status.
const V1_BOGUS: &str = "---\nid: b-second\ntitle: Second\nstatus: bogus\ncreated: 2026-08-01\nupdated: 2026-08-02\n---\n\nThe second.\n";

/// A v1 corpus of `nodes` and, when given, `config`, outside git.
fn v1_corpus_of(config: Option<&str>, nodes: &[(&str, &str)]) -> Corpus {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(root.join("nodes")).unwrap();
    std::fs::create_dir_all(root.join("inbox")).unwrap();
    if let Some(config) = config {
        write(&root.join("config.yaml"), config);
    }
    for (id, text) in nodes {
        write(&root.join("nodes").join(format!("{id}.md")), text);
    }
    Corpus { dir, root }
}

/// Every node is converted in memory before the first write, so a node that
/// cannot be converted late in the corpus refuses the run with the earlier
/// ones untouched, and the run after the fix reports every node it changed.
#[test]
fn a_failing_v1_migration_rewrites_nothing() {
    let c = v1_corpus_of(
        Some(V1_CONFIG),
        &[("a-first", V1_FIRST), ("b-second", V1_BOGUS)],
    );
    let before = every_file_but_lock(&c.root);

    c.run(&["migrate"])
        .assert_fails()
        .says("b-second.md")
        .says("status `bogus` is not a v1 status");
    assert_eq!(before, every_file_but_lock(&c.root));

    write(
        &c.node_file("b-second"),
        &V1_BOGUS.replace("status: bogus", "status: seed"),
    );
    // `a-first` is converted by this run, notes and all; `b-second` is
    // already in v2 form now, so it is examined and left alone.
    c.run(&["migrate"])
        .assert_ok()
        .says("domain `Physics` -> tag `physics`")
        .says("status testing -> hypothesis")
        .says("1 of 2 nodes rewritten");
    c.run(&["check"]).assert_ok().says("2 nodes, 0 errors");
}

/// A corpus an older `neb` stamped at schema 2 over v1 nodes cannot be told
/// apart from a hand edit for certain, so it is still refused; the refusal
/// says which file looks like v1 and how to repair it without the tool.
#[test]
fn a_corpus_already_stamped_v2_over_v1_nodes_names_the_hand_repair() {
    let c = v1_corpus_of(
        Some("schema_version: 2\ncorpus_id: neb-abc123\n"),
        &[("a-first", V1_FIRST)],
    );
    let config = c.root.join("config.yaml");
    let before = every_file_but_lock(&c.root);

    for args in [&["migrate"][..], &["list"]] {
        let run = c
            .run(args)
            .assert_fails()
            .says("a-first.md")
            .says("`domain`")
            .says(config.to_str().unwrap());
        if args[0] == "migrate" {
            run.says("schema_version: 1").says("neb migrate");
        } else {
            run.says("neb check");
        }
        assert_eq!(before, every_file_but_lock(&c.root), "`neb {args:?}` wrote");
    }
    let refused = c.run(&["--json", "list"]).refusal();
    assert_eq!(refused["code"], "unreadable_nodes", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("v1_node_under_current_schema")
    );

    // The repair it names works.
    write(&config, "schema_version: 1\ncorpus_id: neb-abc123\n");
    c.run(&["migrate"])
        .assert_ok()
        .says("1 of 1 node rewritten");
    let migrated = std::fs::read_to_string(&config).unwrap();
    assert!(migrated.contains("corpus_id: neb-abc123"), "{migrated}");
    c.run(&["list"]).assert_ok().says("a-first");
}

/// A corpus from before `config.yaml` existed migrates, and the report says
/// the id it now has was minted rather than carried over. Running it again
/// changes nothing.
#[test]
fn migrate_reports_a_minted_corpus_id() {
    let c = v1_corpus_of(None, &[("a-first", V1_FIRST)]);
    let out = c.run(&["--json", "migrate"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["config_rewritten"], true, "{report}");
    let minted = report["minted_corpus_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no minted id: {report}"));
    assert!(minted.starts_with("neb-"), "{report}");
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains(&format!("corpus_id: {minted}")), "{config}");

    let before = every_file_but_lock(&c.root);
    let out = c.run(&["--json", "migrate"]).assert_ok().stdout();
    let again: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(again["config_rewritten"], false, "{again}");
    assert_eq!(
        again["minted_corpus_id"],
        serde_json::Value::Null,
        "{again}"
    );
    assert_eq!(again["rewritten"], serde_json::json!([]), "{again}");
    c.run(&["migrate"])
        .assert_ok()
        .says("already at schema 2; nothing changed");
    assert_eq!(before, every_file_but_lock(&c.root));

    let plain = v1_corpus_of(None, &[("a-first", V1_FIRST)]);
    plain
        .run(&["migrate"])
        .assert_ok()
        .says("minted corpus_id neb-");
}

#[test]
fn migrate_refuses_a_dirty_git_tree() {
    let c = v1_corpus();
    git(&c.root, &["init", "-q"]);
    c.run(&["migrate"])
        .assert_fails()
        .says("uncommitted changes");
    // Refused before anything was written.
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 1"), "{config}");

    git(&c.root, &["add", "-A"]);
    git(
        &c.root,
        &[
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "v1 corpus",
        ],
    );
    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

/// Whether the tree is clean is unknown when git cannot read the repository,
/// and unknown is not clean: `migrate` refuses before it rewrites a node.
#[test]
fn migrate_refuses_when_git_cannot_report_status() {
    let c = v1_corpus();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "v1 corpus");
    break_head(&c.root);
    let before = snapshot_corpus_files(&c.root);
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    let refused = c.run(&["--json", "migrate"]).refusal();
    assert_eq!(refused["code"], "git", "{refused}");
    assert_eq!(snapshot_corpus_files(&c.root), before);
    assert_eq!(
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap(),
        config
    );
}

// ------------------------------------------------------------------ commit --

/// Run git in `dir`, asserting it succeeded; stdout as text.
fn git(dir: &Path, args: &[&str]) -> String {
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
fn git_init(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "neb-test"]);
    git(dir, &["config", "user.email", "neb-test@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

/// Commit the current index at a fixed date, independent of machine config
/// and wall-clock time, so date-based history assertions are deterministic.
fn git_commit_at(dir: &Path, date: &str, message: &str) {
    git_commit_stamped(dir, &format!("{date}T12:00:00Z"), message);
}

/// Commit everything with `stamp`, a full ISO 8601 date and time with its
/// offset, as both author and committer date.
fn git_commit_stamped(dir: &Path, stamp: &str, message: &str) {
    git(dir, &["add", "-A"]);
    let out = output(
        git_command(dir, support::home())
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_DATE", stamp)
            .env("GIT_COMMITTER_DATE", stamp),
    );
    assert!(
        out.status.success(),
        "git commit failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `git status --porcelain`, ignoring the lock left by legacy-corpus fixtures
/// that deliberately exercise behavior without re-running `neb init`.
fn dirt(dir: &Path) -> String {
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
fn log(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--format=%s"])
        .lines()
        .map(String::from)
        .collect()
}

/// The paths the newest commit touched, relative to the repository's top
/// level, sorted.
fn head_paths(dir: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git(dir, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    paths.sort();
    paths
}

fn assert_only_corpus_paths_in_log(dir: &Path) {
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
fn corpus_repo() -> (Corpus, PathBuf) {
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

#[test]
fn log_and_show_at_read_a_node_sharpened_across_two_commits() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "History has a shape", "--id", "history-shape"])
        .assert_ok()
        .stdout_trim();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", &format!("neb new {id}"));
    let created = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    c.run(&["sharpen", &id, "--kill", "the past cannot be read"])
        .assert_ok();
    git_commit_at(&c.root, "2020-01-02", &format!("neb sharpen {id}"));
    let sharpened = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    let text = c.run(&["log", &id]).assert_ok().stdout();
    let mut lines = text.lines();
    let newest = lines.next().expect("sharpen history line");
    let oldest = lines.next().expect("creation history line");
    assert_eq!(
        newest,
        format!("{}\t2020-01-02\tneb sharpen history-shape", &sharpened[..7]),
        "{text}"
    );
    assert_eq!(
        oldest,
        format!("{}\t2020-01-01\tneb new history-shape", &created[..7]),
        "{text}"
    );
    assert!(lines.next().is_none(), "{text}");

    let json = c.run(&["--json", "log", &id]).assert_ok().stdout();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json).expect("log --json");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["hash"], sharpened);
    assert_eq!(entries[0]["date"], "2020-01-02");
    assert_eq!(entries[0]["message"], "neb sharpen history-shape");
    assert_eq!(entries[1]["hash"], created);

    for at in [&created, "2020-01-01"] {
        let shown = c
            .run(&["--json", "show", &id, "--at", at])
            .assert_ok()
            .stdout();
        let view: serde_json::Value = serde_json::from_str(&shown).expect("historical NodeView");
        assert_eq!(view["node"]["status"], "seed");
        assert_eq!(view["node"]["title_by"], "human");
        assert_eq!(view["body"], "");
        assert_eq!(view["observatory"], serde_json::json!([]));
    }
    for at in [&sharpened, "2020-01-02"] {
        let shown = c
            .run(&["--json", "show", &id, "--at", at])
            .assert_ok()
            .stdout();
        let view: serde_json::Value = serde_json::from_str(&shown).expect("historical NodeView");
        assert_eq!(view["node"]["status"], "hypothesis");
        assert_eq!(view["node"]["kill"], "the past cannot be read");
    }
    c.run(&["show", &id, "--at", "2019-12-31"])
        .assert_fails()
        .says("no node `history-shape` at `2019-12-31`");

    for at in ["not-a-date", "2026-99-99", "2026-13-01", "2027-02-29"] {
        c.run(&["show", &id, "--at", at])
            .assert_fails()
            .says(&format!(
                "`{at}` is not a YYYY-MM-DD date or a git revision"
            ))
            .says("--at");
    }

    // A real leap date is a date, not malformed.
    c.run(&["show", &id, "--at", "2028-02-29"]).assert_ok();
}

/// A commit that does not exist and a commit from before the node existed
/// are different answers: the first is a mistyped or foreign hash, the
/// second a true fact about the node's history.
#[test]
fn show_at_unknown_revision_is_not_no_node_at_revision() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let before = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();

    let unknown = c
        .run(&["--json", "show", &id, "--at", "deadbeef"])
        .refusal();
    assert_eq!(unknown["code"], "unknown_revision", "{unknown}");
    assert!(
        unknown["error"].as_str().unwrap().contains("deadbeef"),
        "{unknown}"
    );
    assert!(
        unknown["hint"].as_str().unwrap().contains("neb log"),
        "{unknown}"
    );

    let absent = c.run(&["--json", "show", &id, "--at", &before]).refusal();
    assert_eq!(absent["code"], "no_node_at_revision", "{absent}");
}

/// A date given to `show --at` names the day `log` prints, which is each
/// commit's own day in the offset it was recorded with, so the reader's
/// timezone never moves it. The commits sit where the end of a day in the
/// reader's zone used to cut them wrongly: at UTC+14 the end of 2020-01-01
/// falls before its noon-UTC commit, and at UTC-11 or UTC-12 it falls after
/// the early-morning commit dated 2020-01-02.
#[test]
fn show_at_a_date_is_timezone_independent() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Days are the log's", "--id", "days-are-the-logs"])
        .assert_ok()
        .stdout_trim();
    git_init(&c.root);
    git_commit_stamped(&c.root, "2020-01-01T12:00:00Z", &format!("neb new {id}"));
    let created = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    c.run(&["sharpen", &id, "--kill", "a zone moves the day"])
        .assert_ok();
    git_commit_stamped(
        &c.root,
        "2020-01-02T05:00:00Z",
        &format!("neb sharpen {id}"),
    );
    let sharpened = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();
    // A commit recorded in its author's own far-east offset is dated by it:
    // 2020-01-03 at UTC+14, which is still 2020-01-02 in UTC.
    c.run(&["note", &id, "written a day ahead"]).assert_ok();
    git_commit_stamped(
        &c.root,
        "2020-01-03T08:00:00+14:00",
        &format!("neb note {id}"),
    );
    let noted = git(&c.root, &["rev-parse", "HEAD"]).trim().to_string();

    let log: Vec<serde_json::Value> =
        serde_json::from_str(&c.run(&["--json", "log", &id]).assert_ok().stdout()).unwrap();
    let dates: Vec<_> = log.iter().map(|entry| entry["date"].clone()).collect();
    assert_eq!(dates, ["2020-01-03", "2020-01-02", "2020-01-01"], "{log:?}");

    // POSIX zones, so the offsets hold without tz data: UTC+14, UTC-11,
    // UTC-12 and UTC itself.
    let zones = ["XYZ-14", "XYZ+11", "XYZ+12", "UTC0"];
    for (date, revision) in [
        ("2020-01-01", &created),
        ("2020-01-02", &sharpened),
        ("2020-01-03", &noted),
    ] {
        let by_hash = c
            .run(&["--json", "show", &id, "--at", revision])
            .assert_ok()
            .stdout();
        for zone in zones {
            let by_date = c
                .run_with_env(&["--json", "show", &id, "--at", date], &[("TZ", zone)])
                .assert_ok()
                .stdout();
            assert_eq!(
                by_date, by_hash,
                "`--at {date}` under TZ={zone} is not the revision `log` dates {date}"
            );
        }
    }
    for zone in zones {
        c.run_with_env(&["show", &id, "--at", "2019-12-31"], &[("TZ", zone)])
            .assert_fails()
            .says("no node `days-are-the-logs` at `2019-12-31`");
    }
}

#[test]
fn log_reports_when_an_uncommitted_node_has_no_history() {
    let c = Corpus::new();
    git_init(&c.root);
    git_commit_at(&c.root, "2020-01-01", "initial repository");
    let id = c
        .run(&["new", "Never committed", "--id", "never-committed"])
        .assert_ok()
        .stdout_trim();

    c.run(&["log", &id])
        .assert_ok()
        .says("no commits touched this node");
    assert_eq!(c.run(&["--json", "log", &id]).assert_ok().stdout(), "[]\n");
}

/// A fixture whose temporary root sits inside someone's repository never
/// finds it: not to stage into it, and not to read history from it
/// (STD-04 §R7). Before the builder set `GIT_CEILING_DIRECTORIES`, running
/// the suite with `TMPDIR` in a work tree staged fixture files into it.
#[test]
fn fixtures_never_discover_an_enclosing_repository() {
    let outer = tempfile::tempdir().expect("tempdir");
    git_init(outer.path());
    let dir = tempfile::tempdir_in(outer.path()).expect("tempdir");
    let root = dir.path().join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("an idea inside someone else's repository", "Not theirs");
    c.run(&["log", &id])
        .assert_fails()
        .says("is not inside a git work tree");

    // With the fixture gone, anything it staged shows as added and deleted.
    drop(c);
    assert_eq!(git(outer.path(), &["status", "--porcelain"]), "");
    assert_eq!(git(outer.path(), &["ls-files"]), "");
}

#[test]
fn history_verbs_refuse_a_corpus_outside_a_git_work_tree() {
    let c = Corpus::new();
    let id = c.seed("an uncommitted past", "An uncommitted past");
    c.run(&["log", &id])
        .assert_fails()
        .says("is not inside a git work tree");
    c.run(&["show", &id, "--at", "2020-01-01"])
        .assert_fails()
        .says("is not inside a git work tree");
}

/// Make the repository at `repo` one git cannot read, as a bad disk or a
/// half-finished edit of `.git` does: git no longer recognises it at all.
fn break_head(repo: &Path) {
    write(&repo.join(".git").join("HEAD"), "garbage\n");
}

/// With `commit` on, a repository git cannot read is reported, never taken
/// for "no repository" and the commit silently skipped. The write stays.
#[test]
fn commit_on_with_broken_git_reports_instead_of_skipping() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    break_head(&c.root);

    let run = c.run(&["capture", "broken head probe"]).assert_fails();
    let run = run
        .says("git rev-parse failed in")
        .says(&c.root.display().to_string());
    assert!(
        !run.stdout_trim().is_empty(),
        "the verb reported its write before the refusal"
    );
    let inbox = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(inbox.contains("broken head probe"), "{inbox}");

    let run = c.run(&["--json", "capture", "second probe"]);
    let refused = run.refusal();
    assert_eq!(refused["code"], "git");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains(&c.root.display().to_string()),
        "{refused}"
    );
    serde_json::from_str::<serde_json::Value>(&run.stdout()).expect("the write's payload");
}

/// A history query in a repository git cannot read is a git failure, not a
/// corpus that is "not inside a git work tree", which would be false.
#[test]
fn history_on_a_broken_repository_is_a_git_error_not_not_a_work_tree() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();
    break_head(&c.root);
    let refused = c.run(&["--json", "log", &id]).refusal();
    assert_eq!(refused["code"], "git", "{refused}");
}

/// `commit` on with no repository anywhere above the corpus: the write lands
/// and the command succeeds, but it says once, on stderr, that nothing was
/// committed and why. The payload on stdout is what it always is.
#[test]
fn commit_on_outside_a_repository_says_it_did_not_commit() {
    let c = Corpus::new();
    c.run(&["config", "commit", "on"]).assert_ok();
    let run = c.run(&["capture", "x"]).assert_ok();
    let id = run.stdout_trim();
    assert_eq!(run.stdout(), format!("{id}\n"));
    assert_eq!(
        run.stderr(),
        format!(
            "note: not committed: {} is not inside a git work tree\n",
            c.root.display()
        )
    );
    // `--no-commit` asks for no commit, so there is nothing to say.
    let run = c.run(&["capture", "y", "--no-commit"]).assert_ok();
    assert_eq!(run.stderr(), "");
}

/// History needs git, and the refusal says how to start one for this corpus
/// and have later writes recorded in it.
#[test]
fn log_outside_git_hints_init() {
    let c = Corpus::new();
    let id = c.seed("an uncommitted past", "An uncommitted past");
    let run = c.run(&["log", &id]);
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.contains(&format!("git -C {} init", c.root.display())),
        "{stderr}"
    );
    assert!(stderr.contains("neb config commit on"), "{stderr}");
    let refused = c.run(&["--json", "log", &id]).refusal();
    assert_eq!(refused["code"], "not_git_work_tree");
    assert!(
        refused["hint"]
            .as_str()
            .is_some_and(|h| h.contains(&format!("git -C {} init", c.root.display()))),
        "{refused}"
    );
}

/// The setting is off by default and the verbs behave as they always did;
/// `neb config commit` reads and writes it, turning it on is itself the
/// first commit, and turning it off leaves that rewrite for you.
/// `--no-commit` is on the help of the verbs that write and nowhere else, as
/// the built binary renders it. Before a writing verb it still parses, as it
/// did when it was global, and warns (see below); after a read-only verb it
/// is a usage error.
#[test]
fn no_commit_is_offered_by_writing_verbs_only() {
    let c = Corpus::new();
    let offers = |args: &[&str]| {
        c.run(args)
            .assert_ok()
            .stdout()
            .lines()
            .any(|l| l.trim_start().starts_with("--no-commit"))
    };
    assert!(!offers(&["--help"]), "the top-level help");
    for verb in ["capture", "promote", "drop", "new", "note", "cite", "tag"] {
        assert!(offers(&[verb, "--help"]), "{verb} writes");
    }
    assert!(offers(&["config", "commit", "--help"]));
    for verb in ["show", "list", "trace", "review", "inbox", "near", "check"] {
        assert!(!offers(&[verb, "--help"]), "{verb} only reads");
    }

    let run = c.run(&["list", "--no-commit"]);
    assert_eq!(run.out.status.code(), Some(2), "a usage error");
    run.says("unexpected argument '--no-commit'");
    c.run(&["--no-commit", "capture", "--quiet", "still parses"])
        .assert_ok();
    c.run(&["capture", "--quiet", "and after", "--no-commit"])
        .assert_ok();
}

const PRE_VERB_WARNING: &str = "warning: `--no-commit` before the verb is deprecated";

/// The spelling from when `--no-commit` was global still skips the commit
/// before a verb that makes one, and says, once, in every mode, that it is
/// deprecated and how to write it now (STD-01 §R35).
#[test]
fn pre_verb_no_commit_warns_before_a_writing_verb() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("an idea", "An idea");
    let other = c.seed("another idea", "Another idea");
    let commits = log(&c.root).len();
    for (args, verb) in [
        (vec!["--no-commit", "capture", "x"], "capture"),
        (vec!["--no-commit", "--json", "capture", "y"], "capture"),
        (vec!["--no-commit", "handoff", &id, "H001"], "handoff"),
        (
            vec!["--no-commit", "--json", "handoff", &other, "H002"],
            "handoff",
        ),
    ] {
        let run = c.run(&args).assert_ok();
        let stderr = run.stderr();
        let warnings: Vec<_> = stderr
            .lines()
            .filter(|l| l.contains("deprecated"))
            .collect();
        assert_eq!(
            warnings,
            [format!(
                "{PRE_VERB_WARNING}; write `neb {verb} … --no-commit`"
            )],
            "neb {}",
            run.args
        );
        assert_eq!(log(&c.root).len(), commits, "neb {} committed", run.args);
    }
    assert!(!dirt(&c.root).is_empty(), "the writes landed, uncommitted");
    // Written after the verb, it is the verb's own flag and says nothing.
    let run = c.run(&["capture", "z", "--no-commit"]).assert_ok();
    assert!(!run.stderr().contains("deprecated"), "{}", run.stderr());
}

/// Before a verb that never commits there is nothing for it to skip, so it
/// is refused, before any work, rather than accepted and ignored (STD-01
/// §R28).
#[test]
fn pre_verb_no_commit_is_refused_before_a_verb_that_does_not_commit() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let fresh = c.workdir().join("fresh");
    let fresh_arg = fresh.to_str().unwrap();
    for (args, verb) in [
        (vec!["list"], "list"),
        (vec!["show", &id], "show"),
        (vec!["review"], "review"),
        (vec!["init", fresh_arg], "init"),
    ] {
        let mut argv = vec!["--no-commit"];
        argv.extend_from_slice(&args);
        let refused = assert_usage_error(&c, &argv, "usage");
        assert_eq!(
            refused["error"],
            format!(
                "`{verb}` never commits, so `--no-commit` before it has nothing to skip; drop it"
            ),
        );
    }
    assert!(!fresh.exists(), "the refused init created nothing");
    // `completions` refuses `--root` itself, so it goes without one.
    let run = run_from_home(
        c.workdir(),
        None,
        &["--no-commit", "completions", "bash"],
        None,
    );
    assert_eq!(run.out.status.code(), Some(2), "{}", run.stderr());
    assert_eq!(run.stdout(), "");
    assert!(
        run.stderr()
            .contains("`completions` never commits, so `--no-commit` before it"),
        "{}",
        run.stderr()
    );
}

#[test]
fn commit_is_off_by_default_and_the_setting_reads_and_writes() {
    let (c, _remote) = corpus_repo();

    c.run(&["config", "commit"]).assert_ok().says("off");
    let out = c.run(&["--json", "config", "commit"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["enabled"], false);
    c.run(&["capture", "before the setting"]).assert_ok();
    assert!(git(&c.root, &["log", "--oneline", "--all"]).is_empty());
    assert!(git(&c.root, &["status", "--porcelain"]).contains("?? "));
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(!raw.contains("commit"), "absent until set: {raw}");

    // Turning it on is itself the first commit, and sweeps up what was
    // already there under the corpus paths.
    c.run(&["config", "commit", "on"])
        .assert_ok()
        .says("on")
        .says("committed ");
    assert_eq!(log(&c.root), ["neb config commit"]);
    assert_eq!(head_paths(&c.root).len(), 3, "{:?}", head_paths(&c.root));
    assert!(head_paths(&c.root).contains(&".gitignore".to_string()));
    assert!(head_paths(&c.root).contains(&"config.yaml".to_string()));
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(raw.contains("commit: true"), "{raw}");
    let out = c.run(&["--json", "config", "commit"]).assert_ok().stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["enabled"], true);

    // Off again: the file loses the key, and that last rewrite is left for
    // you to commit by hand, because off means off.
    c.run(&["config", "commit", "off"]).assert_ok().says("off");
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(!raw.contains("commit"), "{raw}");
    assert!(git(&c.root, &["status", "--porcelain"]).contains(" M config.yaml"));
    git(&c.root, &["commit", "-qam", "off"]);
    c.run(&["capture", "with it off"]).assert_ok();
    assert_eq!(log(&c.root)[0], "off");
    assert!(git(&c.root, &["status", "--porcelain"]).contains(" M inbox/"));
}

/// A deleted `config.yaml` used to come back without `commit: true`, and
/// the write that brought it back went uncommitted. Now the write is refused
/// before it happens, and the file stays gone for whoever deleted it.
#[test]
fn deleted_config_never_turns_commit_off() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let config = c.root.join("config.yaml");
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("commit: true")
    );
    let commits = log(&c.root);
    std::fs::remove_file(&config).unwrap();
    let inbox = snapshot_corpus_files(&c.root);

    let refused = c.run(&["--json", "capture", "x"]).refusal();
    assert_eq!(refused["code"], "missing_config", "{refused}");
    assert!(!config.exists());
    assert_eq!(
        inbox,
        snapshot_corpus_files(&c.root),
        "an inbox line was written"
    );
    assert_eq!(commits, log(&c.root));
}

/// With `commit` on, every mutating verb lands as one commit named after it
/// and touching only corpus paths; `--no-commit` waives that once; the
/// remote is never touched.
#[test]
#[allow(clippy::too_many_lines)] // One step per mutating verb, so it grows with the verb set.
fn commit_on_records_each_mutating_verb_and_never_pushes() {
    let (c, remote) = corpus_repo();
    let early = c.run(&["capture", "before the setting"]).stdout_trim();
    c.run(&["config", "commit", "on"]).assert_ok();
    assert_eq!(log(&c.root), ["neb config commit"]);
    // The observatory root is this machine's, not the corpus's: setting it
    // leaves nothing to commit. Dropping the legacy key from `config.yaml`
    // is a corpus write, and lands as one.
    c.run(&["config", "observatory-root", "/tmp/observatory"])
        .assert_ok();
    assert_eq!(log(&c.root), ["neb config commit"]);
    with_legacy_observatory_root(&c);
    git(&c.root, &["commit", "-qam", "an older neb"]);
    c.run(&["config", "observatory-root", "--drop-legacy"])
        .assert_ok();
    assert_eq!(log(&c.root)[0], "neb config observatory-root");
    assert_eq!(head_paths(&c.root), ["config.yaml"]);

    // Every mutating verb, in lifecycle order, one commit each.
    let entry = c.run(&["capture", "a thought"]).assert_ok().stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {entry}"));
    assert!(head_paths(&c.root).iter().all(|p| p.starts_with("inbox/")));

    let id = c
        .run(&["promote", &entry, "--title", "A thought"])
        .assert_ok()
        .says("committed ")
        .stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb promote {entry} {id}"));
    let paths = head_paths(&c.root);
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.contains(&format!("nodes/{id}.md")));

    c.run(&["drop", &early]).assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb drop {early}"));

    let b = c
        .run(&["new", "B", "--parent", &id])
        .assert_ok()
        .stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb new {b}"));
    assert_eq!(head_paths(&c.root), [format!("nodes/{b}.md")]);

    c.run(&["sharpen", &id, "--kill", "if it fails"])
        .assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb sharpen {id}"));

    c.run(&["status", &id, "abandoned", "--why", "moved on"])
        .assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb status {id}"));

    c.run(&["link", &b, "contradicts", &id]).assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb link {b} {id}"));
    let mut both = [format!("nodes/{b}.md"), format!("nodes/{id}.md")];
    both.sort();
    assert_eq!(
        head_paths(&c.root),
        both,
        "contradicts is written on both ends, in one commit"
    );

    // `new --contradicts` writes the other end too, and names it.
    let rival = c
        .run(&["new", "Rival", "--contradicts", &b])
        .assert_ok()
        .stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb new {rival} {b}"));
    let mut both = [format!("nodes/{b}.md"), format!("nodes/{rival}.md")];
    both.sort();
    assert_eq!(head_paths(&c.root), both, "both ends, in one commit");

    c.run(&["tag", &b, "--add", "physics"]).assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb tag {b}"));

    c.run(&["note", &b, "some reasoning"]).assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb note {b}"));

    c.run(&[
        "cite",
        &b,
        "--uri",
        "https://example.com",
        "--note",
        "because",
    ])
    .assert_ok();
    assert_eq!(log(&c.root)[0], format!("neb cite {b} r1"));

    // `--json` keeps the payload clean: the commit happens, silently.
    let before = log(&c.root).len();
    let out = c
        .run(&["--json", "note", &b, "a second thought"])
        .assert_ok()
        .stdout();
    serde_json::from_str::<serde_json::Value>(&out).expect("note --json is still valid JSON");
    assert_eq!(log(&c.root).len(), before + 1);

    // `--no-commit` skips it once; the next verb sweeps the write up.
    c.run(&["--no-commit", "capture", "kept out of git for now"])
        .assert_ok();
    assert_eq!(log(&c.root).len(), before + 1);
    assert!(git(&c.root, &["status", "--porcelain"]).contains(" M inbox/"));
    let entry = c
        .run(&["capture", "and this one"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {entry}"));
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));

    // A read-only verb and a no-op write commit nothing.
    let n = log(&c.root).len();
    c.run(&["list"]).assert_ok();
    c.run(&["check"]).assert_ok();
    c.run(&["migrate"]).assert_ok().says("nothing changed");
    assert_eq!(log(&c.root).len(), n);

    // Argument-less config is also read-only, even when a corpus path is
    // already dirty.
    let node = c.node_file(&b);
    let contents = std::fs::read_to_string(&node).unwrap();
    write(&node, &(contents + "\nhuman edit\n"));
    assert!(git(&c.root, &["status", "--porcelain"]).contains(" M nodes/"));
    c.run(&["config", "observatory-root"]).assert_ok();
    c.run(&["config", "commit"]).assert_ok();
    assert_eq!(log(&c.root).len(), n);
    assert!(git(&c.root, &["status", "--porcelain"]).contains(" M nodes/"));

    // Nothing was ever pushed, and a commit never contains a stranger.
    assert!(
        git(&remote, &["rev-list", "--all"]).is_empty(),
        "the remote should be empty"
    );
    assert_only_corpus_paths_in_log(&c.root);
}

#[test]
fn a_staged_unicode_node_is_accepted_with_default_git_quoting() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "시간", "--no-commit"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "시간");
    git(&c.root, &["add", "nodes"]);

    c.run(&["note", &id, "new note"])
        .assert_ok()
        .says("committed ");

    assert_eq!(log(&c.root)[0], format!("neb note {id}"));
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

/// A hand-off is one write, so it is one commit: the node's file alone,
/// named for the node and the record it went to.
#[test]
fn handoff_is_one_commit_of_the_node() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();
    let (obs, _) = observatory_with_h012(c.workdir());
    let before = log(&c.root).len();
    c.run_with_env(
        &["handoff", &id, "H012", "--note", "because"],
        &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
    )
    .assert_ok()
    .says("committed ");
    assert_eq!(log(&c.root).len(), before + 1);
    assert_eq!(log(&c.root)[0], format!("neb handoff {id} H012"));
    assert_eq!(head_paths(&c.root), [format!("nodes/{id}.md")]);
}

/// `--quiet` is the id alone, so `E=$(neb capture -q …)` holds just the id:
/// the `committed` line is stderr's (STD-01 §R12). `promote -q` leaves the
/// path to `--json`.
#[test]
fn quiet_capture_and_promote_print_the_id_alone() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();

    let captured = c.run(&["capture", "-q", "x2 probe thought"]).assert_ok();
    let entry = captured.stdout();
    assert_eq!(entry.lines().count(), 1, "{entry:?}");
    let entry = entry.trim_end().to_string();
    assert_eq!(captured.stdout(), format!("{entry}\n"));
    assert!(
        captured.stderr().contains("committed "),
        "{}",
        captured.stderr()
    );

    let promoted = c.run(&["promote", &entry, "-q"]).assert_ok();
    assert_eq!(promoted.stdout(), "x2-probe-thought\n");
    assert!(
        promoted.stderr().contains("committed "),
        "{}",
        promoted.stderr()
    );
    assert!(c.node_file("x2-probe-thought").exists());
}

/// A write's `committed <hash>` goes to stderr in human mode, and nowhere
/// under `--json`, so stdout is the verb's result alone either way.
#[test]
fn commit_notice_is_on_stderr() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();

    let noted = c.run(&["note", &id, "x"]).assert_ok();
    assert_eq!(noted.stdout(), format!("{id}\n"));
    let head = git(&c.root, &["rev-parse", "HEAD"]);
    assert_eq!(noted.stderr(), format!("committed {}\n", &head[..7]));

    let json = c.run(&["--json", "note", &id, "x"]).assert_ok();
    assert!(!json.stderr().contains("committed"), "{}", json.stderr());
    assert!(!json.stdout().contains("committed"), "{}", json.stdout());
    assert_eq!(log(&c.root)[0], format!("neb note {id}"), "still committed");
}

// ----------------------------------------------------------- closed stdout --

/// A stdout closed under `neb`: exit 0, and on stderr exactly `stderr`,
/// what the verb says there with its stdout open. A reader that went away
/// adds no error, and takes no notice away: counts and the `committed` line
/// are stderr's, whoever reads stdout.
fn assert_closed_quietly(run: &Run, stderr: &str) {
    assert_eq!(
        run.out.status.code(),
        Some(0),
        "`neb {}` with a closed stdout:\n{}",
        run.args,
        run.stderr()
    );
    assert_eq!(run.stderr(), stderr, "`neb {}` said something", run.args);
}

/// `neb <read> | head -1` is normal use: a reader that stops early ends the
/// command silently with exit 0, never a panic (STD-01 §R13). The inbox is
/// made larger than any pipe buffer, so its writes cannot all land first.
#[test]
fn closed_stdout_exits_zero_silently_for_reads() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let root = c.seed("the root thought", "Root");
    let id = c
        .run(&["new", "A child", "--parent", &root, "--tag", "physics"])
        .assert_ok()
        .stdout_trim();
    let long = "a thought that goes on ".repeat(120);
    for n in 0..30 {
        c.run(&["capture", "-q", &format!("{n} {long}")])
            .assert_ok();
    }
    let inbox = c.run(&["inbox"]).assert_ok().stdout();
    assert!(inbox.len() > 64 * 1024, "{} bytes", inbox.len());

    for args in [
        vec!["list"],
        vec!["list", "--json"],
        vec!["inbox"],
        vec!["inbox", "--json"],
        vec!["show", &id],
        vec!["trace", &id],
        vec!["graph", "--json"],
        vec!["graph", "--mermaid"],
        vec!["review"],
        vec!["review", "--short"],
        vec!["tag", "list"],
        vec!["near", "child"],
        vec!["log", &id],
    ] {
        let open = c.run(&args).assert_ok().stderr();
        assert_closed_quietly(&c.run_closed_stdout(&args, &[], ""), &open);
    }
    // `completions` refuses `--root`, which every command above is given.
    let mut cmd = neb_command(c.workdir());
    cmd.args(["completions", "bash"]).env("NO_COLOR", "1");
    let run = Run {
        args: "completions bash".to_owned(),
        out: closed_stdout(&mut cmd, ""),
    };
    assert_closed_quietly(&run, "");
}

/// A triage whose screen has gone ends as `q` ends it, with exit 0 and no
/// refusal: nobody is there to see the next entry.
#[test]
fn closed_stdout_exits_zero_silently_for_triage() {
    let c = Corpus::new();
    let entry = c
        .run(&["capture", "-q", "a thought"])
        .assert_ok()
        .stdout_trim();
    assert_closed_quietly(&c.run_closed_stdout(&["triage"], &[], "d\nq\n"), "");
    assert!(
        c.run(&["inbox"]).assert_ok().stdout().contains(&entry),
        "a decision made with nobody watching was applied"
    );
}

/// The write a verb reports is committed even when nobody reads the report:
/// a closed stdout never stops a verb between its write and its commit.
#[test]
fn closed_stdout_still_commits_a_write() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.run(&["new", "An idea"]).assert_ok().stdout_trim();
    let (obs, _) = observatory_with_h012(c.workdir());
    let obs = obs.to_str().unwrap();

    for (args, env, subject) in [
        (
            vec!["--json", "note", &id, "more", "text"],
            vec![],
            format!("neb note {id}"),
        ),
        (vec!["note", &id, "text"], vec![], format!("neb note {id}")),
        (vec!["new", "X", "--json"], vec![], "neb new x".to_string()),
        (
            vec!["capture", "-q", "text"],
            vec![],
            "neb capture ".to_string(),
        ),
        (
            vec!["handoff", &id, "H012", "--note", "n"],
            vec![("OBSERVATORY_ROOT", obs)],
            format!("neb handoff {id} H012"),
        ),
    ] {
        let before = log(&c.root).len();
        let run = c.run_closed_stdout(&args, &env, "");
        let head = git(&c.root, &["rev-parse", "HEAD"]);
        let notice = if args.contains(&"--json") {
            String::new()
        } else {
            format!("committed {}\n", &head[..7])
        };
        assert_closed_quietly(&run, &notice);
        assert_eq!(log(&c.root).len(), before + 1, "`neb {}`", run.args);
        let last = git(&c.root, &["log", "-1", "--format=%s"]);
        assert!(
            last.starts_with(&subject),
            "`neb {}` committed as {last:?}",
            run.args
        );
        assert!(
            dirt(&c.root).is_empty(),
            "`neb {}`: {}",
            run.args,
            dirt(&c.root)
        );
    }
}

/// `git diff --cached --name-only` in `dir`: what is staged, relative to
/// the repository's top level, in git's order.
fn staged(dir: &Path) -> Vec<String> {
    git(dir, &["diff", "--cached", "--name-only"])
        .lines()
        .map(String::from)
        .collect()
}

/// A `neb` commit is exactly the corpus, by pathspec: work already staged
/// elsewhere in the repository neither blocks it nor rides in it, and is
/// still staged afterwards. Both shapes: a repository at the corpus root with
/// a stranger's file in it, and the corpus in a subdirectory of a larger one.
#[test]
fn unrelated_staged_work_does_not_block_or_join_a_neb_commit() {
    // The repository is the corpus root.
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    write(&c.root.join("unrelated.txt"), "theirs\n");
    git(&c.root, &["add", "unrelated.txt"]);
    let entry = c.run(&["capture", "x"]).assert_ok().stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {entry}"));
    let paths = head_paths(&c.root);
    assert!(
        !paths.is_empty() && paths.iter().all(|p| p.starts_with("inbox/")),
        "{paths:?}"
    );
    assert_eq!(staged(&c.root), ["unrelated.txt"]);

    // The corpus is a subdirectory of someone else's repository.
    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    git_init(&outer);
    write(&outer.join("README.md"), "theirs\n");
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    c.run(&["config", "commit", "on"]).assert_ok();
    assert_eq!(head_paths(&outer), ["corpus/config.yaml"]);

    write(&outer.join("README.md"), "theirs, edited\n");
    write(&outer.join("unrelated.txt"), "theirs\n");
    git(&outer, &["add", "README.md", "unrelated.txt"]);
    write(&outer.join("notes.txt"), "never staged\n");

    let entry = c
        .run(&["capture", "x"])
        .assert_ok()
        .says("committed ")
        .stdout_trim();
    assert_eq!(log(&outer)[0], format!("neb capture {entry}"));
    let paths = head_paths(&outer);
    assert!(
        !paths.is_empty() && paths.iter().all(|p| p.starts_with("corpus/inbox/")),
        "{paths:?}"
    );
    assert_eq!(staged(&outer), ["README.md", "unrelated.txt"]);
    assert!(git(&outer, &["status", "--porcelain"]).contains("?? notes.txt"));
    assert_eq!(
        std::fs::read_to_string(outer.join("README.md")).unwrap(),
        "theirs, edited\n"
    );
}

/// The absolute path of the `git` this suite would run.
#[cfg(unix)]
fn real_git() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH").expect("PATH is set"))
        .map(|dir| dir.join("git"))
        .find(|git| {
            std::fs::metadata(git)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .expect("git is on PATH")
}

/// Work someone stages while a `neb` commit is under way cannot ride in it:
/// the commit names the corpus paths, so it never reads the rest of the
/// index. A `git` first on `PATH` stages a stranger's file at the last
/// moment, just before running the real `git commit`.
#[cfg(unix)]
#[test]
fn work_staged_during_the_commit_is_not_swept_in() {
    use std::os::unix::fs::PermissionsExt;
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    write(&c.root.join("foreign.txt"), "theirs\n");
    let shims = c.workdir().join("shims");
    std::fs::create_dir(&shims).unwrap();
    let shim = shims.join("git");
    write(
        &shim,
        &format!(
            "#!/bin/sh\n\
             real='{real}'\n\
             if [ \"$1\" = -C ] && [ \"$3\" = commit ]; then\n\
             \"$real\" -C \"$2\" add foreign.txt || exit 99\n\
             fi\n\
             exec \"$real\" \"$@\"\n",
            real = real_git().display()
        ),
    );
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(shims).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();

    let entry = c
        .run_with_env(&["capture", "x"], &[("PATH", path.to_str().unwrap())])
        .assert_ok()
        .says("committed ")
        .stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {entry}"));
    let paths = head_paths(&c.root);
    assert!(
        !paths.is_empty() && paths.iter().all(|p| p.starts_with("inbox/")),
        "{paths:?}"
    );
    assert_eq!(
        staged(&c.root),
        ["foreign.txt"],
        "the shim ran, and theirs is still staged"
    );
}

/// `commit: on` in a corpus the containing repository ignores would commit
/// nothing forever; that is a typed refusal with the fix in the hint.
#[test]
fn commit_on_in_an_ignored_corpus_is_refused_with_the_fix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let c = Corpus { dir, root };
    c.run(&["init"]).assert_ok();
    git_init(&outer);
    write(&outer.join(".gitignore"), "corpus/\n");
    c.run(&["config", "commit", "on"])
        .assert_fails()
        .says("is ignored by the git repository that contains it")
        .says("git -C")
        .says("init");
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(
        raw.contains("commit: true"),
        "the setting was written: {raw}"
    );
    // A repository at the corpus root is the fix, and needs no other change.
    git_init(&c.root);
    c.run(&["capture", "now it works"])
        .assert_ok()
        .says("committed ");
    assert_eq!(log(&c.root).len(), 1);
}

// --------------------------------------------------------- supervised git --

/// A second repository beside the corpus, with one commit of its own: the
/// one an inherited `GIT_DIR` points at.
fn other_repo(c: &Corpus) -> PathBuf {
    let other = c.workdir().join("other");
    std::fs::create_dir(&other).unwrap();
    git_init(&other);
    write(&other.join("theirs.txt"), "theirs\n");
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "theirs"]);
    other
}

/// A `neb` commit run from inside another repository's hook, or any shell
/// that exported git's own variables, still lands in the corpus repository
/// and leaves the other one exactly as it was.
#[test]
fn a_commit_ignores_an_inherited_git_dir_and_index() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let other = other_repo(&c);
    let other_git = other.join(".git");
    let head = git(&other, &["rev-parse", "HEAD"]);
    let index = std::fs::read(other_git.join("index")).unwrap();
    let history = git(&other, &["log", "--stat"]);

    let run = c
        .run_with_env(
            &["capture", "probe gitdir"],
            &[
                ("GIT_DIR", other_git.to_str().unwrap()),
                ("GIT_WORK_TREE", other.to_str().unwrap()),
                ("GIT_INDEX_FILE", other_git.join("index").to_str().unwrap()),
            ],
        )
        .assert_ok()
        .says("committed ");
    let id = run.stdout_trim();

    assert_eq!(log(&c.root)[0], format!("neb capture {id}"));
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read(other_git.join("index")).unwrap(), index);
    assert_eq!(git(&other, &["log", "--stat"]), history);
    assert!(!other.join("inbox").exists() && !other.join("config.yaml").exists());
}

/// A history query answers from the corpus repository, not from whichever
/// repository an inherited `GIT_DIR` names, where the node has no history
/// and the answer would be a confident "none".
#[test]
fn history_ignores_an_inherited_git_dir() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c
        .run(&["new", "Git dir", "--id", "gd"])
        .assert_ok()
        .stdout_trim();
    let other = other_repo(&c);

    c.run_with_env(
        &["log", &id],
        &[("GIT_DIR", other.join(".git").to_str().unwrap())],
    )
    .assert_ok()
    .says(&format!("neb new {id}"));
}

/// Give the repository at `repo` a `pre-commit` hook that writes its process
/// group to `pgid_file`, atomically, then runs `body`. The hooks directory is
/// named in the repository's own config, so no global `core.hooksPath` can
/// replace it.
#[cfg(unix)]
fn pre_commit_hook(repo: &Path, pgid_file: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let hook = hooks.join("pre-commit");
    let recorded = pgid_file.display();
    write(
        &hook,
        &format!(
            "#!/bin/sh\n\
             ps -o pgid= -p $$ | tr -d ' ' > '{recorded}.tmp' && mv '{recorded}.tmp' '{recorded}'\n\
             {body}\n"
        ),
    );
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The process group a hook recorded.
#[cfg(unix)]
fn recorded_group(pgid_file: &Path) -> rustix::process::Pid {
    let raw = std::fs::read_to_string(pgid_file).expect("the hook ran and recorded its group");
    rustix::process::Pid::from_raw(raw.trim().parse().expect("a process group id"))
        .expect("a positive process group id")
}

/// Whether `group` is empty, by the group's own answer to signal 0. Killed
/// members are orphans that init reaps, so they may linger as zombies for a
/// moment; that moment is bounded here.
#[cfg(unix)]
fn group_gone(group: rustix::process::Pid) -> bool {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if matches!(
            rustix::process::test_kill_process_group(group),
            Err(rustix::io::Errno::SRCH)
        ) {
            return true;
        }
        if std::time::Instant::now() >= give_up {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Killing `neb` mid-commit used to orphan git, which then committed the
/// next writer's staged work under the dead verb's message. The signal now
/// stops git's whole group first, and `neb` still ends by that signal.
#[cfg(unix)]
#[test]
fn a_terminated_neb_takes_its_git_group_with_it() {
    use std::os::unix::process::ExitStatusExt;
    use std::time::{Duration, Instant};
    const HOOK_SLEEP: Duration = Duration::from_secs(5);

    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let pgid_file = c.workdir().join("hook.pgid");
    pre_commit_hook(&c.root, &pgid_file, "sleep 5");
    let before = log(&c.root);

    let mut capture = c.command(&["capture", "orphan probe"]);
    capture
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut neb = ChildGuard::spawn(&mut capture).unwrap_or_else(|e| panic!("{e}"));
    let waiting = Instant::now();
    while !pgid_file.exists() {
        assert!(neb.try_wait().is_none(), "neb finished before its hook ran");
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "the hook never ran"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let hook_started = Instant::now();
    let pid = rustix::process::Pid::from_raw(neb.id().try_into().unwrap()).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
    let status = neb
        .wait(Duration::from_secs(20))
        .unwrap_or_else(|e| panic!("neb did not end after SIGTERM: {e}"));
    assert_eq!(
        status.signal(),
        Some(rustix::process::Signal::TERM.as_raw()),
        "neb ends by the signal it was sent: {status:?}"
    );
    assert!(
        hook_started.elapsed() < HOOK_SLEEP,
        "neb waited for the hook instead of stopping it"
    );
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "git or its hook outlived neb"
    );

    // Past the moment the hook would have returned and an orphaned git
    // would have committed.
    std::thread::sleep(
        (HOOK_SLEEP + Duration::from_secs(2)).saturating_sub(hook_started.elapsed()),
    );
    assert_eq!(log(&c.root), before, "a commit landed after neb was gone");
    assert!(!c.root.join(".git").join("index.lock").exists());

    // The next writer's commit is its own, and sweeps up the stopped one's
    // write, which stayed on disk.
    std::fs::remove_file(c.root.join(".git").join("hooks").join("pre-commit")).unwrap();
    let after = c.run(&["capture", "after"]).assert_ok().stdout_trim();
    assert_eq!(log(&c.root)[0], format!("neb capture {after}"));
    let inbox = c.run(&["inbox"]).assert_ok().stdout();
    assert!(inbox.contains("orphan probe"), "{inbox}");
}

/// A commit that runs out of time is its own refusal, not a plain git
/// failure, and the write it was recording stays.
#[cfg(all(unix, debug_assertions))]
#[test]
fn git_timed_out_is_its_own_refusal_with_a_hook_hint() {
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let pgid_file = c.workdir().join("hook.pgid");
    pre_commit_hook(&c.root, &pgid_file, "sleep 30");
    let before = log(&c.root);
    // The debug-build seam: every git command gets two seconds.
    let short = [("NEBULA_TEST_GIT_DEADLINE_MS", "2000")];

    let run = c.run_with_env(&["--json", "capture", "held up"], &short);
    let envelope = run.refusal();
    assert_eq!(envelope["code"], "git_timed_out", "{envelope}");
    assert!(
        envelope["error"]
            .as_str()
            .unwrap()
            .starts_with("git commit did not finish within 2s"),
        "{envelope}"
    );
    assert!(
        group_gone(recorded_group(&pgid_file)),
        "the hook outlived the timeout"
    );
    assert_eq!(log(&c.root), before);
    let json = c.run(&["--json", "inbox"]).assert_ok().stdout();
    assert!(json.contains("\"held up\""), "the write stays: {json}");

    c.run_with_env(&["capture", "held up again"], &short)
        .assert_fails()
        .says("git commit did not finish within 2s")
        .says("A git hook is the likely cause")
        .says("hook run pre-commit")
        .says("--no-commit");
}

/// A migration is the write most worth its own commit. `migrate` reads the
/// setting through its lenient config model, keeps it, and commits itself.
#[test]
fn migrate_keeps_the_commit_setting_and_commits_itself() {
    let c = v1_corpus();
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    write(
        &c.root.join("config.yaml"),
        &format!("{config}commit: true\n"),
    );
    git_init(&c.root);
    git(&c.root, &["add", "-A"]);
    git(&c.root, &["commit", "-q", "-m", "v1 corpus"]);

    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten")
        .says("committed ");
    assert_eq!(log(&c.root), ["neb migrate", "v1 corpus"]);
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(raw.contains("commit: true"), "{raw}");
    c.run(&["config", "commit"]).assert_ok().says("on");
}

// --------------------------------------------------------- the write lock --

/// Two `neb` processes editing one node's tags at the same time. Each is a
/// load, an edit and a save, so without `<root>/.lock` the second reads the
/// node as it was before the first saved and the later write wins: one tag
/// lands and the other is gone without a word.
///
/// Separate processes deliberately. `flock` is held by the open file
/// description, so only a second *process* exercises it;
/// `crates/nebula-core/tests/core.rs` runs the same race across two threads,
/// which is what exercises the in-process half.
#[test]
fn two_concurrent_tag_writes_from_separate_processes_both_land() {
    let c = Corpus::new();
    let id = c.seed("a node two writers will tag", "Contended node");

    let first = c.spawn(&["tag", &id, "--add", "alpha"]);
    let second = c.spawn(&["tag", &id, "--add", "beta"]);
    first.wait().assert_ok();
    second.wait().assert_ok();

    let shown = c.run(&["--json", "show", &id]).assert_ok().stdout();
    let shown: serde_json::Value = serde_json::from_str(&shown).expect("show --json");
    let mut tags: Vec<&str> = shown["node"]["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| t.as_str().expect("a tag"))
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["alpha", "beta"], "one writer's tag was lost");
    c.run(&["check"]).assert_ok();
}

/// Past the bounded wait the writer refuses, naming who holds the lock and
/// saying what to do. The write never happens, so retrying is safe.
///
/// The lock is held here in the test process and contended by a spawned
/// `neb`, so this is a real cross-process `flock` and not the in-process
/// table standing in for one, and the holder `neb` names is read from the
/// record this process wrote.
#[test]
fn a_writer_that_waits_out_the_lock_refuses_with_a_hint_and_writes_nothing() {
    let c = Corpus::new();
    let id = c.seed("a node nobody else gets to edit", "Locked node");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    let held = nebula_core::CorpusLock::acquire_as(&c.root, nebula_core::LOCK_WAIT, "test holder")
        .expect("holding the lock");
    let named = format!("(`test holder`, pid {}, for ", std::process::id());

    // It waits the full five seconds before giving up, which is the bound
    // under test.
    let refused = c
        .run(&["tag", &id, "--add", "never"])
        .assert_fails()
        .says("another nebula writer is holding")
        .says(&named)
        .says("mid-write")
        .says(&format!("ps -p {}", std::process::id()));
    assert!(
        !refused.stderr().contains("run it again in a moment"),
        "{}",
        refused.stderr()
    );

    // The same refusal under `--json`: the holder is in `error`, and the
    // hint names it too.
    let envelope = c
        .run(&["--json", "tag", &id, "--add", "never"])
        .assert_fails()
        .refusal();
    assert_eq!(envelope["code"], "locked", "{envelope}");
    let error = envelope["error"].as_str().expect("error");
    assert!(error.contains(&named), "{error}");
    let hint = envelope["hint"].as_str().expect("hint");
    assert!(
        hint.contains(&format!("`test holder` (pid {})", std::process::id()))
            && !hint.contains("in a moment"),
        "{hint}"
    );

    assert_eq!(
        std::fs::read_to_string(c.node_file(&id)).unwrap(),
        before,
        "the refusal came before the write"
    );
    // A read never waits on a writer, whoever is holding it. The reading
    // forms of `neb config` are reads too: they answer from the config the
    // open snapshotted and take no lock at all.
    c.run(&["show", &id]).assert_ok();
    c.run(&["list"]).assert_ok();
    c.run(&["check"]).assert_ok();
    c.run(&["config", "commit"]).assert_ok().says("off");
    c.run(&["config", "observatory-root"]).assert_ok();

    // And once it is free, the same write goes through.
    drop(held);
    c.run(&["tag", &id, "--add", "never"]).assert_ok();
    assert!(
        std::fs::read_to_string(c.node_file(&id))
            .unwrap()
            .contains("never")
    );
}

/// The interleave this was found by. A config write rewrites `config.yaml`
/// whole, and the writer doing it opened — and so read the file — before the
/// writer ahead of it in the queue had finished its own config write. Without
/// a reload under the lock the waiter exits 0 and quietly takes the other
/// writer's setting back out.
///
/// A real `flock` held in this process and contended by a spawned `neb`, so
/// this is the cross-process shape; `crates/nebula-core/tests/core.rs`
/// sequences the same staleness by hand, which is what makes it deterministic.
#[test]
fn a_config_write_that_waited_out_the_lock_keeps_the_setting_written_meanwhile() {
    let (c, _remote) = corpus_repo();
    let foreign = with_legacy_observatory_root(&c);

    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    // The waiter opens — snapshotting a config with no `commit` key — and
    // then polls for the lock this thread is holding.
    let waiting = c.spawn(&["config", "observatory-root", "--drop-legacy"]);

    // Long enough for the spawned `neb` to be past its open and into the
    // wait, and far short of the five seconds it would wait in total. On a
    // machine slow enough that it has not opened yet, the snapshot it takes
    // is simply a fresh one and this passes without having raced — the core
    // test is the one that cannot miss.
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The writer ahead finishes its config write and lets go. Taking the
    // lock again underneath is the re-entry the CLI relies on too.
    let mut ahead = nebula_core::Corpus::open(Some(c.root.clone())).expect("open");
    assert!(
        nebula_core::ops::set_commit(&mut ahead, true)
            .unwrap()
            .enabled
    );
    drop(held);

    waiting.wait().assert_ok();

    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(
        raw.contains("commit: true"),
        "the waiter rewrote the config from its pre-lock snapshot: {raw}"
    );
    assert!(
        !raw.contains(&foreign),
        "and the waiter's own change must still have landed: {raw}"
    );
    c.run(&["config", "commit"]).assert_ok().says("on");
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says("no observatory root");
}

/// The lock file is the one thing under the root that is not corpus content,
/// so `neb commit` never stages it and `check` never reads it — nor the
/// holder record written into it, which is there for the whole of every
/// write and the commit that records it.
#[test]
fn the_lock_file_is_never_staged_and_never_checked() {
    let c = Corpus::new();
    git_init(&c.root);
    c.run(&["config", "commit", "on"]).assert_ok();
    // A record a crashed holder left behind: present from before the first
    // write, overwritten by each writer, and emptied as each lets go.
    let stale = r#"{"pid":1,"since":"2026-09-26T00:00:00Z","label":"a crashed neb"}"#;
    write(&c.root.join(".lock"), stale);
    let id = c.seed("a write that takes the lock", "Locked write");

    assert!(c.root.join(".lock").exists(), "the write took the lock");
    assert_eq!(
        std::fs::read_to_string(c.root.join(".lock")).unwrap(),
        "",
        "the writer replaced the stale record and cleared its own on release"
    );
    assert!(
        !git(&c.root, &["ls-files"]).contains(".lock"),
        "the lock file is not tracked"
    );
    assert!(
        !git(&c.root, &["status", "--porcelain"]).contains(".lock"),
        "the ignored lock is absent from repository status"
    );
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
    let paths = head_paths(&c.root);
    assert!(
        paths.contains(&format!("nodes/{id}.md")) && paths.iter().all(|p| p != ".lock"),
        "the commit is the promotion and nothing beside it: {paths:?}"
    );

    // Checked while a holder record sits in the file.
    write(&c.root.join(".lock"), stale);
    c.run(&["check"]).assert_ok().says("0 errors");
    let report = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["nodes"], 1, "the lock file is not read as a node");
    assert_eq!(report["findings"].as_array().unwrap().len(), 0);
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

/// A `neb` holding the lock names itself by its verb and target. Its
/// pre-commit hook stalls with the lock held, the way a slow commit does, and
/// a waiter in this process is told that `neb tag <id>`, at `neb`'s PID, is
/// the holder. The commit made under that record does not stage it.
#[cfg(unix)]
#[test]
fn a_neb_mid_write_is_named_by_its_verb_and_commits_without_its_record() {
    use std::time::{Duration, Instant};
    let (c, _remote) = corpus_repo();
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("a node a slow commit holds", "Slow commit");
    let pgid_file = c.workdir().join("hook.pgid");
    let release = c.workdir().join("release");
    // Bounded, so a test that fails before releasing it cannot wedge `neb`.
    pre_commit_hook(
        &c.root,
        &pgid_file,
        &format!(
            "i=0; while [ ! -e '{}' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done",
            release.display()
        ),
    );

    let tagging = c.spawn(&["tag", &id, "--add", "slow"]);
    let neb = tagging.child.id();
    let waiting = Instant::now();
    while !pgid_file.exists() {
        assert!(
            waiting.elapsed() < Duration::from_secs(30),
            "the hook never ran"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let refused =
        nebula_core::CorpusLock::acquire_within(&c.root, Duration::from_millis(50)).map(|_| ());
    write(&release, "");
    tagging.wait().assert_ok();

    let Err(nebula_core::Error::Locked {
        holder: Some(holder),
        ..
    }) = &refused
    else {
        panic!("expected a named holder, got {refused:?}");
    };
    assert_eq!(holder.label, format!("neb tag {id}"));
    assert_eq!(holder.pid, neb, "the holder is the spawned neb");
    assert_eq!(
        std::fs::read_to_string(c.root.join(".lock")).unwrap(),
        "",
        "neb cleared its record on release"
    );
    let paths = head_paths(&c.root);
    assert_eq!(paths, [format!("nodes/{id}.md")], "{paths:?}");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

/// Plant the temporary files a write killed before its rename leaves, one
/// in each directory a commit stages whole.
fn plant_debris(c: &Corpus) -> [PathBuf; 2] {
    let debris = [
        c.root.join("nodes/x.md.1f2e-0-18d8.tmp"),
        c.root.join("inbox/2026-09.md.1f2e-0-18d8.tmp"),
    ];
    for path in &debris {
        write(path, "half of a write that never landed\n");
    }
    debris
}

/// A killed write's temporary file sits in `nodes/` or `inbox/`, which a
/// commit stages whole. It stays on disk and out of the history, whether or
/// not the corpus `.gitignore` already carries the rule that hides it.
#[test]
fn a_stale_temp_file_is_never_committed() {
    let c = Corpus::new();
    git_init(&c.root);
    // As an older `init` left it: the lock rule and nothing for debris, so
    // the commit's own pathspec is what keeps the debris out.
    write(&c.root.join(".gitignore"), "/.lock\n");
    c.run(&["config", "commit", "on"]).assert_ok();
    let debris = plant_debris(&c);

    c.run(&["capture", "debris test"])
        .assert_ok()
        .says("committed ");

    let tracked = git(&c.root, &["ls-files"]);
    assert!(!tracked.contains(".tmp"), "{tracked}");
    assert!(
        tracked.contains("inbox/"),
        "the capture itself is committed: {tracked}"
    );
    for path in &debris {
        assert!(path.exists(), "{} was deleted", path.display());
    }

    // And with the rule `init` now writes, a person's `git add -A` skips it too.
    c.run(&["init"]).assert_ok();
    git(&c.root, &["add", "-A"]);
    let staged = git(&c.root, &["diff", "--cached", "--name-only"]);
    assert!(!staged.contains(".tmp"), "{staged}");
}

/// The pending-write record is a fact about a write in flight on one
/// machine, like the lock: ignored, never staged, and settled by the next
/// write whether or not that write commits.
#[test]
fn the_pending_record_is_never_staged() {
    let c = Corpus::new();
    git_init(&c.root);
    c.run(&["config", "commit", "on"]).assert_ok();
    let entry = c
        .run(&["capture", "an interrupted promotion idea"])
        .stdout_trim();
    let node = c.run(&["promote", &entry]).assert_ok().stdout_trim();
    // The state a crash between the node and the strike leaves.
    let month = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "md"))
        .unwrap();
    let text = std::fs::read_to_string(&month).unwrap();
    let struck = text
        .lines()
        .find(|l| l.contains(&entry))
        .unwrap()
        .to_string();
    let live = struck
        .replacen("- ~~[", "- [", 1)
        .split("~~ ->")
        .next()
        .unwrap()
        .to_string();
    write(&month, &text.replace(&struck, &live));
    let stamp = live.split(' ').nth(2).unwrap();
    write(
        &c.root.join(".pending"),
        &format!(r#"{{"op":"promote","entry":"{entry}","stamp":"{stamp}","node":"{node}"}}"#),
    );

    assert_eq!(git(&c.root, &["check-ignore", ".pending"]), ".pending\n");
    git(&c.root, &["add", "-A"]);
    assert!(
        !git(&c.root, &["diff", "--cached", "--name-only"]).contains(".pending"),
        "git add -A must not stage the pending record"
    );
    // `check` reads it and leaves it; `inbox` no longer offers the entry.
    c.run(&["check"]).assert_ok().says("did not finish");
    assert!(c.root.join(".pending").exists());
    assert!(!c.run(&["inbox"]).assert_ok().stdout().contains(&entry));

    c.run(&["capture", "the next thought"])
        .assert_ok()
        .says("committed ");

    assert!(
        !c.root.join(".pending").exists(),
        "the next write settled it"
    );
    assert!(
        std::fs::read_to_string(&month)
            .unwrap()
            .contains(&format!("~~ -> {node}")),
        "and finished the promotion"
    );
    assert!(!git(&c.root, &["ls-files"]).contains(".pending"));
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

#[test]
fn git_add_all_cannot_stage_the_lock_or_block_later_neb_commits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    std::fs::create_dir_all(&root).unwrap();
    git_init(&root);
    let c = Corpus { dir, root };

    c.run(&["init"]).assert_ok();
    assert!(c.root.join(".lock").exists());
    assert_eq!(git(&c.root, &["check-ignore", ".lock"]), ".lock\n");
    assert!(
        !git(&c.root, &["status", "--porcelain"]).contains(".lock"),
        "init must not leave the runtime lock visible to git"
    );

    c.run(&["config", "commit", "on"])
        .assert_ok()
        .says("committed ");
    git(&c.root, &["add", "-A"]);
    assert!(
        !git(&c.root, &["diff", "--cached", "--name-only"]).contains(".lock"),
        "git add -A must not stage the runtime lock"
    );

    c.run(&["new", "Still commits", "--id", "still-commits"])
        .assert_ok()
        .says("committed ");
    assert_eq!(log(&c.root)[0], "neb new still-commits");
    assert!(dirt(&c.root).is_empty(), "{}", dirt(&c.root));
}

// ------------------------------------------------------------- node ids --

/// Rewrite the id a node file stores, the way a hand edit would.
fn rewrite_stored_id(path: &Path, to: &str) {
    let raw = std::fs::read_to_string(path).expect("node file");
    let rewritten: String = raw
        .lines()
        .map(|line| {
            if line.starts_with("id: ") {
                format!("id: {to}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    write(path, &rewritten);
}

/// The reproduction this rule exists for: `nodes/safe.md` hand-edited to
/// `id: ../../escaped`, then an ordinary verb. It used to succeed and write
/// `escaped.md` beside the corpus root.
#[test]
fn a_hand_edited_traversal_id_refuses_and_writes_nothing_outside_the_root() {
    let c = Corpus::new();
    c.run(&["new", "Safe"]).assert_ok();
    rewrite_stored_id(&c.node_file("safe"), "../../escaped");

    // Reported against the file it came out of, since that is what the
    // human has to go and look at.
    c.run(&["note", "safe", "a fixture note"])
        .assert_fails()
        .says("nodes/safe.md stores the id `../../escaped`");
    assert!(
        !c.workdir().join("escaped.md").exists(),
        "a write landed beside the corpus root"
    );
    assert!(!c.root.join("escaped.md").exists());
    // Every verb that reads the corpus refuses the same way rather than one
    // of them quietly skipping the file.
    for args in [
        vec!["check"],
        vec!["list"],
        vec!["show", "safe"],
        vec!["tag", "safe", "--add", "fixture"],
    ] {
        c.run(&args)
            .assert_fails()
            .says("is not the node its file name names");
    }
}

/// The same boundary from the side no id check can see: the temporary file a
/// write goes through is a corpus path too, and its name was predictable.
/// Planted as a symlink, it carried an ordinary `note` straight out of the
/// corpus, truncating whatever it pointed at, and then landed as the node.
#[cfg(unix)]
#[test]
fn a_symlinked_temporary_neither_escapes_the_corpus_nor_becomes_the_node() {
    let c = Corpus::new();
    c.run(&["new", "Safe"]).assert_ok();
    let outside = c.workdir().join("outside-sentinel.txt");
    std::fs::write(&outside, "IRREPLACEABLE FIXTURE\n").unwrap();
    let mut planted = c.node_file("safe").into_os_string();
    planted.push(".tmp");
    let planted = PathBuf::from(planted);
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    c.run(&["note", "safe", "probe"]).assert_ok();

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "IRREPLACEABLE FIXTURE\n",
        "the note was written through the planted temporary, outside the corpus"
    );
    assert!(
        std::fs::symlink_metadata(c.node_file("safe"))
            .unwrap()
            .file_type()
            .is_file(),
        "the planted symlink was renamed onto the node"
    );
    // The note landed where it was addressed, and the planted path is left
    // where it was found: nothing here deletes what it did not create.
    c.run(&["show", "safe"]).assert_ok().says("probe");
    assert!(
        std::fs::symlink_metadata(&planted)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    c.run(&["check"]).assert_ok();
}

/// The quieter half: a valid id, but another node's. `save` writes where the
/// id says, so this overwrote the node it named.
#[test]
fn a_node_file_claiming_another_nodes_id_refuses_before_overwriting_it() {
    let c = Corpus::new();
    c.run(&["new", "Safe"]).assert_ok();
    c.run(&["new", "Victim"]).assert_ok();
    let victim = c.node_file("victim");
    let before = std::fs::read_to_string(&victim).unwrap();
    rewrite_stored_id(&c.node_file("safe"), "victim");

    c.run(&["note", "safe", "a fixture note"])
        .assert_fails()
        .says("is not the node its file name names");
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        before,
        "the other node is untouched"
    );
    c.run(&["check"])
        .assert_fails()
        .says("is not the node its file name names");
}

/// The same overwrite reached without touching a file's text: `cp
/// nodes/victim.md nodes/safe.md`. Agreement was once decided by comparing
/// the two files' contents, which a copy makes equal by construction, so
/// `note safe` reported `safe`, left `safe.md` alone and appended to
/// `victim.md` instead.
#[test]
fn a_copied_node_file_refuses_rather_than_redirecting_the_write() {
    let c = Corpus::new();
    c.run(&["new", "Victim"]).assert_ok();
    let victim = c.node_file("victim");
    let safe = c.node_file("safe");
    std::fs::copy(&victim, &safe).expect("copy");
    let before = std::fs::read_to_string(&victim).unwrap();

    c.run(&["note", "safe", "MISDIRECTED"])
        .assert_fails()
        .says("is not the node its file name names");
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        before,
        "the note landed on the node the copy named"
    );
    assert_eq!(
        std::fs::read_to_string(&safe).unwrap(),
        before,
        "the file that was asked for was written to"
    );
    // Reads fail the same way rather than answering from the copy.
    for args in [
        vec!["show", "safe"],
        vec!["list"],
        vec!["check"],
        vec!["tag", "safe", "--add", "fixture"],
    ] {
        c.run(&args)
            .assert_fails()
            .says("is not the node its file name names");
    }
}

/// The same file under two names: `ln nodes/victim.md nodes/safe.md`. The
/// two names once opened one inode, so `note safe` was accepted and printed
/// `safe`, but the atomic rename gave `victim.md` a new file and left
/// `safe.md` holding the old bytes, and the next `note`, `list` and `check`
/// all refused. An alias is refused before the write, from either name.
#[cfg(unix)]
#[test]
fn a_hard_linked_alias_refuses_before_a_write_can_split_it() {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &Path| {
        let metadata = std::fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let c = Corpus::new();
    c.run(&["new", "Victim", "--id", "victim"]).assert_ok();
    let victim = c.node_file("victim");
    let safe = c.node_file("safe");
    std::fs::hard_link(&victim, &safe).expect("hard link");
    let before = std::fs::read_to_string(&victim).unwrap();
    let linked = identity(&victim);
    assert_eq!(identity(&safe), linked, "the fixture is one file");

    for args in [
        vec!["note", "safe", "ALIAS-NOTE"],
        vec!["note", "victim", "OWN-NAME-NOTE"],
        vec!["tag", "safe", "--add", "fixture"],
        vec!["show", "safe"],
        vec!["show", "victim"],
        vec!["list"],
        vec!["check"],
    ] {
        c.run(&args)
            .assert_fails()
            .says("safe.md stores the id `victim`, which is not the node its file name names");
    }
    assert_eq!(identity(&victim), linked, "no write replaced victim.md");
    assert_eq!(
        identity(&safe),
        linked,
        "and the two names are still one file"
    );
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), before);

    // Removing the alias is the repair, and the node is whole again.
    std::fs::remove_file(&safe).unwrap();
    c.run(&["note", "victim", "AFTER-REPAIR"]).assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// `ln -s victim.md nodes/alias.md`. A write through the alias stayed linked,
/// but a scan read the node twice and refused a duplicate id, so the single
/// node door and the whole-corpus door disagreed. Both now refuse the alias
/// by name, before anything is written.
#[cfg(unix)]
#[test]
fn a_symlinked_alias_is_refused_at_every_door_that_meets_it() {
    let c = Corpus::new();
    c.run(&["new", "Victim", "--id", "victim"]).assert_ok();
    let victim = c.node_file("victim");
    let alias = c.node_file("alias");
    std::os::unix::fs::symlink("victim.md", &alias).expect("symlink");
    let before = std::fs::read_to_string(&victim).unwrap();

    for args in [
        vec!["note", "alias", "SYM-NOTE"],
        vec!["show", "alias"],
        vec!["list"],
        vec!["check"],
    ] {
        c.run(&args)
            .assert_fails()
            .says("alias.md stores the id `victim`, which is not the node its file name names");
    }
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        before,
        "nothing was written through the alias"
    );

    std::fs::remove_file(&alias).unwrap();
    c.run(&["note", "victim", "AFTER-REPAIR"]).assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// `nodes/outside.md` symlinked to a valid node outside the root: `neb note`
/// once replaced the link with a regular file and never touched the file
/// outside. The write is refused by name, the link stays a link and the file
/// outside is unchanged; the other doors refuse it the same way.
#[cfg(unix)]
#[test]
fn a_write_to_a_symlinked_node_is_refused_and_leaves_link_and_target_alone() {
    let c = Corpus::new();
    c.run(&["new", "Outside", "--id", "outside"]).assert_ok();
    let node = c.node_file("outside");
    let target = c.workdir().join("outside.md");
    std::fs::rename(&node, &target).unwrap();
    std::os::unix::fs::symlink(&target, &node).expect("symlink");
    let before = std::fs::read(&target).unwrap();

    let refused = c.run(&["--json", "note", "outside", "a fixture note"]);
    assert_eq!(refused.refusal()["code"], "not_regular_file");
    for args in [
        vec!["note", "outside", "a fixture note"],
        vec!["show", "outside"],
        vec!["list"],
        vec!["check"],
    ] {
        c.run(&args).assert_fails().says(&format!(
            "{} is a symlink, not a regular file",
            node.display()
        ));
    }
    assert!(
        std::fs::symlink_metadata(&node)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );
    assert_eq!(std::fs::read(&target).unwrap(), before);
}

/// A hard link from outside `nodes/` is not a second name the corpus sees —
/// a backup made with `cp -al` is the ordinary case — so the node still
/// reads, writes and checks.
#[cfg(unix)]
#[test]
fn a_hard_link_outside_the_nodes_directory_is_not_an_alias() {
    let c = Corpus::new();
    c.run(&["new", "Victim", "--id", "victim"]).assert_ok();
    let backup = c.workdir().join("backup.md");
    std::fs::hard_link(c.node_file("victim"), &backup).expect("hard link");

    c.run(&["note", "victim", "a fixture note"]).assert_ok();
    c.run(&["list"]).assert_ok().says("victim");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// A caller-supplied id is a path the moment a verb uses it, and a real file
/// outside the corpus is exactly what it used to reach.
#[test]
fn a_caller_supplied_traversal_or_absolute_id_is_refused() {
    let c = Corpus::new();
    c.run(&["new", "Safe"]).assert_ok();
    let secret = c.workdir().join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    let leak = secret.join("leak.md");
    write(
        &leak,
        "---\nid: leak\ntitle: fixture node outside the corpus\nstatus: seed\n\
         created: 2026-09-22\nupdated: 2026-09-22\n---\n\nfixture body, not corpus content\n",
    );
    let absolute = secret.join("leak").display().to_string();

    for id in ["../../secret/leak", "..", &absolute] {
        for args in [
            vec!["note", id, "a fixture note"],
            vec!["sharpen", id, "--kill", "a fixture kill"],
            vec!["tag", id, "--add", "fixture"],
            vec!["log", id],
        ] {
            c.run(&args).assert_fails().says("cannot be a node id");
        }
    }
    assert!(
        !c.node_file("leak").exists(),
        "a file outside the root was read into the corpus"
    );
    assert_eq!(
        std::fs::read_to_string(&leak).unwrap().lines().count(),
        9,
        "the file outside the root is untouched"
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// The rule is about path structure, not about the alphabet a thought was
/// named in: a Unicode id captures, notes, shows and checks as before.
#[test]
fn unicode_ids_still_work_end_to_end() {
    let c = Corpus::new();
    let id = c
        .run(&["new", "Ünïcode título → ok"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "ünïcode-título-ok");
    let korean = c
        .run(&["new", "시간은 프레임의 수다"])
        .assert_ok()
        .stdout_trim();

    c.run(&["note", &id, "a fixture note"]).assert_ok();
    c.run(&["note", &korean, "a fixture note"]).assert_ok();
    c.run(&["show", &id]).assert_ok().says("Ünïcode título");
    c.run(&["check"]).assert_ok().says("0 errors");
    assert!(c.node_file(&id).exists() && c.node_file(&korean).exists());
}
