//! The CLI composition root and process exit boundary.

#![expect(
    clippy::disallowed_methods,
    reason = "CLI composition reads the process environment and working directory to build Locations"
)]

use super::failure::Failure;
use super::{parse_from, run};
use crate::output::{self, errln};
use crate::render;
use nebula_core::{CorpusLock, Locations};
use std::process::ExitCode;

/// Parse the command line, run it, and turn the outcome into an exit code.
///
/// A refusal exits 2 when it is a usage error and 1 otherwise, as
/// [`render::Refusal::exit`] says, in either mode. Under `--json` it is one
/// line of JSON on stderr, so stdout still holds nothing but a verb's
/// payload; without, it is the prose it always was.
///
/// A stdout that closed under the verb changes nothing here: the verb ran to
/// its end, and exits as it decided (see [`output`]). A write to stdout that
/// failed some other way is a refusal of its own, reported once the verb is
/// done and only if the verb did not refuse first.
pub(crate) fn main() -> ExitCode {
    let cli = parse_from(std::env::args_os()).unwrap_or_else(|e| e.exit());
    CorpusLock::label_process(&cli.lock_label());
    let locations =
        Locations::from_reader(|name| std::env::var_os(name), std::env::current_dir().ok());
    #[cfg(debug_assertions)]
    let _deadline = test_git_deadline();
    let json = cli.json;
    let outcome = locations
        .validate()
        .map_err(Failure::from)
        .and_then(|()| run(cli, &locations));
    let flushed = output::finish();
    match outcome.and_then(|code| flushed.map(|()| code).map_err(Failure::from)) {
        Ok(code) => code,
        Err(Failure(refused)) => {
            if json {
                errln!("{}", refused.json());
            } else {
                errln!("{} {}", render::error_label(), refused.prose());
            }
            refused.exit.into()
        }
    }
}

/// Debug builds only: every git command this process runs gets
/// `$NEBULA_TEST_GIT_DEADLINE_MS` milliseconds instead of its named deadline,
/// so an end-to-end test of the timeout does not wait two minutes. Read here
/// rather than in core, which never reads the environment; git runs on this
/// thread, which the guard covers. Never read by a release build.
#[cfg(debug_assertions)]
fn test_git_deadline() -> Option<nebula_core::GitDeadlineOverride> {
    let ms = std::env::var(crate::env::NEBULA_TEST_GIT_DEADLINE_MS_ENV)
        .ok()?
        .parse()
        .ok()?;
    Some(nebula_core::GitDeadlineOverride::new(
        std::time::Duration::from_millis(ms),
    ))
}
