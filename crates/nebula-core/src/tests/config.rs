//! Unit tests for `config`.

use crate::error::{Error, Result};
use crate::fs_impl::write_private_atomic;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::config::{ObservatoryRoot, ObservatorySource};

fn pick(env: Option<&str>, machine: Option<&str>, legacy: Option<&str>) -> ObservatoryRoot {
    ObservatoryRoot::from_settings(
        env.map(Into::into),
        machine.map(PathBuf::from),
        legacy.map(PathBuf::from),
    )
}

#[test]
fn the_environment_outranks_the_machine_setting_which_outranks_the_legacy_key() {
    let all = pick(Some("/env"), Some("/machine"), Some("/legacy"));
    assert_eq!(all.root, Some(PathBuf::from("/env")));
    assert_eq!(all.source, ObservatorySource::Env);
    assert_eq!(all.legacy, Some(PathBuf::from("/legacy")));

    let machine = pick(None, Some("/machine"), Some("/legacy"));
    assert_eq!(machine.root, Some(PathBuf::from("/machine")));
    assert_eq!(machine.source, ObservatorySource::Machine);
    assert_eq!(machine.legacy, Some(PathBuf::from("/legacy")));

    let legacy = pick(None, None, Some("/legacy"));
    assert_eq!(legacy.root, Some(PathBuf::from("/legacy")));
    assert_eq!(legacy.source, ObservatorySource::Config);

    let unset = pick(None, None, None);
    assert_eq!(unset.root, None);
    assert_eq!(unset.source, ObservatorySource::Unset);
    assert_eq!(unset.legacy, None);
}

#[test]
fn an_empty_environment_value_is_no_value() {
    let setting = pick(Some(""), Some("/machine"), None);
    assert_eq!(setting.root, Some(PathBuf::from("/machine")));
    assert_eq!(setting.source, ObservatorySource::Machine);
}

#[test]
fn with_no_root_nothing_resolves() {
    assert_eq!(pick(None, None, None).resolve("Q002").unwrap(), None);
}
