//! Byte-exact public CLI fixtures, captured from the built `neb` binary.
//!
//! The corpus and Observatory records used here exist only under each test's
//! temporary directory. `NEB_UPDATE_GOLDENS=1` is the sole update switch.

#![allow(
    clippy::disallowed_methods,
    reason = "golden fixtures and isolated temporary test data are written directly"
)]

use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;

#[path = "../../nebula-core/tests/support/mod.rs"]
mod support;

const HELP_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/goldens/help");
const JSON_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/goldens/json");

#[derive(Clone, Copy)]
enum Setup {
    Empty,
    Base,
    Entry,
}

struct Case {
    name: &'static str,
    setup: Setup,
    args: &'static [&'static str],
}

// Every data verb has a representative --json invocation. Entry cases use
// one generated capture; @ENTRY@ is substituted only in argv, then redacted.
const CASES: &[Case] = &[
    Case {
        name: "init",
        setup: Setup::Empty,
        args: &["init"],
    },
    Case {
        name: "list",
        setup: Setup::Base,
        args: &["list"],
    },
    Case {
        name: "show",
        setup: Setup::Base,
        args: &["show", "first-idea"],
    },
    Case {
        name: "near",
        setup: Setup::Base,
        args: &["near", "First idea"],
    },
    Case {
        name: "trace",
        setup: Setup::Base,
        args: &["trace", "second-idea"],
    },
    Case {
        name: "impact",
        setup: Setup::Base,
        args: &["impact", "first-idea"],
    },
    Case {
        name: "graph",
        setup: Setup::Base,
        args: &["graph"],
    },
    Case {
        name: "check",
        setup: Setup::Base,
        args: &["check"],
    },
    Case {
        name: "review",
        setup: Setup::Base,
        args: &["review"],
    },
    Case {
        name: "review-short",
        setup: Setup::Base,
        args: &["review", "--short"],
    },
    Case {
        name: "inbox",
        setup: Setup::Entry,
        args: &["inbox"],
    },
    Case {
        name: "tag-list",
        setup: Setup::Base,
        args: &["tag", "list"],
    },
    Case {
        name: "config-commit",
        setup: Setup::Base,
        args: &["config", "commit"],
    },
    Case {
        name: "config-observatory-root",
        setup: Setup::Base,
        args: &["config", "observatory-root"],
    },
    Case {
        name: "capture",
        setup: Setup::Base,
        args: &["capture", "A captured insight"],
    },
    Case {
        name: "promote",
        setup: Setup::Entry,
        args: &[
            "promote",
            "@ENTRY@",
            "--title",
            "Promoted thought",
            "--id",
            "promoted-thought",
        ],
    },
    Case {
        name: "drop",
        setup: Setup::Entry,
        args: &["drop", "@ENTRY@"],
    },
    Case {
        name: "new",
        setup: Setup::Base,
        args: &["new", "Third idea", "--id", "third-idea"],
    },
    Case {
        name: "edit",
        setup: Setup::Base,
        args: &["edit", "first-idea"],
    },
    Case {
        name: "sharpen",
        setup: Setup::Base,
        args: &["sharpen", "first-idea", "--kill", "A counterexample"],
    },
    Case {
        name: "status",
        setup: Setup::Base,
        args: &["status", "first-idea", "abandoned", "--why", "Superseded"],
    },
    Case {
        name: "link",
        setup: Setup::Base,
        args: &["link", "first-idea", "contradicts", "second-idea"],
    },
    Case {
        name: "tag",
        setup: Setup::Base,
        args: &["tag", "first-idea", "--add", "bright"],
    },
    Case {
        name: "note",
        setup: Setup::Base,
        args: &["note", "first-idea", "A reason"],
    },
    Case {
        name: "cite-observatory",
        setup: Setup::Base,
        args: &[
            "cite",
            "first-idea",
            "--kind",
            "observatory",
            "--uri",
            "H999",
            "--note",
            "A source",
        ],
    },
    Case {
        name: "handoff",
        setup: Setup::Base,
        args: &["handoff", "first-idea", "H999", "--note", "Continued there"],
    },
    Case {
        name: "refusal-core",
        setup: Setup::Base,
        args: &["show", "nope"],
    },
    Case {
        name: "refusal-cli",
        setup: Setup::Base,
        args: &["cite", "first-idea", "--kind", "paper"],
    },
    Case {
        name: "refusal-clap",
        setup: Setup::Base,
        args: &["near"],
    },
];

