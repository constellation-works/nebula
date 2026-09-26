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

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("neb")
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
        let mut cmd = Command::new(bin());
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1")
            .env("HOME", self.workdir())
            .env_remove("NEBULA_ROOT")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            // Removed rather than trusted: a developer with a real
            // Observatory checkout exported would otherwise resolve records
            // these tests expect to go missing.
            .env_remove("OBSERVATORY_ROOT");
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("running neb");
        Run {
            args: args.join(" "),
            out,
        }
    }

    fn run_with_stdin(&self, args: &[&str], input: &str) -> Run {
        let mut child = Command::new(bin())
            .arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1")
            .env("HOME", self.workdir())
            .env_remove("NEBULA_ROOT")
            .env_remove("OBSERVATORY_ROOT")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("running neb with stdin");
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input.as_bytes())
            .expect("writing stdin");
        let out = child.wait_with_output().expect("waiting for neb");
        Run {
            args: args.join(" "),
            out,
        }
    }

    /// Start the binary without waiting for it, so two of them can be in
    /// flight at once. The returned handle is finished with
    /// [`Spawned::wait`].
    fn spawn(&self, args: &[&str]) -> Spawned {
        let child = Command::new(bin())
            .arg("--root")
            .arg(&self.root)
            .args(args)
            .env("NO_COLOR", "1")
            .env("HOME", self.workdir())
            .env_remove("NEBULA_ROOT")
            .env_remove("OBSERVATORY_ROOT")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawning neb");
        Spawned {
            args: args.join(" "),
            child,
        }
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
/// fixture rather than a property of the developer's shell.
fn run_from_home(
    home: &Path,
    root: Option<&Path>,
    args: &[&str],
    nebula_root: Option<&Path>,
) -> Run {
    let mut cmd = Command::new(bin());
    if let Some(root) = root {
        cmd.arg("--root").arg(root);
    }
    cmd.args(args)
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env_remove("OBSERVATORY_ROOT");
    if let Some(nebula_root) = nebula_root {
        cmd.env("NEBULA_ROOT", nebula_root);
    } else {
        cmd.env_remove("NEBULA_ROOT");
    }
    let out = cmd.output().expect("running neb");
    Run {
        args: args.join(" "),
        out,
    }
}

/// A `neb` still running, so a test can have two of them racing.
struct Spawned {
    args: String,
    child: std::process::Child,
}

impl Spawned {
    fn wait(self) -> Run {
        let out = self.child.wait_with_output().expect("waiting for neb");
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

/// The inbox stamp format is the date plus a time-of-day, but only the date
/// half feeds the fourteen-day rule.
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
        let stamp_end = stamp_start + "2020-01-01T00:00".len();
        raw.replace_range(stamp_start..stamp_end, stamp);
        std::fs::write(&path, raw).unwrap();
        return;
    }
    panic!("no inbox entry contains `{capture_text}`");
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

#[test]
fn multiline_capture_is_refused_without_writing() {
    let c = Corpus::new();

    c.run(&["capture", "first line\nsecond line", "--quiet", "--json"])
        .assert_fails()
        .says("capture text must fit on one line");

    assert_eq!(
        std::fs::read_dir(c.root.join("inbox")).unwrap().count(),
        0,
        "a refused capture must not create an inbox record"
    );
    c.run(&["inbox", "--json"]).assert_ok().says("[]");
}

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

#[test]
fn capture_collision_fallback_promotes_the_new_thought() {
    const TEXT: &str = "the intended new thought";

    for _ in 0..3 {
        let c = Corpus::new();
        c.run(&["capture", TEXT]).assert_ok();
        let month = std::fs::read_dir(c.root.join("inbox"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let raw = std::fs::read_to_string(&month).unwrap();
        let stamp = raw.split_whitespace().nth(2).unwrap();
        let occupied = inbox_id_candidates(stamp, TEXT);
        let mut fixture = String::new();
        for (i, id) in occupied.iter().enumerate() {
            writeln!(fixture, "- [{id}] {stamp} fixture {i}").unwrap();
        }
        std::fs::write(&month, fixture).unwrap();

        let captured = c.run(&["capture", TEXT]).assert_ok().stdout_trim();
        let listing = c.run(&["inbox", "--json"]).assert_ok().stdout();
        let entries: serde_json::Value = serde_json::from_str(&listing).unwrap();
        let new_entry = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == captured)
            .unwrap();
        if new_entry["at"] != stamp {
            continue;
        }

        assert!(!occupied.contains(&captured));
        c.run(&["promote", &captured, "--title", "Intended new thought"])
            .assert_ok();
        let node = std::fs::read_to_string(c.node_file("intended-new-thought")).unwrap();
        assert!(node.ends_with("the intended new thought\n"));
        return;
    }
    panic!("the clock crossed a minute during all three collision fixtures");
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

#[test]
fn capture_works_before_a_corpus_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");
    let out = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .args(["capture", "the thought arrives before the setup"])
        .env_remove("NEBULA_ROOT")
        .output()
        .unwrap();
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
    let out = Command::new(bin())
        .current_dir(dir.path())
        .args(["--root", "typo", "capture", "x"])
        .env("HOME", dir.path().join("home"))
        .env("NO_COLOR", "1")
        .env_remove("NEBULA_ROOT")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
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

/// An exported-but-blank `NEBULA_ROOT` must be treated as unset, not as the
/// current directory: every later `join` would otherwise silently target
/// whatever directory the shell happened to be in.
#[test]
fn empty_nebula_root_env_falls_through_to_configured_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");

    run_from_home(&home, Some(&configured), &["init", "--set-root"], None).assert_ok();

    let out = Command::new(bin())
        .args(["check"])
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env("NEBULA_ROOT", "")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
    Run {
        args: "check".into(),
        out,
    }
    .assert_ok()
    .says("0 nodes");

    let default_home = dir.path().join("default-home");
    let default = default_home.join(".nebula");
    run_from_home(&default_home, Some(&default), &["init"], None).assert_ok();

    let out = Command::new(bin())
        .args(["check"])
        .env("HOME", &default_home)
        .env("NO_COLOR", "1")
        .env("NEBULA_ROOT", "")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
    Run {
        args: "check".into(),
        out,
    }
    .assert_ok()
    .says("0 nodes");
}

#[test]
fn empty_root_flag_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .args(["--root", "", "inbox"])
        .env("HOME", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("NEBULA_ROOT")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
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
    assert_eq!(err.to_string(), "--root cannot be empty");
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
        out.stdout().contains("--set-root")
            && out.stdout().contains(&scratch.display().to_string()),
        "plain init should name the opt-in command in:\n{}",
        out.stdout()
    );

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
        refused.stderr().contains("pass --force to replace it"),
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

#[test]
fn set_root_refuses_a_relative_path_before_mutating_anything() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let from = dir.path().join("from");
    std::fs::create_dir_all(&from).unwrap();

    let out = Command::new(bin())
        .args(["--root", "relative", "init", "--set-root"])
        .current_dir(&from)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env_remove("NEBULA_ROOT")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
    Run {
        args: "--root relative init --set-root".into(),
        out,
    }
    .assert_fails()
    .says("--set-root requires an absolute corpus path")
    .says("rerun with an absolute path");

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

    let init = Command::new(bin())
        .arg("--root")
        .arg(&root)
        .args(["init", "--set-root"])
        .current_dir(&from)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env_remove("NEBULA_ROOT")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
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

    let inbox = Command::new(bin())
        .arg("inbox")
        .current_dir(&elsewhere)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env_remove("NEBULA_ROOT")
        .env_remove("OBSERVATORY_ROOT")
        .output()
        .unwrap();
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
        b"private-notes/\r\n/.lock\n"
    );

    c.run(&["init"]).assert_ok();
    assert_eq!(
        std::fs::read(c.root.join(".gitignore")).unwrap(),
        b"private-notes/\r\n/.lock\n",
        "the setup repair is idempotent"
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
    let target_bytes = b"private-notes/\n/.lock\n";
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
        b"private-notes/\r\n/.lock\n"
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
    assert_eq!(std::fs::read(&ignore).unwrap(), b"/.lock\n");
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

#[test]
fn zsh_completions_include_the_cli_commands() {
    let c = Corpus::new();
    c.run(&["completions", "zsh"])
        .assert_ok()
        .says("#compdef neb")
        .says("capture");
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

fn ids(json: &str) -> Vec<String> {
    let out: serde_json::Value = serde_json::from_str(json).unwrap();
    out.as_array()
        .unwrap_or_else(|| panic!("a bare list: {json}"))
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

    // Text mode: one line per neighbour, score first, the best on top.
    let text = c
        .run(&["near", "tags and domains beat a taxonomy"])
        .assert_ok()
        .stdout();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("tags-beat-domains"), "{text}");
    assert!(lines[1].contains("a-single-global-taxonomy"), "{text}");
    assert!(lines[0].starts_with("0."), "score leads the line: {text}");

    // A limit caps the answer.
    let json = c
        .run(&["near", "--json", "--limit", "1", "tags taxonomy ranking"])
        .assert_ok()
        .stdout();
    assert_eq!(ids(&json).len(), 1, "{json}");
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
    assert_eq!(json.trim(), "[]");
    c.run(&["near", "   "])
        .assert_fails()
        .says("nothing to look near");
}

/// `--no-commit` after the query is a flag; `--quiet` is not a near flag, so
/// it is refused rather than folded into the text.
#[test]
fn near_trailing_no_commit_is_a_flag_and_quiet_is_refused() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let with_flag = c
        .run(&["near", "--json", "one global taxonomy", "--no-commit"])
        .assert_ok()
        .stdout();
    let without = c
        .run(&["near", "--json", "one global taxonomy"])
        .assert_ok()
        .stdout();
    assert_eq!(
        with_flag, without,
        "trailing --no-commit must not join the query"
    );
    c.run(&["near", "one global taxonomy", "--quiet"])
        .assert_fails()
        .says("--quiet");
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
    // after it, the way a refused commit does. Nothing about the capture
    // depended on `nodes/` parsing, and it must not read as a lost thought.
    let run = c.run(&["capture", "another idea"]);
    let out = run.stdout();
    assert_eq!(out.lines().count(), 1, "the id, alone: {out}");
    assert_eq!(out.trim().len(), 4, "{out}");
    run.assert_fails().says("unknown field `verdict`");
    c.run(&["inbox"]).assert_ok().says("another idea");

    // --quiet never reads `nodes/`, so it does not even see the problem.
    c.run(&["capture", "-q", "quietly"]).assert_ok();
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

#[test]
fn every_corpus_reader_names_a_non_regular_node_and_preserves_the_os_cause() {
    let c = Corpus::new();
    let healthy = c.seed("a healthy node", "Healthy");
    let broken = c.node_file("broken");
    std::fs::create_dir(&broken).unwrap();
    let path = broken.display().to_string();
    let cause = std::fs::read_to_string(&broken).unwrap_err().to_string();

    for args in [
        vec!["check"],
        vec!["list"],
        vec!["show", healthy.as_str()],
        vec!["graph", "--mermaid"],
        vec!["trace", healthy.as_str()],
        vec!["review"],
        vec!["open"],
        vec!["near", "thing"],
        vec!["tag", "list"],
    ] {
        c.run(&args)
            .assert_fails()
            .says("reading")
            .says(&path)
            .says(&cause);
    }
}

#[test]
fn opening_a_non_regular_config_names_it_and_preserves_the_os_cause() {
    let c = Corpus::new();
    let config = c.root.join("config.yaml");
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    let path = config.display().to_string();
    let cause = std::fs::read_to_string(&config).unwrap_err().to_string();

    c.run(&["list"])
        .assert_fails()
        .says("reading")
        .says(&path)
        .says(&cause);
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
    assert!(out.get("near").is_none(), "quiet omits the list: {json}");
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
    assert!(out["doc"]["node"].get("edges").is_none(), "no edge: {json}");

    // With a parent, `near` is omitted rather than empty.
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
    assert!(out.get("near").is_none(), "{json}");
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

    // There is no default text form: without a format it explains and exits 2.
    let run = c.run(&["graph"]);
    assert_eq!(run.out.status.code(), Some(2));
    assert!(
        run.stdout().contains("neb graph --json") && run.stdout().contains("neb graph --mermaid"),
        "{}",
        run.stdout()
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

    let tree = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        tree.matches(&root_id).count(),
        2,
        "the shared ancestor appears on both branches"
    );
    assert!(
        tree.contains("shown above"),
        "but it is only expanded once:\n{tree}"
    );
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
        .says("--why only applies");
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
    c.run(&["status", &id, "hypothesis"])
        .assert_fails()
        .says("cannot simply reopen");
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
    c.run(&["new", "Second attempt", "--kill", "if Y"])
        .assert_ok();
    c.run(&["show", "second-attempt"])
        .assert_ok()
        .says("hypothesis");
    c.run(&["link", "second-attempt", "reopens", &id])
        .assert_ok();
    c.run(&["check"]).assert_ok().says("0 errors");
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

    let before = raw;
    for kind in ["bogus", "VERDICT", "", "not a kind"] {
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
        .says("0 errors, 1 warnings");
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
        .says("0 errors, 1 warnings");

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
        .says("0 errors, 1 warnings");

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
        .says("--uri is required unless --kind is discussion");
    for kind in ["Discussion", "discussions"] {
        c.run(&["cite", &id, "--kind", kind, "--note", "missing URI"])
            .assert_fails()
            .says("--uri is required unless --kind is discussion");
    }
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
    .says("--uri is required unless --kind is discussion");

    c.run(&["cite", &id, "--kind", "discussion"]).assert_ok();
    c.run(&["check"])
        .assert_ok()
        .says("reference `r2` has no note saying why it is here")
        .says("0 errors, 1 warnings");

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
        .says("1 errors, 0 warnings");
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
        .says("1 errors");

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
            .says(&format!("neb cite {id} --kind observatory --uri Q002"));
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
        .says("1 errors");
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
        .says("0 errors, 1 warnings");
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
        .says("1 errors");
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
            .says("1 errors");
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
        .says("1 errors");
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
        .says("1 errors");
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
        .says("1 errors");
}

// ------------------------------------------------------------------ triage --

#[test]
fn open_finds_the_hypothesis_with_no_references() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));
    c.run(&["open"])
        .assert_ok()
        .says(&id)
        .says("hypothesis with no references");

    c.run(&["cite", &id, "--uri", "https://example.org", "--note", "n"])
        .assert_ok();
    let after = c.run(&["open"]).assert_ok().stdout();
    assert!(
        !after.contains("no references"),
        "a reference closes the gap:\n{after}"
    );
}

#[test]
fn open_json_items_have_exactly_id_and_why_keys() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    c.run(&["sharpen", &id, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&id), &date_days_ago(14));

    let json = c.run(&["open", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        !items.is_empty(),
        "expected at least one open item:\n{json}"
    );
    for item in &items {
        let keys: std::collections::BTreeSet<&str> = item
            .as_object()
            .expect("open --json item is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from(["id", "why"]),
            "open --json item key set drifted from {{id, why}}: {item}"
        );
    }
}

#[test]
fn open_and_review_apply_the_no_references_grace_at_fourteen_days() {
    let c = Corpus::new();
    let grace = c.seed("still in grace", "Still in grace");
    c.run(&["sharpen", &grace, "--kill", "if X"]).assert_ok();
    set_created(&c.node_file(&grace), &date_days_ago(13));

    let due = c.seed("now due", "Now due");
    c.run(&["sharpen", &due, "--kill", "if Y"]).assert_ok();
    c.run(&["note", &due, "reasoning is not a reference"])
        .assert_ok();
    set_created(&c.node_file(&due), &date_days_ago(14));

    for verb in ["open", "review"] {
        let out = c.run(&[verb]).assert_ok().stdout();
        assert!(
            !out.contains(&grace),
            "{verb} raised a 13-day-old node:\n{out}"
        );
        assert!(
            out.contains(&due),
            "{verb} missed a 14-day-old node:\n{out}"
        );
    }
}

#[test]
fn open_finds_seeds_untouched_for_ninety_days_and_filters_by_tag() {
    let c = Corpus::new();
    let old = c.seed("an old seed", "An old seed");
    c.run(&["tag", &old, "--add", "physics"]).assert_ok();
    set_updated(&c.node_file(&old), &date_days_ago(90));
    let fresh = c.seed("a fresh seed", "A fresh seed");
    set_updated(&c.node_file(&fresh), &date_days_ago(89));

    let out = c.run(&["open"]).assert_ok().stdout();
    assert!(
        out.contains(&old) && out.contains("seed untouched for ninety days"),
        "{out}"
    );
    assert!(!out.contains(&fresh), "{out}");

    let out = c.run(&["open", "--tag", "physics"]).assert_ok().stdout();
    assert!(out.contains(&old), "{out}");
    let out = c.run(&["open", "--tag", "orrery"]).assert_ok().stdout();
    assert!(!out.contains(&old), "{out}");
}

#[test]
fn open_finds_inbox_captures_waiting_over_fourteen_days() {
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

    c.run(&["open"])
        .assert_ok()
        .says("1 captures waiting over fourteen days; promote or drop them");

    let json = c.run(&["open", "--json"]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items.iter().any(|item| item["id"] == "inbox"),
        "inbox finding missing: {json}"
    );
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
    let confirmed = json(&["sharpen", "--json", "idea-to-sharpen", "--confirm"]);
    assert_eq!(confirmed["node"]["kill"], "the evidence changes");
    assert!(confirmed["node"].get("kill_by").is_none());

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
    json(&["list", "--json"]);
    json(&["near", "--json", "direct design"]);
    json(&["trace", "--json", "child-idea"]);
    json(&["impact", "--json", "parent-idea"]);
    json(&["graph", "--json"]);
    json(&["open", "--json"]);
    json(&["review", "--json"]);

    let checked = json(&["check", "--json"]);
    assert!(checked["nodes"].as_u64().unwrap() >= 6);
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
        .says("machine-learning 1")
        .says("physics 2");
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
        .says("tags `Physics` and `physics` differ only by case")
        .says("tags `sim` and `sims` differ only by a trailing `s`")
        .says("0 errors, 2 warnings");
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
fn open_and_review_report_only_aged_hypotheses_with_no_references() {
    let c = Corpus::new();
    let seed = c.seed("an old seed", "An old seed");
    set_created(&c.node_file(&seed), &date_days_ago(30));

    let hypothesis = c.seed("an old hypothesis", "An old hypothesis");
    c.run(&["sharpen", &hypothesis, "--kill", "if X"])
        .assert_ok();
    set_created(&c.node_file(&hypothesis), &date_days_ago(30));

    for verb in ["open", "review"] {
        let out = c.run(&[verb]).assert_ok().stdout();
        assert!(
            !out.contains(&seed),
            "{verb} incorrectly reported an old seed with no references:\n{out}"
        );
        assert!(
            out.contains(&hypothesis),
            "{verb} missed an old hypothesis with no references:\n{out}"
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
        out.contains("1 captures waiting over fourteen days; promote or drop them"),
        "{out}"
    );

    let fresh = Corpus::new();
    fresh.run(&["capture", "a recent capture"]).assert_ok();
    set_inbox_stamp_for(&fresh.root, "a recent capture", &stamp_days_ago(10));
    let out = fresh.run(&["review"]).assert_ok().stdout();
    assert!(
        !out.contains("captures waiting over fourteen days"),
        "{out}"
    );
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

#[test]
fn review_out_writes_the_report_and_prints_nothing_else() {
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
    let report = std::fs::read_to_string(&out_path).unwrap();
    assert!(report.contains(&format!("`{bare}`")));
    assert!(report.contains("## Nodes with no references"));
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
        .says("0 errors, 1 warnings");

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
        .says("0 errors, 1 warnings");
    c.run(&["show", &id])
        .assert_ok()
        .says("does not resolve; check the observatory root");

    // $OBSERVATORY_ROOT is the fallback, so a machine that exports one needs
    // no per-corpus setting at all.
    c.run_with_env(&["check"], &[("OBSERVATORY_ROOT", obs.to_str().unwrap())])
        .assert_ok()
        .says("0 errors, 0 warnings");
}

/// The setting is read and written by one verb, `config.yaml` stays whole
/// and machine-written, and the corpus's own value wins over the
/// environment's.
#[test]
fn the_observatory_root_is_a_corpus_setting_that_config_reads_and_writes() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    let elsewhere = c.workdir().join("elsewhere");

    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says("no observatory root");

    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok()
        .says(obs.to_str().unwrap())
        .says("config.yaml");

    let raw = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(
        raw.starts_with("# nebula corpus configuration. Not edited by hand.\n"),
        "{raw}"
    );
    assert!(raw.contains("schema_version: 2"), "{raw}");
    assert!(raw.contains("corpus_id:"), "{raw}");
    assert!(
        raw.contains(&format!("observatory_root: {}", obs.display())),
        "{raw}"
    );

    // Set on purpose for this corpus, so it outranks whatever the shell says.
    c.run_with_env(
        &["config", "observatory-root"],
        &[("OBSERVATORY_ROOT", elsewhere.to_str().unwrap())],
    )
    .assert_ok()
    .says(obs.to_str().unwrap())
    .says("config.yaml");

    let out = c
        .run_with_env(
            &["--json", "config", "observatory-root"],
            &[("OBSERVATORY_ROOT", elsewhere.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).expect("config --json is valid JSON");
    assert_eq!(v["root"], obs.to_str().unwrap());
    assert_eq!(v["source"], "config");

    // And with nothing in the file, the environment answers.
    let bare = Corpus::new();
    let out = bare
        .run_with_env(
            &["--json", "config", "observatory-root"],
            &[("OBSERVATORY_ROOT", obs.to_str().unwrap())],
        )
        .assert_ok()
        .stdout();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["root"], obs.to_str().unwrap());
    assert_eq!(v["source"], "env");
}

/// `migrate` rewrites `config.yaml` whole, so the one setting the file
/// carries has to survive it — and a corpus already at v2 still changes
/// nothing.
#[test]
fn migrate_keeps_the_observatory_root() {
    let c = Corpus::new();
    let obs = observatory(c.workdir());
    c.run(&["config", "observatory-root", obs.to_str().unwrap()])
        .assert_ok();
    let before = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();

    c.run(&["migrate"]).assert_ok().says("nothing changed");

    assert_eq!(
        before,
        std::fs::read_to_string(c.root.join("config.yaml")).unwrap()
    );
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says(obs.to_str().unwrap());
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
        .says("--uri is required unless --kind is discussion");
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
    let git = Command::new("git")
        .arg("-C")
        .arg(&c.root)
        .args(["init", "-q"])
        .output()
        .expect("running git init");
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    let git = Command::new("git")
        .arg("-C")
        .arg(&c.root)
        .args([
            "-c",
            "user.name=neb-test",
            "-c",
            "user.email=neb-test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "add",
            "-A",
        ])
        .output()
        .expect("running git add");
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    let git = Command::new("git")
        .arg("-C")
        .arg(&c.root)
        .args([
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
        ])
        .output()
        .expect("running git commit");
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
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

#[test]
fn migrate_refuses_a_dirty_git_tree() {
    let c = v1_corpus();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&c.root)
            .args(args)
            .output()
            .expect("running git");
        assert!(
            out.status.success(),
            "git {args:?} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    c.run(&["migrate"])
        .assert_fails()
        .says("uncommitted changes");
    // Refused before anything was written.
    let config = std::fs::read_to_string(c.root.join("config.yaml")).unwrap();
    assert!(config.contains("schema_version: 1"), "{config}");

    git(&["add", "-A"]);
    git(&[
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
    ]);
    c.run(&["migrate"])
        .assert_ok()
        .says("4 of 4 nodes rewritten");
    c.run(&["check"]).assert_ok().says("0 errors, 0 warnings");
}

// ------------------------------------------------------------------ commit --

/// Run git in `dir`, asserting it succeeded; stdout as text.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("running git");
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
    git(dir, &["add", "-A"]);
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", format!("{date}T12:00:00Z"))
        .env("GIT_COMMITTER_DATE", format!("{date}T12:00:00Z"))
        .output()
        .expect("running git commit");
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
    assert!(newest.starts_with(&sharpened[..7]), "{text}");
    assert!(
        newest.contains("2020-01-02 neb sharpen history-shape"),
        "{text}"
    );
    assert!(oldest.starts_with(&created[..7]), "{text}");
    assert!(
        oldest.contains("2020-01-01 neb new history-shape"),
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
        assert!(view.get("observatory").is_none());
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
            .says(&format!("invalid --at value `{at}`"));
    }

    // A real leap date is a date, not malformed.
    c.run(&["show", &id, "--at", "2028-02-29"]).assert_ok();
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

/// The setting is off by default and the verbs behave as they always did;
/// `neb config commit` reads and writes it, turning it on is itself the
/// first commit, and turning it off leaves that rewrite for you.
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

/// With `commit` on, every mutating verb lands as one commit named after it
/// and touching only corpus paths; `--no-commit` waives that once; the
/// remote is never touched.
#[test]
fn commit_on_records_each_mutating_verb_and_never_pushes() {
    let (c, remote) = corpus_repo();
    let early = c.run(&["capture", "before the setting"]).stdout_trim();
    c.run(&["config", "commit", "on"]).assert_ok();
    assert_eq!(log(&c.root), ["neb config commit"]);
    c.run(&["config", "observatory-root", "/tmp/observatory"])
        .assert_ok();
    assert_eq!(log(&c.root)[0], "neb config observatory-root");

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

/// The corpus nested in a larger repository, the shape the refusal exists
/// for: something staged outside the corpus must not ride in a `neb`
/// commit, and the write must never be undone because of it.
#[test]
fn a_staged_change_outside_the_corpus_refuses_the_commit_and_keeps_the_write() {
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
    git(&outer, &["add", "README.md"]);
    write(&outer.join("notes.txt"), "never staged\n");
    let head = git(&outer, &["rev-parse", "HEAD"]);

    let run = c.run(&["new", "An idea"]).assert_fails();
    let run = run
        .says("staged changes outside the corpus (README.md)")
        .says("the write is in place");
    assert!(
        run.stdout().contains("an-idea"),
        "the verb reported its write before the refusal:\n{}",
        run.stdout()
    );
    assert!(c.node_file("an-idea").exists(), "the write stays");
    assert_eq!(git(&outer, &["rev-parse", "HEAD"]), head, "no commit");
    let status = git(&outer, &["status", "--porcelain"]);
    assert!(
        status.contains("M  README.md"),
        "their staging is intact:\n{status}"
    );
    assert!(
        status.contains("?? notes.txt"),
        "nothing outside was touched:\n{status}"
    );
    assert!(
        status.contains("?? corpus/nodes/"),
        "and the node was not even staged:\n{status}"
    );
    assert_eq!(
        std::fs::read_to_string(outer.join("README.md")).unwrap(),
        "theirs, edited\n"
    );

    // With their change committed, the next verb sweeps the node up too.
    git(&outer, &["commit", "-q", "-m", "theirs"]);
    let b = c.run(&["new", "B"]).assert_ok().stdout_trim();
    assert_eq!(log(&outer)[0], format!("neb new {b}"));
    assert_eq!(
        head_paths(&outer),
        [
            "corpus/nodes/an-idea.md".to_string(),
            format!("corpus/nodes/{b}.md")
        ]
    );
    assert!(git(&outer, &["status", "--porcelain"]).contains("?? notes.txt"));
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

/// Past the bounded wait the writer refuses, with the hint that says what to
/// do. The write never happens, so retrying is safe.
///
/// The lock is held here in the test process and contended by a spawned
/// `neb`, so this is a real cross-process `flock` and not the in-process
/// table standing in for one.
#[test]
fn a_writer_that_waits_out_the_lock_refuses_with_a_hint_and_writes_nothing() {
    let c = Corpus::new();
    let id = c.seed("a node nobody else gets to edit", "Locked node");
    let before = std::fs::read_to_string(c.node_file(&id)).unwrap();

    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    // It waits the full five seconds before giving up, which is the bound
    // under test.
    c.run(&["tag", &id, "--add", "never"])
        .assert_fails()
        .says("another nebula writer is holding")
        .says("mid-write")
        .says("run it again in a moment");

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
    let observatory = c.workdir().join("observatory");
    let observatory = observatory.to_str().expect("a utf-8 temporary path");

    let held = nebula_core::CorpusLock::acquire(&c.root).expect("holding the lock");

    // The waiter opens — snapshotting a config with no `commit` key — and
    // then polls for the lock this thread is holding.
    let waiting = c.spawn(&["config", "observatory-root", observatory]);

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
        raw.contains(observatory),
        "and the waiter's own setting must still have landed: {raw}"
    );
    c.run(&["config", "commit"]).assert_ok().says("on");
    c.run(&["config", "observatory-root"])
        .assert_ok()
        .says(observatory);
}

/// The lock file is the one thing under the root that is not corpus content,
/// so `neb commit` never stages it and `check` never reads it.
#[test]
fn the_lock_file_is_never_staged_and_never_checked() {
    let c = Corpus::new();
    git_init(&c.root);
    c.run(&["config", "commit", "on"]).assert_ok();
    let id = c.seed("a write that takes the lock", "Locked write");

    assert!(c.root.join(".lock").exists(), "the write took the lock");
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

    c.run(&["check"]).assert_ok().says("0 errors");
    let report = c.run(&["--json", "check"]).assert_ok().stdout();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["nodes"], 1, "the lock file is not read as a node");
    assert_eq!(report["findings"].as_array().unwrap().len(), 0);
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
