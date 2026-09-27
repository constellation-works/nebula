//! The inbox: captures appended one line each to `inbox/YYYY-MM.md`, read
//! back as entries, and settled by striking the line through in place.
//! Nothing is ever removed from it.

use super::{Corpus, list_directory};
use crate::error::{Error, Result};
use crate::fs_impl::{
    append_private, create_private_dir_all, read_regular_text, write_private_atomic,
};
use crate::id::fnv;
use crate::stamp::{rfc3339_stamp, stamp};
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

impl Corpus {
    /// Append a capture to the current month's inbox file.
    ///
    /// Text that spans lines is joined onto one with [`capture_line`] rather
    /// than refused: the inbox holds one entry per line, and a refusal would
    /// lose the thought at the moment it arrived. Only text that is nothing
    /// but whitespace is refused, by [`validate_capture`].
    ///
    /// The line, with the newline that repairs a month file missing its last
    /// one, is built whole and handed to one append, so a crash can lose the
    /// capture but never leave half of it in the inbox.
    pub(crate) fn capture(&self, text: &str) -> Result<InboxEntry> {
        self.capture_at(text, &stamp())
    }

    /// [`Corpus::capture`], stamped `stamp` rather than now.
    ///
    /// The stamp decides the month file and seeds the entry id, so this is
    /// the seam a unit test fixes the capture clock at instead of racing the
    /// wall clock. It is crate-internal: nothing outside nebula can back-date
    /// a capture. `stamp` is an inbox stamp in either form an inbox line
    /// holds, and its first seven characters name the month file.
    pub(crate) fn capture_at(&self, text: &str, stamp: &str) -> Result<InboxEntry> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        let text = validate_capture(text)?;
        let dir = self.root.join("inbox");
        refuse_inbox_symlink(&dir)?;
        create_private_dir_all(&dir)?;
        refuse_inbox_symlink(&dir)?;
        let month = &stamp[..7];
        let path = dir.join(format!("{month}.md"));
        refuse_inbox_symlink(&path)?;
        // Read, and appended to below, only as a regular file: the append
        // opens `O_NOFOLLOW` (`append_private`), so the lstat above and the
        // open share one resolution, and a FIFO or a device here is refused
        // rather than waited on (STD-05 §R7).
        let existing = read_regular_text(&path)?.unwrap_or_default();
        let inbox = self.inbox()?;
        let id = unique_entry_id(&format!("{stamp}{text}"), &inbox)?;
        let line = existing.lines().count();
        refuse_inbox_symlink(&dir)?;
        refuse_inbox_symlink(&path)?;
        let repair = if !existing.is_empty() && !existing.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        append_private(
            &path,
            format!("{repair}- [{id}] {stamp} {text}\n").as_bytes(),
        )?;
        Ok(InboxEntry {
            id,
            at: rfc3339_stamp(stamp).unwrap_or_else(|| stamp.to_string()),
            stamp: stamp.to_string(),
            text,
            file: path,
            line,
        })
    }

    /// Inbox entries that have not been promoted or dropped.
    ///
    /// A line still live on disk whose recorded promotion has already written
    /// its node is left out: that entry is settled, and the next writer will
    /// strike it (see `pending.rs`).
    pub fn inbox(&self) -> Result<Inbox> {
        Ok(Inbox(self.waiting(self.live_entries()?)?))
    }

    /// Every inbox line that is not struck through, as it stands on disk.
    pub(crate) fn live_entries(&self) -> Result<Vec<InboxEntry>> {
        let mut out = Vec::new();
        for file in self.inbox_files()? {
            for (lineno, line) in read_inbox_file(&file)?.lines().enumerate() {
                if let Some(e) = InboxEntry::parse(line, &file, lineno) {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    /// The inbox's month files, oldest first. None when there is no inbox
    /// yet, which is a corpus nothing has been captured into.
    fn inbox_files(&self) -> Result<Vec<PathBuf>> {
        let dir = self.root.join("inbox");
        refuse_inbox_symlink(&dir)?;
        let listing = match list_directory(&dir) {
            Ok(listing) => listing,
            Err(Error::IoAt { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let mut files = Vec::new();
        for entry in listing.into_strict()? {
            // A month name is a candidate even when it is not a regular
            // file: the safe reader must report it, never silently omit it.
            if is_inbox_month_filename(&entry.file_name()) {
                files.push(entry.path());
            }
        }
        files.sort();
        Ok(files)
    }

    /// Find one live inbox entry by id.
    ///
    /// An id that only a struck-through line carries is refused with how
    /// that line says it was settled, rather than as an id nobody captured:
    /// the record is there, so the refusal can point at what became of it.
    pub(crate) fn inbox_entry(&self, id: &str) -> Result<InboxEntry> {
        crate::id::require_safe_id(id)?;
        if let Some(entry) = self.inbox()?.0.into_iter().find(|e| e.id == id) {
            return Ok(entry);
        }
        Err(match self.settlement(id)? {
            Some(settlement) => Error::InboxEntrySettled {
                id: id.to_string(),
                settlement,
            },
            None => Error::NoSuchInboxEntry(id.to_string()),
        })
    }

    /// How the settled entry `id` was settled, if a struck-through line
    /// records it.
    ///
    /// Ids are unique among live entries only, so one id can be settled more
    /// than once over the life of a corpus. The latest line wins: it is the
    /// capture whose id was most recently on offer.
    fn settlement(&self, id: &str) -> Result<Option<Settlement>> {
        let mut found = None;
        for file in self.inbox_files()? {
            for line in read_inbox_file(&file)?.lines() {
                if let Some((settled, settlement)) = Settlement::parse(line)
                    && settled == id
                {
                    found = Some(settlement);
                }
            }
        }
        Ok(found)
    }

    /// Settle an inbox entry by striking it through in place.
    ///
    /// The line is never removed. What an idea looked like before it had a name
    /// is part of its history, and a dropped capture is a record of a road not
    /// taken rather than a mistake to erase.
    pub(crate) fn settle_inbox(&self, entry: &InboxEntry, outcome: &str) -> Result<()> {
        self.locations
            .write_gate(crate::locations::WriteIntent::Ordinary)?;
        // Guard against settling an entry that belongs to a different corpus,
        // which would silently strike a line in someone else's inbox. The
        // file is the caller's to set, so it must be exactly
        // `<root>/inbox/<month>.md`, the one layout whose two entries below
        // the root are judged by `lstat` next. A prefix match would also take
        // `inbox/../config.yaml`, or a month file under a symlinked
        // subdirectory of the inbox, and write outside it (STD-05 §R6).
        let inbox = self.root.join("inbox");
        let in_layout = entry.file.parent() == Some(inbox.as_path())
            && entry.file.file_name().is_some_and(is_inbox_month_filename);
        if !in_layout {
            return Err(Error::InboxEntryForeign {
                id: entry.id.clone(),
                file: entry.file.clone(),
                inbox,
            });
        }
        refuse_inbox_symlink(&inbox)?;
        refuse_inbox_symlink(&entry.file)?;
        let content = read_inbox_file(&entry.file)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        // Two ways for the line to have moved, told apart: the file got
        // shorter, or the line now holds some other text.
        let Some(slot) = lines.get_mut(entry.line) else {
            return Err(Error::InboxEntryMissing {
                id: entry.id.clone(),
                file: entry.file.clone(),
                line: entry.line,
            });
        };
        if !slot.starts_with(&format!("- [{}]", entry.id)) {
            return Err(Error::InboxEntryChanged {
                id: entry.id.clone(),
                file: entry.file.clone(),
                line: entry.line,
            });
        }
        // The stamp goes back as the line held it, so settling a legacy
        // entry never rewrites when it was captured (STD-02 §R16).
        *slot = format!(
            "- ~~[{}] {} {}~~ {outcome}",
            entry.id, entry.stamp, entry.text
        );
        refuse_inbox_symlink(&self.root.join("inbox"))?;
        refuse_inbox_symlink(&entry.file)?;
        write_private_atomic(&entry.file, lines.join("\n") + "\n")?;
        Ok(())
    }
}

/// The one inbox line a piece of captured text is stored as.
///
/// Every line break, `\n` or `\r` alike, becomes a single space, together
/// with the whitespace around it, and blank lines vanish, so text pasted or
/// piped in over several lines reads as the sentence it was. Whitespace
/// inside a line is the author's and stays; the ends are trimmed. The result
/// never holds a line break, which is what keeps the inbox at one entry per
/// line, and it is empty exactly when the text was nothing but whitespace.
///
/// [`Corpus::capture`] applies it, so the CLI, the desktop app and anything
/// else that captures store the same line for the same text.
pub(crate) fn capture_line(text: &str) -> String {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The one rule for what may be captured: `text` as [`capture_line`] stores
/// it, or [`Error::EmptyCapture`] when that is nothing.
///
/// [`Corpus::capture`] applies it, and a surface that may create a corpus
/// for the capture calls it first, so blank text is refused before any
/// directory exists rather than after one was made for it (STD-02 §R24,
/// §R34).
pub(crate) fn validate_capture(text: &str) -> Result<String> {
    let line = capture_line(text);
    if line.is_empty() {
        return Err(Error::EmptyCapture);
    }
    Ok(line)
}

/// One inbox month candidate's text, or a refusal naming the entry. It is
/// read only as a regular file.
fn read_inbox_file(path: &Path) -> Result<String> {
    read_regular_text(path)?
        .ok_or_else(|| Error::io_at("reading", path, std::io::ErrorKind::NotFound.into()))
}

/// Check the named inbox entry itself, without resolving the corpus root. A
/// symlinked root is a supported way to reach a corpus, but a symlink planted
/// at `inbox/` or a month file must not redirect an inbox write.
fn refuse_inbox_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(Error::InboxSymlink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io_at("inspecting", path, error)),
    }
}

/// Whether a file name is one of the inbox's `YYYY-MM.md` month files.
pub(crate) fn is_inbox_month_filename(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let bytes = name.as_bytes();
    bytes.len() == 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && matches!(&bytes[5..7], [b'0', b'1'..=b'9'] | [b'1', b'0'..=b'2'])
        && &bytes[7..] == b".md"
}

/// Every capture still waiting to be promoted or dropped.
///
/// Serializes as the bare list, which is the shape `neb inbox --json` has
/// always had.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Inbox(pub Vec<InboxEntry>);

impl Inbox {
    /// The earliest other waiting entry that says what `entry` says.
    ///
    /// "Says" is equality after folding case and collapsing whitespace, so a
    /// thought typed twice with different capitals or spacing is caught and a
    /// reworded one is not: that is `near`'s job, and it searches nodes. Only
    /// live entries are here, so a settled capture never counts.
    pub fn same_as(&self, entry: &InboxEntry) -> Option<&InboxEntry> {
        let said = fold(&entry.text);
        self.0
            .iter()
            .find(|other| other.id != entry.id && fold(&other.text) == said)
    }
}

/// Text as the duplicate check compares it: lowercase NFC, one space between words.
fn fold(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .collect()
}

/// What became of a settled inbox entry, as its struck-through line records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    /// Promoted into the node with this id.
    Promoted(String),
    /// Dropped without becoming a node.
    Dropped,
}

impl Settlement {
    /// The id and outcome of a line [`Corpus::settle_inbox`] wrote, which is
    /// `- ~~[<id>] <at> <text>~~ <outcome>`. A struck line whose outcome is
    /// neither of the two that verb writes was edited by hand, and is not
    /// read as either.
    fn parse(line: &str) -> Option<(&str, Self)> {
        let rest = line.strip_prefix("- ~~[")?;
        let (id, rest) = rest.split_once("] ")?;
        let (_, outcome) = rest.rsplit_once("~~")?;
        let outcome = outcome.trim();
        if outcome == "dropped" {
            return Some((id, Self::Dropped));
        }
        let node = outcome.strip_prefix("->")?.trim();
        (!node.is_empty()).then(|| (id, Self::Promoted(node.to_string())))
    }
}

impl std::fmt::Display for Settlement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Promoted(node) => write!(f, "promoted to `{node}`"),
            Self::Dropped => f.write_str("dropped"),
        }
    }
}

/// One unprocessed capture.
///
/// Where the line lives is how [`Corpus::settle_inbox`] finds it again, and is
/// a detail of this corpus rather than part of the entry, so it stays out of
/// the serialized form.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct InboxEntry {
    /// Short id, unique among the corpus's live inbox entries.
    pub id: String,
    /// When it was captured: RFC 3339 with the offset the stamp was taken
    /// at, ending in `Z` when the local offset could not be read. A legacy
    /// `YYYY-MM-DDTHH:MM` stamp reads as local time with the offset this
    /// machine has for that instant. A stamp that is neither, edited in by
    /// hand, is passed through as written.
    pub at: String,
    /// The stamp as the inbox line holds it, which settling writes back
    /// unchanged.
    #[serde(skip)]
    pub(crate) stamp: String,
    /// What you wrote.
    pub text: String,
    /// Which inbox file it lives in.
    #[serde(skip)]
    pub file: PathBuf,
    /// Zero-based line within that file.
    #[serde(skip)]
    pub line: usize,
}

