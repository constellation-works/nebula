// size: the Error enum is one cohesive item, a documented variant per failure (STD-02 §R3)

//! The one error type the library returns.
//!
//! Every public function in [`crate::ops`], [`crate::graph`], [`crate::check`],
//! [`crate::migrate`] and [`crate::store`] returns `Result<_, Error>`. The
//! variants are the invariants: a caller matches on the one it cares about
//! rather than parsing a sentence. The CLI turns them into messages and exit
//! codes, the desktop turns them into UI, and neither reads a string.
//!
//! The messages here name what is wrong and nothing else. Advice that tells
//! you which command to run next is presentation, so it lives in the consumer
//! that has commands to suggest.
//!
//! This file holds the enum. `class` gives each variant its stable code and
//! its [`ErrorClass`]; `phrase` holds the phrases its messages share.

mod class;
mod phrase;

#[cfg(test)]
mod tests;

pub use class::ErrorClass;
use phrase::{holder_phrase, shown_candidates};

use crate::fs_impl::EntryKind;
use crate::lock::LockHolder;
use crate::model::{EdgeType, FrontmatterProblem, Status};
use crate::store::Settlement;
use std::ffi::OsString;
use std::path::PathBuf;

/// The library's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong reading or changing a corpus.
///
/// `#[non_exhaustive]` because it is the library's public error (STD-02
/// §R11): a consumer outside this crate keeps a wildcard arm, so a new
/// variant is a new code rather than a broken build.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A graph query cannot give a complete answer while node files are bad.
    #[error("{count} unreadable node file(s): {details}")]
    UnreadableNodes {
        /// The number of failed files or directory entries.
        count: usize,
        /// Every affected path, code and cause.
        details: String,
    },
    /// No node with that id exists.
    #[error("no node `{0}`")]
    NoSuchNode(String),

    /// No inbox entry with that id, waiting or settled.
    #[error("no open inbox entry `{0}`")]
    NoSuchInboxEntry(String),

    /// The inbox entry was promoted or dropped already, and its
    /// struck-through line says which.
    #[error("`{id}` was already {settlement}")]
    InboxEntrySettled {
        /// The entry asked for.
        id: String,
        /// What became of it.
        settlement: Settlement,
    },

    /// There is no corpus where one was expected.
    #[error("no corpus at {}", .0.display())]
    NoCorpus(PathBuf),

    /// A history query needs a git work tree around the corpus, and there is
    /// none: no repository at or above the root, or git says the root is
    /// outside its work tree. A repository git cannot read is [`Error::Git`]
    /// instead.
    #[error("{} is not inside a git work tree", .0.display())]
    NotGitWorkTree(PathBuf),

    /// The node did not exist at the requested commit or date.
    #[error("no node `{node}` at `{revision}`")]
    NoNodeAtRevision {
        /// The node requested.
        node: String,
        /// The hash or date requested.
        revision: String,
    },

    /// The requested commit does not exist in the corpus's repository, as
    /// distinct from a commit that exists but did not hold the node.
    #[error("no commit `{revision}` in the corpus's repository")]
    UnknownRevision {
        /// The hash requested.
        revision: String,
    },

    /// An explicit root (`--root`) was empty. Every `join` built from it
    /// would silently resolve to the current directory, which is exactly the
    /// bug this refuses.
    #[error("an explicit corpus root cannot be empty")]
    EmptyRoot,

    /// This machine's corpus root setting, `~/.config/nebula/root`, is there
    /// but names nothing. Refused rather than skipped: skipping would send
    /// every command to `~/.nebula` without a word.
    #[error("the corpus root setting {} is empty", .0.display())]
    EmptyRootSetting(PathBuf),

    /// This machine's corpus root setting holds, or was about to be given, a
    /// relative path. The setting is read from whatever directory a command
    /// runs in, so a relative one would find or create a different corpus
    /// from each.
    #[error(
        "the corpus root {} must be an absolute path, not `{}`",
        .setting.as_ref().map_or_else(|| "argument".to_owned(), |p| format!("setting {}", p.display())),
        .root.display()
    )]
    RelativeRootSetting {
        /// The machine setting file it was read from; absent for an argument.
        setting: Option<PathBuf>,
        /// The path it holds or would hold.
        root: PathBuf,
    },

    /// `HOME` is not set, so there is no home directory to find this
    /// machine's settings or the default corpus under.
    #[error("HOME is not set")]
    HomeUnset,

    /// `HOME` is set, but not to valid UTF-8. Refused rather than read as
    /// unset: the directory it names is there, and treating it as missing
    /// would skip this machine's settings without saying so.
    #[error("HOME is not valid UTF-8: `{}`", .0.to_string_lossy())]
    HomeNotUnicode(OsString),

    /// A read-only environment switch had a value other than empty or one.
    #[error("{name} must be empty or `1`")]
    InvalidReadOnlyEnvironment {
        /// The rejected environment variable.
        name: &'static str,
    },

    /// An Orbit provenance value could not be stored without changing it.
    #[error("{name} is not valid UTF-8; provenance was not written")]
    InvalidOriginEnvironment {
        /// The rejected environment variable.
        name: &'static str,
    },

    /// This invocation is configured to refuse all corpus writes.
    #[error("this invocation is read-only; nothing was written")]
    ReadOnly,

    /// An Orbit run must identify the author of newly supplied words.
    #[error("an Orbit run requires an explicit author for these words; nothing was written")]
    ByRequired,

    /// Only the human may adopt an agent's proposed kill condition.
    #[error("only a human outside an Orbit run may confirm a kill condition; nothing was written")]
    HumanOnly,

    /// A machine-local root setting already names a different corpus. The
    /// caller must opt in to replacing it rather than redirecting commands
    /// silently.
    #[error("{} already points to {}, not {}", .path.display(), .configured.display(), .requested.display())]
    RootConfigConflict {
        /// The machine-local setting file.
        path: PathBuf,
        /// The corpus it currently names.
        configured: PathBuf,
        /// The corpus the caller asked to make the default.
        requested: PathBuf,
    },

    /// `init` was given a corpus location twice, as `--root` and as its
    /// path, and the two name different directories. Either could be meant,
    /// so neither is created.
    #[error("--root {} and the path {} name different corpora", .root.display(), .path.display())]
    RootAndPathDiffer {
        /// The location `--root` gave.
        root: PathBuf,
        /// The location the path argument gave.
        path: PathBuf,
    },

    /// An observatory root that is not an absolute path, given to be stored
    /// or read back from this machine's setting. A machine-wide setting is
    /// read from whatever directory a command runs in, so a relative one
    /// would name a different checkout from each.
    #[error(
        "the observatory root must be an absolute path, not `{}`{}",
        .root.display(),
        .setting.as_ref().map(|p| format!(" (in {})", p.display())).unwrap_or_default()
    )]
    RelativeObservatoryRoot {
        /// The path as given, possibly empty.
        root: PathBuf,
        /// The machine setting file it was read from, when it was read.
        setting: Option<PathBuf>,
    },

    /// A corpus root with no `config.yaml`, so which schema its files follow,
    /// its id and its settings are all unknown.
    ///
    /// Every verb but `migrate` refuses it rather than assume anything:
    /// synthesizing a config would stamp the current schema over files that
    /// may predate it, mint an id the corpus never had, and read a deleted
    /// `commit: true` as off. `migrate` reads the absence as a corpus from
    /// before the file existed, and `init` completes a root that holds no
    /// content yet. Absent means absent: a `config.yaml` that is a symlink,
    /// dangling or not, or any other entry but a regular file, is
    /// [`Error::NotRegularFile`], and one that cannot be read is
    /// [`Error::IoAt`].
    #[error("{} does not exist, so the corpus's schema and settings are unknown; nothing was read or written", .path.display())]
    MissingConfig {
        /// Where the config was expected.
        path: PathBuf,
    },

    /// The corpus on disk follows a schema this build does not read.
    #[error("{} is schema_version {found}, and this build understands {expected}", .path.display())]
    SchemaMismatch {
        /// The config file that declared it.
        path: PathBuf,
        /// What the corpus says it is.
        found: u32,
        /// What this build reads and writes.
        expected: u32,
    },

    /// A corpus that already declares this build's schema holds a node file
    /// that does not parse under it.
    ///
    /// Raised by [`crate::migrate`] alone, and before it writes anything.
    /// Migration reads nodes through a deliberately lenient v1 model that
    /// tolerates unknown keys, because a v1 file holds retired ones; content
    /// already at the current schema gets no such latitude, since the only
    /// thing that leniency could do there is drop a field this build does
    /// not recognise.
    #[error(
        "the corpus already declares schema_version {version}, and {source}; \
         migration will not rewrite a node it cannot read, so nothing was changed"
    )]
    CurrentSchemaUnreadable {
        /// The node file that would not parse.
        path: PathBuf,
        /// The schema the corpus declares, which is this build's own.
        version: u32,
        /// Why it would not parse, as the current model complained.
        source: Box<Error>,
    },

    /// A corpus whose `config.yaml` declares this build's schema holds a node
    /// in the v1 shape: it fails the current model, reads under the v1 one,
    /// and carries a key only v1 had.
    ///
    /// The likeliest cause is an older `neb` that stamped a fresh
    /// `config.yaml` over a corpus written before the file existed. That
    /// cannot be told apart for certain from a hand edit, so nothing guesses
    /// and nothing is rewritten; the refusal names the file and the keys, so
    /// whoever knows which it was can repair it.
    #[error(
        "{} is a v1 node (it carries {}), but {} declares schema_version {version}",
        .path.display(),
        .keys.iter().map(|key| format!("`{key}`")).collect::<Vec<_>>().join(", "),
        .config.display()
    )]
    V1NodeUnderCurrentSchema {
        /// The node file in the v1 shape.
        path: PathBuf,
        /// The `config.yaml` that declares the current schema.
        config: PathBuf,
        /// The schema it declares, which is this build's own.
        version: u32,
        /// The v1-only keys the node carries.
        keys: Vec<String>,
    },

    /// The edge would make a node its own ancestor. Genealogy is a DAG.
    #[error("that edge would make `{from}` its own ancestor")]
    Cycle {
        /// The node the edge starts at.
        from: String,
        /// The node the edge points at.
        to: String,
    },

    /// A node cannot link to itself.
    #[error("a node cannot link to itself")]
    SelfLoop,

    /// That exact edge is already recorded, or a new node names it twice.
    #[error("the edge `{from}` {kind} `{to}` already exists")]
    DuplicateEdge {
        /// The node the edge starts at.
        from: String,
        /// The relation.
        kind: EdgeType,
        /// The node the edge points at.
        to: String,
    },

    /// A status name that is not one of the four.
    #[error("`{0}` is not a status")]
    NotAStatus(String),

    /// An edge type name that is not one of the five.
    #[error("`{0}` is not an edge type")]
    NotAnEdgeType(String),

    /// An author label holding the punctuation a note line uses to carry its
    /// author in the prose, so the note would not parse back.
    #[error("`{0}` cannot be an author label: no parentheses, newlines or `: `")]
    InvalidAuthorLabel(String),

    /// A node file whose frontmatter cannot even be found, before any YAML
    /// is parsed.
    #[error("in {}: {problem}", .path.display())]
    MalformedFrontmatter {
        /// The node file, or the path it has in the tree it was read from.
        path: PathBuf,
        /// Which delimiter is missing.
        problem: FrontmatterProblem,
    },

    /// A new node names the same node as a parent and as the refuted idea it
    /// reopens. `reopens` is genealogy already, so the pair would be two
    /// parallel claims of one descent.
    #[error("`{0}` is named as both a parent and the node this reopens")]
    ParentAndReopens(String),

    /// The status must name what would falsify the idea, and none is named.
    #[error("`{0}` needs a kill condition first")]
    NeedsKill(Status),

    /// An empty kill condition is not a kill condition.
    #[error("a kill condition cannot be empty; that is the whole point of it")]
    EmptyKill,

    /// Confirming a kill condition as the human's own, on a node that names
    /// none.
    #[error("`{0}` has no kill condition to confirm")]
    NoKillToConfirm(String),

    /// A falsifier is content, not a field that `sharpen` may silently replace.
    #[error("kill condition is already `{0}`; it was not replaced")]
    KillAlreadySet(String),

    /// Refuting asserts the kill condition fired, and that has to be written.
    #[error("refuted needs a reason: say how the kill condition fired")]
    RefutedNeedsWhy,

    /// A reason given for a move to an open status. Only a closing has
    /// something for a reason to be the reason for.
    #[error("a reason only applies to refuted or abandoned, not `{0}`")]
    ReasonOnOpenStatus(Status),

    /// A ruled-out idea cannot quietly return to active work.
    #[error("a refuted node cannot simply reopen")]
    RefutedCannotReopen,

    /// A verb that closes a node was given one that is closed already. A
    /// refuted node carries a verdict and an abandoned one a reason, and
    /// closing either again would silently replace what `closed` records.
    #[error("`{id}` is already {status}, so it cannot be closed again")]
    AlreadyClosed {
        /// The node asked for.
        id: String,
        /// The closed status it is in.
        status: Status,
    },

    /// A node that names a kill condition cannot go back to `seed`. The kill
    /// is content, so the move would keep it, and a seed carrying one is a
    /// state no verb otherwise produces. Reopening such a node is a move to
    /// `hypothesis`.
    #[error("a node with a kill condition cannot go back to seed")]
    SeedWithKill,

    /// A triage action named a candidate parent by a number the current
    /// entry was not shown. Candidates count from 1, best first.
    #[error("there is no candidate {number}; {}", shown_candidates(*.shown))]
    NoSuchCandidate {
        /// The number asked for.
        number: usize,
        /// How many candidates the entry has.
        shown: usize,
    },

    /// A verb that takes its decisions one at a time from a person was asked
    /// for a machine-readable answer it has no way to give.
    #[error("`{0}` is interactive and has no JSON form")]
    Interactive(String),

    /// A capture that is nothing but whitespace. Refused before a corpus is
    /// opened or created; see [`crate::store::validate_capture`].
    #[error("nothing to capture")]
    EmptyCapture,

    /// A note that is nothing but whitespace.
    #[error("a note cannot be empty")]
    EmptyNote,

    /// Every one of the 65,536 inbox ids is held by a live entry.
    #[error("inbox id namespace exhausted; nothing captured")]
    InboxIdsExhausted,

    /// An inbox entry handed to a corpus whose inbox it is not in. Striking
    /// it would change another corpus's inbox.
    #[error("inbox entry `{id}` is in {}, not in this corpus's inbox {}", .file.display(), .inbox.display())]
    InboxEntryForeign {
        /// The entry.
        id: String,
        /// The month file the entry was read from.
        file: PathBuf,
        /// This corpus's inbox directory.
        inbox: PathBuf,
    },

    /// The entry's month file now ends before the line the entry was read
    /// from. Nothing was written.
    #[error(
        "inbox entry `{id}` was on line {} of {}, which now ends before it; nothing was written",
        .line + 1,
        .file.display()
    )]
    InboxEntryMissing {
        /// The entry.
        id: String,
        /// Its month file.
        file: PathBuf,
        /// The line it was read from, counted from 0.
        line: usize,
    },

    /// The line the entry was read from now holds something else. Nothing
    /// was written.
    #[error(
        "line {} of {} no longer holds inbox entry `{id}`; nothing was written",
        .line + 1,
        .file.display()
    )]
    InboxEntryChanged {
        /// The entry.
        id: String,
        /// Its month file.
        file: PathBuf,
        /// The line it was read from, counted from 0.
        line: usize,
    },

    /// `nodes/` is a symlink. A root reached through one is fine; node paths
    /// under a symlinked `nodes/` could reach a different tree.
    #[error("{} is a symlink; node operations require a real directory", .0.display())]
    NodesSymlink(PathBuf),

    /// `inbox/` or a month file under it is a symlink, which must not
    /// redirect an inbox write.
    #[error("{} is a symlink; inbox operations require real paths", .0.display())]
    InboxSymlink(PathBuf),

    /// A reference that must say where it is, given without a URI. Only a
    /// `discussion` may have none.
    #[error("a reference of kind `{kind}` needs a URI; only a discussion may have none")]
    UriRequired {
        /// The reference kind, as normalised.
        kind: String,
    },

    /// A point in a node's history that is neither a date nor a commit hash.
    #[error("`{0}` is not a YYYY-MM-DD date or a git revision")]
    InvalidAt(String),

    /// Two nodes claim the same id.
    #[error("duplicate node id `{0}`")]
    DuplicateId(String),

    /// A node file is already there.
    #[error("node `{0}` already exists")]
    NodeExists(String),

    /// A local reference points at a path that is not there.
    #[error("`{uri}` does not resolve from {}; local references are relative to nodes/", .from.display())]
    UnresolvedUri {
        /// The reference as written.
        uri: String,
        /// The directory it was resolved against.
        from: PathBuf,
    },

    /// A local reference names a place on one machine's filesystem — an
    /// absolute path or a `file:` URI — rather than a path relative to
    /// `nodes/`. Refused whether or not it exists here: the corpus is
    /// synced between machines, and on every other one it names nothing.
    #[error("`{0}` is an absolute local path; local references are relative to nodes/")]
    AbsoluteUri(String),

    /// A status move the lifecycle does not allow, judged from the pair of
    /// statuses alone. The refusals that exist today name themselves
    /// ([`Error::NeedsKill`], [`Error::RefutedCannotReopen`],
    /// [`Error::SeedWithKill`] and [`Error::AlreadyClosed`]); this is what
    /// a guard added later reports, and what a consumer matches on to mean
    /// "that move is not allowed" without enumerating the specific rules.
    #[error("`{from}` cannot become `{to}`")]
    InvalidTransition {
        /// Where the node is now.
        from: Status,
        /// Where it was asked to go.
        to: Status,
    },

    /// A named parent is not in the corpus.
    #[error("parent `{0}` does not exist")]
    MissingParent(String),

    /// A title with nothing in it that survives slugification.
    #[error("title `{0}` does not reduce to a usable id")]
    UnusableTitle(String),

    /// A new reference's `kind` is not in [`crate::check::REFERENCE_KINDS`],
    /// even lowercased. Carries the kind as it was given. Only new writes
    /// are held to the vocabulary; `check` reports an unexpected kind already
    /// on disk as a warning instead.
    #[error(
        "`{0}` is not an accepted reference kind; accepted kinds: {accepted}",
        accepted = crate::check_impl::REFERENCE_KINDS.join(", ")
    )]
    UnknownReferenceKind(String),

    /// An `observatory` reference's `uri` is not a bare record id.
    #[error(
        "`{0}` is not an Observatory record id: one of Q, H, T or R followed by digits, such as `Q<nnn>`"
    )]
    InvalidObservatoryId(String),

    /// An Observatory record id that does not resolve under the observatory
    /// root this machine has set. Raised where a write would claim the
    /// record exists, such as a hand-off; a plain citation stays a `check`
    /// warning, since the checkout may simply be behind.
    #[error("Observatory record `{record}` does not resolve under {}", .root.display())]
    UnresolvedObservatoryRecord {
        /// The record id, as it would be stored.
        record: String,
        /// The observatory root it was looked for under.
        root: PathBuf,
    },

    /// A user-supplied `--id` does not follow the slug rules: lowercase
    /// words joined by single dashes, no leading, trailing, or doubled
    /// dash, 60 characters or fewer.
    #[error(
        "`{0}` is not a valid id: ids are lowercase words joined by single dashes, 60 characters or fewer"
    )]
    InvalidId(String),

    /// An id that cannot name a file under `nodes/`: it holds a path
    /// separator, a `.`/`..` or root component, a control character, or
    /// surrounding whitespace. Refused before any read or write derives a
    /// path from it, whether it came from a caller or out of a node file
    /// somebody edited by hand.
    #[error(
        "`{0}` cannot be a node id: an id names one file under nodes/, so it cannot hold a path separator, `.`, `..`, or a control character"
    )]
    UnsafeId(String),

    /// A node file's name and the id it stores disagree. Nothing is read or
    /// written: a node's id decides where a write lands, so a file that
    /// claims to be another node would make the next verb overwrite that
    /// other node.
    #[error("{} stores the id `{id}`, which is not the node its file name names; nothing was read or written", .path.display())]
    IdMismatch {
        /// The file that was read.
        path: PathBuf,
        /// The id it stores.
        id: String,
    },

    /// An entry below the corpus root that nebula reads or writes by name —
    /// a node file, `config.yaml`, `.lock`, the pending-write record, an
    /// inbox month file, `.gitignore` — is not a regular file.
    ///
    /// Judged on the entry itself, never on what it points at, so a symlink
    /// is refused whether or not it leads anywhere and nothing at its far end
    /// is read, written or created. A FIFO or a device is refused before it
    /// is opened, so it can neither hang the reader nor feed it without end.
    /// The corpus root may itself be reached through a symlink; the entries
    /// beneath it may not (STD-05 §R6, §R7; see `4_decisions.md`).
    #[error("{} is {found}, not a regular file; nothing was read or written through it", .path.display())]
    NotRegularFile {
        /// The entry, as the corpus names it.
        path: PathBuf,
        /// What it is instead.
        found: EntryKind,
    },

    /// Another writer holds the corpus lock, or the machine-setting lock, and
    /// did not release it within the bounded wait. Nothing was written: the
    /// refusal comes before the op reads anything, so there is no
    /// half-applied change to undo.
    #[error("another nebula writer is holding {} ({}); nothing was written", .root.display(), holder_phrase(.holder.as_ref()))]
    Locked {
        /// The directory whose `.lock` is held: a corpus root, or
        /// `~/.config/nebula` for this machine's settings.
        root: PathBuf,
        /// Who holds it, as its holder record says, or `None` when there is
        /// no readable record. Diagnostic only: `None` is still a held lock.
        holder: Option<LockHolder>,
    },

    /// A body edited outside the lock — in `$EDITOR` — was about to be saved
    /// over a body another writer changed since it was loaded. Nothing was
    /// written: saving would erase that writer's change. A change to the
    /// frontmatter alone is not a conflict; the edit lands on top of it.
    #[error("`{0}`'s body changed while it was being edited; nothing was written")]
    EditConflict(String),

    /// A new body removes, reorders or changes a `## Notes` section the node
    /// already has. Notes are append-only; nothing was written.
    #[error(
        "an existing ## Notes section was removed, reordered, or changed; notes are append-only"
    )]
    NotesChanged,

    /// Text read from standard input was larger than the ceiling for what it
    /// is. Refused before the corpus is opened for writing, so nothing was
    /// written and no other writer waited on the read.
    #[error("{what} on standard input is larger than {limit} bytes; nothing was written")]
    InputTooLarge {
        /// What the text was for: `a capture` or `a body`.
        what: &'static str,
        /// The ceiling, in bytes.
        limit: usize,
    },

    /// `<root>/.pending` records a write that did not finish, and this build
    /// cannot read the record: it does not parse, or it names an operation
    /// this build does not know. Every writer refuses, and so does the inbox
    /// read, until a person has looked at it: finishing a write this build
    /// cannot read would be a guess, and ignoring it would expose the
    /// half-written state it records (STD-03 §R9).
    #[error("{} records an unfinished write that this build cannot read: {reason}; nothing was written", .path.display())]
    PendingWriteUnreadable {
        /// The record.
        path: PathBuf,
        /// Why it could not be read, as the parser put it.
        reason: String,
    },

    /// Standard input could not be read. There is no path to name, so the
    /// stream and what it was read for are named instead (STD-02 §R26).
    #[error("reading {what} from standard input: {source}")]
    IoStdin {
        /// What the text was for: `a capture`, `a body`, `a triage key`.
        what: &'static str,
        /// The operating system's reason.
        source: std::io::Error,
    },

    /// Text read from standard input is not UTF-8. Refused before the
    /// corpus is opened for writing, so nothing was written.
    #[error("{what} on standard input is not valid UTF-8; nothing was written")]
    StdinNotUtf8 {
        /// What the text was for: `a capture` or `a body`.
        what: &'static str,
    },

    /// `commit` is on, but the repository containing the corpus ignores it,
    /// so there is nothing git would ever record.
    #[error("{} is ignored by the git repository that contains it; nothing can be committed", .0.display())]
    CorpusIgnored(PathBuf),

    /// A git command did not succeed, or git could not read the repository
    /// around the corpus. A write it was meant to record is in place.
    #[error("git {context} failed in {}: {stderr}", .root.display())]
    Git {
        /// The corpus root the command ran in.
        root: PathBuf,
        /// Which command.
        context: String,
        /// What git said, cut at [`crate::GIT_OUTPUT_CAP`] with the cut
        /// marked.
        stderr: String,
    },

    /// A git command ran past its deadline, and it and everything it started
    /// were stopped. A write it was recording is in place, uncommitted; the
    /// corpus lock is free again.
    #[error("git {context} did not finish within {after:?} in {} and was stopped", .root.display())]
    GitTimedOut {
        /// The corpus root the command ran in.
        root: PathBuf,
        /// Which command.
        context: String,
        /// The deadline it ran past.
        after: std::time::Duration,
    },

    /// `git log` or `git rev-list` gave output that does not split into
    /// whole records.
    #[error("git {command} returned a malformed history record for `{pathspec}` in {}", .root.display())]
    MalformedHistory {
        /// The corpus root the command ran in.
        root: PathBuf,
        /// Which git command.
        command: &'static str,
        /// The node file whose history was asked for, as git was given it.
        pathspec: String,
    },

    /// Every name tried for keeping a refused edit was taken.
    #[error(
        "no free name to keep the edit of `{id}` under in {} after {attempts} tries",
        .dir.display()
    )]
    NoFreeKeepName {
        /// The node the edit was for.
        id: String,
        /// Where refused edits are kept.
        dir: PathBuf,
        /// How many names were tried.
        attempts: u32,
    },

    /// A migration was asked of a corpus whose git work tree has uncommitted
    /// changes. Nothing was changed: the migration must land as its own
    /// commit so the state before it stays recoverable.
    #[error("{} has uncommitted changes, and the migration must be its own commit; nothing was changed", .0.display())]
    DirtyTree(PathBuf),

    /// A migration step produced a node this build cannot read back. Found
    /// in memory, before anything was written.
    #[error(
        "in {}: the migration produced a node this build cannot read: {source}; nothing was written",
        .path.display()
    )]
    MigratedNodeUnreadable {
        /// The node file the step rewrote.
        path: PathBuf,
        /// Why the current model refused it.
        source: Box<Error>,
    },

    /// A v1 node file names a status v1 never had.
    #[error("in {}: status `{status}` is not a v1 status", .path.display())]
    NotAV1Status {
        /// The node file.
        path: PathBuf,
        /// The status as written.
        status: String,
    },

    /// A v1 node file names an edge type v1 never had.
    #[error("in {}: edge type `{edge_type}` is not a v1 edge type", .path.display())]
    NotAV1EdgeType {
        /// The node file.
        path: PathBuf,
        /// The edge type as written.
        edge_type: String,
    },

    /// An atomic write failed, and removing its temporary file failed too,
    /// so the temporary is still there beside the target.
    #[error(
        "writing {} failed: {write}; removing the temporary file {} also failed: {cleanup}",
        .path.display(),
        .tmp.display()
    )]
    TempCleanupFailed {
        /// The file that was being written.
        path: PathBuf,
        /// The temporary file left behind.
        tmp: PathBuf,
        /// Why the write failed.
        #[source]
        write: std::io::Error,
        /// Why the temporary could not be removed.
        cleanup: std::io::Error,
    },

    /// Every temporary name tried beside a file was taken.
    #[error(
        "no free temporary name beside {} after {attempts} tries; something is creating files under them",
        .path.display()
    )]
    NoFreeTempName {
        /// The file a temporary was wanted for.
        path: PathBuf,
        /// How many names were tried.
        attempts: u32,
    },

    /// Reading or writing a known path failed. There is no pathless I/O
    /// variant, so an `io::Error` cannot reach a caller without naming what
    /// it touched (STD-02 §R26).
    #[error("{action} {}: {source}", .path.display())]
    IoAt {
        /// What operation was attempted.
        action: &'static str,
        /// The path the operation targeted.
        path: PathBuf,
        /// The operating system's reason for refusing or failing it.
        source: std::io::Error,
    },

    /// A YAML document would not parse or render. The context says which.
    #[error("{context}: {source}")]
    Yaml {
        /// What was being read or written.
        context: String,
        /// The parser's own complaint, which names the field and the line.
        source: serde_yaml_ng::Error,
    },

    /// A JSON document would not parse or render.
    #[error("{context}: {source}")]
    Json {
        /// What was being read or written.
        context: String,
        /// The parser's own complaint.
        source: serde_json::Error,
    },
}
