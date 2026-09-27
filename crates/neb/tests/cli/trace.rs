//! `neb trace` and `neb impact` over the lineage DAG: diamonds, edge kinds,
//! depth bounds, and the link refusals that keep the graph acyclic.

use crate::harness::Corpus;
use crate::listing::{assert_envelope, json_of};

#[test]
fn a_node_can_descend_from_two_parents_and_trace_shows_the_diamond() {
    let c = Corpus::new();
    let a = c.seed("gravity might be about scarcity", "Gravity as scarcity");
    let b = c.seed(
        "a moving source should drag the field",
        "Moving source drag",
    );

    c.run(&[
        "new",
        "Retardation in the wake",
        "--parent",
        &a,
        "--parent",
        &b,
    ])
    .assert_ok();

    let tree = c
        .run(&["trace", "retardation-in-the-wake"])
        .assert_ok()
        .stdout();
    assert!(
        tree.contains(&a) && tree.contains(&b),
        "both parents belong in the trace:\n{tree}"
    );
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn a_shared_ancestor_is_reached_by_both_branches_and_expanded_once() {
    let c = Corpus::new();
    let root_id = c.seed("space might have a density of something", "Density hunch");
    c.run(&["new", "Left branch", "--parent", &root_id])
        .assert_ok();
    c.run(&["new", "Right branch", "--parent", &root_id])
        .assert_ok();
    c.run(&[
        "new",
        "Synthesis",
        "--parent",
        "left-branch",
        "--parent",
        "right-branch",
    ])
    .assert_ok();

    // Piped, the walk is one line per node, so the shared ancestor is one
    // line; the tree a terminal gets points to it from both branches
    // (`render::tree`'s unit tests).
    let lines = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        traced_ids(&lines),
        ["synthesis", "left-branch", root_id.as_str(), "right-branch"],
        "the shared ancestor is reached once:\n{lines}"
    );
}

/// The ids of piped `trace` output, one per line, from its second field.
fn traced_ids(lines: &str) -> Vec<&str> {
    lines
        .lines()
        .map(|l| l.split('\t').nth(1).expect("an id field"))
        .collect()
}

#[test]
fn trace_names_edge_kinds_and_draws_parallel_edges_as_one_line() {
    let c = Corpus::new();
    let root = c.seed("gravity might be about scarcity", "Gravity as scarcity");
    c.run(&["new", "Left branch", "--parent", &root])
        .assert_ok();
    c.run(&["new", "Right branch", "--parent", &root])
        .assert_ok();
    c.run(&[
        "new",
        "Synthesis",
        "--parent",
        "left-branch",
        "--parent",
        "right-branch",
    ])
    .assert_ok();
    // A parallel edge: synthesis now both derives from and reopens `left-branch`.
    c.run(&["link", "synthesis", "reopens", "left-branch"])
        .assert_ok();

    // Piped, each line is `depth, id, status, title, kinds`.
    let steps = |lines: &str| -> Vec<(String, String, String)> {
        lines
            .lines()
            .map(|l| {
                let fields: Vec<&str> = l.split('\t').collect();
                assert_eq!(fields.len(), 5, "{l:?}");
                (fields[0].into(), fields[1].into(), fields[4].into())
            })
            .collect()
    };
    let step = |depth: &str, id: &str, kinds: &str| (depth.into(), id.into(), kinds.into());

    let up = c.run(&["trace", "synthesis"]).assert_ok().stdout();
    assert_eq!(
        steps(&up),
        [
            step("0", "synthesis", "-"),
            step("1", "left-branch", "derives-from,reopens"),
            step("2", &root, "derives-from"),
            step("1", "right-branch", "derives-from"),
        ],
        "every step names its edge, and the parallel pair is one step:\n{up}"
    );
    assert!(!up.contains("shown above"), "{up}");

    let down = c.run(&["trace", "--down", &root]).assert_ok().stdout();
    assert_eq!(
        steps(&down),
        [
            step("0", &root, "-"),
            step("1", "left-branch", "derives-from"),
            step("2", "synthesis", "derives-from,reopens"),
            step("1", "right-branch", "derives-from"),
        ],
        "descent names the same edges, and reaches the parallel pair once:\n{down}"
    );

    let json = c
        .run(&["trace", "--json", "synthesis"])
        .assert_ok()
        .stdout();
    let walk: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(walk[0]["via"].is_null(), "the start has no step: {json}");
    assert_eq!(
        walk[0]["parents"],
        serde_json::json!(["left-branch", "right-branch"]),
        "a parent is named once however many edges reach it: {json}"
    );
    assert_eq!(walk[1]["id"], "left-branch");
    assert_eq!(
        walk[1]["via"],
        serde_json::json!({ "from": "synthesis", "kinds": ["derives-from", "reopens"] }),
        "{json}"
    );
    assert_eq!(walk.as_array().unwrap().len(), 4, "{json}");
}

