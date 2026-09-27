//! Committing: the commit setting and policy, one commit of corpus paths
//! per write, staged work outside the corpus, and history by date.

use crate::cite::{citing, handing};
use crate::config::{V1_NODE, configless, with_legacy_observatory_root};
use crate::harness::{
    committed, committed_paths, corpus, git, git_init, head, process_locations, seed,
};
use crate::supervised_git::{committing_corpus, pre_commit_hook};
use crate::support;
use nebula_core::verb::{self, CommitPolicy, WriteOptions};
use nebula_core::{
    CommitOutcome, Corpus, EdgeType, Error, NewNode, Promotion, Status, Triage, ops,
};
use std::path::Path;

/// Every `.rs` file of this suite, `tests/core/`, as its path and text.
fn suite_sources() -> Vec<(String, String)> {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("core");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&suite)
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

/// Every child this suite starts comes from the isolating builder in
/// `support`, which is the one place a `Command` is created.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    let containment = (
        "support/mod.rs".to_owned(),
        include_str!("../support/mod.rs").to_owned(),
    );
    for (file, source) in suite_sources().into_iter().chain([containment]) {
        let allowed: &[&str] = if file == "support/mod.rs" {
            &["command"]
        } else {
            &[]
        };
        let strays = support::commands_outside(&source, allowed);
        assert!(
            strays.is_empty(),
            "{file} creates a child outside `support::command`:\n{}",
            strays.join("\n")
        );
    }
}

/// Each way a write can end up uncommitted is its own outcome, and a real
/// change is a commit named for the verb and the ids it touched.
#[test]
fn commit_outcome_distinguishes_disabled_not_a_repo_and_nothing_to_commit() {
    // Outside any repository: off is `Disabled`, on is `NotARepository`.
    let (_plain, mut plain) = corpus();
    let a = seed(&plain, "A", &[]);
    assert!(matches!(
        ops::commit(&plain, "new", &[&a]),
        Ok(CommitOutcome::Disabled)
    ));
    ops::set_commit(&mut plain, true).unwrap();
    assert!(matches!(
        ops::commit(&plain, "new", &[&a]),
        Ok(CommitOutcome::NotARepository)
    ));

    // In a repository: a real change commits, and the same call again finds
    // nothing left to record.
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    let b = seed(&corpus, "B", &[]);
    let done = committed(ops::commit(&corpus, "new", &[&b]));
    assert_eq!(done.message, format!("neb new {b}"));
    assert_eq!(done.hash, head(&root));
    assert!(matches!(
        ops::commit(&corpus, "new", &[&b]),
        Ok(CommitOutcome::NothingToCommit)
    ));
    assert_eq!(head(&root), done.hash);
}

#[test]
fn commit_is_off_by_default_and_off_means_nothing_is_committed() {
    // Not a repository at all: with the setting off, git is never consulted.
    let (_plain, plain) = corpus();
    assert!(!plain.commit_setting().enabled);
    seed(&plain, "A", &[]);
    assert!(matches!(
        ops::commit(&plain, "new", &["a"]),
        Ok(CommitOutcome::Disabled)
    ));

    // In a repository, off still means the write is left uncommitted.
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::commit(&corpus, "new", &["a"]),
        Ok(CommitOutcome::Disabled)
    ));
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? nodes/"),
        "the write is there, not staged, not committed"
    );

    // The setting is a `config.yaml` key that is absent until turned on, so
    // a file written before it existed renders back unchanged.
    let config = || std::fs::read_to_string(root.join("config.yaml")).unwrap();
    assert!(!config().contains("commit"), "{}", config());
    assert!(ops::set_commit(&mut corpus, true).unwrap().enabled);
    assert!(config().contains("commit: true"), "{}", config());
    assert!(!ops::set_commit(&mut corpus, false).unwrap().enabled);
    assert!(!config().contains("commit"), "{}", config());
}

