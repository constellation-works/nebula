//! Unit tests for `env`: registry completeness and README table sync.

use crate::env::{
    EDITOR_ENV, NEBULA_TEST_GIT_DEADLINE_MS_ENV, README_TABLE_BEGIN, README_TABLE_END, VISUAL_ENV,
    all, extract_readme_table, readme_table, replace_readme_table,
};
use std::collections::BTreeSet;

#[test]
fn registry_covers_every_env_var_once_with_docs() {
    let vars = all();
    let names: BTreeSet<&str> = vars.iter().map(|v| v.name).collect();
    assert_eq!(names.len(), vars.len(), "duplicate env var entries");

    let expected: BTreeSet<&str> = [
        nebula_core::NEBULA_ROOT_ENV,
        nebula_core::OBSERVATORY_ROOT_ENV,
        nebula_core::ORBIT_RUN_ID_ENV,
        nebula_core::ORBIT_TASK_ID_ENV,
        nebula_core::NEBULA_READ_ONLY_ENV,
        nebula_core::GIT_CEILING_DIRECTORIES_ENV,
        nebula_core::HOME_ENV,
        nebula_core::PWD_ENV,
        nebula_core::XDG_STATE_HOME_ENV,
        crate::output::terminal::NO_COLOR_ENV,
        crate::output::terminal::TERM_ENV,
        VISUAL_ENV,
        EDITOR_ENV,
        NEBULA_TEST_GIT_DEADLINE_MS_ENV,
    ]
    .into_iter()
    .collect();
    assert_eq!(names, expected);

    for var in &vars {
        assert!(!var.name.trim().is_empty());
        assert!(
            !var.meaning.trim().is_empty(),
            "empty meaning for {}",
            var.name
        );
        assert!(
            !var.default.trim().is_empty(),
            "empty default for {}",
            var.name
        );
        assert!(
            var.overridden_by.is_none_or(|flag| !flag.trim().is_empty()),
            "empty override flag for {}",
            var.name
        );
    }
}

#[test]
fn readme_table_extraction_and_replacement_detect_drift() {
    let generated = readme_table();
    let doc = format!("# Intro\n\n{README_TABLE_BEGIN}\nstale\n{README_TABLE_END}\n\n# Tail\n");
    assert_ne!(extract_readme_table(&doc).unwrap(), generated);

    let updated = replace_readme_table(&doc, &generated).unwrap();
    assert_eq!(extract_readme_table(&updated).unwrap(), generated);
    assert!(updated.starts_with("# Intro\n\n"));
    assert!(updated.ends_with("\n# Tail\n"));
}
