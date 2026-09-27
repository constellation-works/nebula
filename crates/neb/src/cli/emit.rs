//! What a handler writes besides the payload it renders: JSON on stdout, the
//! notices and warnings on stderr, and the report of the commit after a write.
//! Everything goes out through [`crate::output`].

use super::failure::{Failure, shell_word};
use crate::output::{self, errln, outln};
use crate::render::{self, json};
use nebula_core::verb::{CommitPolicy, RootWarning, WriteOptions};
use nebula_core::{
    CloseTag, CommitOutcome, Corpus, Error, OBSERVATORY_ROOT_ENV, ObservatoryLink, ObservatoryRoot,
    ObservatorySource,
};
use std::io::Write;
use std::path::Path;

/// After a write that added tags, say on stderr which of them read as a
/// variant of a tag already in use, in every output mode.
///
/// Advice, never a refusal: there is no declared list to refuse against, and
/// `check` rule 11 reports the same drift later. Core read them once the
/// write lock was released, and a corpus it could not read for the
/// comparison costs the note, not the command.
pub(super) fn note_close_tags(close: &[CloseTag]) {
    for c in close {
        errln!(
            "note: tag {} is close to {} ({} node{})",
            c.tag,
            c.near,
            c.nodes,
            if c.nodes == 1 { "" } else { "s" }
        );
    }
}

/// What every mutating arm needs to decide whether to commit and how to say
/// so: `--no-commit` waives the setting once, and `--json` leaves the
/// `committed` notice out.
#[derive(Clone, Copy)]
pub(crate) struct CommitOpts {
    pub(crate) skip: bool,
    pub(crate) json: bool,
}

impl CommitOpts {
    /// How a verb writes: the default lock wait, and the commit unless
    /// `--no-commit` waived it.
    pub(super) fn options(self) -> WriteOptions {
        WriteOptions {
            commit: if self.skip {
                CommitPolicy::Skip
            } else {
                CommitPolicy::Configured
            },
            ..WriteOptions::default()
        }
    }
}

/// Say on stderr, in every mode, when a machine setting will keep commands
/// from finding the corpus about to be created at `target`.
pub(super) fn warn_root(warning: &RootWarning, target: &Path) {
    match warning {
        RootWarning::DefaultWhileConfigured {
            setting,
            configured,
        } => errln!(
            "warning: creating ~/.nebula while {} points to {}",
            setting.display(),
            configured.display()
        ),
        RootWarning::Shadowed {
            setting,
            configured,
        } => {
            // The command that repoints the machine default, never a hand
            // edit: absolute, since the setting is read from every
            // directory, and quoted, so it runs as printed (STD-02 §R26).
            // Made absolute lexically, as capture's note is.
            let target = std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf());
            errln!(
                "warning: {} still points to {}, not {}; run `neb init {} --set-root --force` to point commands at this corpus",
                setting.display(),
                configured.display(),
                target.display(),
                shell_word(&target.to_string_lossy())
            );
        }
    }
}
/// Say where an `observatory` reference's record is on this machine: the
/// path on stdout, as part of what `cite --kind observatory` and `handoff`
/// report, or why it cannot be located on stderr. The path is the payload's
/// own, so the prose shows what `--json` carries (STD-01 §R6); `setting`
/// only says why there is none.
pub(super) fn print_record_location(setting: &ObservatoryRoot, link: &ObservatoryLink) {
    let record = &link.record;
    match setting.root.as_deref() {
        None => errln!(
            "{}",
            render::notice(&format!(
                "No observatory root set, so `{record}` cannot be located. \
                 Set one with `neb config observatory-root <DIR>` or \
                 ${OBSERVATORY_ROOT_ENV}."
            ))
        ),
        Some(dir) => match &link.path {
            Some(path) => outln!("{}", render::dim(&path.display().to_string())),
            None => errln!(
                "{}",
                render::notice(&format!(
                    "`{record}` does not resolve under {}; `check` will keep \
                     saying so until the checkout has it.",
                    dir.display()
                ))
            ),
        },
    }
}

/// Say on stderr, in every mode, when the observatory root a verb resolves
/// records against is the legacy `observatory_root` key in `config.yaml`.
///
/// The key is deprecated (STD-01 §R35): it is one machine's path in a file
/// every machine shares, and it answers only while neither
/// `$OBSERVATORY_ROOT` nor this machine's setting does.
pub(super) fn warn_legacy_observatory_root(corpus: &Corpus, setting: &ObservatoryRoot) {
    if setting.source == ObservatorySource::Config {
        errln!(
            "warning: `observatory_root` in {} is deprecated, and the release after 0.2.0 stops \
             reading it; set this machine's own with `neb config observatory-root <DIR>`",
            corpus.root().join("config.yaml").display()
        );
    }
}