#[test]
fn commit_on_records_each_write_as_one_commit_of_corpus_paths_only() {
    let (dir, mut corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    ops::set_commit(&mut corpus, true).unwrap();
    let start = committed(ops::commit(&corpus, "config", &["commit"]));
    assert_eq!(start.message, "neb config commit");
    assert_eq!(
        committed_paths(&root, "HEAD"),
        [".gitignore", "config.yaml"]
    );

    // A capture creates inbox/, a promote writes a node and settles the
    // capture: one commit each, naming what the verb touched.
    let entry = ops::capture(&corpus, "a thought").unwrap();
    let captured = committed(ops::commit(&corpus, "capture", &[&entry.id]));
    assert_eq!(captured.message, format!("neb capture {}", entry.id));
    assert_eq!(captured.hash, head(&root));
    assert!(
        committed_paths(&root, "HEAD")
            .iter()
            .all(|p| p.starts_with("inbox/")),
        "{:?}",
        committed_paths(&root, "HEAD")
    );

    let id = ops::promote(&corpus, &entry.id, &Promotion::default(), 0)
        .unwrap()
        .doc
        .node
        .id;
    let promoted = committed(ops::commit(&corpus, "promote", &[&entry.id, &id]));
    assert_eq!(promoted.message, format!("neb promote {} {id}", entry.id));
    let mut paths = committed_paths(&root, "HEAD");
    paths.sort();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths.iter().any(|p| p == &format!("nodes/{id}.md")));
    assert!(paths.iter().any(|p| p.starts_with("inbox/")));

    // A stray file under the root that is not a corpus path is left alone,
    // and a write that changed nothing produces no commit.
    std::fs::write(root.join("scratch.txt"), "not the corpus\n").unwrap();
    let before = head(&root);
    assert!(matches!(
        ops::commit(&corpus, "note", &[&id]),
        Ok(CommitOutcome::NothingToCommit)
    ));
    assert_eq!(head(&root), before);
    assert!(
        git(&root, &["status", "--porcelain"]).contains("?? scratch.txt"),
        "scratch.txt is neither staged nor committed"
    );
    assert!(
        git(
            &root,
            &[
                "status",
                "--porcelain",
                "--",
                "nodes",
                "inbox",
                "config.yaml"
            ]
        )
        .is_empty()
    );
    assert!(
        git(&root, &["remote"]).is_empty(),
        "nothing to push to, and nothing pushed"
    );
}

/// A `neb` commit is exactly the corpus, by pathspec: work staged elsewhere
/// in the repository neither blocks it nor rides in it, and stays staged.
#[test]
fn staged_work_outside_the_corpus_stays_staged_and_out_of_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join("README.md"), "theirs\n").unwrap();
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));
    assert_eq!(committed_paths(&outer, "HEAD"), ["corpus/config.yaml"]);

    std::fs::write(outer.join("README.md"), "theirs, edited\n").unwrap();
    std::fs::write(root.join("unrelated.txt"), "also theirs\n").unwrap();
    git(&outer, &["add", "README.md", "corpus/unrelated.txt"]);
    let a = seed(&corpus, "A", &[]);
    let done = committed(ops::commit(&corpus, "new", &[&a]));
    assert_eq!(done.message, format!("neb new {a}"));
    assert_eq!(
        committed_paths(&outer, "HEAD"),
        [format!("corpus/nodes/{a}.md")]
    );
    let staged = git(&outer, &["diff", "--cached", "--name-only"]);
    assert_eq!(
        staged.lines().collect::<Vec<_>>(),
        ["README.md", "corpus/unrelated.txt"],
        "their staging is intact"
    );
}