/// `root <- branch <- cut <- deep`, with `cut` also descending from `root`
/// directly: walking down, `cut` is two steps out through `branch` and one
/// step out on its own.
fn shortcut(c: &Corpus) {
    c.run(&["new", "Root"]).assert_ok();
    c.run(&["new", "Branch", "--parent", "root"]).assert_ok();
    c.run(&["new", "Cut", "--parent", "branch", "--parent", "root"])
        .assert_ok();
    c.run(&["new", "Deep", "--parent", "cut"]).assert_ok();
}

/// `--depth` bounds the walk and the JSON alike, stderr says how many nodes
/// it left out, and a node within reach along any path is kept.
#[test]
fn trace_depth_bounds_the_tree_and_the_json_alike() {
    let c = Corpus::new();
    shortcut(&c);
    let walked = |args: &[&str]| -> Vec<String> {
        let json: serde_json::Value =
            serde_json::from_str(&c.run(args).assert_ok().stdout()).unwrap();
        assert_eq!(json["total"], 4, "the whole walk: {json}");
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap().to_string())
            .collect()
    };

    let one = c
        .run(&["trace", "root", "--down", "--depth", "1"])
        .assert_ok();
    assert_eq!(traced_ids(&one.stdout()), ["root", "branch", "cut"]);
    assert_eq!(
        one.stderr(),
        "1 more node beyond --depth 1; raise --depth for more\n"
    );
    assert_eq!(
        walked(&["trace", "--json", "root", "--down", "--depth", "1"]),
        ["root", "branch", "cut"]
    );

    // Through `branch`, `deep` is three steps out; through the shortcut, two.
    let two = c
        .run(&["trace", "root", "--down", "--depth", "2"])
        .assert_ok();
    assert!(
        traced_ids(&two.stdout()).contains(&"deep"),
        "{}",
        two.stdout()
    );
    assert_eq!(two.stderr(), "", "nothing was left out");
    assert_eq!(
        walked(&["trace", "--json", "root", "--down", "--depth", "2"]).len(),
        4
    );

    for depth in ["1", "2", "3"] {
        let drawn = c
            .run(&["trace", "root", "--down", "--depth", depth])
            .assert_ok()
            .stdout();
        let listed = walked(&["trace", "--json", "root", "--down", "--depth", depth]);
        assert_eq!(
            traced_ids(&drawn),
            listed,
            "--depth {depth}: the lines and the JSON disagree:\n{drawn}"
        );
        assert!(!drawn.contains("--depth"), "{drawn}");
    }

    let up = c.run(&["trace", "deep", "--depth", "1"]).assert_ok();
    assert_eq!(traced_ids(&up.stdout()), ["deep", "cut"]);
    assert_eq!(
        up.stderr(),
        "2 more nodes beyond --depth 1; raise --depth for more\n"
    );
}

