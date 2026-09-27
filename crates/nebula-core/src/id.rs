//! Ids: the corpus id, node ids derived from a title or a raw capture, and
//! the two rules an id is held to, [`is_slug`] for a new one and
//! [`is_path_safe_id`] for every one that becomes a path.

use crate::stamp::stamp;
use std::ffi::OsStr;
use std::path::{Component, Path};

/// A stable id for a corpus, derived from where it was created and when.
/// Opaque by design: it identifies, it does not describe.
pub(crate) fn corpus_id(root: &Path) -> String {
    let seed = format!("{}{}", root.display(), stamp());
    format!("neb-{:06x}", fnv(&seed) & 0xff_ffff)
}

pub(crate) fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Turn a title into a node id.
///
/// A slug over 60 characters is cut at the last `-` at or before the limit,
/// never mid-word. A title with no dash in its first 60 characters (one long
/// word) cuts to nothing; the empty-id check at the call site turns that into
/// a refusal rather than a truncated word standing in for the whole title.
pub(crate) fn slugify(s: &str) -> String {
    cap(&dashed(s))
}

/// Lowercase Unicode letters and digits, every other run a single `-`, with
/// no leading or trailing dash. A slug before [`cap`] is applied.
fn dashed(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase().filter(|lower| lower.is_alphanumeric()));
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Cut a dashed slug to 60 characters at the last `-` at or before the limit.
fn cap(dashed: &str) -> String {
    if dashed.chars().count() <= 60 {
        return dashed.to_string();
    }
    let cut: String = dashed.chars().take(60).collect();
    match cut.rfind('-') {
        Some(i) => cut[..i].to_string(),
        None => String::new(),
    }
}

/// How many words an id minted from a raw capture keeps.
pub(crate) const CAPTURE_ID_WORDS: usize = 5;

/// Words that carry grammar rather than the idea, dropped from an id minted
/// from a raw capture. Negations (`not`, `no`, `never`, `without`) and
/// quantifiers (`all`, `only`, `every`) are deliberately absent: dropping one
/// would name the opposite idea.
const STOP_WORDS: &[&str] = &[
    "a", "about", "am", "an", "and", "any", "are", "as", "at", "be", "been", "being", "but", "by",
    "can", "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "may", "maybe", "might", "must", "of", "on",
    "onto", "or", "perhaps", "really", "s", "shall", "she", "should", "so", "some", "than", "that",
    "the", "their", "then", "there", "these", "they", "this", "those", "to", "very", "was", "we",
    "were", "what", "when", "where", "which", "who", "why", "will", "with", "would", "you",
];

/// The ids a capture promoted without `--title` or `--id` may take, most
/// preferred first. Empty when the text reduces to no usable id.
///
/// The title is then the whole captured sentence, and an id is permanent, so
/// the sentence's slug would be carried by every trace, link and edge. A
/// capture of [`CAPTURE_ID_WORDS`] words or fewer keeps exactly that slug.
/// A longer one drops [`STOP_WORDS`] and keeps the first
/// [`CAPTURE_ID_WORDS`] words left (or the first words of the sentence, when
/// every word is a stop word). The later candidates are fallbacks for a
/// collision, still derived from the text: one more significant word at a
/// time, then the full slug [`slugify`] would give. Every candidate follows
/// the slug rules, so it passes [`is_slug`].
pub(crate) fn capture_ids(text: &str) -> Vec<String> {
    let full = dashed(text);
    let words: Vec<&str> = full.split('-').filter(|w| !w.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    if words.len() > CAPTURE_ID_WORDS {
        let significant: Vec<&str> = words
            .iter()
            .copied()
            .filter(|w| !STOP_WORDS.contains(w))
            .collect();
        let significant = if significant.is_empty() {
            &words
        } else {
            &significant
        };
        for n in CAPTURE_ID_WORDS.min(significant.len())..=significant.len() {
            let id = cap(&significant[..n].join("-"));
            if !id.is_empty() && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    let full = cap(&full);
    if !full.is_empty() && !out.contains(&full) {
        out.push(full);
    }
    out
}

/// Whether `s` is already exactly what [`slugify`] would turn it into:
/// lowercase words joined by single dashes, no leading, trailing, or doubled
/// dash, 60 characters or fewer. Used to validate a user-supplied `--id`
/// against the same rule a derived id already has to follow.
pub(crate) fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= 60 && slugify(s) == s
}

/// Whether `id` can name a node file and nothing else.
///
/// Every id becomes a path — `nodes/<id>.md` — so it has to be exactly one
/// ordinary file name: no separator, no `.` or `..`, no root or drive
/// prefix, no control character, and no surrounding whitespace that would
/// make two ids look like one name. `../../escaped` and `/etc/passwd` are
/// what this refuses; `ünïcode-título-ok` and `시간은-프레임의-수다` are
/// ordinary ids and stay valid, because the rule is about path structure
/// rather than about which alphabet an idea was named in.
///
/// Distinct from [`is_slug`], which is the stricter shape a *new* id has to
/// take. This is the weaker rule every id must satisfy, including one read
/// back out of a file somebody edited by hand, so tightening the id a verb
/// creates never silently makes an existing corpus unreadable.
///
/// Asked before any read or write derives a path, and nothing is
/// canonicalized: the components are judged as written, which is what keeps
/// the answer the same under a symlinked root.
pub(crate) fn is_path_safe_id(id: &str) -> bool {
    if id.is_empty() || id.trim() != id {
        return false;
    }
    if id
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\' || c == std::path::MAIN_SEPARATOR)
    {
        return false;
    }
    // One `Normal` component spelled exactly as the id: `.`, `..`, a root and
    // a Windows prefix are each their own component kind, and an id that
    // parses to anything else is not a file name.
    let mut components = Path::new(id).components();
    let single =
        matches!(components.next(), Some(Component::Normal(name)) if name == OsStr::new(id));
    single && components.next().is_none()
}
