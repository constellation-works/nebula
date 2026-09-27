//! Unit tests for the crate, one file per source file
//! (STD-02 §R19). Being siblings of the modules they test,
//! they reach `pub(crate)` items.

#![allow(unused_imports)]

mod cli;
mod output;
mod output_terminal;
mod render;
mod render_error;
mod render_json;
mod render_report;
mod render_table;
mod render_tree;
