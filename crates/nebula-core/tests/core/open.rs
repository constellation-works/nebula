//! Opening, discovering and initialising a corpus, and the machine root
//! setting `init --set-root` writes.

use crate::harness::process_locations;
use crate::support;
use nebula_core::{Corpus, Error, ops};

#[test]
fn opening_a_missing_corpus_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere");
    assert!(matches!(
        Corpus::open(&process_locations(), Some(missing.clone())),
        Err(Error::NoCorpus(p)) if p == missing
    ));
}

#[test]
fn open_or_init_says_whether_it_created_the_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fresh");

    let (_, created) =
        Corpus::open_or_init(&process_locations(), Some(root.clone())).expect("first open");
    assert!(created, "there was no corpus, so this call made one");
    assert!(root.join("nodes").is_dir() && root.join("inbox").is_dir());

    let (_, created) = Corpus::open_or_init(&process_locations(), Some(root)).expect("second open");
    assert!(!created, "an existing corpus is opened, not created");
}

#[test]
fn discover_finds_the_nearest_corpus_at_or_above_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let inner = outer.join("projects").join("inner");
    Corpus::init(&process_locations(), &outer).unwrap();
    Corpus::init(&process_locations(), &inner).unwrap();
    let deep = inner.join("notes").join("deep");
    std::fs::create_dir_all(&deep).unwrap();

    assert_eq!(Corpus::discover(&outer), Some(outer.clone()));
    assert_eq!(Corpus::discover(&outer.join("nodes")), Some(outer.clone()));
    assert_eq!(Corpus::discover(&outer.join("projects")), Some(outer));
    assert_eq!(Corpus::discover(&inner.join("nodes")), Some(inner.clone()));
    assert_eq!(Corpus::discover(&deep), Some(inner));
    assert_eq!(Corpus::discover(dir.path()), None);
    // The start need not exist: the walk is over the path, not the disk.
    assert_eq!(
        Corpus::discover(&deep.join("not-yet")),
        Corpus::discover(&deep)
    );
}

#[test]
fn discover_requires_nodes_and_positive_corpus_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("nodes-only", true, None),
        ("foreign-config", true, Some("name: some other tool\n")),
        ("scalar-config", true, Some("some other tool\n")),
        ("corpus-id-only", true, Some("corpus_id: null\n")),
        (
            "config-only",
            false,
            Some("schema_version: 2\ncorpus_id: neb-000001\n"),
        ),
        ("no-corpus-id", true, Some("schema_version: 2\n")),
        (
            "empty-corpus-id",
            true,
            Some("schema_version: 2\ncorpus_id: ''\n"),
        ),
        ("not-yaml", true, Some("corpus_id: [unclosed\n")),
        // An older schema still counts, so `neb migrate` works from inside.
        (
            "v1",
            true,
            Some("schema_version: 1\ncorpus_id: neb-000001\ndomains: []\n"),
        ),
    ];
    for (name, nodes, config) in cases {
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        if nodes {
            std::fs::create_dir(root.join("nodes")).unwrap();
        }
        if let Some(config) = config {
            std::fs::write(root.join("config.yaml"), config).unwrap();
        }
        let expected = (nodes
            && !matches!(name, "nodes-only" | "foreign-config" | "scalar-config"))
        .then(|| root.clone());
        assert_eq!(Corpus::discover(&root), expected, "{name}");
    }
}

#[test]
fn discover_finds_configless_corpora_by_inbox_or_lock() {
    let dir = tempfile::tempdir().unwrap();
    for marker in ["inbox", ".lock"] {
        let root = dir.path().join(marker);
        std::fs::create_dir_all(root.join("nodes")).unwrap();
        if marker == "inbox" {
            std::fs::create_dir(root.join(marker)).unwrap();
        } else {
            std::fs::write(root.join(marker), "").unwrap();
        }
        assert_eq!(Corpus::discover(&root.join("nodes")), Some(root.clone()));
        assert!(matches!(
            Corpus::open(&process_locations(), Some(root)),
            Err(Error::MissingConfig { .. })
        ));
    }
}

#[cfg(unix)]
#[test]
fn discover_walks_the_path_as_spelled_and_never_resolves_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("real").join("corpus");
    Corpus::init(&process_locations(), &root).unwrap();

    // Through a symlinked parent, the root comes back in the alias spelling.
    let alias = dir.path().join("alias");
    symlink(dir.path().join("real"), &alias).unwrap();
    assert_eq!(
        Corpus::discover(&alias.join("corpus").join("nodes")),
        Some(alias.join("corpus"))
    );

    // A link into the corpus from outside is a directory outside it, the way
    // `cd ..` from there leaves it: the walk does not follow the link back.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    symlink(root.join("nodes"), elsewhere.join("ideas")).unwrap();
    assert_eq!(Corpus::discover(&elsewhere.join("ideas")), None);
}

