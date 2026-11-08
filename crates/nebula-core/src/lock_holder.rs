//! Who holds a lock, as a value.
//!
//! A leaf below both `lock` (which writes and reads the holder record) and
//! `error` (which names the holder in [`crate::Error::Locked`]), so neither
//! has to import the other for this type.

use std::time::SystemTime;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Who holds a lock, as the holder recorded it in the lock file.
///
/// **Diagnostic only** (STD-03 §R7, §R14): it names who to wait for in an
/// [`Error::Locked`](crate::Error::Locked) and nothing more. Whether the lock
/// is held is decided by `flock` alone, never by this record; the PID is
/// never signalled and never probed for liveness. It can be stale — a holder
/// that crashed leaves its record behind until the next holder overwrites it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockHolder {
    /// The holding process's id.
    pub pid: u32,
    /// When it took the lock, to the second.
    pub since: SystemTime,
    /// Which writer it is: `neb edit a-node`, `desktop capture`, or
    /// `nebula` when the writer did not say.
    pub label: String,
}

impl LockHolder {
    /// [`Self::since`] as RFC 3339 in UTC, as the record stores it.
    pub fn since_rfc3339(&self) -> String {
        OffsetDateTime::from(self.since)
            .format(&Rfc3339)
            .unwrap_or_else(|_| "an unreadable time".to_owned())
    }
}
