//! `neb promote` and the capture payloads around it: suggested parents,
//! minted ids and the JSON shape of the created node.

use crate::harness::Corpus;
use crate::near::lexical_fixture;

/// Plant the durable record left between writing a node and striking its capture.
fn pending_promotion(c: &Corpus) -> (String, std::path::PathBuf, String) {
    let id = c
        .run(&["capture", "-q", "valuable thought"])
        .assert_ok()
        .stdout_trim();
    let inbox = std::fs::read_dir(c.root.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let raw = std::fs::read_to_string(&inbox).unwrap();
    let line = raw
        .lines()
        .find(|line| line.starts_with(&format!("- [{id}] ")))
        .unwrap();
    let stamp = line.split_once("] ").unwrap().1.split_once(' ').unwrap().0;
    crate::harness::write(
        &c.root.join(".pending"),
        &serde_json::json!({
            "op": "promote", "entry": id, "stamp": stamp, "node": "promoted"
        })
        .to_string(),
    );
    (id, inbox, raw)
}

fn pending_target_is_refused(plant: impl FnOnce(&Corpus)) {
    let c = Corpus::new();
    let (_id, inbox, raw) = pending_promotion(&c);
    let record = std::fs::read(c.root.join(".pending")).unwrap();
    plant(&c);
    // Both the writer and the lock-free reader must refuse, preserving evidence.
    for args in [&["capture", "next thought", "--quiet"][..], &["inbox"][..]] {
        c.run(args)
            .assert_fails()
            .says(c.node_file("promoted").to_str().unwrap());
        assert_eq!(std::fs::read(c.root.join(".pending")).unwrap(), record);
        assert_eq!(std::fs::read_to_string(&inbox).unwrap(), raw);
    }
}

#[test]
fn pending_promotion_refuses_a_directory() {
    pending_target_is_refused(|c| std::fs::create_dir(c.node_file("promoted")).unwrap());
}

#[test]
fn pending_promotion_refuses_invalid_nodes() {
    for bytes in [
        b"not a node".as_slice(),
        b"---\nid: [invalid\n---\n",
        b"\xff",
    ] {
        pending_target_is_refused(|c| std::fs::write(c.node_file("promoted"), bytes).unwrap());
    }
}

#[cfg(unix)]
#[test]
fn pending_promotion_refuses_an_unreadable_node() {
    use std::os::unix::fs::PermissionsExt as _;

    pending_target_is_refused(|c| {
        let path = c.node_file("promoted");
        crate::harness::write(&path, "unreadable");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
    });
}

#[test]
fn pending_promotion_refuses_a_node_with_a_different_id() {
    pending_target_is_refused(|c| {
        crate::harness::write(
            &c.node_file("promoted"),
            "---\nid: another\ntitle: Another\nstatus: seed\ncreated: 2026-09-27\nupdated: 2026-09-27\n---\nA thought\n",
        );
    });
}

#[cfg(unix)]
#[test]
fn pending_promotion_refuses_symlinks_including_dangling_ones() {
    for dangling in [false, true] {
        pending_target_is_refused(|c| {
            let target = c.workdir().join("target.md");
            if !dangling {
                crate::harness::write(&target, "not a node");
            }
            std::os::unix::fs::symlink(target, c.node_file("promoted")).unwrap();
        });
    }
}

#[test]
fn pending_promotion_with_an_absent_node_keeps_the_capture_waiting() {
    let c = Corpus::new();
    let (id, inbox, raw) = pending_promotion(&c);
    c.run(&["inbox"]).assert_ok().says(&id);
    assert!(c.root.join(".pending").exists(), "reads do not recover");
    c.run(&["capture", "next thought", "-q"]).assert_ok();
    assert!(!c.root.join(".pending").exists());
    assert!(!c.node_file("promoted").exists());
    assert!(std::fs::read_to_string(inbox).unwrap().starts_with(&raw));
    c.run(&["inbox"]).assert_ok().says(&id);
}

#[test]
fn pending_promotion_replays_a_completed_node_idempotently() {
    let c = Corpus::new();
    let (id, inbox, raw) = pending_promotion(&c);
    // Complete the normal promotion, then restore its pre-strike crash state.
    c.run(&["promote", &id, "--id", "promoted", "-q"])
        .assert_ok();
    let node = std::fs::read(c.node_file("promoted")).unwrap();
    let stamp = raw
        .lines()
        .find(|line| line.starts_with(&format!("- [{id}] ")))
        .unwrap()
        .split_once("] ")
        .unwrap()
        .1
        .split_once(' ')
        .unwrap()
        .0;
    let record =
        serde_json::json!({"op": "promote", "entry": id, "stamp": stamp, "node": "promoted"})
            .to_string();
    crate::harness::write(&inbox, &raw);
    crate::harness::write(&c.root.join(".pending"), &record);
    assert!(!c.run(&["inbox"]).assert_ok().stdout().contains(&id));
    assert_eq!(std::fs::read_to_string(&inbox).unwrap(), raw);
    c.run(&["capture", "next thought", "-q"]).assert_ok();
    let settled = std::fs::read_to_string(&inbox).unwrap();
    assert!(settled.contains("~~ -> promoted"));
    assert!(!c.root.join(".pending").exists());
    crate::harness::write(&c.root.join(".pending"), &record);
    c.run(&["capture", "another thought", "-q"]).assert_ok();
    assert!(
        std::fs::read_to_string(&inbox)
            .unwrap()
            .starts_with(&settled)
    );
    assert!(!c.root.join(".pending").exists());
    assert_eq!(std::fs::read(c.node_file("promoted")).unwrap(), node);
}

#[test]
fn capture_json_carries_the_entry_and_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let json = c
        .run(&["capture", "--json", "a taxonomy of tags"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["entry"]["text"], "a taxonomy of tags", "{json}");
    assert_eq!(out["entry"]["id"].as_str().unwrap().len(), 4, "{json}");
    assert!(out["entry"]["at"].is_string(), "{json}");
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(near[0]["score"].is_number(), "{json}");
    assert!(near[0]["band"].is_string(), "{json}");

    let json = c
        .run(&[
            "capture",
            "--json",
            "--quiet",
            "a taxonomy of tags, quietly",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        out["near"],
        serde_json::json!([]),
        "quiet empties it: {json}"
    );
    assert!(out["entry"]["id"].is_string(), "{json}");
}

#[test]
fn promote_without_a_parent_suggests_and_proceeds_as_a_root() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "search ranking decays with age"])
        .assert_ok()
        .stdout_trim();

    let out = c.run(&["promote", &entry]).assert_ok().stdout();
    let mut lines = out.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("search-ranking-decays-with-age "),
        "id and path first: {out}"
    );
    assert_eq!(lines.next(), Some("near:"), "{out}");
    let rest: Vec<&str> = lines.collect();
    assert_eq!(rest, [rest[0]], "one node shares a word: {out}");
    assert!(rest[0].contains("ranking-decay-half-life"), "{out}");

    // Promoted as a root: no parent, no edge, whatever was suggested.
    let raw = std::fs::read_to_string(c.node_file("search-ranking-decays-with-age")).unwrap();
    assert!(!raw.contains("edges:"), "suggesting never links:\n{raw}");
    let trace = c
        .run(&["trace", "--json", "search-ranking-decays-with-age"])
        .assert_ok()
        .stdout();
    let walk: serde_json::Value = serde_json::from_str(&trace).unwrap();
    assert_eq!(walk.as_array().unwrap().len(), 1, "{trace}");
    assert!(walk[0]["parents"].as_array().unwrap().is_empty(), "{trace}");

    // --quiet: the id and path alone.
    let entry = c
        .run(&["capture", "-q", "ranking decay, quietly"])
        .assert_ok()
        .stdout_trim();
    let out = c.run(&["promote", "--quiet", &entry]).assert_ok().stdout();
    assert_eq!(out.lines().count(), 1, "{out}");

    // A parent named is a decision made: nothing to suggest.
    let entry = c
        .run(&["capture", "-q", "ranking decay, parented"])
        .assert_ok()
        .stdout_trim();
    let out = c
        .run(&["promote", &entry, "--parent", "ranking-decay-half-life"])
        .assert_ok()
        .stdout();
    assert_eq!(out.lines().count(), 1, "{out}");
}

