//! Tell the windows when the corpus changes underneath them.
//!
//! The CLI and the agent write to the same files this app reads, so a
//! `notify` watcher sits on `nodes/` and `inbox/` and the frontend refetches
//! on `corpus-changed`. Editors and `neb` alike produce a burst of events per
//! save (create, write, rename), so bursts are collapsed: one event after the
//! directory has been quiet for [`DEBOUNCE`], capped at [`MAX_LATENCY`].

use crate::{fail_open, tray};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};

/// How long the corpus has to be quiet before a burst counts as one change.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(300);

/// Longest time a continuous run of events can wait before a refresh.
pub(crate) const MAX_LATENCY: Duration = Duration::from_secs(1);

/// The event every window listens for. Carries no payload: the frontend
/// refetches what it shows rather than reasoning about which file moved.
pub(crate) const EVENT: &str = "corpus-changed";

/// Watch `root/nodes` and `root/inbox`. The returned watcher stops when
/// dropped, so the caller keeps it for the life of the app.
pub(crate) fn start<R: Runtime>(
    app: AppHandle<R>,
    root: &Path,
) -> notify::Result<RecommendedWatcher> {
    // Bounded at one (STD-03 §R2). When the queue is full, `signal` drops the
    // new signal: a signal carries no payload, so a second says nothing the
    // first does not, and the one already waiting guarantees a refresh that
    // reads the corpus after this event. A burst costs one slot, however
    // slow the consumer.
    let (tx, rx) = sync_channel::<()>(1);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        // Fail open: an error from the OS watcher is logged; the next event
        // that does arrive refreshes everything anyway.
        let Some(event) = fail_open("watching the corpus", res) else {
            return;
        };
        if is_change_event(&event) {
            signal(&tx);
        }
    })?;
    // `Corpus::open` requires `nodes/`; `inbox/` appears on first capture, and
    // a watch on a missing directory fails, so make it exist. This is what
    // `Corpus::init` and `capture` both do.
    let inbox = root.join("inbox");
    nebula_core::fs::create_private_dir_all(&inbox)
        .map_err(|error| notify::Error::generic(&error.to_string()))?;
    watcher.watch(&root.join("nodes"), RecursiveMode::Recursive)?;
    watcher.watch(&inbox, RecursiveMode::Recursive)?;

    std::thread::Builder::new()
        .name("corpus-watcher".into())
        .spawn(move || {
            for () in debounced(&rx, DEBOUNCE, MAX_LATENCY) {
                // Fail open: a window that misses one event refetches on the
                // next, and the tray is refreshed regardless.
                fail_open("emitting corpus-changed", app.emit(EVENT, ()));
                tray::refresh(&app);
            }
        })?;
    Ok(watcher)
}

/// Ask for one refresh without ever waiting. A full queue already holds a
/// signal, so this one is dropped (see [`start`]); a closed one means the app
/// is shutting down.
pub(crate) fn signal(tx: &SyncSender<()>) {
    match tx.try_send(()) {
        Ok(()) | Err(TrySendError::Full(()) | TrySendError::Disconnected(())) => {}
    }
}

pub(crate) fn is_change_event(event: &Event) -> bool {
    matches!(
        &event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Collapse bursts after `window` of quiet, but never wait longer than
/// `max_latency` from the first event in a burst. Ends when every sender is
/// gone, flushing a burst in progress first.
pub(crate) fn debounced(
    rx: &Receiver<()>,
    window: Duration,
    max_latency: Duration,
) -> impl Iterator<Item = ()> + '_ {
    std::iter::from_fn(move || {
        rx.recv().ok()?;
        let started = Instant::now();
        loop {
            let remaining = max_latency.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Some(());
            }
            match rx.recv_timeout(window.min(remaining)) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    return Some(());
                }
            }
        }
    })
}