#[test]
fn staged_corpus_paths_with_unicode_spaces_and_newlines_are_unambiguous() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    git(&outer, &["add", "-A"]);
    git(&outer, &["commit", "-q", "-m", "start"]);
    ops::set_commit(&mut corpus, true).unwrap();
    committed(ops::commit(&corpus, "config", &["commit"]));

    let names = ["시간.md", "two words.md", "two\nlines.md"];
    for name in names {
        std::fs::write(root.join("inbox").join(name), "fixture\n").unwrap();
    }
    git(&outer, &["add", "--", "corpus/inbox"]);

    let id = seed(&corpus, "A", &[]);
    committed(ops::commit(&corpus, "new", &[&id]));

    let committed = git(&outer, &["show", "--name-only", "--format=", "-z", "HEAD"]);
    let paths: Vec<&str> = committed
        .split('\0')
        .filter(|path| !path.is_empty())
        .collect();
    for name in names {
        assert!(
            paths.contains(&format!("corpus/inbox/{name}").as_str()),
            "missing {name:?} from {paths:?}"
        );
    }
    assert!(paths.contains(&format!("corpus/nodes/{id}.md").as_str()));
}

#[test]
fn commit_on_in_a_corpus_the_containing_repository_ignores_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&process_locations(), &root).unwrap();
    git_init(&outer);
    std::fs::write(outer.join(".gitignore"), "corpus/\n").unwrap();
    ops::set_commit(&mut corpus, true).unwrap();
    let a = seed(&corpus, "A", &[]);
    assert!(matches!(
        ops::commit(&corpus, "new", &[&a]),
        Err(Error::CorpusIgnored(r)) if r == root
    ));
    assert!(corpus.node_path(&a).unwrap().exists());
}

/// Commit everything in `dir` dated `stamp`, an ISO 8601 time with its offset.
fn commit_stamped(dir: &std::path::Path, stamp: &str) {
    git(dir, &["add", "-A"]);
    let out = support::output(
        support::git_command(dir, support::home())
            .args(["commit", "-q", "-m", stamp])
            .env("GIT_AUTHOR_DATE", stamp)
            .env("GIT_COMMITTER_DATE", stamp),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A date names each commit by its own day, in the offset it was recorded
/// with: the day `history` prints. Here the two commits' UTC days are the
/// other way round, and no single reader's zone puts both on their own days,
/// so neither UTC nor the reader's local day would pass.
#[test]
fn load_at_a_date_takes_each_commit_on_its_own_day() {
    let (dir, corpus) = corpus();
    let root = dir.path().join("corpus");
    git_init(&root);
    let id = seed(&corpus, "Days travel", &[]);
    // 2020-01-02T11:30Z, late on the first in UTC-12.
    commit_stamped(&root, "2020-01-01T23:30:00-12:00");
    ops::note(&corpus, &id, "written a day ahead", None).unwrap();
    // 2020-01-01T18:00Z, early on the second in UTC+14.
    commit_stamped(&root, "2020-01-02T08:00:00+14:00");

    let days: Vec<String> = corpus
        .history(&id)
        .unwrap()
        .into_iter()
        .map(|entry| entry.date)
        .collect();
    assert_eq!(days, ["2020-01-02", "2020-01-01"]);
    let noted = |date| {
        corpus
            .load_at(&id, date)
            .unwrap()
            .body
            .contains("written a day ahead")
    };
    assert!(!noted("2020-01-01"), "the first day is before the note");
    assert!(noted("2020-01-02"), "the second day has it");
    assert!(matches!(
        corpus.load_at(&id, "2019-12-31"),
        Err(Error::NoNodeAtRevision { .. })
    ));
}

/// The migration's commit is made inside the migration's lock: a hook run
/// by that commit finds this process's holder record in `.lock`, and the
/// record is gone once `migrate` returns.
#[cfg(unix)]
#[test]
fn migrate_commits_under_its_own_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = configless(dir.path(), "first", V1_NODE);
    std::fs::write(
        root.join("config.yaml"),
        "schema_version: 1\ncorpus_id: neb-abc123\ncommit: true\n",
    )
    .unwrap();
    git_init(&root);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "v1 corpus"]);
    let seen = dir.path().join("holder-seen");
    pre_commit_hook(
        &root,
        &dir.path().join("hook.pgid"),
        &format!("cat .lock > '{}'", seen.display()),
    );

    let migrated = verb::migrate(
        &process_locations(),
        Some(root.clone()),
        &WriteOptions::default(),
    )
    .unwrap();
    let done = committed(migrated.commit.expect("the setting is on"));
    assert_eq!(done.message, "neb migrate");
    assert_eq!(migrated.value.root, root);

    let holder = std::fs::read_to_string(&seen).expect("the hook ran");
    assert!(
        holder.contains(&std::process::id().to_string()),
        "the commit ran without this process holding the lock: {holder:?}"
    );
    // Released: the holder empties its record before the guard lets go.
    // Read from the file rather than by taking the lock from another thread,
    // which a sibling test's fork can briefly make look held.
    assert_eq!(
        std::fs::read_to_string(root.join(nebula_core::LOCK_FILE)).unwrap(),
        ""
    );
    assert_eq!(
        git(&root, &["log", "--format=%s"])
            .lines()
            .collect::<Vec<_>>(),
        ["neb migrate", "v1 corpus"]
    );
}