struct Fixture {
    dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    observatory: PathBuf,
    editor: PathBuf,
    entry: Option<String>,
    at: Option<String>,
    corpus_id: Option<String>,
}

impl Fixture {
    fn new(setup: Setup) -> Self {
        let dir = tempfile::tempdir().expect("temporary fixture root");
        let root = dir.path().join("corpus");
        let home = dir.path().join("home");
        let observatory = dir.path().join("observatory");
        let editor = dir.path().join("editor.sh");
        std::fs::create_dir_all(&home).expect("fixture home");
        std::fs::create_dir_all(observatory.join("hypotheses")).expect("fixture Observatory");
        std::fs::write(
            observatory.join("hypotheses/H999-golden.md"),
            "# Synthetic record\n",
        )
        .expect("fixture Observatory record");
        std::fs::write(
            &editor,
            "#!/bin/sh\nprintf '\nGolden edited body.\n' >> \"$1\"\n",
        )
        .expect("fixture editor");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o700))
                .expect("fixture editor permissions");
        }
        let mut fixture = Self {
            dir,
            root,
            home,
            observatory,
            editor,
            entry: None,
            at: None,
            corpus_id: None,
        };
        if !matches!(setup, Setup::Empty) {
            fixture.setup();
            if matches!(setup, Setup::Entry) {
                let out = fixture.run(&["--json", "capture", "--quiet", "Pending insight"]);
                assert!(out.status.success(), "setting up capture: {out:?}");
                fixture.remember_entry(&out.stdout);
            }
        }
        fixture
    }

    fn command(&self) -> std::process::Command {
        let mut cmd = support::command(env!("CARGO_BIN_EXE_neb"), &self.home);
        cmd.current_dir(self.dir.path())
            .env("PWD", self.dir.path())
            .env("NEBULA_ROOT", &self.root)
            .env("OBSERVATORY_ROOT", &self.observatory)
            .env("VISUAL", &self.editor)
            .env("EDITOR", &self.editor)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env_remove("COLUMNS")
            .env("TZ", "UTC")
            .env("LC_ALL", "C");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        support::output(self.command().args(args), support::DEADLINE)
            .unwrap_or_else(|err| panic!("running {args:?}: {err}"))
    }

    fn setup(&self) {
        for args in [
            vec!["init"],
            vec![
                "new",
                "First idea",
                "--id",
                "first-idea",
                "--body",
                "A stable body",
            ],
            vec![
                "new",
                "Second idea",
                "--id",
                "second-idea",
                "--parent",
                "first-idea",
            ],
        ] {
            let out = self.run(&args);
            assert!(out.status.success(), "setting up {args:?}: {out:?}");
        }
        // Corpus IDs are minted randomly. A fixed test-only ID keeps every
        // later payload deterministic without checking corpus content in.
        let config = self.root.join("config.yaml");
        let raw = std::fs::read_to_string(&config).expect("fixture config");
        let id = corpus_id(&raw);
        std::fs::write(config, raw.replace(&id, "neb-golden")).expect("fixing fixture corpus ID");
    }

    fn remember_entry(&mut self, stdout: &[u8]) {
        let value: serde_json::Value = serde_json::from_slice(stdout).expect("capture JSON");
        self.entry = Some(value["entry"]["id"].as_str().expect("entry id").to_owned());
        self.at = Some(value["entry"]["at"].as_str().expect("entry at").to_owned());
    }

    fn redacted(&self, value: &str) -> String {
        // Only the documented dynamic values are redacted. Order matters:
        // the full timestamp must be replaced before its date substring.
        let mut result = value.to_owned();
        if let Some(at) = &self.at {
            result = result.replace(at, "<AT>");
        }
        if let Some(entry) = &self.entry {
            result = result.replace(entry, "<ENTRY>");
        }
        if let Some(id) = &self.corpus_id {
            result = result.replace(id, "<CORPUS_ID>");
        }
        let today = time::OffsetDateTime::now_utc().date().to_string();
        result = result.replace(&today, "<TODAY>");
        result.replace(
            self.dir.path().to_str().expect("UTF-8 fixture path"),
            "<ROOT>",
        )
    }
}

