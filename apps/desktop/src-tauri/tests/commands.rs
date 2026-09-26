//! The IPC contract where the webview meets it: the app's own command table,
//! called through Tauri's mock runtime with the same request the webview
//! sends, answered with the JSON the webview receives (STD-04 §R1). A test of
//! `session` alone would pass if the translator mapped `locked` to anything
//! else; these would not.

use nebula_core::{Committed, Corpus, Error, ops};
use nebula_desktop::error::IpcError;
use nebula_desktop::session::{self, CommitReport, Written};
use nebula_desktop::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

// This process runs with a temporary `HOME` and git environment, set before
// any test thread starts, and every child comes from its builder.
#[path = "../../../../crates/nebula-core/tests/support/mod.rs"]
mod support;

mod lock_holder;

use lock_holder::LockHolder;

/// The app as `run` builds it, minus the window, tray and plugins: the same
/// command table over `state`.
fn app(state: AppState) -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .manage(state)
        .invoke_handler(nebula_desktop::handler())
        .build(mock_context(noop_assets()))
        .expect("building the mock app");
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("building the mock webview");
    (app, webview)
}

/// Call `cmd` with `args` as `invoke(cmd, args)` in the webview does, and
/// return what the promise resolves or rejects with.
fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    let url = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: url.parse().expect("the webview's origin"),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: tauri::http::HeaderMap::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().expect("a JSON response"))
}

/// A fresh corpus with one waiting capture; the directory, the root and the
/// entry's id.
fn corpus_with_one_capture() -> (tempfile::TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    let corpus = Corpus::init(&root).unwrap();
    let entry = session::capture(&corpus, "settle me").unwrap().value;
    (dir, root, entry.id)
}

/// Run git in `dir`, asserting it succeeded.
fn git(dir: &Path, args: &[&str]) {
    let output = support::output(
        support::git_command(dir, support::home()).args(args),
        support::DEADLINE,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A corpus with commits on, inside a repository whose `.gitignore` ignores
/// it: every write lands and every commit is refused as `corpus_ignored`.
/// The directory and the root.
fn corpus_the_repository_ignores() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let root = outer.join("corpus");
    let mut corpus = Corpus::init(&root).unwrap();
    git(&outer, &["init", "-q"]);
    nebula_core::fs::write_private_atomic(&outer.join(".gitignore"), "corpus/\n").unwrap();
    ops::set_commit(&mut corpus, true).unwrap();
    (dir, root)
}

/// Lines across every inbox file under `root`.
fn inbox_lines(root: &Path) -> usize {
    std::fs::read_dir(root.join("inbox"))
        .unwrap()
        .map(|file| std::fs::read_to_string(file.unwrap().path()).unwrap())
        .map(|text| text.lines().count())
        .sum()
}

/// The inbox as the webview reads it.
fn inbox(webview: &WebviewWindow<MockRuntime>) -> Value {
    invoke(webview, "inbox", json!({})).expect("inbox reads while a writer holds the lock")
}

#[test]
fn capture_command_reports_busy_while_another_process_holds_the_lock() {
    let (_dir, root, _) = corpus_with_one_capture();
    let (_app, webview) = app(AppState::with_root(Ok(root.clone())));
    let holder = LockHolder::start(&root);

    let start = Instant::now();
    let refused = invoke(&webview, "capture", json!({ "text": "retry me" }))
        .expect_err("capture must refuse while the lock is held");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "capture waited {:?}, not the desktop's short lock wait",
        start.elapsed()
    );
    assert_eq!(refused["code"], "locked", "{refused}");
    let message = refused["message"].as_str().expect("a message");
    assert!(
        message.contains("another nebula writer is holding")
            && message.contains(&format!(
                "`{}`, pid {}, since ",
                lock_holder::LABEL,
                holder.pid()
            )),
        "the message names the holder: {message}"
    );
    assert_eq!(
        refused.as_object().map(serde_json::Map::len),
        Some(2),
        "exactly `code` and `message`: {refused}"
    );

    holder.release();
    let captured = invoke(&webview, "capture", json!({ "text": "retry me" }))
        .expect("the same capture goes through once the lock is free");
    assert_eq!(captured["value"]["text"], "retry me");
    assert_eq!(captured["commit"], json!({ "status": "disabled" }));
}

