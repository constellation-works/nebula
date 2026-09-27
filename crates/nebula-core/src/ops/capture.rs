//! Opening a corpus, and the inbox's verbs: capture, suggest, drop and
//! promote.

use super::node::build;
use super::{Captured, Created, Initialized, NewNode, Promotion};
use crate::error::{Error, Result};
use crate::fs_impl::create_private_dir_all;
use crate::graph_impl::{self as graph, Graph, Neighbour};
use crate::id::capture_ids;
use crate::locations::Locations;
use crate::lock::{self, CorpusLock};
use crate::model::Status;
use crate::pending::{Pending, PendingWrite};
use crate::store::{Corpus, InboxEntry};
use std::path::PathBuf;
use unicode_normalization::UnicodeNormalization;

/// Where [`init`] creates a corpus: `path` if given, else the resolved root.
///
/// `root` is an explicit `--root`, and given alongside `path` it has to name
/// the same directory, compared once both are made absolute (lexically, never
/// canonicalized); otherwise this refuses with [`Error::RootAndPathDiffer`]
/// rather than let one of them go unused (STD-01 §R28). A caller asks here
/// before any work, so a refusal has created neither.
pub fn init_target(
    locations: &Locations,
    root: Option<PathBuf>,
    path: Option<PathBuf>,
) -> Result<PathBuf> {
    if let (Some(root), Some(path)) = (&root, &path) {
        if root.as_os_str().is_empty() || path.as_os_str().is_empty() {
            return Err(Error::EmptyRoot);
        }
        // Only an empty path fails to be made absolute on Unix, and that is
        // refused above; elsewhere a path that cannot be is compared as given.
        let absolute = |p: &PathBuf| locations.absolute(p);
        if absolute(root) != absolute(path) {
            return Err(Error::RootAndPathDiffer {
                root: root.clone(),
                path: path.clone(),
            });
        }
    }
    Corpus::resolve_root(locations, path.or(root))
}

/// Create an empty corpus where [`init_target`] says, which refuses a `root`
/// and a `path` that differ. Optionally make it the machine-local default,
/// refusing to replace a different setting unless `force` is set.
///
/// With `set_root`, the machine-setting lock is held from before the check
/// until after the write, so two of these cannot both find the setting free
/// and then both write it. It is taken first, before anything is created,
/// and the corpus lock inside it: the order `lock.rs` documents.
pub fn init(
    locations: &Locations,
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Result<Initialized> {
    init_with_effect(locations, root, path, set_root, force).map(|(initialized, _)| initialized)
}

/// Initialize and report whether corpus content was installed or repaired.
pub(crate) fn init_with_effect(
    locations: &Locations,
    root: Option<PathBuf>,
    path: Option<PathBuf>,
    set_root: bool,
    force: bool,
) -> Result<(Initialized, bool)> {
    locations.write_gate(crate::locations::WriteIntent::Ordinary)?;
    let target = init_target(locations, root, path)?;
    let _settings = set_root
        .then(|| Corpus::lock_machine_settings(locations))
        .transpose()?;
    if set_root {
        Corpus::check_root_config(locations, &target, force)?;
    }
    // The lock lives inside the root, so the root has to exist before it can
    // be taken. `Corpus::init` would create it a moment later anyway.
    create_private_dir_all(&target)?;
    let _lock = CorpusLock::acquire(&target)?;
    let (_, changed) = Corpus::init_with_effect(locations, &target)?;
    if set_root {
        Corpus::write_root_config(locations, &target, force)?;
    }
    Ok((Initialized { root: target }, changed))
}

pub(crate) use capture_locked as capture;

/// The five-second path: append a thought to the inbox.
///
/// No parent, no title, no decisions. A capture step that requires decisions
/// is a capture step you will skip at the exact moment the idea arrives.
pub fn capture_locked(corpus: &Corpus, text: &str) -> Result<InboxEntry> {
    let _lock = corpus.lock()?;
    corpus.capture(text)
}

/// [`capture`], then the `k` existing nodes the text reads closest to.
///
/// The capture lands first and is never undone: the suggestions are for the
/// triage that follows, and a corpus that cannot be read for them is an
/// error reported *after* the write, the same way a refused commit is. `k`
/// of zero skips the read altogether, which is what `--quiet` means. A
/// caller that wants the entry id out before that read happens runs
/// [`capture`] and then [`suggest`] itself.
pub fn capture_near(corpus: &Corpus, text: &str, k: usize) -> Result<Captured> {
    // Only the capture is locked. The suggestions are a read, and holding a
    // writer's lock over one would make every other writer wait on it; so a
    // caller holding the lock itself calls `capture`, releases, and then
    // `suggest`, as `neb capture` does.
    let entry = capture(corpus, text)?;
    let near = suggest(corpus, &entry.text, k)?;
    Ok(Captured { entry, near })
}

/// The `k` nodes closest to `text`, over the corpus as it is on disk now.
///
/// What [`capture_near`] and [`promote`] run, read before the node a caller
/// is about to write so a promotion is not its own nearest neighbour. It
/// reads `nodes/`, which a capture never needs, so a node file that will not
/// parse fails here and not the capture.
///
/// A scan of every node, and advice, so never run under the write lock
/// (STD-03 §R1); a debug build panics if the calling thread holds it. With
/// `k` of zero nothing is read, so that is free anywhere.
pub fn suggest(corpus: &Corpus, text: &str, k: usize) -> Result<Vec<Neighbour>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    debug_assert!(
        !lock::held_by_this_thread(corpus.root()),
        "suggest scans nodes/ for advice and must not run under the corpus lock"
    );
    let docs = corpus.load_all()?;
    Ok(graph::near(&Graph::build(&docs)?, text, k)?.0)
}

