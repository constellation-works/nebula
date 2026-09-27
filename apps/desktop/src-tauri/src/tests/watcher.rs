//! Unit tests for `watcher`.

use crate::{fail_open, tray};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};

use crate::watcher::*;

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
