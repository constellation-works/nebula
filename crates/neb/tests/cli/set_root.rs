//! The machine root setting: `init --set-root`, its force and lock, and the
//! owner-only permissions of everything nebula writes.

use crate::harness::{Corpus, Run, bin, neb_command, output, run_command, run_from_home};
use crate::support;
use std::path::{Path, PathBuf};

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