/// Discard a capture, struck through rather than deleted.
pub fn drop(corpus: &Corpus, entry: &str) -> Result<InboxEntry> {
    let _lock = corpus.lock()?;
    let e = corpus.inbox_entry(entry)?;
    corpus.settle_inbox(&e, "dropped")?;
    Ok(e)
}

/// Turn an inbox entry into a seed node.
///
/// Deliberately separate from capture. Most captures should never be
/// promoted, and dropping one is a normal outcome rather than a failure.
///
/// Without a parent, `near` on the result names the existing nodes the
/// title and captured text read closest to, so the human can see whether
/// a parent was defensible. The node is written as a root either way: the
/// suggestion never blocks the promotion and never becomes an edge. `near_k`
/// is how many to look for; zero looks for none.
///
/// The suggestions are read first, without the lock, by [`promotion_near`]; a
/// caller that holds the lock across the promotion and its commit runs that
/// itself before taking it, and hands the result to [`promote_with`].
pub fn promote(corpus: &Corpus, entry: &str, args: &Promotion, near_k: usize) -> Result<Created> {
    let near = promotion_near(corpus, entry, args, near_k)?;
    promote_with(corpus, entry, args, near)
}

/// What [`promote`] suggests for `entry`: the `near_k` nodes its title and
/// captured text read closest to, or none when `args` names a parent or
/// `near_k` is zero.
///
/// Read without the lock (STD-03 §R1), before the node is written, so the new
/// node is not among its own neighbours. The entry is read here without the
/// lock too; [`promote_with`] finds it again under the lock, and refuses
/// there if another writer settled it meanwhile.
pub fn promotion_near(
    corpus: &Corpus,
    entry: &str,
    args: &Promotion,
    near_k: usize,
) -> Result<Vec<Neighbour>> {
    if near_k == 0 || !args.parents.is_empty() {
        return Ok(Vec::new());
    }
    let e = corpus.inbox_entry(entry)?;
    let title = args.title.as_deref().unwrap_or(&e.text);
    suggest(corpus, &format!("{title}\n{}", e.text), near_k)
}

/// [`promote`], with its suggestions already read by [`promotion_near`]:
/// `near` is returned on the [`Created`] as it was given.
pub fn promote_with(
    corpus: &Corpus,
    entry: &str,
    args: &Promotion,
    near: Vec<Neighbour>,
) -> Result<Created> {
    if args.title.is_some() || args.body_supplied || !args.body.trim().is_empty() {
        corpus
            .locations()
            .write_gate(crate::locations::WriteIntent::Authored(args.by.as_deref()))?;
    }
    // Held across the whole verb: the node is written and *then* the inbox
    // line is struck, and a capture landing between the two would shift the
    // line this entry was found at.
    let _lock = corpus.lock()?;
    let e = corpus.inbox_entry(entry)?;
    let title = args.title.clone().unwrap_or_else(|| e.text.clone());
    // An explicit id or title decides the id as it always has. Only an id
    // minted from the raw capture is shortened; see `id::capture_ids`.
    let id = match (&args.id, &args.title) {
        (Some(id), _) => Some(id.clone()),
        (None, Some(_)) => None,
        (None, None) => free_capture_id(corpus, &e.text)?,
    };
    let body = if args.body.trim().is_empty() {
        e.text.clone()
    } else {
        format!("{}\n\n{}", e.text, args.body.trim())
    };
    let mut doc = build(
        corpus,
        &NewNode {
            title,
            body: String::new(),
            parents: args.parents.clone(),
            reopens: None,
            contradicts: Vec::new(),
            kill: None,
            tags: args.tags.clone(),
            origin: args.origin.clone(),
            id,
            by: args.by.clone(),
        },
        Status::Seed,
        &body,
    )?;
    // A capture promoted as it was captured is titled in the human's own
    // words. Whoever ran the verb authored the edges, not that sentence.
    if args.title.is_none() {
        doc.node.title_by = None;
    }
    // Two files, so two renames, and a crash can fall between them. The
    // record written first is what lets the next writer tell a promotion
    // that got as far as its node from a second capture of the same text,
    // and finish it (STD-03 §R9; see `pending.rs`).
    let pending = Pending::begin(
        corpus,
        &PendingWrite::Promote {
            entry: e.id.clone(),
            stamp: e.stamp.clone(),
            node: doc.node.id.clone(),
        },
    )?;
    corpus.create(&doc)?;
    corpus.settle_inbox(&e, &format!("-> {}", doc.node.id))?;
    pending.finished()?;
    Ok(Created {
        path: corpus.node_path(&doc.node.id)?,
        doc,
        near,
    })
}

/// The id for a capture promoted with neither a title nor an id: the first
/// of [`capture_ids`] no node has taken.
///
/// A candidate is passed over only when the node holding it is a different
/// idea. One titled with this very text is the same thought already
/// promoted, so its id is returned and the create refuses it as
/// [`Error::NodeExists`], exactly as before ids were shortened: a duplicate
/// node in a lineage graph is worse than a refusal. When every candidate is
/// another idea's, the last — the full slug — is returned and refused the
/// same way. `None` when the text reduces to no id, which the build refuses
/// as [`Error::UnusableTitle`]. Called under the corpus lock, so nothing can
/// take the id between this check and the write.
fn free_capture_id(corpus: &Corpus, text: &str) -> Result<Option<String>> {
    let candidates = capture_ids(text);
    for id in &candidates {
        if !corpus.node_path(id)?.exists() {
            return Ok(Some(id.clone()));
        }
        if corpus
            .load(id)?
            .node
            .title
            .trim()
            .nfc()
            .eq(text.trim().nfc())
        {
            return Ok(Some(id.clone()));
        }
    }
    Ok(candidates.last().cloned())
}
