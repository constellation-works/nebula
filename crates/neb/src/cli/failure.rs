//! The refusals the CLI exits on: [`Failure`], and the CLI's own errors that
//! become one.

use crate::output;
use crate::render;
use nebula_core::Error;

/// A refusal the CLI exits on. Core errors become one through [`render`], so
/// the advice that names commands stays in the crate that has commands.
pub(crate) struct Failure(pub(super) render::Refusal);

/// Refusals specific to the terminal-owned editor flow.
#[derive(Debug, thiserror::Error)]
pub(super) enum EditorError {
    #[error("neither $VISUAL nor $EDITOR names an editor")]
    NotConfigured,
    #[error("editor command `{0}` is empty or has unmatched quotes")]
    InvalidCommand(String),
    #[error("could not start editor `{editor}`: {source}")]
    Start {
        editor: String,
        source: std::io::Error,
    },
    #[error("editor `{0}` exited unsuccessfully; the node was not changed")]
    Unsuccessful(String),
}

impl EditorError {
    /// The refusal's `code` under `--json`; exhaustive for the same reason
    /// as [`Error::code`].
    fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "editor_not_configured",
            Self::InvalidCommand(_) => "editor_invalid_command",
            Self::Start { .. } => "editor_start",
            Self::Unsuccessful(_) => "editor_unsuccessful",
        }
    }
}

/// A report destination that would mutate the corpus being reviewed.
#[derive(Debug, thiserror::Error)]
pub(super) enum ReportError {
    #[error("report destination {} resolves inside corpus {}; choose a file outside the corpus", path.display(), root.display())]
    InCorpus {
        path: std::path::PathBuf,
        root: std::path::PathBuf,
    },
}

impl From<ReportError> for Failure {
    fn from(e: ReportError) -> Self {
        let code = match &e {
            ReportError::InCorpus { .. } => "report_in_corpus",
        };
        Self::of(code, e.to_string())
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self(render::refusal(&e))
    }
}

impl From<serde_json::Error> for Failure {
    fn from(e: serde_json::Error) -> Self {
        Self::of("json", format!("could not write JSON: {e}"))
    }
}

impl From<output::StdoutFailed> for Failure {
    fn from(e: output::StdoutFailed) -> Self {
        Self(e.into())
    }
}

/// Input `neb triage` cannot act on.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KeyError {
    /// A line that is not a triage key.
    #[error("`{0}` is not a triage key; use p, a candidate number, t, d, s or q (? lists them)")]
    Unknown(String),
    /// Input ended while a title for `entry` was waiting to be used: after
    /// `t <title>`, or after `t` alone, before its line (STD-01 §R27).
    #[error("{}", title_lost(entry, title.as_deref()))]
    TitleLost {
        entry: String,
        title: Option<String>,
    },
}

/// [`KeyError::TitleLost`]'s message, which says whether the title arrived.
fn title_lost(entry: &str, title: Option<&str>) -> String {
    match title {
        Some(title) => format!(
            "input ended before the title `{title}` was used for `{entry}`; nothing was promoted"
        ),
        None => {
            format!("input ended before the title for `{entry}` was given; nothing was promoted")
        }
    }
}

impl KeyError {
    /// The refusal's `code` under `--json`, exhaustive like [`Error::code`].
    fn code(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "triage_key",
            Self::TitleLost { .. } => "triage_title_lost",
        }
    }

    /// The single verb that does what the lost input was for.
    fn hint(&self) -> Option<String> {
        match self {
            Self::Unknown(_) => None,
            Self::TitleLost { entry, title } => Some(format!(
                "Promote it with that title without triage:\n  neb promote {entry} --title {}",
                title
                    .as_deref()
                    .map_or_else(|| "\"...\"".to_owned(), shell_word)
            )),
        }
    }
}

impl From<KeyError> for Failure {
    fn from(e: KeyError) -> Self {
        Self(render::Refusal::new(e.code(), e.to_string()).hinted(e.hint()))
    }
}

/// `word` as one shell word: quoted when it needs quoting, so a command
/// printed for pasting runs as shown.
pub(crate) fn shell_word(word: &str) -> String {
    shlex::try_quote(word).map_or_else(|_| word.to_owned(), std::borrow::Cow::into_owned)
}

impl From<EditorError> for Failure {
    fn from(e: EditorError) -> Self {
        Self::of(e.code(), e.to_string())
    }
}

impl Failure {
    /// An error raised about one node, so the hint can name it.
    pub(super) fn about(e: &Error, node: &str) -> Self {
        Self(render::refusal_about(e, node))
    }

    /// Arguments that parsed but ask for nothing that can be done: a usage
    /// error, which exits 2 as clap's own do (STD-01 §R20). Clap's never get
    /// here: they exit 2, in prose, before `run`.
    pub(super) fn say(message: impl Into<String>) -> Self {
        Self(render::Refusal::usage("usage", message))
    }

    /// A refusal of the CLI's own, with its `snake_case` `code` under
    /// `--json`.
    fn of(code: &'static str, message: impl Into<String>) -> Self {
        Self(render::Refusal::new(code, message))
    }
}