/// `--depth 0` would print the node alone, although the help says 1 is its
/// parents: it is a usage error, and the least depth still walks.
#[test]
fn trace_depth_zero_is_a_usage_error() {
    let c = Corpus::new();
    shortcut(&c);
    for json in [false, true] {
        let json = if json { ["--json"].as_slice() } else { &[] };
        let zero = c.run(&[json, &["trace", "root", "--depth", "0"]].concat());
        assert_eq!(zero.out.status.code(), Some(2), "{}", zero.stderr());
        assert_eq!(zero.stdout(), "");
        assert!(
            zero.stderr().contains("--depth") && zero.stderr().contains("at least 1"),
            "{}",
            zero.stderr()
        );
    }
    let one = c
        .run(&["trace", "root", "--down", "--depth", "1"])
        .assert_ok();
    assert_eq!(traced_ids(&one.stdout()), ["root", "branch", "cut"]);
}

/// Without `--depth` the walk is whole, and a bound the corpus never reaches
/// prints exactly the same lines and no notice.
#[test]
fn trace_without_depth_is_unchanged_and_an_unreached_depth_matches_it() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [["trace", "root", "--down"].as_slice(), &["trace", "deep"]] {
        let whole = c.run(args).assert_ok();
        assert_eq!(whole.stderr(), "");
        let ids = traced_ids(&whole.stdout()).join(" ");
        assert!(ids.contains("root") && ids.contains("deep"), "{ids}");
        let bounded = c.run(&[args, &["--depth", "10"]].concat()).assert_ok();
        assert_eq!(bounded.stdout(), whole.stdout());
        assert_eq!(bounded.stderr(), "");
    }
}

/// Piped, `trace` draws no tree: one line per node with the same fields on
/// every line, and none of the box-drawing glyphs (STD-01 §R9).
#[test]
fn piped_trace_has_no_box_glyphs() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [
        ["trace", "deep"].as_slice(),
        &["trace", "root", "--down"],
        &["trace", "root", "--down", "--depth", "1"],
    ] {
        let out = c.run(args).assert_ok().stdout();
        assert!(!out.contains(['└', '├', '│', '─']), "{out}");
        assert!(!out.contains('\x1b'), "{out}");
        let fields: Vec<usize> = out.lines().map(|l| l.split('\t').count()).collect();
        assert!(!fields.is_empty(), "{out}");
        assert!(fields.iter().all(|n| *n == 5), "{out}");
    }
}

/// The lines `trace` prints are the payload `--json` returns: the same
/// nodes, whole or bounded (STD-01 §R6).
#[test]
fn trace_human_matches_payload() {
    let c = Corpus::new();
    shortcut(&c);
    for args in [
        ["trace", "deep"].as_slice(),
        &["trace", "root", "--down"],
        &["trace", "root", "--down", "--depth", "1"],
        &["trace", "cut", "--down", "--depth", "1"],
    ] {
        let printed = c.run(args).assert_ok().stdout();
        let json: serde_json::Value = serde_json::from_str(
            &c.run(&[["--json"].as_slice(), args].concat())
                .assert_ok()
                .stdout(),
        )
        .unwrap();
        // Bounded, the walk is the capped-list envelope (STD-01 §R34).
        let walk = if json.is_object() {
            &json["items"]
        } else {
            &json
        };
        let mut from_payload: Vec<&str> = walk
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        let mut from_lines = traced_ids(&printed);
        from_payload.sort_unstable();
        from_lines.sort_unstable();
        assert_eq!(from_lines, from_payload, "{args:?}:\n{printed}");
    }
}

