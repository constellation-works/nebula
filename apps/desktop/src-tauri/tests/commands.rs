//! The IPC contract where the webview meets it: the app's own command table,
//! called through Tauri's mock runtime with the same request the webview
//! sends, answered with the JSON the webview receives (STD-04 §R1). A test of
//! `session` alone would pass if the translator mapped `locked` to anything
//! else; these would not.

use nebula_core::{Corpus, Error};
use nebula_desktop::error::IpcError;
use nebula_desktop::session;
use nebula_desktop::state::AppState;
use serde_json::{Value, json};
use std::path::PathBuf;
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
    let entry = session::capture(&corpus, "settle me").unwrap();
    (dir, root, entry.id)
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
        message.contains("another nebula writer is holding"),
        "{message}"
    );
    assert_eq!(
        refused.as_object().map(serde_json::Map::len),
        Some(2),
        "exactly `code` and `message`: {refused}"
    );

    holder.release();
    let captured = invoke(&webview, "capture", json!({ "text": "retry me" }))
        .expect("the same capture goes through once the lock is free");
    assert_eq!(captured["text"], "retry me");
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
