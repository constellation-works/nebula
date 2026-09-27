//! A deterministic synthetic corpus, for timing `neb` at a size no real
//! corpus has reached yet.
//!
//! ```sh
//! cargo run --locked --release -p nebula-core --example synthetic_corpus -- \
//!     --out <DIR> [--seed <N>] [--nodes <N>]
//! ```
//!
//! Writes a corpus at `<DIR>`, which must not exist yet: `--nodes` nodes
//! (default 5,000, at least 100) across every status, with tags, references,
//! `## Notes` sections and edges of every kind, including a few long
//! `derives-from` chains and some mutual `contradicts` pairs, and an inbox
//! spread over eight months with promoted, dropped and live captures. The
//! same seed and size give a byte-identical corpus: every date is fixed, and
//! nothing reads the clock. `<DIR>/synthetic.json` names the nodes
//! `scripts/bench.sh` times queries on, and counts what was written.
//!
//! Dev-only, and a consumer of nebula-core's public API like any surface
//! (STD-02 §R14). Nodes are [`Node`] values rendered with the serializer core
//! reads them back with; the `---` frame around that YAML, the inbox line
//! shapes and the `corpus_id` line are core's private rendering, restated
//! here, since no public call writes a node or a capture with a date other
//! than today. So the generator does not trust itself: before it returns it
//! reads the corpus back through core, and refuses to finish unless every
//! file parses, `check` reports nothing, and the inbox holds exactly the
//! live captures it wrote. Dropped captures are settled by core's own
//! `ops::drop`.
//!
//! It prints nothing. A failure is the error `main` returns.

use nebula_core::fs::{
    create_private_dir_all, create_private_new, private_open_options, write_private_atomic,
};
use nebula_core::{
    Closed, Corpus, Edge, EdgeType, Locations, Node, Origin, Reference, Status, ops, verb,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use time::{Date, Duration, Month};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: synthetic_corpus --out <DIR> [--seed <N>] [--nodes <N>]";

/// Nodes are created one every `SPAN_DAYS / nodes` days from `FIRST_DAY`.
const SPAN_DAYS: i64 = 1_000;
/// The long `derives-from` chains `trace` walks.
const CHAINS: usize = 4;
/// How many months the inbox spans, and the first of them.
const INBOX_MONTHS: u8 = 8;
const INBOX_FIRST: (i32, Month) = (2025, Month::October);

/// Labels on the words somebody other than the human wrote. Free text, as
/// `--by` is; none of them is an agent family.
const AUTHORS: [&str; 4] = ["session-7f3a", "crew-night", "session-b21c", "reviewer-2"];

#[rustfmt::skip]
const ADJECTIVES: &[&str] = &[
    "sparse", "dense", "latent", "noisy", "stable", "fragile", "recursive", "local", "global",
    "slow", "fast", "hidden", "shared", "partial", "implicit", "explicit", "bounded", "open",
    "discrete", "continuous", "adaptive", "brittle", "coupled", "layered", "emergent", "shallow",
    "deep", "compressed", "redundant", "linear", "periodic", "chaotic", "symmetric", "skewed",
    "delayed", "early", "stale", "narrow", "broad", "quiet",
];

#[rustfmt::skip]
const NOUNS: &[&str] = &[
    "attention", "memory", "gradient", "entropy", "habit", "network", "signal", "boundary",
    "feedback", "incentive", "language", "grammar", "friction", "trust", "latency", "cache",
    "schedule", "market", "price", "scarcity", "gravity", "curvature", "clock", "rhythm", "sleep",
    "focus", "reward", "curiosity", "error", "variance", "prior", "evidence", "model", "metaphor",
    "archive", "index", "lineage", "ancestry", "mutation", "selection", "niche", "protocol",
    "consensus", "ledger", "queue", "backlog", "deadline", "workflow", "review", "habitat",
    "forest", "river", "sediment", "erosion", "climate", "harvest", "soil", "seed", "circuit",
    "resistance", "voltage", "spectrum", "resonance", "phase", "orbit", "tide", "compiler",
    "parser", "type", "proof", "invariant", "contract", "interface", "abstraction", "notation",
    "diagram", "sketch", "draft", "argument", "question", "answer", "doubt", "belief", "intuition",
    "analogy", "pattern", "anomaly", "outlier", "threshold", "baseline", "benchmark", "regression",
    "drift", "decay", "growth", "cycle", "loop", "fixpoint",
];

#[rustfmt::skip]
const VERBS: &[&str] = &[
    "predicts", "explains", "limits", "amplifies", "dampens", "mirrors", "precedes", "follows",
    "constrains", "enables", "masks", "reveals", "erodes", "stabilizes", "shapes", "outlasts",
    "replaces", "compresses", "delays", "accelerates", "fragments", "unifies", "tracks", "inverts",
];

#[rustfmt::skip]
const CONNECTIVES: &[&str] = &[
    "because", "whenever", "unless", "only after", "long before", "even though", "as soon as",
    "wherever",
];

#[rustfmt::skip]
const TAGS: &[&str] = &[
    "physics", "cognition", "economics", "biology", "linguistics", "software", "ecology",
    "writing", "tooling", "research", "health", "music", "history", "mathematics", "design",
    "teaching", "finance", "sleep-science", "attention-span", "distributed-systems", "compilers",
    "evolution", "climate", "urbanism", "philosophy", "statistics", "games", "security", "reading",
    "craft", "gardening", "energy", "memory-research", "networking", "psychology", "perception",
    "ethics", "organizations", "workflow-design", "open-questions",
];

#[rustfmt::skip]
const REFERENCE_KINDS: &[&str] = &[
    "paper", "study", "article", "note", "discussion", "book", "dataset", "thread",
];

fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1))?;
    // A surface builds its `Locations` from its own environment, as `neb`
    // does; the corpus root is always the explicit `--out`.
    let locations =
        Locations::from_reader(|name| std::env::var_os(name), std::env::current_dir().ok());
    generate(&args, &locations)
}

