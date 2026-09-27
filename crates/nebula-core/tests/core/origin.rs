//! Where a write may come from: the read-only environment refuses every write
//! op before it writes, and an Orbit origin must be valid to be recorded.

use crate::harness::{corpus, process_locations, seed};
use nebula_core::{
    Citation, Corpus, EdgeType, Error, Handoff, Locations, NewNode, Promotion, Status, ops,
};

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "enumerates every direct core write path"
)]
fn read_only_environment_refuses_every_write_op_before_any_write() {
    let (_dir, writable) = corpus();
    let node = seed(&writable, "First idea", &[]);
    let other = seed(&writable, "Second idea", &[]);
    let inbox = ops::capture(&writable, "Captured thought").unwrap();
    let root = writable.root().to_path_buf();
    let read_only = Locations {
        nebula_read_only: Some("1".into()),
        ..process_locations()
    };
    let mut c = Corpus::open(&read_only, Some(root.clone())).unwrap();
    let before_node = std::fs::read(c.node_path(&node).unwrap()).unwrap();
    let before_inbox = std::fs::read(&inbox.file).unwrap();
    let before_config = std::fs::read(root.join("config.yaml")).unwrap();
    let setting = std::path::PathBuf::from(read_only.home.clone().expect("home"))
        .join(".config")
        .join("nebula")
        .join("observatory-root");
    let before_setting = std::fs::read(&setting).ok();
    let before_entries = std::fs::read_dir(root.join("nodes")).unwrap().count();
    macro_rules! read_only {
        ($name:expr, $op:expr) => {
            assert!(matches!($op, Err(Error::ReadOnly)), "{}", $name);
        };
    }
    read_only!(
        "init",
        ops::init(&read_only, Some(root.join("new")), None, false, false)
    );
    read_only!(
        "migrate",
        nebula_core::migrate::run(&read_only, Some(root.clone()))
    );
    read_only!("capture", ops::capture(&c, "more thoughts"));
    read_only!("drop", ops::drop(&c, &inbox.id));
    read_only!(
        "promote",
        ops::promote(&c, &inbox.id, &Promotion::default(), 0)
    );
    read_only!(
        "new",
        ops::new_node(
            &c,
            &NewNode {
                title: "Third idea".into(),
                ..NewNode::default()
            }
        )
    );
    read_only!("edit", ops::set_body(&c, &node, "edited"));
    read_only!("edit-if", ops::set_body_if(&c, &node, "", "edited"));
    read_only!("sharpen", ops::sharpen(&c, &node, "if false", None));
    read_only!("confirm", ops::confirm_kill(&c, &node));
    read_only!(
        "status",
        ops::set_status(&c, &node, Status::Abandoned, Some("done"))
    );
    read_only!(
        "link",
        ops::link(&c, &node, EdgeType::Refines, &other, None)
    );
    read_only!("note", ops::note(&c, &node, "more words", None));
    read_only!(
        "cite",
        ops::cite(
            &c,
            &node,
            &Citation {
                uri: Some("https://example.org".into()),
                kind: "other".into(),
                ..Citation::default()
            }
        )
    );
    read_only!(
        "handoff",
        ops::handoff(
            &c,
            &node,
            &Handoff {
                record: "H012".into(),
                ..Handoff::default()
            },
            None
        )
    );
    read_only!("tag", ops::retag(&c, &node, &["alpha".into()], &[]));
    read_only!("config commit", ops::set_commit(&mut c, true));
    read_only!("config observatory", ops::set_observatory_root(&c, &root));
    read_only!("config legacy", ops::drop_legacy_observatory_root(&mut c));
    read_only!("commit", ops::commit(&c, "test", &[]));
    assert_eq!(
        std::fs::read(c.node_path(&node).unwrap()).unwrap(),
        before_node
    );
    assert_eq!(std::fs::read(&inbox.file).unwrap(), before_inbox);
    assert_eq!(
        std::fs::read(root.join("config.yaml")).unwrap(),
        before_config
    );
    assert_eq!(std::fs::read(&setting).ok(), before_setting);
    assert_eq!(
        std::fs::read_dir(root.join("nodes")).unwrap().count(),
        before_entries
    );
    assert!(!root.join("new").exists());
}

#[cfg(unix)]
#[test]
fn invalid_orbit_origin_is_refused_without_lossy_provenance() {
    use std::os::unix::ffi::OsStringExt;
    let locations = Locations {
        orbit_run_id: Some(std::ffi::OsString::from_vec(vec![0xff])),
        ..Locations::default()
    };
    assert!(matches!(
        locations.origin(None, None),
        Err(Error::InvalidOriginEnvironment { .. })
    ));
    let explicit = locations
        .origin(None, Some("jrun-explicit".into()))
        .unwrap()
        .unwrap();
    assert_eq!(explicit.run.as_deref(), Some("jrun-explicit"));
}
