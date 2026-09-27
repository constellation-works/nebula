//! Unit tests for the crate's top-level modules, one file per source file
//! (STD-02 §R19). They reach only what the crate itself can.

#![allow(unused_imports)]

mod config;
mod fs;
mod git;
mod git_forward;
mod id;
mod lock;
mod model;
mod pending;
mod stamp;
mod triage;
#[cfg(feature = "ts")]
mod ts_export;
