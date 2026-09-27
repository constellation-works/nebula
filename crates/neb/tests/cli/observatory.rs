//! Observatory references: bare record ids, the machine's observatory root,
//! and the legacy config key it replaced.

use crate::harness::{Corpus, corpus_repo, git, log, write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
#[test]
fn observatory_machine_setting_refuses_non_regular_files() {
    crate::root::machine_setting_refuses_non_regular_files(
        "observatory-root",
        &["--json", "config", "observatory-root"],
    );
}

#[cfg(unix)]
#[test]
fn machine_settings_follow_regular_symlinks_and_allow_missing_targets() {
    use crate::harness::run_from_home;
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("configured-corpus");
    let obs = dir.path().join("observatory");
    run_from_home(&home, Some(&root), &["init", "--set-root"], None).assert_ok();
    run_from_home(
        &home,
        Some(&root),
        &["config", "observatory-root", obs.to_str().unwrap()],
        None,
    )
    .assert_ok();
    for name in ["root", "observatory-root"] {
        let setting = home.join(".config/nebula").join(name);
        let target = dir.path().join(name);
        std::fs::rename(&setting, &target).unwrap();
        symlink(&target, &setting).unwrap();
    }
    run_from_home(&home, None, &["list"], None).assert_ok();
    run_from_home(&home, None, &["config", "observatory-root"], None)
        .assert_ok()
        .says(obs.to_str().unwrap());

    for name in ["root", "observatory-root"] {
        std::fs::remove_file(dir.path().join(name)).unwrap();
    }
    run_from_home(&home, Some(&home.join(".nebula")), &["init"], None).assert_ok();
    run_from_home(&home, None, &["list"], None).assert_ok();
    run_from_home(&home, None, &["config", "observatory-root"], None)
        .assert_ok()
        .says("no observatory root");
}

/// An Observatory checkout in research layout v2: records are files under
/// `questions/`, `hypotheses/` and `theories/`, and directories under
/// `research/`. Only what a test cites is created, so an id that should not
/// resolve genuinely does not.
pub(super) fn observatory(at: &Path) -> PathBuf {
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
pub(super) fn with_legacy_observatory_root(c: &Corpus) -> String {
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
