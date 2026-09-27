//! Settings a verb writes, the machine's and the corpus's, and the commit
//! that may follow any write.

use super::DroppedLegacy;
use crate::config::{CommitSetting, ObservatoryRoot};
use crate::error::Result;
use crate::store::{CommitOutcome, Corpus};
use std::path::Path;

/// Record where the Observatory checkout is on this machine, in
/// `~/.config/nebula/observatory-root`. Nothing under the corpus changes, so
/// no corpus lock is taken: the checkout's path belongs to the machine, and
/// the corpus travels between machines. The machine-setting lock is, so this
/// write is serialized with every other one to `~/.config/nebula`.
///
/// Returns the setting as `corpus` now resolves it, which is the machine
/// setting unless `$OBSERVATORY_ROOT` outranks it.
pub fn set_observatory_root(corpus: &Corpus, dir: &Path) -> Result<ObservatoryRoot> {
    let _settings = Corpus::lock_machine_settings(corpus.locations())?;
    Corpus::write_observatory_root_config(corpus.locations(), dir)?;
    corpus.observatory_root()
}

/// Remove the legacy `observatory_root` key from the corpus's `config.yaml`,
/// the one place a machine path was ever stored in the corpus. When the file
/// does not carry it, nothing is written, and the result says so: the caller
/// must not report a removal that did not happen (STD-01 §R30).
pub fn drop_legacy_observatory_root(corpus: &mut Corpus) -> Result<DroppedLegacy> {
    let _lock = corpus.lock()?;
    let removed = corpus.drop_legacy_observatory_root()?;
    Ok(DroppedLegacy {
        removed,
        setting: corpus.observatory_root()?,
    })
}

/// Record in the corpus's `config.yaml` whether each write is committed.
///
/// Returns the setting as it now stands.
pub fn set_commit(corpus: &mut Corpus, enabled: bool) -> Result<CommitSetting> {
    let _lock = corpus.lock()?;
    corpus.set_commit(enabled)?;
    Ok(corpus.commit_setting())
}

/// Commit the corpus after a successful write, as `neb <verb> <ids>`.
///
/// Does nothing unless `commit: true` is set in `config.yaml` and the root is
/// inside a git work tree, and the [`CommitOutcome`] says which of those it
/// was. Stages and commits only `nodes/`, `inbox/`, `config.yaml` and the
/// generated `.gitignore` under the root, by pathspec, so anything else staged
/// in the repository stays staged and out of the commit. Never pushes. A
/// repository git cannot read is [`Error::Git`](crate::Error::Git), not a skipped commit. Called
/// after the write it records, which stays on disk whatever happens here.
pub fn commit(corpus: &Corpus, verb: &str, ids: &[&str]) -> Result<CommitOutcome> {
    // Two commits racing would race on git's index. Taking the lock here
    // covers a caller that commits on its own; a caller that already holds it
    // from the write this records re-enters, which is the point.
    let _lock = corpus.lock()?;
    corpus.commit(verb, ids)
}
