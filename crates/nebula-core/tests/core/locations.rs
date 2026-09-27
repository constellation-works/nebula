//! Resolved locations: the root and the observatory root from the flag,
//! the environment, the working directory, the machine file and the default.

use crate::config::with_legacy_observatory_root;
use crate::support;
use nebula_core::verb::{self, RootWarning, WriteOptions};
use nebula_core::{Corpus, Error, Locations, ops};
use std::path::Path;

/// A `Locations` naming only `home`: no working directory, no variables.
fn at_home(home: &Path) -> Locations {
    Locations {
        home: Some(home.as_os_str().to_owned()),
        ..Locations::default()
    }
}

/// Every answer `resolve_root` gives comes from the `Locations` it is handed,
/// in order: `--root`, then `NEBULA_ROOT` (empty counts as unset), then the
/// corpus the working directory is in, then the machine file, then
/// `~/.nebula` under the given home. With no home the default is refused
/// rather than read from the process.
#[test]
fn resolve_root_follows_explicit_then_nebula_root_then_cwd_then_machine_file_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let configured = dir.path().join("configured");
    let named = dir.path().join("named");
    let explicit = dir.path().join("explicit");
    let found = dir.path().join("found");
    let base = at_home(&home);
    Corpus::init(&base, &found).unwrap();
    let setting_dir = home.join(".config").join("nebula");
    std::fs::create_dir_all(&setting_dir).unwrap();
    std::fs::write(
        setting_dir.join("root"),
        format!("{}\n", configured.display()),
    )
    .unwrap();

    let every = Locations {
        cwd: Some(found.join("nodes")),
        nebula_root: Some(named.as_os_str().to_owned()),
        ..base.clone()
    };
    let resolve = |locations: &Locations| Corpus::resolve_root(locations, None).unwrap();

    assert_eq!(
        Corpus::resolve_root(&every, Some(explicit.clone())).unwrap(),
        explicit
    );
    assert_eq!(resolve(&every), named);

    let unset = Locations {
        nebula_root: Some("".into()),
        ..every.clone()
    };
    assert_eq!(resolve(&unset), found, "an empty NEBULA_ROOT is unset");

    let outside = Locations {
        cwd: Some(home.clone()),
        ..unset.clone()
    };
    assert_eq!(resolve(&outside), configured);
    assert_eq!(outside.corpus_root().unwrap(), configured);

    let fresh = dir.path().join("fresh-home");
    let unconfigured = Locations {
        home: Some(fresh.as_os_str().to_owned()),
        ..outside.clone()
    };
    assert_eq!(resolve(&unconfigured), fresh.join(".nebula"));

    let homeless = Locations {
        home: None,
        ..outside
    };
    assert!(matches!(
        Corpus::resolve_root(&homeless, None),
        Err(Error::HomeUnset)
    ));
}

/// The observatory setting is `OBSERVATORY_ROOT` as `Locations` holds it,
/// else the machine file under its home, else the legacy `config.yaml` key,
/// which is reported whichever wins. An empty variable is unset, and a
/// corpus opened with no home still reads the legacy key.
#[test]
fn observatory_root_prefers_env_then_machine_file_then_legacy_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let root = dir.path().join("corpus");
    let locations = at_home(&home);
    Corpus::init(&locations, &root).unwrap();
    let foreign = with_legacy_observatory_root(&root);
    let setting = |locations: &Locations| {
        Corpus::open(locations, Some(root.clone()))
            .unwrap()
            .observatory_root()
            .unwrap()
    };

    let legacy = setting(&locations);
    assert_eq!(legacy.source, nebula_core::ObservatorySource::Config);
    assert_eq!(legacy.root.as_deref(), Some(foreign.as_path()));
    assert_eq!(setting(&Locations::default()).root, legacy.root);

    let machine = dir.path().join("machine-observatory");
    let corpus = Corpus::open(&locations, Some(root.clone())).unwrap();
    ops::set_observatory_root(&corpus, &machine).unwrap();
    let from_file = setting(&locations);
    assert_eq!(from_file.source, nebula_core::ObservatorySource::Machine);
    assert_eq!(from_file.root.as_deref(), Some(machine.as_path()));
    assert_eq!(from_file.legacy.as_deref(), Some(foreign.as_path()));

    let env = dir.path().join("env-observatory");
    let exported = Locations {
        observatory_root: Some(env.as_os_str().to_owned()),
        ..locations.clone()
    };
    let from_env = setting(&exported);
    assert_eq!(from_env.source, nebula_core::ObservatorySource::Env);
    assert_eq!(from_env.root.as_deref(), Some(env.as_path()));
    assert_eq!(from_env.legacy.as_deref(), Some(foreign.as_path()));

    let empty = Locations {
        observatory_root: Some("".into()),
        ..locations
    };
    assert_eq!(
        setting(&empty).source,
        nebula_core::ObservatorySource::Machine,
        "an empty OBSERVATORY_ROOT is unset"
    );
}

