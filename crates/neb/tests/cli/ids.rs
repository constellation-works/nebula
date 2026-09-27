//! Node ids: slugs from titles, explicit `--id`, and the stored id and file
//! name that must agree before any write, including aliases, symlinks and
//! traversal.

use crate::harness::{Corpus, write};
use std::path::{Path, PathBuf};

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

#[test]
fn unsafe_ids_have_one_argument_refusal_across_verbs() {
    let c = Corpus::new();
    c.run(&["new", "Safe"]).assert_ok();
    let entry = c
        .run(&["capture", "Fixture capture"])
        .assert_ok()
        .stdout_trim();
    for id in ["../x", "/x", "..", "x\\y"] {
        for args in [
            vec!["show", id],
            vec!["show", id, "--at", "abcd"],
            vec!["trace", id],
            vec!["impact", id],
            vec!["log", id],
            vec!["edit", id],
            vec!["sharpen", id, "--kill", "fixture"],
            vec!["sharpen", id, "--confirm"],
            vec!["status", id, "abandoned"],
            vec!["tag", id, "--add", "fixture"],
            vec!["note", id, "fixture"],
            vec!["cite", id, "--uri", "https://example.test"],
            vec!["handoff", id, "H012"],
            vec!["link", id, "derives-from", "safe"],
            vec!["link", "safe", "derives-from", id],
            vec!["link", id, "derives-from", id],
            vec!["new", "Fixture", "--id", id],
            vec!["new", "Fixture", "--parent", id],
            vec!["new", "Fixture", "--reopens", id],
            vec!["new", "Fixture", "--contradicts", id],
            vec!["promote", id],
            vec!["drop", id],
            vec!["promote", &entry, "--id", id],
            vec!["promote", &entry, "--parent", id],
        ] {
            let mut json_args = vec!["--json"];
            json_args.extend(args);
            let run = c.run(&json_args);
            assert_eq!(run.usage_refusal()["code"], "unsafe_id", "{json_args:?}");
            assert!(run.stdout().is_empty());
        }
        let graph = c.run(&["graph", "--mermaid", "--from", id]);
        assert_eq!(graph.out.status.code(), Some(2), "{}", graph.stderr());
        graph.says("cannot be a node id");
    }
    c.run(&["check"]).assert_ok();
}

#[test]
fn nfc_titles_share_slugs_and_duplicate_checks() {
    for (first, second) in [
        ("Cafe\u{301} NFC", "Café NFC"),
        ("Café NFC", "Cafe\u{301} NFC"),
    ] {
        let c = Corpus::new();
        assert_eq!(c.run(&["new", first]).assert_ok().stdout_trim(), "café-nfc");
        assert_eq!(
            c.run(&["--json", "new", second]).refusal()["code"],
            "node_exists"
        );
    }
}

#[test]
fn nfc_capture_duplicates_do_not_fall_back_to_longer_ids() {
    let c = Corpus::new();
    let first = "Café particles scatter beyond distant stellar clouds";
    let second = "Cafe\u{301} particles scatter beyond distant stellar clouds";
    let entry = c.run(&["capture", first]).assert_ok().stdout_trim();
    let repeated = c.run(&["capture", second]).assert_ok();
    assert!(repeated.stderr().contains(&format!("same as {entry}")));
    let repeated = repeated.stdout_trim();
    assert_eq!(
        c.run(&["promote", &entry]).assert_ok().stdout_trim(),
        "café-particles-scatter-beyond-distant"
    );
    assert_eq!(
        c.run(&["--json", "promote", &repeated]).refusal()["code"],
        "node_exists"
    );
}

#[test]
fn existing_non_nfc_ids_are_read_and_written_without_migration() {
    let c = Corpus::new();
    c.run(&["new", "Legacy", "--id", "legacy"]).assert_ok();
    let legacy = "cafe\u{301}";
    rewrite_stored_id(&c.node_file("legacy"), legacy);
    std::fs::rename(c.node_file("legacy"), c.node_file(legacy)).unwrap();
    c.run(&["show", legacy]).assert_ok();
    c.run(&["note", legacy, "fixture note"]).assert_ok();
    let raw = std::fs::read_to_string(c.node_file(legacy)).unwrap();
    assert!(raw.contains(&format!("id: {legacy}\n")));
    c.run(&["check"]).assert_ok();
}
