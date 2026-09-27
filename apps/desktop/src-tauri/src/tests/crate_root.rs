//! Unit tests for `crate_root`.

use crate::{fail_open, shortcut, shortcut_warning};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// What a test subscriber wrote.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }
}

#[test]
fn side_channel_failures_are_logged_not_propagated() {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();

    let (failed, passed) = tracing::subscriber::with_default(subscriber, || {
        (
            fail_open::<(), _>("hiding the capture window", Err("no such window")),
            fail_open::<u8, &str>("updating the tray count", Ok(7)),
        )
    });

    assert_eq!(failed, None);
    assert_eq!(passed, Some(7));
    let text = log.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "one event, for the failure only: {text}");
    assert!(lines[0].contains("WARN"), "{text}");
    assert!(
        lines[0].contains("hiding the capture window failed: no such window"),
        "{text}"
    );
}

#[test]
fn shortcut_registration_warning_names_shortcut_and_settings_path() {
    let path = Path::new("/user/config/settings.json");
    let refused = shortcut::parse("CmdOrCtrl+Shift+NoSuchKey").unwrap_err();
    let warning = shortcut_warning(path, "CmdOrCtrl+Shift+NoSuchKey", &refused);

    assert!(warning.contains("CmdOrCtrl+Shift+NoSuchKey"));
    assert!(warning.contains("/user/config/settings.json"));
    assert!(warning.contains("invalid accelerator"), "{warning}");
}
