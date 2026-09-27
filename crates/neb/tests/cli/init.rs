//! `neb init` over an existing corpus: byte-for-byte preservation, the
//! `.gitignore` rules and symlinks, invalid configs, and the warnings about
//! shadowing the configured root.

use crate::harness::{
    Corpus, dirt, git, git_command, git_init, log, output, run_from_home, run_in,
    snapshot_corpus_files, write,
};
use crate::support;

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

/// The warning and its runnable remedy use the shell's path through a
/// symlinked working directory, including when the init target is relative.
#[cfg(unix)]
#[test]
fn init_shadowing_warning_keeps_symlinked_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");
    let config_path = home.join(".config/nebula/root");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, format!("{}\n", configured.display())).unwrap();

    let real = dir.path().join("real");
    let alias = dir.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    let run = run_in(&alias, &home, None, &["init", "new-corpus"], None).assert_ok();
    let expected = alias.join("new-corpus");
    assert!(
        run.stderr()
            .contains(&format!("not {}", expected.display())),
        "{}",
        run.stderr()
    );
    assert!(
        run.stderr().contains(&format!(
            "run `neb init {} --set-root --force`",
            expected.display()
        )),
        "{}",
        run.stderr()
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