impl InboxEntry {
    fn parse(line: &str, file: &Path, lineno: usize) -> Option<Self> {
        // `- ~~...~~` is a settled entry: promoted or dropped, kept for the record.
        let rest = line.strip_prefix("- [")?;
        let (id, rest) = rest.split_once("] ")?;
        let (stamp, text) = rest.split_once(' ')?;
        Some(Self {
            id: id.to_string(),
            at: rfc3339_stamp(stamp).unwrap_or_else(|| stamp.to_string()),
            stamp: stamp.to_string(),
            text: text.trim().to_string(),
            file: file.to_path_buf(),
            line: lineno,
        })
    }
}

/// A short id unique in the live inbox, or a refusal when all ids are occupied.
fn unique_entry_id(seed: &str, inbox: &Inbox) -> Result<String> {
    let used: HashSet<&str> = inbox.0.iter().map(|entry| entry.id.as_str()).collect();
    let mut h = fnv(seed);
    for _ in 0..64 {
        let id = format!("{:04x}", (h & 0xffff) as u16);
        if !used.contains(id.as_str()) {
            return Ok(id);
        }
        h = fnv(&format!("{h}"));
    }

    // Hashing keeps the ordinary path short and makes ids hard to predict from
    // their position. Once that bounded path collides, walk the finite id space
    // rather than returning an occupied candidate. A full inbox is unusual but
    // valid input, and refusing it is the only unambiguous result.
    for candidate in 0..=u16::MAX {
        let id = format!("{candidate:04x}");
        if !used.contains(id.as_str()) {
            return Ok(id);
        }
    }
    Err(Error::InboxIdsExhausted)
}