#[test]
fn a_capture_whose_commit_is_refused_returns_the_entry_and_the_refusal() {
    let (_dir, root) = corpus_the_repository_ignores();
    let (_app, webview) = app(AppState::with_root(Ok(root.clone())));

    let captured = invoke(&webview, "capture", json!({ "text": "landed once" }))
        .expect("the line is written, so the capture succeeds");

    assert_eq!(captured["value"]["text"], "landed once", "{captured}");
    let commit = &captured["commit"];
    assert_eq!(commit["status"], "refused", "{captured}");
    assert_eq!(commit["error"]["code"], "corpus_ignored", "{captured}");
    assert!(
        commit["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("nothing can be committed")),
        "{captured}"
    );
    assert_eq!(inbox_lines(&root), 1);
    let listed = inbox(&webview);
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!(listed[0]["id"], captured["value"]["id"]);
}

#[test]
fn a_settle_whose_commit_is_refused_reports_the_settlement() {
    let (_dir, root) = corpus_the_repository_ignores();
    let (_app, webview) = app(AppState::with_root(Ok(root.clone())));
    let corpus = session::open(&root).unwrap();
    let to_drop = session::capture(&corpus, "drop me").unwrap().value;
    let to_promote = session::capture(&corpus, "promote me").unwrap().value;

    let dropped = invoke(&webview, "drop_entry", json!({ "entry": to_drop.id }))
        .expect("the entry is dropped, so the command succeeds");
    assert_eq!(dropped["value"]["id"], to_drop.id.as_str(), "{dropped}");
    assert_eq!(dropped["commit"]["status"], "refused", "{dropped}");
    assert_eq!(dropped["commit"]["error"]["code"], "corpus_ignored");

    let promoted = invoke(&webview, "promote_root", json!({ "entry": to_promote.id }))
        .expect("the node is written, so the command succeeds");
    let node = promoted["value"]["doc"]["node"]["id"]
        .as_str()
        .expect("the created node's id");
    assert!(root.join("nodes").join(format!("{node}.md")).is_file());
    assert_eq!(promoted["commit"]["status"], "refused", "{promoted}");
    assert_eq!(promoted["commit"]["error"]["code"], "corpus_ignored");

    assert_eq!(inbox(&webview), json!([]), "both entries are settled");
}

#[test]
fn commit_report_serializes_to_the_shape_api_ts_declares() {
    let committed = CommitReport::Committed {
        commit: Committed {
            hash: "0123abcd".into(),
            message: "neb capture a1b2".into(),
        },
    };
    let refused = CommitReport::Refused {
        error: Error::CorpusIgnored(PathBuf::from("/corpus")).into(),
    };
    let cases = [
        (
            committed,
            json!({
                "status": "committed",
                "commit": { "hash": "0123abcd", "message": "neb capture a1b2" },
            }),
        ),
        // What GIT2's `CommitOutcome` tells apart, `skipped` before it.
        (CommitReport::Disabled, json!({ "status": "disabled" })),
        (
            CommitReport::NotARepository,
            json!({ "status": "not_a_repository" }),
        ),
        (
            CommitReport::NothingToCommit,
            json!({ "status": "nothing_to_commit" }),
        ),
        (
            refused,
            json!({
                "status": "refused",
                "error": {
                    "code": "corpus_ignored",
                    "message": "/corpus is ignored by the git repository that contains it; nothing can be committed",
                },
            }),
        ),
    ];
    for (report, expected) in cases {
        assert_eq!(serde_json::to_value(&report).unwrap(), expected);
    }
    let written = Written {
        value: 7,
        commit: CommitReport::Disabled,
    };
    assert_eq!(
        serde_json::to_value(&written).unwrap(),
        json!({ "value": 7, "commit": { "status": "disabled" } })
    );

    // The hand-written TypeScript declares exactly these, so a change on
    // either side fails here rather than in the webview.
    let api = include_str!("../../src/api.ts");
    for declared in [
        r#"| { status: "committed"; commit: Committed }"#,
        r#"| { status: "disabled" }"#,
        r#"| { status: "not_a_repository" }"#,
        r#"| { status: "nothing_to_commit" }"#,
        r#"| { status: "refused"; error: IpcError };"#,
        "export interface Written<T> {\n  value: T;\n  commit: CommitReport;\n}",
    ] {
        assert!(
            api.contains(declared),
            "api.ts no longer declares {declared}"
        );
    }
}

#[test]
fn settle_commands_report_busy_with_the_same_code() {
    let (_dir, root, id) = corpus_with_one_capture();
    let (_app, webview) = app(AppState::with_root(Ok(root.clone())));
    let before = inbox(&webview);
    let holder = LockHolder::start(&root);

    for cmd in ["drop_entry", "promote_root"] {
        let start = Instant::now();
        let refused = invoke(&webview, cmd, json!({ "entry": id }))
            .expect_err("a settle must refuse while the lock is held");
        assert!(start.elapsed() < Duration::from_secs(1), "{cmd} waited");
        assert_eq!(refused["code"], "locked", "{cmd}: {refused}");
        assert_eq!(inbox(&webview), before, "{cmd} changed the inbox");
    }

    holder.release();
    invoke(&webview, "drop_entry", json!({ "entry": id })).expect("drops once free");
    assert_eq!(inbox(&webview), json!([]));
}

#[test]
fn an_unresolvable_root_is_reported_not_replaced_with_a_literal_home_path() {
    // What resolution fails with: an empty root, and a root setting that
    // cannot be read.
    let failures = [
        Corpus::resolve_root(Some(PathBuf::new())).unwrap_err(),
        Error::IoAt {
            action: "read",
            path: PathBuf::from("/home/someone/.config/nebula/root"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        },
    ];
    for failure in failures {
        let code = failure.code();
        let cause = failure.to_string();
        let (_app, webview) = app(AppState::with_root(Err(failure)));

        let warnings = invoke(&webview, "startup_warnings", json!({})).unwrap();
        let warnings = warnings.as_array().expect("a list of warnings");
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().is_some_and(|w| w.contains(&cause))),
            "no startup warning carries `{cause}`: {warnings:?}"
        );
        let path = invoke(&webview, "corpus_path", json!({})).unwrap();
        assert_eq!(path, Value::Null, "the root is unknown, not a path");
        let refused = invoke(&webview, "inbox", json!({})).expect_err("no corpus to read");
        assert_eq!(refused["code"], code, "{refused}");
        assert!(
            refused["message"]
                .as_str()
                .is_some_and(|m| m.contains(&cause)),
            "{refused}"
        );
        let reload = invoke(&webview, "reload", json!({})).expect_err("nothing to reload");
        assert_eq!(reload["code"], code, "{reload}");

        let everything = format!("{warnings:?} {path} {refused} {reload}");
        assert!(!everything.contains("~/.nebula"), "{everything}");
    }
}

