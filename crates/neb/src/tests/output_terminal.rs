//! Unit tests for `output_terminal`.

use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::sync::OnceLock;

use crate::output::terminal::*;

/// A [`Terminal`] from `vars`, with every stream a terminal unless
/// `ttys` says otherwise.
fn env(vars: &[(&str, &str)], ttys: Ttys) -> Terminal {
    Terminal::from_env(
        |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        },
        ttys,
    )
}

const ALL: Ttys = Ttys {
    stdin: true,
    stdout: true,
    stderr: true,
};

const PIPED: Ttys = Ttys {
    stdin: false,
    stdout: false,
    stderr: false,
};

#[test]
fn a_terminal_is_coloured_and_a_pipe_is_not() {
    let t = env(&[("TERM", "xterm-256color")], ALL);
    assert!(t.colours(Stream::Stdout) && t.colours(Stream::Stderr));
    let t = env(&[("TERM", "xterm-256color")], PIPED);
    assert!(!t.colours(Stream::Stdout) && !t.colours(Stream::Stderr));
}

#[test]
fn term_dumb_disables_colour() {
    let t = env(&[("TERM", "dumb")], ALL);
    assert!(!t.colours(Stream::Stdout) && !t.colours(Stream::Stderr));
}

#[test]
fn empty_no_color_does_not_disable() {
    let t = env(&[("NO_COLOR", "")], ALL);
    assert!(t.colours(Stream::Stdout) && t.colours(Stream::Stderr));
    let t = env(&[("NO_COLOR", "1")], ALL);
    assert!(!t.colours(Stream::Stdout) && !t.colours(Stream::Stderr));
}

#[test]
fn no_color_outranks_clicolor_force() {
    let t = env(&[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")], ALL);
    assert!(!t.colours(Stream::Stdout) && !t.colours(Stream::Stderr));
    let t = env(&[("TERM", "dumb"), ("CLICOLOR_FORCE", "1")], ALL);
    assert!(!t.colours(Stream::Stdout), "TERM=dumb outranks it too");
}

#[test]
fn clicolor_force_never_colours_a_pipe() {
    let t = env(&[("CLICOLOR_FORCE", "1")], PIPED);
    assert!(!t.colours(Stream::Stdout) && !t.colours(Stream::Stderr));
    let t = env(&[("CLICOLOR_FORCE", "1")], ALL);
    assert!(t.colours(Stream::Stdout), "a terminal stays coloured");
}

#[test]
fn stderr_colour_is_decided_from_stderr() {
    let only_stdout = Ttys {
        stdout: true,
        ..PIPED
    };
    let t = env(&[], only_stdout);
    assert!(t.colours(Stream::Stdout));
    assert!(!t.colours(Stream::Stderr), "`2>file` stays plain");

    let only_stderr = Ttys {
        stderr: true,
        ..PIPED
    };
    let t = env(&[], only_stderr);
    assert!(!t.colours(Stream::Stdout), "`| less` stays plain");
    assert!(t.colours(Stream::Stderr));
}

#[test]
fn stdin_is_read_apart_from_the_output_streams() {
    let t = env(
        &[],
        Ttys {
            stdin: true,
            ..PIPED
        },
    );
    assert!(t.ttys.stdin && !t.ttys.stdout);
}

#[test]
fn unmapped_value_renders_neutral() {
    assert_eq!(Role::of("status", "seed"), Role::Active);
    assert_eq!(Role::of("severity", "error"), Role::Error);
    assert_eq!(Role::of("status", "someday"), Role::Neutral);
    assert_eq!(Role::of("colour", "seed"), Role::Neutral);
    assert_eq!(painted(true, Role::Neutral.sgr(), "plain"), "plain");
}

#[test]
fn every_role_is_basic_sixteen_colour_or_plain() {
    for role in [
        Role::Ok,
        Role::Warn,
        Role::Error,
        Role::Active,
        Role::Muted,
        Role::Neutral,
    ] {
        for param in role.sgr().into_iter().flat_map(|s| s.split(';')) {
            let n: u8 = param.parse().unwrap();
            assert!(matches!(n, 1 | 2 | 30..=37 | 90..=97), "{role:?} uses {n}");
        }
    }
    assert_eq!(painted(true, Role::Warn.sgr(), "x"), "\x1b[33mx\x1b[0m");
    assert_eq!(painted(false, Role::Warn.sgr(), "x"), "x");
}
