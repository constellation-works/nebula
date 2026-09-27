//! Each [`Error`] variant's stable code and its [`ErrorClass`], written from
//! one table, and the constructors the rest of the crate labels errors with.

use std::path::PathBuf;

use super::Error;

/// Writes [`Error::code`] and [`Error::class`] from one
/// `Variant => "code", Class,` line per variant, and, for the tests, the
/// same lines as data.
///
/// The matches it expands to are exhaustive, so a new variant does not
/// compile until it has a line with both. The table the tests walk is built
/// from those same lines, so no variant can have a code that the uniqueness
/// and spelling checks never see. `Self::Variant { .. }` matches a variant of
/// any shape, which is why a line names the variant alone. A variant whose
/// class turns on a field names it in braces, one line per case:
/// `Variant { field: None, } => "code", Argument,`; its lines share a code.
macro_rules! codes {
    ($($variant:ident $({ $($field:tt)* })? => $code:literal, $class:ident,)*) => {
        /// The variant's stable machine name: `neb --json` reports it as the
        /// refusal's `code`, so a script matches `no_such_node` rather than
        /// parsing the message.
        ///
        /// It is the variant's name in `snake_case`, spelled out rather than
        /// derived, and exhaustive on purpose. A new variant does not compile
        /// until it names its code, and renaming a variant does not quietly
        /// rename a code that scripts already match.
        pub fn code(&self) -> &'static str {
            match self {
                $(Self::$variant { $($($field)*)? .. } => $code,)*
            }
        }

        /// Whether the refusal is about the arguments alone or about what
        /// they met: see [`ErrorClass`]. Exhaustive like [`Self::code`], so a
        /// new variant does not compile until it is placed.
        pub fn class(&self) -> ErrorClass {
            match self {
                $(Self::$variant { $($($field)*)? .. } => ErrorClass::$class,)*
            }
        }

        /// Every variant's name beside its code and class, one entry per
        /// line of the `codes!` invocation.
        #[cfg(test)]
        pub(crate) const CODES: &[(&str, &str, ErrorClass)] =
            &[$((stringify!($variant), $code, ErrorClass::$class),)*];
    };
}

/// What a refusal is about, which is what decides how a surface reports it:
/// `neb` exits 2 for an [`ErrorClass::Argument`] and 1 for an
/// [`ErrorClass::State`] (STD-01 §R20).
///
/// It lives here, beside the codes, because [`Error`] is `#[non_exhaustive]`:
/// a match on it in any other crate needs a wildcard, so only here can a new
/// variant be refused compilation until it is placed (STD-02 §R27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// An argument no corpus could accept, whatever it holds: a value of the
    /// wrong shape, a flag the verb's other arguments rule out, or a verb
    /// asked for a form it has none of.
    Argument,
    /// Anything that turns on what the corpus, the machine or git holds.
    State,
}