/// The nudge after a reference written without `--note`, on stderr.
pub(super) fn print_bare_note() {
    errln!(
        "{}",
        render::notice("No note. Add one saying why it is here, or this is a link that rots.")
    );
}

/// Write a result's [`render::Notice`] to stderr, unless `--json` leaves it
/// out: counts and hints are for a person, while an empty or cut result is
/// said in every mode (STD-01 §R16, §R34).
pub(super) fn notify(json: bool, notice: Option<render::Notice>) {
    if let Some(line) = notice.and_then(|n| n.line(json)) {
        errln!("{line}");
    }
}

/// Say on stderr what the commit after a write did: the notice is not the
/// verb's payload (STD-01 §R12), so `E=$(neb capture -q …)` holds the id
/// alone. `--json` leaves the `committed` notice out.
///
/// Called after the verb has printed its own result, because the write has
/// already landed and a refused commit, returned here, must never read as
/// the write failing. `root` names the corpus in the note a commit asked for
/// and not made gets.
pub(super) fn report_commit(
    root: &Path,
    opts: CommitOpts,
    commit: Option<nebula_core::Result<CommitOutcome>>,
) -> std::result::Result<(), Failure> {
    match commit.transpose()? {
        Some(CommitOutcome::Committed(done)) if !opts.json => {
            let short = done.hash.get(..7).unwrap_or(&done.hash);
            errln!("{}", render::notice(&format!("committed {short}")));
        }
        // Asked for and not done: said on stderr in every mode, so the
        // payload on stdout reads the same either way.
        Some(CommitOutcome::NotARepository) => errln!(
            "note: not committed: {} is not inside a git work tree",
            root.display()
        ),
        Some(
            CommitOutcome::Committed(_) | CommitOutcome::Disabled | CommitOutcome::NothingToCommit,
        )
        | None => {}
    }
    Ok(())
}

/// Write rendered text to `out`. On stdout this never fails: the output
/// layer absorbs a closed pipe, so the commit after it still runs.
pub(super) fn say(out: &mut impl Write, text: &str) -> std::result::Result<(), Failure> {
    out.write_all(text.as_bytes())
        .map_err(output::StdoutFailed::from)?;
    Ok(())
}

/// Cut a listing to `--limit`, when one was given, and return how many
/// matched before the cut and whether it dropped any, so the text can say
/// how much was left out. `--json` puts both in the [`json::List`] envelope
/// whenever the flag is given (STD-01 §R34). The flag is at least
/// [`MIN_COUNT`], so a cut never empties a listing that had anything in it.
pub(super) fn cap<T>(items: &mut Vec<T>, limit: Option<usize>) -> (usize, bool) {
    let total = items.len();
    if let Some(n) = limit {
        items.truncate(n);
    }
    (total, items.len() < total)
}

/// Pretty JSON on stdout, which is what `--json` means everywhere.
pub(super) fn out_json<T: serde::Serialize>(v: &T) -> std::result::Result<(), Failure> {
    outln!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// `node` as `show --json` prints it, over the corpus as it is now: what
/// `edit --json` and `note --json` print once their write is committed.
/// Core read it with the lock released (STD-03 §R1).
pub(super) fn out_node_view(
    view: nebula_core::Result<nebula_core::NodeView>,
) -> std::result::Result<(), Failure> {
    out_json(&json::NodeView::from(&view?))
}

/// Send a rendered report to a file, or print it, per `--out`.
#[allow(
    clippy::disallowed_methods,
    reason = "`--out` names a report file of the user's choosing, not nebula state, so it is \
              written where and how they asked rather than through the private durable helper"
)]
pub(super) fn write_report(out: Option<&Path>, text: &str) -> std::result::Result<(), Failure> {
    match out {
        Some(path) => {
            std::fs::write(path, format!("{text}\n"))
                .map_err(|e| Error::io_at("writing", path, e))?;
            // stdout stays empty, so the file is named on stderr, in every
            // mode: a write says what it wrote (STD-01 §R30).
            errln!("{}", render::notice(&format!("wrote {}", path.display())));
        }
        None => outln!("{text}"),
    }
    Ok(())
}
