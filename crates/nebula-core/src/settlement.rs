//! What became of a settled inbox entry, as a value.
//!
//! A leaf below both `store` (which reads and writes the struck-through
//! inbox line that records it) and `error` (which names it in
//! [`crate::Error::InboxEntrySettled`]), so neither has to import the other
//! for this type.

/// What became of a settled inbox entry, as its struck-through line records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    /// Promoted into the node with this id.
    Promoted(String),
    /// Dropped without becoming a node.
    Dropped,
}

impl Settlement {
    /// The id and outcome of a line `Corpus::settle_inbox` wrote, which is
    /// `- ~~[<id>] <at> <text>~~ <outcome>`. A struck line whose outcome is
    /// neither of the two that verb writes was edited by hand, and is not
    /// read as either.
    pub(crate) fn parse(line: &str) -> Option<(&str, Self)> {
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
