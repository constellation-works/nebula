//! Unit tests for the crate, one file per source file
//! (STD-02 §R19). Being siblings of the modules they test,
//! they reach `pub(crate)` items.

#![allow(unused_imports)]

mod crate_root;
mod session;
mod settings;
mod shortcut;
mod watcher;
