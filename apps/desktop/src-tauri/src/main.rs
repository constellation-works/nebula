//! Starts the Nebula desktop application.

#![deny(clippy::print_stderr, clippy::print_stdout)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
// Prevents an extra console window on Windows in release; harmless elsewhere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nebula_desktop::run();
}