impl Error {
    codes! {
        UnreadableNodes => "unreadable_nodes", State,
        NoSuchNode => "no_such_node", State,
        NoSuchInboxEntry => "no_such_inbox_entry", State,
        InboxEntrySettled => "inbox_entry_settled", State,
        NoCorpus => "no_corpus", State,
        NotGitWorkTree => "not_git_work_tree", State,
        NoNodeAtRevision => "no_node_at_revision", State,
        UnknownRevision => "unknown_revision", State,
        EmptyRoot => "empty_root", Argument,
        EmptyRootSetting => "empty_root_setting", State,
        RelativeRootSetting => "relative_root_setting", State,
        HomeUnset => "home_unset", State,
        HomeNotUnicode => "home_not_unicode", State,
        InvalidReadOnlyEnvironment => "invalid_read_only_environment", State,
        InvalidOriginEnvironment => "invalid_origin_environment", State,
        ReadOnly => "read_only", State,
        ByRequired => "by_required", State,
        HumanOnly => "human_only", State,
        RootConfigConflict => "root_config_conflict", State,
        RootAndPathDiffer => "root_and_path_differ", Argument,
        RelativeObservatoryRoot { setting: None, } => "relative_observatory_root", Argument,
        RelativeObservatoryRoot { setting: Some(_), } => "relative_observatory_root", State,
        MissingConfig => "missing_config", State,
        SchemaMismatch => "schema_mismatch", State,
        CurrentSchemaUnreadable => "current_schema_unreadable", State,
        V1NodeUnderCurrentSchema => "v1_node_under_current_schema", State,
        NotAStatus => "not_a_status", Argument,
        NotAnEdgeType => "not_an_edge_type", Argument,
        InvalidAuthorLabel => "invalid_author_label", Argument,
        MalformedFrontmatter => "malformed_frontmatter", State,
        Cycle => "cycle", State,
        SelfLoop => "self_loop", Argument,
        DuplicateEdge => "duplicate_edge", State,
        ParentAndReopens => "parent_and_reopens", Argument,
        NeedsKill => "needs_kill", State,
        EmptyKill => "empty_kill", Argument,
        NoKillToConfirm => "no_kill_to_confirm", State,
        KillAlreadySet => "kill_already_set", State,
        RefutedNeedsWhy => "refuted_needs_why", Argument,
        ReasonOnOpenStatus => "reason_on_open_status", Argument,
        RefutedCannotReopen => "refuted_cannot_reopen", State,
        AlreadyClosed => "already_closed", State,
        SeedWithKill => "seed_with_kill", State,
        NoSuchCandidate => "no_such_candidate", State,
        Interactive => "interactive", Argument,
        EmptyCapture => "empty_capture", Argument,
        EmptyNote => "empty_note", Argument,
        InboxIdsExhausted => "inbox_ids_exhausted", State,
        ReferenceIdsExhausted => "reference_ids_exhausted", State,
        InboxEntryForeign => "inbox_entry_foreign", State,
        InboxEntryMissing => "inbox_entry_missing", State,
        InboxEntryChanged => "inbox_entry_changed", State,
        NodesSymlink => "nodes_symlink", State,
        InboxSymlink => "inbox_symlink", State,
        UriRequired => "uri_required", Argument,
        InvalidAt => "invalid_at", Argument,
        DuplicateId => "duplicate_id", State,
        NodeExists => "node_exists", State,
        UnresolvedUri => "unresolved_uri", State,
        AbsoluteUri => "absolute_uri", Argument,
        InvalidTransition => "invalid_transition", State,
        MissingParent => "missing_parent", State,
        UnusableTitle => "unusable_title", Argument,
        UnknownReferenceKind => "unknown_reference_kind", Argument,
        InvalidObservatoryId => "invalid_observatory_id", Argument,
        UnresolvedObservatoryRecord => "unresolved_observatory_record", State,
        InvalidId => "invalid_id", Argument,
        UnsafeId => "unsafe_id", Argument,
        IdMismatch => "id_mismatch", State,
        NotRegularFile => "not_regular_file", State,
        Locked => "locked", State,
        EditConflict => "edit_conflict", State,
        NotesChanged => "notes_changed", State,
        InputTooLarge => "input_too_large", State,
        PendingWriteUnreadable => "pending_write_unreadable", State,
        IoStdin => "io_stdin", State,
        StdinNotUtf8 => "stdin_not_utf8", State,
        CorpusIgnored => "corpus_ignored", State,
        Git => "git", State,
        GitTimedOut => "git_timed_out", State,
        MalformedHistory => "malformed_history", State,
        NoFreeKeepName => "no_free_keep_name", State,
        DirtyTree => "dirty_tree", State,
        MigratedNodeUnreadable => "migrated_node_unreadable", State,
        NotAV1Status => "not_a_v1_status", State,
        NotAV1EdgeType => "not_a_v1_edge_type", State,
        TempCleanupFailed => "temp_cleanup_failed", State,
        NoFreeTempName => "no_free_temp_name", State,
        IoAt => "io_at", State,
        Yaml => "yaml", State,
        Json => "json", State,
    }

    /// A YAML failure, labelled with what was being read.
    pub(crate) fn yaml(context: impl Into<String>, source: serde_yaml_ng::Error) -> Self {
        Self::Yaml {
            context: context.into(),
            source,
        }
    }

    /// An I/O failure labelled with the path the user can inspect or repair.
    /// Public so a surface's own I/O (a report file, an editor's temporary
    /// file) is reported the same way the library's is.
    pub fn io_at(action: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::IoAt {
            action,
            path: path.into(),
            source,
        }
    }
}
