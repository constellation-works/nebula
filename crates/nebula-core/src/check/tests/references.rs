//! Unit tests for `check/references.rs`.

use crate::check_impl::{is_absolute_local, is_local_path};

#[test]
fn schemes_and_urls_are_not_local_paths() {
    for uri in [
        "https://example.org",
        "doi:10.1000/x",
        "orbit:DANI-10345",
        "neb:some-node",
        "[[almanac/page]]",
    ] {
        assert!(!is_local_path(uri), "{uri}");
    }
    for uri in ["./notes/x.md", "notes/x.md", "C:/x.md", "x"] {
        assert!(is_local_path(uri), "{uri}");
    }
}

/// Absolute on any platform is absolute on all of them, and nothing a
/// scheme, a URL or a path under `nodes/` looks like is caught with it.
#[test]
fn absolute_paths_and_file_uris_are_absolute_on_every_platform() {
    for uri in [
        "/etc/hostname",
        "\\\\server\\share\\x.md",
        "C:/x.md",
        "c:\\x.md",
        "C:x.md",
        "file:///etc/hostname",
        "FILE://host/x.md",
        "file:/etc/hostname",
    ] {
        assert!(is_absolute_local(uri), "{uri}");
    }
    for uri in [
        "./notes/x.md",
        "notes/x.md",
        "../../studies/x.md",
        "x",
        "https://example.org",
        "http://example.org/file:///x",
        "mailto:someone@example.org",
        "orbit:DANI-10345",
        "neb:some-node",
        "doi:10.1000/x",
        "[[almanac/page]]",
        "files/x.md",
    ] {
        assert!(!is_absolute_local(uri), "{uri}");
    }
}
