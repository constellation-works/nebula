//! Containment: no child inherits the host's Orbit, read-only or git
//! environment, every child comes from the isolating builder, and a guarded
//! child is killed and reaped rather than left running.

use crate::harness::{Corpus, git_command, neb_command, output};
use crate::support::{self, ChildGuard};
use std::path::{Path, PathBuf};
use std::process::Stdio;

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
fn harness_children_never_inherit_orbit_or_read_only_env() {
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

/// Every `.rs` file of this suite, `tests/cli/`, as its path and text.
fn suite_sources() -> Vec<(String, String)> {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&suite)
        .expect("reading the suite directory")
        .map(|entry| entry.expect("reading a suite entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    assert!(
        files.iter().any(|file| file.ends_with("harness.rs")),
        "no suite files under {}",
        suite.display()
    );
    files
        .into_iter()
        .map(|file| {
            let source = std::fs::read_to_string(&file).expect("reading a suite file");
            (file.display().to_string(), source)
        })
        .collect()
}

/// The check that every child comes from the builder reads every file of
/// this suite, and it catches a command created anywhere else.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    for (file, source) in suite_sources() {
        let strays = support::commands_outside(&source, &["neb_command"]);
        assert!(
            strays.is_empty(),
            "{file} creates a child outside `neb_command`/`git_command`:\n{}",
            strays.join("\n")
        );
    }

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
