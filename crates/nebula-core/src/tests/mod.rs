//! Unit tests for the crate's top-level modules, one file per source file
//! (STD-02 §R19). They reach only what the crate itself can.

#![allow(unused_imports)]

mod check;
mod config;
mod error;
mod fs;
mod git;
mod git_forward;
mod lock;
mod migrate;
mod model;
mod pending;
mod store;
mod triage;
#[cfg(feature = "ts")]
mod ts_export;