#[derive(Debug)]
struct Args {
    out: PathBuf,
    seed: u64,
    nodes: usize,
}

impl Args {
    fn parse(mut raw: impl Iterator<Item = String>) -> Result<Self> {
        let mut out = None;
        let mut seed = 1;
        let mut nodes = 5_000;
        while let Some(flag) = raw.next() {
            let mut value = || {
                raw.next()
                    .ok_or_else(|| format!("{flag} needs a value; {USAGE}"))
            };
            match flag.as_str() {
                "--out" => out = Some(PathBuf::from(value()?)),
                "--seed" => seed = value()?.parse()?,
                "--nodes" => nodes = value()?.parse()?,
                _ => return Err(format!("unknown argument `{flag}`; {USAGE}").into()),
            }
        }
        let out = out.ok_or(USAGE)?;
        if nodes < 100 {
            return Err(format!("--nodes must be at least 100, not {nodes}").into());
        }
        Ok(Self { out, seed, nodes })
    }
}

/// `SplitMix64`: small, fast, and the same sequence on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`, for `n > 0`.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the remainder is below `n`, which is a usize"
    )]
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Uniform in `lo..=hi`.
    fn between(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// The first day any node was created.
fn first_day() -> Result<Date> {
    Ok(Date::from_calendar_date(2023, Month::January, 2)?)
}

fn iso(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn days(n: usize) -> Duration {
    Duration::days(i64::try_from(n).unwrap_or(i64::MAX))
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn sentence(rng: &mut Rng) -> String {
    let (a, n1, v, n2) = (
        rng.pick(ADJECTIVES),
        rng.pick(NOUNS),
        rng.pick(VERBS),
        rng.pick(NOUNS),
    );
    let head = format!("{} {n1} {v} {n2}", capitalized(a));
    if rng.chance(55) {
        let (c, a2, n3, v2, n4) = (
            rng.pick(CONNECTIVES),
            rng.pick(ADJECTIVES),
            rng.pick(NOUNS),
            rng.pick(VERBS),
            rng.pick(NOUNS),
        );
        format!("{head} {c} {a2} {n3} {v2} the {n4}.")
    } else {
        format!("{head}.")
    }
}

fn paragraph(rng: &mut Rng, lo: usize, hi: usize) -> String {
    (0..rng.between(lo, hi))
        .map(|_| sentence(rng))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A label for words somebody other than the human wrote, `percent` times
/// in a hundred; `None`, the human, otherwise.
fn author(rng: &mut Rng, percent: u64) -> Option<String> {
    rng.chance(percent).then(|| rng.pick(&AUTHORS).to_string())
}

/// Where each chain's nodes sit: chain `c`'s `k`th node is at
/// `chain_slot(c, k)`, which rises with `k`, so every edge in a chain points
/// at an earlier node.
fn chain_slot(nodes: usize, length: usize, c: usize, k: usize) -> usize {
    1 + k * ((nodes - 1) / length) + c
}

/// The node plan before it is written: everything but the rendered file.
struct Plan {
    doc_nodes: Vec<Node>,
    bodies: Vec<String>,
    deep: usize,
    near: usize,
    depth: usize,
}

#[allow(
    clippy::too_many_lines,
    reason = "one pass over the nodes, in the order a corpus grows"
)]
fn plan(rng: &mut Rng, nodes: usize) -> Result<Plan> {
    let start = first_day()?;
    let length = (nodes / 100).clamp(2, 60);
    let mut chain_of: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for c in 0..CHAINS {
        for k in 0..length {
            chain_of.insert(chain_slot(nodes, length, c, k), (c, k));
        }
    }

    let mut doc_nodes: Vec<Node> = Vec::with_capacity(nodes);
    let mut bodies: Vec<String> = Vec::with_capacity(nodes);
    let mut refuted: Vec<usize> = Vec::new();
    let span = usize::try_from(SPAN_DAYS)?;
    for i in 0..nodes {
        let created = start + days(i * span / nodes);
        let updated = created + days(rng.below(181));

        let words = [
            rng.pick(ADJECTIVES),
            rng.pick(NOUNS),
            rng.pick(VERBS),
            rng.pick(ADJECTIVES),
            rng.pick(NOUNS),
        ];
        let title = capitalized(&words.join(" "));
        let mut id = String::new();
        for word in words {
            if id.len() + word.len() + 1 > 48 {
                break;
            }
            if !id.is_empty() {
                id.push('-');
            }
            id.push_str(word);
        }
        id = format!("{id}-{i}");

        // Genealogy: the chains are fixed; node 0 is the root every chain
        // starts from; every other node points only at earlier ones, so the
        // graph is acyclic by construction and diamonds happen on their own.
        let by = author(rng, 6);
        let mut edges: Vec<Edge> = Vec::new();
        let add = |edges: &mut Vec<Edge>, kind: EdgeType, to: usize, by: &Option<String>| {
            let to = doc_nodes[to].id.clone();
            if !edges.iter().any(|e| e.to == to) {
                edges.push(Edge {
                    kind,
                    to,
                    by: by.clone(),
                });
            }
        };
        if let Some(&(c, k)) = chain_of.get(&i) {
            let parent = if k == 0 {
                0
            } else {
                chain_slot(nodes, length, c, k - 1)
            };
            add(&mut edges, EdgeType::DerivesFrom, parent, &by);
        } else if i > 0 && !rng.chance(12) {
            let parents = match rng.below(100) {
                0..85 => 1,
                85..98 => 2,
                _ => 3,
            };
            for _ in 0..parents {
                let parent = if rng.chance(70) {
                    rng.between(i.saturating_sub(200), i - 1)
                } else {
                    rng.below(i)
                };
                let kind = match rng.below(100) {
                    0..75 => EdgeType::DerivesFrom,
                    75..90 => EdgeType::Refines,
                    _ => EdgeType::Generalizes,
                };
                add(&mut edges, kind, parent, &by);
            }
            if !refuted.is_empty() && rng.chance(3) {
                let target = refuted[rng.below(refuted.len())];
                add(&mut edges, EdgeType::Reopens, target, &by);
            }
        }

        let status = if i == 0 || chain_of.contains_key(&i) {
            if rng.chance(60) {
                Status::Hypothesis
            } else {
                Status::Seed
            }
        } else {
            match rng.below(100) {
                0..50 => Status::Seed,
                50..75 => Status::Hypothesis,
                75..87 => Status::Refuted,
                _ => Status::Abandoned,
            }
        };
        if status == Status::Refuted {
            refuted.push(i);
        }
        let kill = status.needs_kill().then(|| {
            format!(
                "If {} {} does not {} {} within a year, this is wrong.",
                rng.pick(ADJECTIVES),
                rng.pick(NOUNS),
                rng.pick(VERBS).trim_end_matches('s'),
                rng.pick(NOUNS)
            )
        });
        let kill_by = kill.as_ref().and_then(|_| author(rng, 15));
        let closed = (!status.is_open()).then(|| Closed {
            why: if status == Status::Refuted {
                format!("The kill condition fired: {}", sentence(rng))
            } else {
                format!("Lost interest; {}", sentence(rng).to_lowercase())
            },
            at: iso(updated),
        });

        let mut tags: Vec<String> = Vec::new();
        for _ in 0..rng.between(0, 4) {
            let tag = rng.pick(TAGS).to_string();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }

        let mut references: Vec<Reference> = Vec::new();
        if !rng.chance(35) {
            for r in 1..=rng.between(1, 3) {
                let kind = rng.pick(REFERENCE_KINDS);
                let slug = format!("{}-{}", rng.pick(NOUNS), rng.pick(NOUNS));
                let uri = match kind {
                    "paper" | "study" => Some(format!("doi:10.{}/{slug}", 1000 + rng.below(9000))),
                    "note" => Some(format!("[[{slug}]]")),
                    "book" => Some(format!("isbn:978{:010}", rng.next() % 10_000_000_000)),
                    "thread" => Some(format!("https://forum.example.net/t/{}", rng.below(99_999))),
                    "discussion" => None,
                    _ => Some(format!("https://example.org/{kind}/{slug}")),
                };
                let added = created
                    + days(rng.below(usize::try_from((updated - created).whole_days())? + 1));
                references.push(Reference {
                    id: format!("r{r}"),
                    kind: kind.to_string(),
                    uri,
                    title: Some(capitalized(&slug.replace('-', " "))),
                    note: Some(sentence(rng)),
                    added: iso(added),
                    by: author(rng, 10),
                    origin: None,
                });
            }
        }

        let origin = rng.chance(10).then(|| Origin {
            task: Some(format!("ORB-{}", 10_000 + rng.below(4_000))),
            run: Some(format!("jrun-{}", iso(created).replace('-', ""))),
            ..Origin::default()
        });

        let mut body = paragraph(rng, 2, 6);
        if rng.chance(20) {
            body.push_str("\n\n");
            body.push_str(&paragraph(rng, 1, 4));
        }
        if rng.chance(35) {
            // The shape `neb note` appends: a `## Notes` section of dated
            // lines, the human's without an author and anyone else's with one.
            body.push_str("\n\n## Notes\n");
            let gap = usize::try_from((updated - created).whole_days())?;
            let mut day = created;
            for _ in 0..rng.between(1, 4) {
                day += days(rng.below(gap / 4 + 1));
                let text = sentence(rng);
                match author(rng, 15) {
                    Some(by) => write!(body, "\n- {} ({by}): {text}", iso(day))?,
                    None => write!(body, "\n- {}: {text}", iso(day))?,
                }
            }
        }

        doc_nodes.push(Node {
            id,
            title,
            title_by: author(rng, 8),
            status,
            created: iso(created),
            updated: iso(updated),
            kill,
            kill_by,
            tags,
            edges,
            references,
            closed,
            origin,
        });
        bodies.push(body);
    }

    // A few mutual contradictions, recorded on both ends as `link` does.
    let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    while pairs.len() < (nodes / 200).max(3) {
        let (a, b) = (rng.below(nodes), rng.below(nodes));
        if a != b {
            pairs.insert((a.min(b), a.max(b)));
        }
    }
    for (a, b) in pairs {
        let by = author(rng, 20);
        for (from, to) in [(a, b), (b, a)] {
            let to = doc_nodes[to].id.clone();
            doc_nodes[from].edges.push(Edge {
                kind: EdgeType::Contradicts,
                to,
                by: by.clone(),
            });
        }
    }

    let near = (nodes / 2..nodes)
        .find(|i| !chain_of.contains_key(i))
        .unwrap_or(nodes / 2);
    Ok(Plan {
        doc_nodes,
        bodies,
        deep: chain_slot(nodes, length, 0, length - 1),
        near,
        depth: length,
    })
}

/// Create `path`, owner-only as every corpus file is, but without the
/// fsyncs [`create_private_new`] makes. A generated corpus is thrown away
/// after it is timed, and ten thousand pairs of fsyncs would be most of the
/// time it takes to write.
fn write_new(path: &Path, contents: &str) -> Result<()> {
    private_open_options()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(contents.as_bytes())?;
    Ok(())
}

/// A node file as core renders one: YAML frontmatter between `---` lines,
/// a blank line, the trimmed body.
fn render(node: &Node, body: &str) -> Result<String> {
    let front = serde_yaml_ng::to_string(node)?;
    Ok(format!("---\n{front}---\n\n{}\n", body.trim()))
}

/// What the inbox was written with.
#[derive(Default)]
struct InboxCounts {
    live: usize,
    promoted: usize,
    dropped: usize,
    months: Vec<String>,
}

/// The inbox: `nodes / 10` captures over [`INBOX_MONTHS`] month files, as
/// `neb capture` appends them, with the promoted ones struck through as
/// `neb promote` leaves them. The ones to drop are returned, still live, for
/// core to settle.
fn inbox(rng: &mut Rng, root: &Path, nodes: &[Node]) -> Result<(InboxCounts, Vec<String>)> {
    let dir = root.join("inbox");
    create_private_dir_all(&dir)?;
    let (year, month) = INBOX_FIRST;
    let first = Date::from_calendar_date(year, month, 1)?;
    let total = nodes.len() / 10;
    let offsets = ["+00:00", "-07:00", "+09:00", "+01:00"];
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut counts = InboxCounts::default();
    let mut to_drop = Vec::new();
    let mut files: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for _ in 0..total {
        let mut at = first;
        for _ in 0..rng.below(usize::from(INBOX_MONTHS)) {
            at = at.replace_day(1)? + Duration::days(32);
            at = at.replace_day(1)?;
        }
        let at = at.replace_day(u8::try_from(rng.between(1, 28))?)?;
        let stamp = format!(
            "{}T{:02}:{:02}:{:02}{}",
            iso(at),
            rng.below(24),
            rng.below(60),
            rng.below(60),
            offsets[rng.below(offsets.len())]
        );
        let id = loop {
            let id = format!("{:04x}", rng.next() & 0xffff);
            if used.insert(id.clone()) {
                break id;
            }
        };
        let text = sentence(rng);
        let line = match rng.below(100) {
            0..60 => {
                counts.promoted += 1;
                let node = &nodes[rng.between(nodes.len() / 2, nodes.len() - 1)].id;
                format!("- ~~[{id}] {stamp} {text}~~ -> {node}")
            }
            60..85 => {
                to_drop.push(id.clone());
                format!("- [{id}] {stamp} {text}")
            }
            _ => {
                counts.live += 1;
                format!("- [{id}] {stamp} {text}")
            }
        };
        files
            .entry(stamp[..7].to_string())
            .or_default()
            .push((stamp, line));
    }
    for (month, mut lines) in files {
        lines.sort();
        let text: String = lines.into_iter().map(|(_, line)| line + "\n").collect();
        write_new(&dir.join(format!("{month}.md")), &text)?;
        counts.months.push(month);
    }
    counts.dropped = to_drop.len();
    Ok((counts, to_drop))
}

#[allow(
    clippy::too_many_lines,
    reason = "setup, write, settle, verify: read top to bottom"
)]
fn generate(args: &Args, locations: &Locations) -> Result<()> {
    let root = &args.out;
    if std::fs::symlink_metadata(root).is_ok() {
        return Err(format!(
            "{} already exists; give a path that does not",
            root.display()
        )
        .into());
    }
    let corpus = Corpus::init(locations, root)?;

    // `init` names the corpus from its path and the clock; the one thing in
    // it that would differ between two runs. Replace it with the seed's.
    let config_path = root.join("config.yaml");
    let config = std::fs::read_to_string(&config_path)?;
    let mut rng = Rng(args.seed);
    let corpus_id = format!("corpus_id: neb-{:06x}", rng.next() & 0xff_ffff);
    let mut replaced = false;
    let config: String = config
        .lines()
        .map(|line| {
            if line.starts_with("corpus_id:") {
                replaced = true;
                corpus_id.clone()
            } else {
                line.to_string()
            }
        })
        .map(|line| line + "\n")
        .collect();
    if !replaced {
        return Err("config.yaml has no `corpus_id:` line to make deterministic".into());
    }
    write_private_atomic(&config_path, config)?;

    let plan = plan(&mut rng, args.nodes)?;
    for (node, body) in plan.doc_nodes.iter().zip(&plan.bodies) {
        write_new(&corpus.node_path(&node.id)?, &render(node, body)?)?;
    }
    let (inbox, to_drop) = inbox(&mut rng, root, &plan.doc_nodes)?;
    for id in &to_drop {
        ops::drop(&corpus, id)?;
    }

    // Read back through core: every file parses, nothing is found, and the
    // inbox is exactly the live captures.
    let corpus = Corpus::open(locations, Some(root.clone()))?;
    let report = verb::check(&corpus)?;
    if !report.unreadable.is_empty() || !report.findings.is_empty() {
        return Err(format!(
            "the generated corpus does not check clean: {} unreadable, {} findings; first: {:?}",
            report.unreadable.len(),
            report.findings.len(),
            report
                .findings
                .first()
                .map(|f| f.message.clone())
                .or_else(|| report.unreadable.first().map(|u| format!("{u:?}")))
        )
        .into());
    }
    if report.nodes != args.nodes {
        return Err(format!("check read {} nodes, not {}", report.nodes, args.nodes).into());
    }
    let waiting = corpus.inbox()?.0.len();
    if waiting != inbox.live {
        return Err(format!(
            "the inbox reads {waiting} live captures, not {}",
            inbox.live
        )
        .into());
    }

    let mut edges: BTreeMap<String, usize> = BTreeMap::new();
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    for node in &plan.doc_nodes {
        *statuses.entry(node.status.to_string()).or_default() += 1;
        for edge in &node.edges {
            *edges.entry(edge.kind.to_string()).or_default() += 1;
        }
    }
    let references: usize = plan.doc_nodes.iter().map(|n| n.references.len()).sum();
    let with_notes = plan
        .bodies
        .iter()
        .filter(|b| b.contains("## Notes"))
        .count();
    let manifest = json!({
        "seed": args.seed,
        "nodes": args.nodes,
        "root": plan.doc_nodes[0].id,
        "deep": plan.doc_nodes[plan.deep].id,
        "depth": plan.depth,
        "near": plan.doc_nodes[plan.near].id,
        "statuses": statuses,
        "edges": edges,
        "references": references,
        "with_notes": with_notes,
        "inbox": {
            "months": inbox.months,
            "live": inbox.live,
            "promoted": inbox.promoted,
            "dropped": inbox.dropped,
        },
    });
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    create_private_new(&root.join("synthetic.json"), text)?;

    // The lock file records who last held it and when. Nothing holds it now,
    // and a corpus handed over without it is the same on every run.
    std::fs::remove_file(root.join(nebula_core::LOCK_FILE))?;
    Ok(())
}