/// A symlinked `nodes/` is found, not walked past, so opening it refuses by
/// name rather than resolution quietly settling on some other corpus.
#[cfg(unix)]
#[test]
fn discover_finds_a_corpus_whose_nodes_is_a_symlink_so_open_can_refuse_it() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    Corpus::init(&process_locations(), &outer).unwrap();
    let root = outer.join("linked");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "schema_version: 2\ncorpus_id: neb-000001\n",
    )
    .unwrap();
    for target in [outer.join("nodes"), outer.join("missing")] {
        std::os::unix::fs::symlink(target, root.join("nodes")).unwrap();
        assert_eq!(Corpus::discover(&root), Some(root.clone()));
        let refused = Corpus::open(&process_locations(), Some(root.clone()))
            .expect_err("a symlinked nodes/ is refused");
        assert!(refused.to_string().contains("is a symlink"), "{refused}");
        std::fs::remove_file(root.join("nodes")).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn a_corpus_that_cannot_be_created_names_the_root_it_aimed_at() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("read-only");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let root = parent.join("corpus");

    let init = Corpus::init(&process_locations(), &root).map(|_| ());
    let open_or_init = Corpus::open_or_init(&process_locations(), Some(root.clone())).map(|_| ());
    let ops_init =
        ops::init(&process_locations(), Some(root.clone()), None, false, false).map(|_| ());
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();

    for result in [init, open_or_init, ops_init] {
        assert!(
            matches!(
                &result,
                Err(Error::IoAt { action: "creating", path, source })
                    if *path == root && source.kind() == std::io::ErrorKind::PermissionDenied
            ),
            "expected a creation failure naming {}, got {result:?}",
            root.display()
        );
    }
    assert!(!root.exists());
}

/// `init` given a root and a path that name different directories refuses
/// before creating either (STD-01 §R28). One directory spelled two ways, with
/// a `.` segment or a trailing slash, is one corpus.
#[test]
fn init_refuses_a_root_and_a_path_that_name_different_directories() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));

    let refused = ops::init(
        &process_locations(),
        Some(a.clone()),
        Some(b.clone()),
        false,
        false,
    )
    .map(|_| ());
    assert!(
        matches!(&refused, Err(Error::RootAndPathDiffer { root, path }) if *root == a && *path == b),
        "{refused:?}"
    );
    assert_eq!(refused.unwrap_err().code(), "root_and_path_differ");
    assert!(!a.exists() && !b.exists(), "neither is created");

    let spelled = dir.path().join(".").join("a").join("");
    assert_eq!(
        ops::init_target(&process_locations(), Some(a.clone()), Some(spelled.clone())).unwrap(),
        spelled
    );
    ops::init(
        &process_locations(),
        Some(a.clone()),
        Some(spelled),
        false,
        false,
    )
    .unwrap();
    assert!(a.join("nodes").is_dir());
}

/// Set in a child copy of this test binary that [`in_own_process`] started.
const IN_OWN_PROCESS: &str = "NEBULA_CORE_TEST_IN_OWN_PROCESS";

/// Whether the caller is the child copy of this test binary that runs `test`
/// alone. In the parent, start that child, require it to pass, and say no.
///
/// Machine settings live under `HOME`, and every test in this process shares
/// its one isolated home (`support`), so a test that writes
/// `~/.config/nebula` would race the tests beside it. `support` gives each
/// test process a fresh home of its own, so the child's settings are nobody
/// else's (STD-03 §R20). The child comes from the isolating builder and is
/// waited for under its guard and deadline.
#[cfg(unix)]
fn in_own_process(test: &str) -> bool {
    if std::env::var_os(IN_OWN_PROCESS).is_some() {
        return true;
    }
    let mut cmd = support::command(
        std::env::current_exe().expect("test binary"),
        support::home(),
    );
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(IN_OWN_PROCESS, "1");
    let out = support::output(&mut cmd, support::DEADLINE).unwrap_or_else(|e| panic!("{e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "the child run of {test} did not pass:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// `~/.config/nebula/root` is replaced whole, like every other file nebula
/// writes. Written by name, as it once was, a symlink there carried the new
/// path into whatever file it pointed at and stayed a symlink.
#[cfg(unix)]
#[test]
fn set_root_replaces_a_symlinked_setting_instead_of_writing_through() {
    use std::os::unix::fs::PermissionsExt;

    if !in_own_process("open::set_root_replaces_a_symlinked_setting_instead_of_writing_through") {
        return;
    }

    let settings = support::home().join(".config").join("nebula");
    std::fs::create_dir_all(&settings).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside-target");
    std::fs::write(&outside, "/an/older/corpus\n").unwrap();
    let setting = settings.join("root");
    std::os::unix::fs::symlink(&outside, &setting).unwrap();
    let target = dir.path().join("new-corpus");

    ops::init(&process_locations(), None, Some(target.clone()), true, true)
        .expect("init --set-root --force");

    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"/an/older/corpus\n",
        "the write went through the symlink"
    );
    let metadata = std::fs::symlink_metadata(&setting).unwrap();
    assert!(
        metadata.file_type().is_file(),
        "the setting is still a symlink"
    );
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        std::fs::read_to_string(&setting).unwrap(),
        format!("{}\n", target.display())
    );
}
