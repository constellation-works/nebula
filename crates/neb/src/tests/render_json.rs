//! Unit tests for `render_json`.

use nebula_core::{Closed, EdgeType, InboxEntry, Neighbour, Note, Status, TraceHop};
use serde::Serialize;
use std::path::PathBuf;

use crate::render::json::*;

/// A node with nothing optional set: the case where core's own
/// serialisation leaves out the most.
fn bare() -> nebula_core::Node {
    serde_json::from_value(serde_json::json!({
        "id": "a-node",
        "title": "A node",
        "status": "seed",
        "created": "2026-09-01",
        "updated": "2026-09-01",
    }))
    .expect("a minimal node")
}

fn keys(v: &serde_json::Value) -> Vec<&str> {
    v.as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn an_absent_field_is_null_and_an_empty_one_is_an_empty_list() {
    let v = serde_json::to_value(Node::from(&bare())).unwrap();
    assert_eq!(
        keys(&v),
        [
            "closed",
            "created",
            "edges",
            "id",
            "kill",
            "kill_by",
            "origin",
            "references",
            "status",
            "tags",
            "title",
            "title_by",
            "updated",
        ]
    );
    assert_eq!(v["title_by"], "human");
    assert!(v["kill"].is_null() && v["kill_by"].is_null(), "{v}");
    assert!(v["closed"].is_null() && v["origin"].is_null(), "{v}");
    for list in ["tags", "edges", "references"] {
        assert_eq!(v[list], serde_json::json!([]), "{v}");
    }
}

#[test]
fn nested_optionals_are_null_and_authors_are_stated() {
    let mut node = bare();
    node.edges.push(nebula_core::Edge {
        kind: EdgeType::DerivesFrom,
        to: "parent".into(),
        by: None,
    });
    node.origin = Some(nebula_core::Origin {
        task: Some("ORB-1".into()),
        ..nebula_core::Origin::default()
    });
    let v = serde_json::to_value(Node::from(&node)).unwrap();
    assert_eq!(
        v["edges"],
        serde_json::json!([{"type": "derives-from", "to": "parent", "by": "human"}])
    );
    assert_eq!(
        v["origin"],
        serde_json::json!({
            "task": "ORB-1",
            "workspace": null,
            "run": null,
            "artifact": null,
            "agent": null,
            "at": null,
        })
    );
}

#[test]
fn a_list_is_bare_unless_bounded_and_the_envelope_carries_the_cut() {
    let bare = serde_json::to_value(List::new(vec![1, 2], None)).unwrap();
    assert_eq!(bare, serde_json::json!([1, 2]));
    let cut = serde_json::to_value(List::new(vec![1, 2], Some((5, true)))).unwrap();
    assert_eq!(
        cut,
        serde_json::json!({"items": [1, 2], "total": 5, "truncated": true})
    );
    let whole = serde_json::to_value(List::new(vec![1, 2], Some((2, false)))).unwrap();
    assert_eq!(
        whole,
        serde_json::json!({"items": [1, 2], "total": 2, "truncated": false})
    );
}
