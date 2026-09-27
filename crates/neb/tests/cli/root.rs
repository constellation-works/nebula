//! Finding the corpus: capture creating one on first use, and root
//! resolution from `--root`, `$NEBULA_ROOT`, the working directory, the
//! machine setting and the default.

use crate::harness::{Run, neb_command, output, run_from_home, run_in, write};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let err =
        nebula_core::Corpus::resolve_root(&nebula_core::Locations::default(), Some(PathBuf::new()))
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
