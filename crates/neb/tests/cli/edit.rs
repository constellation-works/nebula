//! Bodies and `neb edit`: stdin bodies, the editor and its arguments,
//! protected notes, and edits kept when something changed underneath.

use crate::harness::{
    Corpus, Spawned, corpus_repo, date_days_ago, editor_script, git, log, set_updated, write,
};
use std::path::{Path, PathBuf};

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

/// `edit --by` is a deliberate authorship statement under an Orbit run,
/// although the body has no author field in the stored schema.
#[test]
fn edit_by_is_accepted() {
    let c = Corpus::new();
    let id = c.seed("an idea", "An idea");
    let before = std::fs::read(c.node_file(&id)).unwrap();
    let run = c.run_with_env(&["edit", &id, "--by", "x"], &[("EDITOR", "true")]);
    run.assert_ok();
    assert_eq!(std::fs::read(c.node_file(&id)).unwrap(), before);
    let help = c.run(&["edit", "--help"]).assert_ok().stdout();
    assert!(help.contains("--by"), "{help}");
}

pub(super) fn corpus_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    visit(root, root, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

pub(super) fn shown(c: &Corpus, id: &str) -> serde_json::Value {
    serde_json::from_str(&c.run(&["--json", "show", id]).assert_ok().stdout()).unwrap()
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