/// What is left uncommitted in a corpus that is its own repository, the
/// lock file aside.
#[cfg(unix)]
fn dirt(root: &Path) -> String {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude).lock",
        ],
    )
}

/// With commits on, each writing verb commits what it wrote, as
/// `neb <verb> <ids>`, and leaves nothing behind for the next one.
#[cfg(unix)]
#[test]
fn every_write_op_commits_what_it_wrote_when_commit_is_on() {
    let (_dir, mut corpus, root) = committing_corpus();
    let options = WriteOptions::default();
    let expect = |commit: Option<Result<CommitOutcome, Error>>, message: &str| {
        let done = committed(commit.expect("the policy commits"));
        assert_eq!(done.message, message);
        assert!(dirt(&root).is_empty(), "{message} left: {}", dirt(&root));
    };

    let captured = verb::capture(&corpus, "a thought to keep", &options).unwrap();
    let kept = captured.value.id.clone();
    expect(captured.commit, &format!("neb capture {kept}"));
    let gone = verb::capture(&corpus, "a thought to drop", &options).unwrap();
    expect(gone.commit, &format!("neb capture {}", gone.value.id));
    let dropped = verb::drop(&corpus, &gone.value.id, &options).unwrap();
    expect(dropped.commit, &format!("neb drop {}", gone.value.id));

    let promotion = Promotion {
        title: Some("Kept".to_string()),
        ..Promotion::default()
    };
    let promoted = verb::promote(&corpus, &kept, &promotion, 0, &options).unwrap();
    let a = promoted.value.doc.node.id.clone();
    expect(promoted.commit, &format!("neb promote {kept} {a}"));

    let created = verb::new_node(
        &corpus,
        &NewNode {
            title: "Other".to_string(),
            ..NewNode::default()
        },
        &options,
    )
    .unwrap();
    let b = created.value.doc.node.id.clone();
    expect(created.commit, &format!("neb new {b}"));

    let before = corpus.load(&a).unwrap().body;
    let edited = verb::edit(
        &corpus,
        &a,
        &before,
        &format!("{before}\nMore prose.\n"),
        None,
        false,
        &options,
    )
    .unwrap();
    assert!(edited.value.changed);
    expect(edited.commit, &format!("neb edit {a}"));

    let noted = verb::note(&corpus, &a, "a dated line", None, false, &options).unwrap();
    expect(noted.commit, &format!("neb note {a}"));

    let sharpened =
        verb::sharpen(&corpus, &a, "it never shows up", Some("claude"), &options).unwrap();
    expect(sharpened.commit, &format!("neb sharpen {a}"));
    let confirmed = verb::confirm_kill(&corpus, &a, &options).unwrap();
    expect(confirmed.commit, &format!("neb sharpen {a}"));

    let status = verb::set_status(&corpus, &b, Status::Hypothesis, None, &options);
    assert!(
        status.is_err(),
        "a seed with no kill cannot be a hypothesis"
    );
    let refuted =
        verb::set_status(&corpus, &a, Status::Refuted, Some("it fired"), &options).unwrap();
    expect(refuted.commit, &format!("neb status {a}"));

    let linked = verb::link(&corpus, &b, EdgeType::DerivesFrom, &a, None, &options).unwrap();
    expect(linked.commit, &format!("neb link {b} {a}"));

    let tagged = verb::retag(&corpus, &b, &["physics".to_string()], &[], &options).unwrap();
    expect(tagged.commit, &format!("neb tag {b}"));

    let cited = verb::cite(&corpus, &b, &citing("https://example.com/paper"), &options).unwrap();
    let reference = cited.value.cited.reference.clone();
    expect(cited.commit, &format!("neb cite {b} {reference}"));

    let handed = verb::handoff(&corpus, &b, &handing("H012"), &options).unwrap();
    expect(handed.commit, &format!("neb handoff {b} H012"));

    let triaged = verb::capture(&corpus, "a thought triage drops", &options).unwrap();
    let entry = triaged.value.id.clone();
    expect(triaged.commit, &format!("neb capture {entry}"));
    let mut session = Triage::start(&corpus, None).unwrap();
    let step = session
        .decide(&corpus, nebula_core::triage::Action::Drop, &options)
        .unwrap();
    step.value.unwrap();
    expect(step.commit, &format!("neb drop {entry}"));

    with_legacy_observatory_root(&root);
    git(&root, &["commit", "-q", "-am", "a legacy key"]);
    let dropped = verb::set_observatory_root(&mut corpus, None, true, &options).unwrap();
    assert!(dropped.value.dropped.is_some_and(|d| d.removed.is_some()));
    expect(dropped.commit, "neb config observatory-root");

    // Off means off: the setting that turns commits off is not committed.
    let off = verb::set_commit(&mut corpus, false, &options).unwrap();
    assert!(matches!(off.commit, Some(Ok(CommitOutcome::Disabled))));
    assert!(dirt(&root).contains("config.yaml"), "{}", dirt(&root));
}

