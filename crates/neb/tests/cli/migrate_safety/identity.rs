//! Migration must leave nodes readable through the normal CLI doors.

use super::{V1_CONFIG, V1_FIRST, every_file_but_lock, v1_corpus_of, write};

#[test]
fn migration_refuses_filename_id_mismatch_before_any_write() {
    for converted in [false, true] {
        let second = V1_FIRST.replace("a-first", "b-second");
        let c = v1_corpus_of(
            Some(V1_CONFIG),
            &[("a-first", V1_FIRST), ("b-second", &second)],
        );
        if converted {
            // An interrupted migration can leave an unchanged v2 node
            // beside a v1 node. It still needs filename validation.
            let reference = v1_corpus_of(Some(V1_CONFIG), &[("b-second", &second)]);
            reference.run(&["migrate"]).assert_ok();
            write(
                &c.node_file("b-second"),
                &std::fs::read_to_string(reference.node_file("b-second")).unwrap(),
            );
        }
        std::fs::rename(c.node_file("b-second"), c.node_file("wrong")).unwrap();
        let before = every_file_but_lock(&c.root);
        c.run(&["migrate"])
            .assert_fails()
            .says("wrong.md")
            .says("stores the id `b-second`");
        assert_eq!(before, every_file_but_lock(&c.root));

        std::fs::rename(c.node_file("wrong"), c.node_file("b-second")).unwrap();
        c.run(&["migrate"]).assert_ok();
        c.run(&["list"])
            .assert_ok()
            .says("a-first")
            .says("b-second");
        // Even a current-schema, otherwise no-op migration must agree
        // with normal loading about the renamed file.
        std::fs::rename(c.node_file("b-second"), c.node_file("wrong")).unwrap();
        let before = every_file_but_lock(&c.root);
        let refused = c.run(&["--json", "migrate"]).refusal();
        assert_eq!(refused["code"], "id_mismatch");
        c.run(&["list"]).assert_fails().says("wrong.md");
        assert_eq!(before, every_file_but_lock(&c.root));
    }
}

#[test]
fn migration_preserves_unicode_ids_and_filesystem_identity() {
    for (filename, id) in [
        ("café", "café"),
        ("cafe\u{301}", "cafe\u{301}"),
        ("cafe\u{301}", "café"),
        ("CAFÉ", "café"),
    ] {
        let text = V1_FIRST.replace("a-first", id);
        let c = v1_corpus_of(Some(V1_CONFIG), &[(filename, &text)]);
        // One entry exists. The alternate spelling is an alias only if
        // this filesystem resolves it; Linux and macOS differ here.
        if c.node_file(id).exists() {
            c.run(&["migrate"]).assert_ok();
            c.run(&["list"]).assert_ok().says(id);
            c.run(&["show", id]).assert_ok();
            let before = every_file_but_lock(&c.root);
            c.run(&["migrate"]).assert_ok().says("nothing changed");
            assert_eq!(before, every_file_but_lock(&c.root));
        } else {
            let before = every_file_but_lock(&c.root);
            c.run(&["migrate"]).assert_fails().says("stores the id");
            assert_eq!(before, every_file_but_lock(&c.root));
        }
    }
}

#[cfg(unix)]
#[test]
fn migration_identity_works_through_a_symlinked_root() {
    let text = V1_FIRST.replace("a-first", "cafe\u{301}");
    let mut c = v1_corpus_of(Some(V1_CONFIG), &[("cafe\u{301}", &text)]);
    let alias = c.dir.path().join("alias");
    std::os::unix::fs::symlink(&c.root, &alias).unwrap();
    c.root = alias;
    c.run(&["migrate"]).assert_ok();
    c.run(&["list"]).assert_ok().says("cafe\u{301}");
    c.run(&["show", "cafe\u{301}"]).assert_ok();
}

#[cfg(unix)]
#[test]
fn migration_refuses_hard_link_aliases_before_any_write() {
    let c = v1_corpus_of(Some(V1_CONFIG), &[("a-first", V1_FIRST)]);
    std::fs::hard_link(c.node_file("a-first"), c.node_file("alias")).unwrap();
    let before = every_file_but_lock(&c.root);
    c.run(&["migrate"])
        .assert_fails()
        .says("alias.md")
        .says("stores the id `a-first`");
    assert_eq!(before, every_file_but_lock(&c.root));
}