#[test]
fn every_core_error_translates_to_its_own_code() {
    let samples = [
        Error::Locked {
            root: PathBuf::from("/corpus"),
            holder: None,
        },
        Error::NoCorpus(PathBuf::from("/nowhere")),
        Error::InboxEntrySettled {
            id: "a1b2".into(),
            settlement: nebula_core::Settlement::Dropped,
        },
        Error::IoAt {
            action: "write",
            path: PathBuf::from("/corpus/inbox/2026-09.md"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        },
    ];
    for e in samples {
        let (code, message) = (e.code(), e.to_string());
        let translated = IpcError::from(e);
        assert_eq!(translated.code, code);
        assert_eq!(translated.message, message);
        assert_eq!(
            serde_json::to_value(&translated).unwrap(),
            json!({ "code": code, "message": message })
        );
    }
}

/// The desktop's node is core's `NodeView` as serde writes it: an absent
/// value is left out, where `neb show --json` states every key
/// (`session::node` says so, and `NodeView.ts` marks the fields optional).
#[test]
fn the_node_command_leaves_absent_fields_out() {
    let (_dir, root, id) = corpus_with_one_capture();
    let corpus = session::open(&root).unwrap();
    let created = session::promote_root(&corpus, &id).unwrap().value;
    let (_app, webview) = app(AppState::with_root(Ok(root)));

    let view = invoke(&webview, "node", json!({ "id": created.doc.node.id })).unwrap();

    let node = view["node"].as_object().expect("the node's fields");
    assert_eq!(node["title"], "settle me", "{view}");
    for absent in ["kill", "kill_by", "closed", "origin"] {
        assert!(!node.contains_key(absent), "`{absent}` is present: {view}");
    }
    let view = view.as_object().expect("the view");
    for absent in ["notes", "observatory", "handed_off_to"] {
        assert!(
            !view.contains_key(absent),
            "`{absent}` is present: {view:?}"
        );
    }
}

/// Every child this suite starts comes from the isolating builder in
/// `support`.
#[test]
fn every_child_command_comes_from_the_isolating_builder() {
    for (file, source) in [
        ("commands.rs", include_str!("commands.rs")),
        ("lock_holder/mod.rs", include_str!("lock_holder/mod.rs")),
    ] {
        let strays = support::commands_outside(source, &[]);
        assert!(
            strays.is_empty(),
            "{file} creates a child outside `support::command`:\n{}",
            strays.join("\n")
        );
    }
}