fn corpus_id(config: &str) -> String {
    config
        .lines()
        .find_map(|line| line.strip_prefix("corpus_id: "))
        .expect("corpus_id in config")
        .to_owned()
}

#[derive(Serialize)]
struct Recorded {
    argv: Vec<String>,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_case(case: &Case) -> Recorded {
    let mut fixture = Fixture::new(case.setup);
    let args: Vec<String> = std::iter::once("--json".to_owned())
        .chain(case.args.iter().map(|arg| {
            if *arg == "@ENTRY@" {
                fixture.entry.as_ref().expect("entry setup").clone()
            } else {
                (*arg).to_owned()
            }
        }))
        .collect();
    let out = fixture.run(&args.iter().map(String::as_str).collect::<Vec<_>>());
    let expected_exit = match case.name {
        "refusal-core" => 1,
        "refusal-cli" | "refusal-clap" => 2,
        _ => 0,
    };
    assert_eq!(
        out.status.code(),
        Some(expected_exit),
        "{}: {out:?}",
        case.name
    );
    if case.name == "capture" {
        fixture.remember_entry(&out.stdout);
    }
    if case.name == "init" {
        let config =
            std::fs::read_to_string(fixture.root.join("config.yaml")).expect("initialized config");
        fixture.corpus_id = Some(corpus_id(&config));
    }
    Recorded {
        argv: args.iter().map(|arg| fixture.redacted(arg)).collect(),
        exit_code: out.status.code().expect("normal exit"),
        stdout: fixture.redacted(std::str::from_utf8(&out.stdout).expect("UTF-8 stdout")),
        stderr: fixture.redacted(std::str::from_utf8(&out.stderr).expect("UTF-8 stderr")),
    }
}

fn updating() -> bool {
    std::env::var("NEB_UPDATE_GOLDENS").as_deref() == Ok("1")
}

fn diff(expected: &str, actual: &str) -> String {
    let old: Vec<_> = expected.split_inclusive('\n').collect();
    let new: Vec<_> = actual.split_inclusive('\n').collect();
    let mut result = String::from("--- expected\n+++ actual\n");
    for index in 0..old.len().max(new.len()) {
        if old.get(index) != new.get(index) {
            if let Some(line) = old.get(index) {
                write!(result, "- {line}").expect("writing diff");
            }
            if let Some(line) = new.get(index) {
                write!(result, "+ {line}").expect("writing diff");
            }
        }
    }
    result
}

fn assert_golden(path: &Path, actual: &str) {
    if updating() {
        std::fs::create_dir_all(path.parent().expect("fixture directory"))
            .expect("creating golden directory");
        std::fs::write(path, actual).expect("writing golden");
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "missing golden {}: {err}; run make goldens UPDATE=1",
            path.display()
        )
    });
    assert!(
        expected.as_bytes() == actual.as_bytes(),
        "golden {} differs; run make goldens UPDATE=1\n{}",
        path.display(),
        diff(&expected, actual)
    );
}

fn help_paths() -> Vec<String> {
    serde_json::from_str(
        &std::fs::read_to_string(Path::new(HELP_DIR).join("commands.json"))
            .expect("help command manifest"),
    )
    .expect("help command manifest JSON")
}

fn help_output(path: &str) -> String {
    let fixture = Fixture::new(Setup::Empty);
    let mut args: Vec<&str> = path.split_whitespace().collect();
    args.push("--help");
    let out = fixture.run(&args);
    assert!(out.status.success(), "help {path:?}: {out:?}");
    assert!(out.stderr.is_empty(), "help {path:?} wrote stderr: {out:?}");
    fixture.redacted(std::str::from_utf8(&out.stdout).expect("UTF-8 help"))
}

fn help_file(path: &str) -> PathBuf {
    if path.is_empty() {
        Path::new(HELP_DIR).join("neb.txt")
    } else {
        Path::new(HELP_DIR).join(format!("{}.txt", path.replace(' ', "/")))
    }
}

