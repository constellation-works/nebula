#![deny(clippy::print_stderr, clippy::print_stdout)]

//! Entry point for the `neb` binary.

fn main() -> std::process::ExitCode {
    neb::main()
}
