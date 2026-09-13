//! Tell the windows when the corpus changes underneath them.
//!
//! The CLI and the agent write to the same files this app reads, so a
//! `notify` watcher sits on `nodes/` and `inbox/` and the frontend refetches
//! on `corpus-changed`. Editors and `neb` alike produce a burst of events per
//! save (create, write, rename), so bursts are collapsed: one event, once the
//! directory has been quiet for [`DEBOUNCE`].

use crate::tray;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// How long the corpus has to be quiet before a burst counts as one change.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// The event every window listens for. Carries no payload: the frontend
/// refetches what it shows rather than reasoning about which file moved.
pub const EVENT: &str = "corpus-changed";

/// Watch `root/nodes` and `root/inbox`. The returned watcher stops when
/// dropped, so the caller keeps it for the life of the app.
pub fn start(app: AppHandle, root: &Path) -> notify::Result<RecommendedWatcher> {
    let (tx, rx) = channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            // A closed receiver means the app is gone; nothing to do.
            let _ = tx.send(());
        }
    })?;
    // `Corpus::open` requires `nodes/`; `inbox/` appears on first capture, and
    // a watch on a missing directory fails, so make it exist. This is what
    // `Corpus::init` and `capture` both do.
    let inbox = root.join("inbox");
    std::fs::create_dir_all(&inbox)?;
    watcher.watch(&root.join("nodes"), RecursiveMode::Recursive)?;
    watcher.watch(&inbox, RecursiveMode::Recursive)?;

    std::thread::Builder::new()
        .name("corpus-watcher".into())
        .spawn(move || {
            for () in debounced(&rx, DEBOUNCE) {
                let _ = app.emit(EVENT, ());
                tray::refresh(&app);
            }
        })?;
    Ok(watcher)
}

/// Collapse bursts: yield once per run of events, after `window` of quiet.
/// Ends when every sender is gone, flushing a burst in progress first.
pub fn debounced(rx: &Receiver<()>, window: Duration) -> impl Iterator<Item = ()> + '_ {
    std::iter::from_fn(move || {
        rx.recv().ok()?;
        loop {
            match rx.recv_timeout(window) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    return Some(());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    const WINDOW: Duration = Duration::from_millis(40);

    #[test]
    fn a_burst_is_one_change() {
        let (tx, rx) = channel();
        for _ in 0..5 {
            tx.send(()).unwrap();
        }
        drop(tx);
        assert_eq!(debounced(&rx, WINDOW).count(), 1);
    }

    #[test]
    fn quiet_between_bursts_separates_them() {
        let (tx, rx) = channel();
        let producer = std::thread::spawn(move || {
            tx.send(()).unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(WINDOW * 3);
            tx.send(()).unwrap();
        });
        assert_eq!(debounced(&rx, WINDOW).count(), 2);
        producer.join().unwrap();
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        let (tx, rx) = channel::<()>();
        drop(tx);
        assert_eq!(debounced(&rx, WINDOW).count(), 0);
    }
}