fn fixture_files(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(dir: &Path, files: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("golden directory") {
            let path = entry.expect("golden entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "txt") {
                files.insert(path);
            }
        }
    }
    let mut files = BTreeSet::new();
    visit(root, &mut files);
    files
}

#[test]
fn help_goldens_match_the_binary() {
    let paths = help_paths();
    let expected: BTreeSet<_> = paths.iter().map(|path| help_file(path)).collect();
    for path in &paths {
        assert_golden(&help_file(path), &help_output(path));
    }
    assert_eq!(
        fixture_files(Path::new(HELP_DIR)),
        expected,
        "unexpected or missing help fixture"
    );
}

#[test]
fn json_goldens_match_the_binary() {
    let expected: BTreeSet<_> = CASES
        .iter()
        .map(|case| Path::new(JSON_DIR).join(format!("{}.txt", case.name)))
        .collect();
    for case in CASES {
        let actual = format!(
            "{}\n",
            serde_json::to_string_pretty(&run_case(case)).expect("serialize golden")
        );
        assert_golden(
            &Path::new(JSON_DIR).join(format!("{}.txt", case.name)),
            &actual,
        );
    }
    assert_eq!(
        fixture_files(Path::new(JSON_DIR)),
        expected,
        "unexpected or missing JSON fixture"
    );
}

// A small scanner keeps the boundary explicit without a regex dependency.
fn has_forbidden_id(text: &str) -> bool {
    fn word(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    let bytes = text.as_bytes();
    for (index, rest) in bytes.iter().enumerate() {
        if (*rest == b'Q' || *rest == b'H' || *rest == b'R' || *rest == b'T')
            && bytes
                .get(index + 1..index + 4)
                .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
            && text[..index].chars().next_back().is_none_or(|ch| !word(ch))
            && text[index + 4..].chars().next().is_none_or(|ch| !word(ch))
        {
            return true;
        }
        if bytes[index..].starts_with(b"ORB-") || bytes[index..].starts_with(b"DANI-") {
            let offset = if bytes[index..].starts_with(b"ORB-") {
                4
            } else {
                5
            };
            if bytes.get(index + offset).is_some_and(u8::is_ascii_digit) {
                return true;
            }
        }
        if *rest == b'F'
            && bytes
                .get(index + 1..index + 5)
                .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
            && bytes.get(index + 5) == Some(&b'-')
            && bytes
                .get(index + 6..index + 8)
                .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
            && bytes.get(index + 8) == Some(&b'-')
            && bytes
                .get(index + 9..index + 12)
                .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
        {
            return true;
        }
    }
    false
}

#[test]
fn forbidden_id_scanner_uses_word_boundaries() {
    for text in [
        "ORB-1",
        "DANI-42",
        "F2026-09-123",
        "Q001",
        "x H123 y",
        "T999.",
    ] {
        assert!(has_forbidden_id(text), "missed {text:?}");
    }
    for text in ["H<nnn>", "_Q001", "Q001_", "αQ001", "Q001β", "F2026-09-12"] {
        assert!(!has_forbidden_id(text), "false match in {text:?}");
    }
}

#[test]
fn help_tree_has_no_tracker_or_real_record_ids() {
    for path in help_paths() {
        let rendered = help_output(&path);
        assert!(
            !has_forbidden_id(&rendered),
            "forbidden id in help {path:?}: {rendered}"
        );
    }
    for case in CASES
        .iter()
        .filter(|case| case.name.starts_with("refusal-"))
    {
        let path = Path::new(JSON_DIR).join(format!("{}.txt", case.name));
        let rendered = serde_json::to_string(&run_case(case)).expect("refusal serialization");
        assert!(
            !has_forbidden_id(&rendered),
            "forbidden id in rendered {}",
            path.display()
        );
        // Update mode creates these files concurrently with this test.
        if !updating() {
            let fixture = std::fs::read_to_string(&path).expect("refusal fixture");
            assert!(
                !has_forbidden_id(&fixture),
                "forbidden id in {}",
                path.display()
            );
        }
    }
}