/// `--no-commit` writes and does not commit, whatever the setting says:
/// the verb reports no commit attempted and `HEAD` stays where it was.
#[cfg(unix)]
#[test]
fn write_op_with_skip_policy_does_not_commit() {
    let (_dir, corpus, root) = committing_corpus();
    let skip = WriteOptions {
        commit: CommitPolicy::Skip,
        ..WriteOptions::default()
    };
    let before = head(&root);

    let captured = verb::capture(&corpus, "an uncommitted thought", &skip).unwrap();
    assert!(captured.commit.is_none());
    let created = verb::new_node(
        &corpus,
        &NewNode {
            title: "Loose".to_string(),
            ..NewNode::default()
        },
        &skip,
    )
    .unwrap();
    assert!(created.commit.is_none());
    let id = created.value.doc.node.id;
    let noted = verb::note(&corpus, &id, "still loose", None, false, &skip).unwrap();
    assert!(noted.commit.is_none());

    assert_eq!(head(&root), before);
    assert!(
        dirt(&root).contains(&format!("nodes/{id}.md")),
        "the writes landed on disk: {}",
        dirt(&root)
    );
}

/// The append-only notes rule is enforced where the body is written, not
/// only by the CLI's editor: a body whose `## Notes` section was rewritten
/// or removed is refused and the file is left as it was, while prose
/// outside the notes stays editable.
#[test]
fn set_body_refuses_a_rewritten_notes_section() {
    let (_dir, corpus) = corpus();
    let id = seed(&corpus, "Noted", &[]);
    ops::note(&corpus, &id, "the first thought", None).unwrap();
    let path = corpus.node_path(&id).unwrap();
    let file = std::fs::read_to_string(&path).unwrap();
    let body = corpus.load(&id).unwrap().body;
    assert!(body.contains("## Notes"), "{body}");

    let without = body.split("## Notes").next().unwrap().to_string();
    for rewritten in [
        body.replace("the first thought", "a better thought"),
        without,
    ] {
        assert!(
            matches!(
                ops::set_body(&corpus, &id, &rewritten),
                Err(Error::NotesChanged)
            ),
            "{rewritten:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    }

    let prose = format!("Prose added above.\n\n{body}");
    assert!(ops::set_body(&corpus, &id, &prose).unwrap().is_some());
}