/// A capture promoted without `--title` or `--id` is titled with the whole
/// sentence but takes a short id. A different thought colliding with it
/// falls back to a longer id from its own text; the same thought again is
/// refused, as it always was.
#[test]
fn promote_without_title_or_id_mints_a_short_id_from_a_long_capture() {
    let c = Corpus::new();
    let text = "gravity might be a scarcity gradient in some shared resource";
    let short = "gravity-scarcity-gradient-shared-resource";
    let full = "gravity-might-be-a-scarcity-gradient-in-some-shared-resource";

    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, short);
    let raw = std::fs::read_to_string(c.node_file(short)).unwrap();
    assert!(raw.contains(&format!("id: {short}\n")), "{raw}");
    assert!(raw.contains(&format!("title: {text}\n")), "{raw}");

    // The same sentence again: already a node, so refused by that node's
    // id rather than duplicated under a fallback, and the capture waits.
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    c.run(&["promote", "-q", &entry])
        .assert_fails()
        .says(&format!("node `{short}` already exists"));
    assert!(c.run(&["inbox"]).assert_ok().stdout().contains(&entry));
    assert!(!c.node_file(full).exists(), "no duplicate node");

    // A different thought sharing the first five significant words falls
    // back to one more word, and the node holding the short id is untouched.
    let entry = c
        .run(&["capture", "-q", &format!("{text} pool")])
        .assert_ok()
        .stdout_trim();
    let id = c.run(&["promote", "-q", &entry]).assert_ok().stdout_trim();
    assert_eq!(id, format!("{short}-pool"));
    assert_eq!(
        std::fs::read_to_string(c.node_file(short)).unwrap(),
        raw,
        "the node holding the short id is untouched"
    );

    // `--title` and `--id` decide the id exactly as they always have.
    let entry = c
        .run(&["capture", "-q", "a thought"])
        .assert_ok()
        .stdout_trim();
    let id = c
        .run(&[
            "promote",
            "-q",
            &entry,
            "--title",
            "Gravity might be a scarcity gradient in a shared pool",
        ])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "gravity-might-be-a-scarcity-gradient-in-a-shared-pool");
    let entry = c.run(&["capture", "-q", text]).assert_ok().stdout_trim();
    let id = c
        .run(&["promote", "-q", &entry, "--id", "scarcity-gravity"])
        .assert_ok()
        .stdout_trim();
    assert_eq!(id, "scarcity-gravity");
}

