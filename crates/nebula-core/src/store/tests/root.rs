//! Unit tests for `store/root.rs`.

use crate::error::Error;
use crate::store::Corpus;
use std::path::{Path, PathBuf};

/// Load and `--set-root` share this one rule, and it reads only what it
/// is handed: no `HOME`, no file.
#[test]
fn root_setting_is_validated_by_one_function() {
    let setting = Path::new("/h/.config/nebula/root");

    for contents in ["", "\n", "  \t\n"] {
        let error = Corpus::root_setting(setting, contents).unwrap_err();
        assert!(
            matches!(&error, Error::EmptyRootSetting(path) if path == setting),
            "{contents:?}: {error:?}"
        );
        assert!(
            error.to_string().contains("/h/.config/nebula/root"),
            "{error}"
        );
    }

    for contents in ["relcorpus", "relcorpus\n", "./corpus\n", "~/corpus\n"] {
        let error = Corpus::root_setting(setting, contents).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::RelativeRootSetting { setting: path, root }
                    if path == setting && root == Path::new(contents.trim())
            ),
            "{contents:?}: {error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("/h/.config/nebula/root"), "{message}");
        assert!(message.contains(contents.trim()), "{message}");
    }

    for contents in ["/srv/corpus", "/srv/corpus\n", "  /srv/my corpus \n"] {
        assert_eq!(
            Corpus::root_setting(setting, contents).unwrap(),
            PathBuf::from(contents.trim())
        );
    }

    // The write path hands it exactly what it would write.
    let root = Path::new("/srv/corpus");
    assert_eq!(
        Corpus::root_setting(setting, &Corpus::root_setting_contents(root)).unwrap(),
        root
    );
}
