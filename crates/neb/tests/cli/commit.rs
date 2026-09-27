//! Committing: the `commit` setting, `--no-commit`, one commit per write of
//! corpus paths only, and work staged outside it.

use crate::handoff::observatory_with_h012;
use crate::harness::{
    Corpus, assert_only_corpus_paths_in_log, corpus_repo, dirt, git, git_init, head_paths, log,
    run_from_home, snapshot_corpus_files, write,
};
use crate::observatory::with_legacy_observatory_root;
use crate::refusals::assert_usage_error;
use std::path::{Path, PathBuf};

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
