//! Central registry of environment variables read by `neb` and `nebula-core`,
//! and generator for the environment-variable table in `README.md` (STD-01 §R25).

use nebula_core::EnvVar;
use std::fmt::Write as _;

/// Shell-quoted editor command used by `neb edit`, taking precedence over `EDITOR`.
///
/// Meaning: names the interactive editor command (split with `shlex`) for editing a node body; empty counts as unset.
/// Default: falls back to `EDITOR`; `neb edit` refuses if neither is set.
/// Overridden flag: none.
pub const VISUAL_ENV: &str = "VISUAL";

/// Fallback shell-quoted editor command used by `neb edit` when `VISUAL` is unset or empty.
///
/// Meaning: names the fallback interactive editor command (split with `shlex`); empty counts as unset.
/// Default: none (`neb edit` refuses if both `VISUAL` and `EDITOR` are unset or empty).
/// Overridden flag: none.
pub const EDITOR_ENV: &str = "EDITOR";

/// Debug builds only: overrides the supervised `git` command deadline in milliseconds for timeout tests.
///
/// Meaning: shortens the `git` timeout from the production deadlines (`GIT_DEADLINE` / `GIT_COMMIT_DEADLINE`); invalid values are ignored, and release builds never read it.
/// Default: `30000` ms (`120000` ms for `git commit`).
/// Overridden flag: none.
pub const NEBULA_TEST_GIT_DEADLINE_MS_ENV: &str = "NEBULA_TEST_GIT_DEADLINE_MS";

/// CLI-specific environment variables outside `nebula-core` and `output::terminal`.
const CLI_ENV_VARS: &[EnvVar] = &[
    EnvVar {
        name: VISUAL_ENV,
        meaning: "Shell-quoted editor command for `neb edit`, preferred over `EDITOR`; empty counts as unset.",
        default: "Falls back to `EDITOR`",
        overridden_by: None,
    },
    EnvVar {
        name: EDITOR_ENV,
        meaning: "Fallback shell-quoted editor command for `neb edit` when `VISUAL` is unset or empty.",
        default: "None (`neb edit` refuses if both are unset)",
        overridden_by: None,
    },
    EnvVar {
        name: NEBULA_TEST_GIT_DEADLINE_MS_ENV,
        meaning: "Debug builds only: supervised `git` deadline in ms for timeout tests; non-integers are ignored.",
        default: "30000 ms (120000 ms for `git commit`)",
        overridden_by: None,
    },
];

/// Opening marker enclosing the generated table in `README.md`.
pub const README_TABLE_BEGIN: &str = "<!-- env-vars:begin -->";

/// Closing marker enclosing the generated table in `README.md`.
pub const README_TABLE_END: &str = "<!-- env-vars:end -->";

/// Every environment variable read by `neb` and `nebula-core`, in documentation order.
pub fn all() -> Vec<EnvVar> {
    nebula_core::CORE_ENV_VARS
        .iter()
        .chain(crate::output::TERMINAL_ENV_VARS.iter())
        .chain(CLI_ENV_VARS.iter())
        .copied()
        .collect()
}

/// Escape markdown table cell separators and line breaks.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Render the Markdown environment-variable table for `README.md` from [`all`].
pub fn readme_table() -> String {
    let mut out = String::from(
        "| Variable | Meaning | Default | Overridden by |\n| --- | --- | --- | --- |\n",
    );
    for var in all() {
        let overridden = var
            .overridden_by
            .map_or_else(|| "-".to_owned(), |flag| format!("`{}`", cell(flag)));
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} |",
            cell(var.name),
            cell(var.meaning),
            cell(var.default),
            overridden,
        );
    }
    out
}

/// Extract the generated table body between [`README_TABLE_BEGIN`] and [`README_TABLE_END`].
pub fn extract_readme_table(doc: &str) -> Result<&str, String> {
    let (start, end) = table_span(doc)?;
    Ok(&doc[start..end])
}

/// Replace the generated table body between [`README_TABLE_BEGIN`] and [`README_TABLE_END`].
pub fn replace_readme_table(doc: &str, table: &str) -> Result<String, String> {
    let (start, end) = table_span(doc)?;
    let mut updated = String::with_capacity(doc.len() + table.len());
    updated.push_str(&doc[..start]);
    updated.push_str(table);
    updated.push_str(&doc[end..]);
    Ok(updated)
}

fn table_span(doc: &str) -> Result<(usize, usize), String> {
    if doc.match_indices(README_TABLE_BEGIN).count() != 1 {
        return Err(format!(
            "expected exactly one `{README_TABLE_BEGIN}` marker in README.md"
        ));
    }
    if doc.match_indices(README_TABLE_END).count() != 1 {
        return Err(format!(
            "expected exactly one `{README_TABLE_END}` marker in README.md"
        ));
    }
    let begin = doc
        .find(README_TABLE_BEGIN)
        .ok_or_else(|| format!("missing `{README_TABLE_BEGIN}` marker in README.md"))?;
    let after_begin = begin + README_TABLE_BEGIN.len();
    let start = doc[after_begin..]
        .strip_prefix('\n')
        .map(|_| after_begin + 1)
        .ok_or_else(|| format!("`{README_TABLE_BEGIN}` must be followed by a newline"))?;
    let end = doc
        .find(README_TABLE_END)
        .ok_or_else(|| format!("missing `{README_TABLE_END}` marker in README.md"))?;
    if end < start {
        return Err(format!(
            "`{README_TABLE_END}` appears before `{README_TABLE_BEGIN}` in README.md"
        ));
    }
    Ok((start, end))
}