/// `~/.nebula`, the machine file and a corpus created with nothing else set
/// all sit under the home `Locations` names, not the one this process has.
#[test]
fn default_root_comes_from_locations_home_not_process_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("elsewhere");
    assert_ne!(home, support::home());
    let locations = at_home(&home);

    assert_eq!(
        Corpus::root_config_path(&locations).unwrap(),
        home.join(".config").join("nebula").join("root")
    );

    let captured = verb::capture_at(
        &locations,
        None,
        "a thought with nowhere else to go",
        0,
        &WriteOptions::default(),
    )
    .unwrap()
    .value;
    assert_eq!(captured.root, home.join(".nebula"));
    assert!(captured.created);
    assert!(home.join(".nebula").join("nodes").is_dir());
}

/// `$PWD` names the working directory only when it is absolute, has no `.`
/// or `..`, and is the same directory as the OS's answer; then a corpus
/// reached through a symlink is found under the spelling the shell used.
#[cfg(unix)]
#[test]
fn logical_pwd_is_used_only_when_it_names_the_same_directory_as_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    let other = dir.path().join("other");
    let link = dir.path().join("link");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let working = |pwd: Option<&Path>, cwd: Option<&Path>| {
        Locations {
            cwd: cwd.map(Path::to_path_buf),
            pwd: pwd.map(|p| p.as_os_str().to_owned()),
            ..Locations::default()
        }
        .working_dir()
    };

    assert_eq!(working(Some(&link), Some(&real)), Some(link.clone()));
    assert_eq!(working(Some(&real), Some(&real)), Some(real.clone()));
    assert_eq!(working(Some(&other), Some(&real)), Some(real.clone()));
    assert_eq!(
        working(Some(Path::new("link")), Some(&real)),
        Some(real.clone()),
        "a relative PWD is not trusted"
    );
    assert_eq!(
        working(Some(&other.join("..").join("link")), Some(&real)),
        Some(real.clone()),
        "a PWD with `..` is not trusted, even naming the same directory"
    );
    assert_eq!(working(None, Some(&real)), Some(real.clone()));
    assert_eq!(working(Some(&link), None), None);

    let corpus = dir.path().join("corpus");
    let linked = dir.path().join("linked");
    Corpus::init(&Locations::default(), &corpus).unwrap();
    std::os::unix::fs::symlink(&corpus, &linked).unwrap();
    let inside = Locations {
        cwd: Some(corpus.join("nodes")),
        pwd: Some(linked.join("nodes").into_os_string()),
        ..at_home(&dir.path().join("home"))
    };
    assert_eq!(Corpus::resolve_root(&inside, None).unwrap(), linked);
}

/// `init` names the machine setting that would otherwise send every later
/// command elsewhere: `~/.nebula` created while it names another corpus,
/// and any other corpus created beside it. Setting the root warns of
/// neither, and says where the setting is.
#[test]
fn init_returns_default_and_shadowing_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let locations = at_home(&home);
    let setting = home.join(".config").join("nebula").join("root");
    let configured = dir.path().join("configured");

    let first = verb::init(&locations, None, Some(configured.clone()), true, false).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert_eq!(first.setting, setting);
    assert!(!first.suggest_set_root);

    let default = verb::init(&locations, Some(home.join(".nebula")), None, false, false).unwrap();
    assert_eq!(
        default.warnings,
        [RootWarning::DefaultWhileConfigured {
            setting: setting.clone(),
            configured: configured.clone(),
        }]
    );

    let other = verb::init(
        &locations,
        None,
        Some(dir.path().join("other")),
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        other.warnings,
        [RootWarning::Shadowed {
            setting: setting.clone(),
            configured,
        }]
    );
    assert!(!other.suggest_set_root, "a machine setting already exists");

    let third = dir.path().join("third");
    let moved = verb::init(&locations, None, Some(third.clone()), true, true).unwrap();
    assert!(moved.warnings.is_empty(), "{:?}", moved.warnings);
    assert_eq!(
        std::fs::read_to_string(&setting).unwrap().trim(),
        third.to_str().unwrap()
    );
}
