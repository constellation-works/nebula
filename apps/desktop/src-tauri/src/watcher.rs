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
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// Longest time a continuous run of events can wait before a refresh.
pub const MAX_LATENCY: Duration = Duration::from_secs(1);

/// The event every window listens for. Carries no payload: the frontend
/// refetches what it shows rather than reasoning about which file moved.
pub const EVENT: &str = "corpus-changed";

/// Watch `root/nodes` and `root/inbox`. The returned watcher stops when
/// dropped, so the caller keeps it for the life of the app.
pub fn start<R: Runtime>(app: AppHandle<R>, root: &Path) -> notify::Result<RecommendedWatcher> {
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
pub fn signal(tx: &SyncSender<()>) {
    match tx.try_send(()) {
        Ok(()) | Err(TrySendError::Full(()) | TrySendError::Disconnected(())) => {}
    }
}

fn is_change_event(event: &Event) -> bool {
    matches!(
        &event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Collapse bursts after `window` of quiet, but never wait longer than
/// `max_latency` from the first event in a burst. Ends when every sender is
/// gone, flushing a burst in progress first.
pub fn debounced(
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

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_millis(40);
    const MAX_LATENCY: Duration = Duration::from_millis(120);

    #[test]
    fn filters_access_events_and_forwards_mutations() {
        let access = Event::new(EventKind::Access(notify::event::AccessKind::Open(
            notify::event::AccessMode::Read,
        )));
        let read = Event::new(EventKind::Access(notify::event::AccessKind::Read));
        let create = Event::new(EventKind::Create(notify::event::CreateKind::File));
        let modify = Event::new(EventKind::Modify(notify::event::ModifyKind::Any));
        let rename = Event::new(EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::Both,
        )));
        let remove = Event::new(EventKind::Remove(notify::event::RemoveKind::File));

        assert!(!is_change_event(&access));
        assert!(!is_change_event(&read));
        assert!(is_change_event(&create));
        assert!(is_change_event(&modify));
        assert!(is_change_event(&rename));
        assert!(is_change_event(&remove));
    }

    #[test]
    fn a_burst_is_one_change() {
        let (tx, rx) = sync_channel(5);
        for _ in 0..5 {
            tx.send(()).unwrap();
        }
        drop(tx);
        assert_eq!(debounced(&rx, WINDOW, MAX_LATENCY).count(), 1);
    }

    #[test]
    fn quiet_between_bursts_separates_them() {
        // No producer thread: a sleep between bursts races the scheduler, and
        // a late wake-up would find the next event already queued. The first
        // change can only come after a quiet window, so an event sent after it
        // is by construction a second burst.
        let (tx, rx) = sync_channel(2);
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        let mut changes = debounced(&rx, WINDOW, MAX_LATENCY);
        assert_eq!(changes.next(), Some(()));
        tx.send(()).unwrap();
        drop(tx);
        assert_eq!(changes.next(), Some(()));
        assert_eq!(changes.next(), None);
    }

    #[test]
    fn continuous_events_yield_by_the_maximum_latency() {
        // Room for every send, so the producer never blocks on a consumer
        // that stops after the first change.
        let (tx, rx) = sync_channel(128);
        let producer = std::thread::spawn(move || {
            let end = Instant::now() + Duration::from_millis(400);
            while Instant::now() < end {
                tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let started = Instant::now();
        assert!(debounced(&rx, WINDOW, MAX_LATENCY).next().is_some());
        // Without the cap the first change would wait for the producer to stop
        // and then a quiet window: at least 440 ms. The cap is 120 ms; the
        // rest of the bound is slack for a loaded CI runner.
        assert!(started.elapsed() < Duration::from_millis(400));
        producer.join().unwrap();
    }

    #[test]
    fn a_full_queue_coalesces_rather_than_grows() {
        let (tx, rx) = sync_channel(1);
        let started = Instant::now();
        for _ in 0..10_000 {
            signal(&tx);
        }
        // Nothing consumed, and nothing waited: an unbounded queue would hold
        // all 10,000, and a blocking send would never return.
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(rx.try_iter().count(), 1);

        for _ in 0..10_000 {
            signal(&tx);
        }
        drop(tx);
        assert_eq!(debounced(&rx, WINDOW, MAX_LATENCY).count(), 1);
    }

    #[test]
    fn a_signal_after_the_app_is_gone_is_dropped() {
        let (tx, rx) = sync_channel(1);
        drop(rx);
        signal(&tx);
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        let (tx, rx) = sync_channel::<()>(1);
        drop(tx);
        assert_eq!(debounced(&rx, WINDOW, MAX_LATENCY).count(), 0);
    }
}