/// `trace --depth` answers in the envelope, with the size of the whole walk
/// as `total`; without the flag it is the bare array.
#[test]
fn trace_depth_json_envelope() {
    let c = Corpus::new();
    shortcut(&c);
    let cut = json_of(&c, &["trace", "--json", "root", "--down", "--depth", "1"]);
    assert_envelope(&cut, 3, 4, true);
    assert!(cut["items"][0]["handed_off_to"].is_null(), "{cut}");
    assert_envelope(
        &json_of(&c, &["trace", "--json", "root", "--down", "--depth", "10"]),
        4,
        4,
        false,
    );
    assert_envelope(
        &json_of(&c, &["trace", "--json", "deep", "--depth", "1"]),
        2,
        4,
        true,
    );
    let bare = json_of(&c, &["trace", "--json", "root", "--down"]);
    assert_eq!(bare.as_array().map(Vec::len), Some(4), "{bare}");
}

#[test]
fn genealogy_cycles_are_refused_at_the_point_of_linking() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    c.run(&["new", "Second", "--parent", &a]).assert_ok();
    c.run(&["link", &a, "derives-from", "second"])
        .assert_fails()
        .says("its own ancestor");
    c.run(&["check"]).assert_ok().says("0 errors");
}

/// A repeated edge is refused by name, both ends and the kind, so the
/// refusal says which edge; from `link` it points at the node that has it.
#[test]
fn duplicate_edge_refusal_names_the_edge() {
    let c = Corpus::new();
    let a = c.seed("alpha", "Alpha");
    let b = c.seed("beta", "Beta");
    c.run(&["link", &b, "derives-from", &a]).assert_ok();
    let run = c.run(&["link", &b, "derives-from", &a]);
    assert_eq!(run.out.status.code(), Some(1));
    assert_eq!(
        run.stderr(),
        format!(
            "error: the edge `{b}` derives-from `{a}` already exists\n\n\
             See its edges with:  neb show {b}\n"
        )
    );
    let refused = c.run(&["--json", "link", &b, "derives-from", &a]).refusal();
    assert_eq!(refused["code"], "duplicate_edge");
    assert_eq!(
        refused["hint"],
        format!("See its edges with:  neb show {b}")
    );

    // From `new` the repeat is a flag given twice, and nothing is written.
    let run = c.run(&["new", "X", "--parent", &a, "--parent", &a]);
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.starts_with(&format!(
            "error: the edge `x` derives-from `{a}` already exists\n"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("`--parent {a}` is given twice")),
        "{stderr}"
    );
    assert!(!c.node_file("x").exists());
}

#[test]
fn contradicts_is_recorded_on_both_nodes() {
    let c = Corpus::new();
    let a = c.seed("first", "First");
    let b = c.seed("second", "Second");
    c.run(&["link", &a, "contradicts", &b]).assert_ok();
    c.run(&["show", &b])
        .assert_ok()
        .says("contradicts")
        .says(&a);
    c.run(&["check"]).assert_ok().says("0 errors");
}

#[test]
fn impact_reports_descendants_and_contradictions() {
    let c = Corpus::new();
    let base = c.seed("foundation", "Foundation");
    c.run(&["new", "Child", "--parent", &base]).assert_ok();
    c.run(&["new", "Grandchild", "--parent", "child"])
        .assert_ok();
    let rival = c.seed("the other way round", "Rival");
    c.run(&["link", &base, "contradicts", &rival]).assert_ok();
    let unrelated = c.seed("nothing to do with it", "Unrelated");

    let out = c.run(&["impact", &base]).assert_ok().stdout();
    assert!(out.contains("child") && out.contains("grandchild"), "{out}");
    assert!(out.contains(&rival), "{out}");
    assert!(!out.contains(&unrelated), "{out}");

    let json = c.run(&["--json", "impact", &base]).assert_ok().stdout();
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["id"] == "grandchild" && i["via"] == "descends"),
        "{json}"
    );
    assert!(
        items
            .iter()
            .any(|i| i["id"] == rival && i["via"] == "contradicts"),
        "{json}"
    );
    c.run(&["impact", &unrelated])
        .assert_ok()
        .says("nothing descends from or contradicts");
}