#[test]
fn promote_json_is_the_created_node_with_its_neighbours() {
    let c = Corpus::new();
    lexical_fixture(&c);
    let entry = c
        .run(&["capture", "-q", "decay of a taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&["promote", "--json", &entry, "--title", "Taxonomies decay"])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["doc"]["node"]["id"], "taxonomies-decay", "{json}");
    assert_eq!(out["doc"]["body"], "decay of a taxonomy", "{json}");
    assert!(
        out["path"]
            .as_str()
            .unwrap()
            .ends_with("taxonomies-decay.md"),
        "{json}"
    );
    let near = out["near"].as_array().unwrap();
    assert_eq!(near[0]["id"], "a-single-global-taxonomy", "{json}");
    assert!(
        near.iter().all(|n| n["id"] != "taxonomies-decay"),
        "a promotion is not its own neighbour: {json}"
    );
    assert_eq!(out["doc"]["node"]["edges"], serde_json::json!([]), "{json}");

    // With a parent, `near` is empty: present, never omitted.
    let entry = c
        .run(&["capture", "-q", "another taxonomy"])
        .assert_ok()
        .stdout_trim();
    let json = c
        .run(&[
            "promote",
            "--json",
            &entry,
            "--parent",
            "a-single-global-taxonomy",
        ])
        .assert_ok()
        .stdout();
    let out: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(out["near"], serde_json::json!([]), "{json}");
}
