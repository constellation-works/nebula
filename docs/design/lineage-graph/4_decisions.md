---
title: Lineage Graph — Decisions
owner: claude
last_updated: 2026-09-26
last_validated: 2026-09-25
status: Accepted
feature: lineage-graph
doc_role: decisions
type: design
summary: The choices that shaped nebula, each with the reasoning and the condition that would reverse it.
tags: [lineage-graph, v0.2]
paths: ["crates/**"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Lineage Graph — Decisions

## A DAG, not a tree

A rooted tree is the special case of a DAG where every node has exactly one
parent. Ideas do not behave that way: two observations converge, and a synthesis
descends from both. That is a diamond, which has no directed cycle and is
therefore legal. Git's commit history is the canonical example, since every merge
commit creates one.

Storing a tree would tempt nested directories or a single `parent` field, and the
first merge would become a migration.

## Acyclicity constrains genealogy only

An idea cannot be its own ancestor, so genealogy is enforced acyclic and `neb
link` refuses the edge that would close a loop. `contradicts` is left
unconstrained by comparison: it is symmetric and just states that two ideas
cannot both be true, not a claim that could itself cycle. v0.1 additionally ran
a second, cycle-tolerant relation beside genealogy that could legitimately loop
back on itself; it warned rather than failed, on the theory that the loop was
itself a finding. v0.2 cut that second relation entirely (see "References over
evidence" below), which makes this decision narrower than it once was: today
only genealogy is required to be acyclic; `contradicts` remains a symmetric
relation outside the cycle check and `trace`.

## Stage four is an edge, not a stage

Refinement creates a new node with a `refines` edge back. Killing flips a status.
Nodes are never rewritten into something else and never deleted. Dead branches
are the highest-value content in a corpus like this, because they stop you
re-treading ground. principia already had this instinct in its refuted wall,
which forbids claim ids from vanishing; nebula applies it to everything.

## Refuted and abandoned are separate

One is wrong, the other is untouched. Collapsing them loses the distinction
between an idea that failed and an idea you stopped caring about, which are
completely different signals when you return years later.

## Domains inside a corpus

**Reversed 2026-09-12:** one more decision per node; tags do the job and keep
everything in one place (the exact reasoning is in
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md), "What is removed"). See "Tags
over domains" below for what replaced it.

Tags were the only thing distinguishing physics from ranking signals, and tags
are open and optional, so a year of typing produces `principia`, `Principia`
and `physics` for one thing and nodes with nothing at all. A required
categorical field validated against a closed set declared centrally cannot
drift or be forgotten. It was assigned at the point a title and parents were
already being decided, and never at capture, which had to stay decision-free.

## One corpus per trust boundary

Splitting the corpus itself by subject area was rejected and remains rejected
in v0.2. Edges cannot cross corpora, the checker would flag any that tried as
dangling, and `trace` would stop dead at the boundary. The graph is the
product, and cross-field edges are the ones worth finding later. The split
that does justify a second corpus is ownership: work ideas and personal ideas
have different owners, backups and legal standing, so they are separate
corpora with separate configs, and a lineage that genuinely crosses that line
is recorded as a reference on the receiving node rather than an edge.

## References cannot carry a verdict, as it applied to evidence

**Reversed 2026-09-12:** the verdict/strength ladder was precision the corpus
never earned; a reference with a good note carries the same information (see
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md), "What is removed"). See
"References over evidence" below.

The separation was enforced by `deny_unknown_fields`, so a reference carrying a
verdict failed to parse rather than merely warning. Had it merely warned,
findings would have been routed through references to dodge the discipline the
verdict/strength ladder demanded — which was the entire reason the ladder
existed. The mechanical guard (a reference's schema has no room for a verdict
key) survives into v2 unchanged; what is reversed is the reason it needs to,
since there is no longer a stricter sibling field for a reference to be
smuggled past.

## Capture has a five-second budget

Capture takes one line, requires no decisions, and works before a corpus exists.
Text that arrives over several lines is joined onto one rather than refused, since
a refusal loses the thought at the moment it arrived (see the
[capture path](specs/capture-path.md#text-over-several-lines)).
If capture ever asks which parent an idea belongs to, it will be skipped at the
exact moment an idea arrives, and the corpus dies. Promotion is a separate act,
and most captures should never be promoted.

## Rust, not Python

The capture path must never fail. A Python CLI depends on an interpreter and a
virtualenv that break eventually, and the failure mode is that the command used
ten times a day stops working. The machine this runs on already has a Python
ahead of its own package ecosystem. A single binary has no such failure. Serde
also makes schema churn cheaper, since changing a struct makes the compiler
enumerate every site to fix — which is exactly what made the 2026-09-12
reduction a bounded, checkable diff rather than a rewrite.

## One file per node

Reverses cleanly if the workload changes. Splitting references into separate
files would buy conflict-free concurrent appends; writes here are infrequent,
the corpus is small, and even in session mode there is effectively one writer
at a time. Reading one file shows the whole node with no join and no tooling,
and one `git log` gives its full history. Revisit if frontmatter routinely
outgrows prose, or a second writer appears.

## Full scan instead of an index

An index would be a second source of truth able to drift, bought with time that
is not currently scarce. Revisit when a scan is measurably slow.

## Graduating requires a falsifier

**Reversed 2026-09-12:** downstream hand-off happens by the downstream artifact
referencing the node, not by a status here (see
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md), "What is removed"). The
`handoff` verb (ORB-13077) records the nebula side for an Observatory record
as an existing reference plus `abandoned`, not a status of its own, and asks
for no falsifier (same document, "Hand-off").

principia and orbit-research both demand a kill condition, so nebula refused to
move a node to a graduated status without one rather than exporting the gap
downstream. That protection is no longer needed once graduating is not a
status at all: a downstream artifact that cites a node either names what it is
relying on or it does not, and that is a property of the downstream artifact,
not something nebula can enforce by refusing a transition.

## A module tree, not a flat `src/`

The tool was small enough in v0.1 that a flat directory would have worked. It
was laid out as a tree anyway, because the layering is the part worth keeping
honest over years: the data layer must never learn about the terminal, and the
verbs must never learn about clap. A directory per layer makes a violation
visible in an import path. The layering also fixed the seam for a future
library crate without deciding at the time whether one was needed — see "A
core-library workspace" below for the point it got cashed in, and
[2_design.md](2_design.md) "Crates" for the layout that resulted.

## Tags over domains

A tag costs nothing to add and nothing to be wrong about; a required
categorical field validated against a config-file list costs a decision on
every node and a migration every time the list needs to change. Five days and
eight nodes into v0.1, the list had exactly the drift problem it was meant to
prevent — one node's field differed from another's only by case — while
contributing no query nobody could get from a tag. Tags, normalised on write
and checked for near-duplicates, do the same job with less machinery and one
fewer place to disagree with yourself. Reverses if a corpus grows enough
distinct large areas that cross-cutting queries need a required, exhaustive
partition rather than an open, best-effort one — nothing today is close to
that scale.

## References over evidence

Evidence's verdict/strength ladder existed to keep rigor honest, but rigor is
downstream's job, and three coarse strength levels bought false precision
without buying real calibration: nothing here runs an actual weighing
procedure, so a strength label was an opinion wearing a schema. A reference
with a note that says why it matters and which way it cuts carries the same
information without pretending to be more than that. This also collapses two
near-identical attachment shapes (evidence and reference) that mostly differed
by which one theoretically got to move a status, into the one that is left.
Reverses if the corpus starts running an actual quantitative weighing
procedure over its attachments, which nothing in the current design does or is
meant to do.

## Agent-managed machinery, with a session/routine split

v0.1 already had the instinct that curation is work nobody does, so a
maintenance loop was specified as an Orbit routine that proposes into a review
file and never mutates a node. v0.2 keeps the hard rule and generalizes the
loop into two named modes: **session mode**, where a human is present and
directing and the agent runs verbs directly (running `neb check` after every
write), and **routine mode**, unattended, where the agent only reads and
writes proposals to `review.md`. Naming both modes explicitly, rather than
leaving "an agent runs this sometimes" implicit, is what lets a skill
([docs/design/v0.2/2_architecture.md](../v0.2/2_architecture.md),
"skills/nebula") state which one it is in before it acts, instead of every
verb having to guess whether a human is watching.

## A core-library workspace

The v0.1 module tree already kept `corpus`, `check` and `render` ignorant of
the layers above them, on the stated theory that a second consumer would
appear eventually. It appeared: a desktop app now draws the graph and captures
from a shortcut, on top of the same corpus the CLI and an agent skill also
read and write. Cashing in that promise means an actual crate boundary —
`nebula-core` as a pure library with typed errors and `Serialize` return
values, `neb` as a thin CLI wrapping it — rather than three consumers each
re-parsing `--json` output or shelling out to the binary. The alternative,
letting the desktop app shell out to `neb --json` the way an agent does, was
rejected because a webview-and-shortcut app polling a subprocess for a
file-watch-driven graph view is slower and more fragile than linking the
library it would otherwise be reimplementing.

## Ids stay strings, checked where they become paths

This departs from STD-02@2 §R14, a SHOULD: node ids and Observatory record ids
are `String`, not validated newtypes. The rule's worry is validate-then-use,
where every caller has to remember the check. Here the check sits where an id
gains authority:

- **Node ids, read from disk.** `model::parse` refuses a node whose `id` is not
  `is_path_safe_id` as it parses the file, and `Corpus::load_all` and
  `Corpus::load` refuse a file whose name and stored id disagree. No `Doc`
  with an unsafe id exists for a verb to act on.
- **Node ids, from a caller.** `Corpus::node_path` is where an id becomes a
  file path, and it refuses an unsafe id before joining it. `history` and
  `load_at` also build a git pathspec from the id, and each asks `node_path`
  first. Those two are the validate-then-use shape the rule warns about; both
  are in `store.rs`, so the check is one screen away from the use.
- **Observatory record ids.** `is_observatory_id` is checked when `cite` or
  `handoff` stores the id, and again by `resolve_observatory` before it is
  joined under the root.

A newtype would change every signature that carries an id: the ops, the graph
queries, the store, the CLI, the desktop commands and the generated TypeScript
types. It would change no refusal. What is given up is a compiler proof: a new
function that turns an `&str` into a path without `node_path` would compile.
Scope: node ids and Observatory record ids. Reference ids are node-local
counters that never become paths. Reverses if an id starts to reach anything
else with authority (a URL, a git ref, a shell argument) outside `store.rs`, or
if `nebula-core` gains consumers outside this workspace. At that point the
check belongs in the type.

## The two integration-test files stay whole

This departs from STD-02@2 §R18, a SHOULD, for `crates/neb/tests/cli.rs` and
`crates/nebula-core/tests/core.rs` only. Each is a flat list of independent
end-to-end cases over one shared fixture: the `Corpus` harness and its run
helpers in `cli.rs`, and `corpus()` and `seed()` in `core.rs`. Size here does
not hide several responsibilities, which is what the rule's split protects
against. Splitting the files now would move every test, and it would collide
with every in-flight task that adds one. The standards wave this decision
belongs to filed 32 tasks, and 27 of them name one of these two files.

What is given up: diffs and searches in an 8,500-line file are harder to
review. What still holds the line: a new test goes beside the related tests,
not at the end of the file. Source files are still held to about 800 lines,
and ORB-13174 splits the ones over it. Reverses when the tree is quiet enough
for a single mechanical move into `tests/cli/main.rs` plus one module per verb
family. That layout keeps one test binary per crate and leaves the test count
unchanged.

## Unknown fields are refused, never dropped

STD-02@2 §R16 asks that a removed field be "warned about and ignored (or
migrated) rather than silently rejected", and that round trips be lossless.
Nebula takes the migrate branch for retired keys and refuses everything else
loudly; it never warns and ignores. This restates "References cannot carry a
verdict, as it applied to evidence" above against that rule.

Every stored shape carries `deny_unknown_fields`: `Node`, `Edge`, `Reference`,
`Origin`, `Closed` and `Config`. A key this build does not know fails the read
with the file named, so no value is fabricated or discarded. Ignoring the key
would not be lossless: every verb rewrites the whole node file, so a key
ignored on read would be deleted on the next save. The other cases are:

- **Keys that v0.2 retired** (`domain`, `evidence` with its verdict and
  strength, `tasks`, `graduated_to`, and the removed edge kinds) are migrated.
  `neb migrate` reads v1 through a lenient model and turns each of them into a
  reference, a tag or a closing reason.
- **A corpus from a newer build** is refused by its `schema_version` before any
  node is parsed (`Error::SchemaMismatch`).
- **`migrate` over a corpus that already declares this schema** reads every
  node with the strict model first (`Error::CurrentSchemaUnreadable`), so its
  lenient v1 model cannot drop what it does not know.

One key is kept and warned about instead: the legacy `observatory_root` in
`config.yaml`, which `check` reports until it is dropped. What is given up: a
hand-added key breaks every verb that reads that node until it is removed, and
the refusal's path says where. One residual gap remains. The v1 model
tolerates keys that v1 never defined, so migrating a v1 node drops such a key.
The clean-tree refusal keeps the pre-migration file in git history when the
corpus is under git. Reverses if nebula ever has to read a sibling tool's
extra keys, which would call for a separate `*Document` type that carries them.

## The invariant-table guard stays a Rust test

`check.rs`'s `published_invariant_tables_match_checker_rules` departs from
STD-02@2 §R21. It reads three documents with `include_str!`: the v0.2 spec,
the skill's `invariants.md`, and the lineage-graph invariants spec. For each
`| # |` table, it compares the number and label columns with the `Rule` enum's
discriminants and with the label list kept beside them. The rule bans tests
that read text rather than run code. STD-04@1 §R5 allows a narrow structural
guard that names what it protects. This test protects the meaning of `[N]`:
the number `neb check` prints has to mean the same rule in all three
documents.

The numbers exist only in the `Rule` enum. A shell or docs-lint gate would
have to restate them or grep the Rust source for them. Either option copies
the rule table into a second place, or does the text-matching the rule
forbids in a worse form. What is given up: rewording a table label fails a
core test, which is the point, because the labels are the published meaning.
Scope: this test only. No other test reads repository docs or source.
Reverses if the tables are generated from the `Rule` enum. The guard then has
nothing left to compare.

## Migration commits by writing `config.yaml` last

A file corpus has no transactions, so this departs from the clause of
STD-03@2 §R23 that applies each migration in one transaction with its ledger
record. The ledger is `config.yaml`'s `schema_version`, and the equivalent is:

- Convert every node in memory before writing any, so a node that cannot be
  converted refuses the run with nothing written. This step is ORB-13175's.
  Until it lands, a bad v1 node late in the corpus leaves the earlier nodes
  rewritten.
- Write each node through the corpus's one atomic-write helper.
- Write `config.yaml` last. Until it is rewritten, every verb except `migrate`
  refuses the corpus with `Error::SchemaMismatch`, so no reader treats a
  half-converted corpus as current.
- Re-running is safe and idempotent. A node already in v2 form renders back to
  the bytes on disk and is left alone, so a second run finishes an interrupted
  one without bumping `updated`.

ORB-13175 also adds the ordered, append-only registry and the fresh-versus-
migrated parity that the rest of the rule asks for. What is given up is
rollback: a crash mid-write leaves some nodes converted under an old ledger,
and recovery runs forward rather than back. Under git, the clean-tree refusal
means the pre-migration state is also one `git checkout` away. Reverses if the
corpus moves into a store with transactions.

## No durable copy of git's output

This departs from the last clause of STD-03@2 §R15: nebula keeps no durable log
of a child's output. The only children whose output it captures are `git`
and the hooks git runs.
Bounding their captured output and marking the cut is the git runner's job
(`crates/nebula-core/src/git.rs`): it keeps `GIT_OUTPUT_CAP` (1 MiB) of each
stream, ends a cut message in `… [truncated N bytes]`, refuses to parse cut
output, and discards the rest rather than keeping it. That is enough here because every git call nebula makes can be repeated by hand to
see the full text. A refused commit leaves the write on disk and staged, so
`git -C <root> commit` runs the same hooks over the same index. The other
calls are reads and queries, repeatable as they are.

What is given up: output from a failure that does not reproduce, such as a
flaky hook, is seen only in its truncated form. Scope: git and its hooks.
The interactive `$EDITOR` child's output goes to the terminal and is never
captured. Reverses if nebula starts a child whose run cannot be repeated,
such as one that talks to a network service or a model.

## The cycle check runs under the write lock

This departs from STD-03@2 §R1, which forbids holding a lock across a scan of a
large directory. `link` and `new` read every node (`Corpus::load_all`) to prove
that a genealogy edge closes no loop, and they do it while holding the corpus
write lock. The scan is the read half of the write's read-modify-write. Doing
it outside the lock needs a compare-and-set against the corpus as it was read,
and nebula has no corpus-wide version to compare: node files are also edited
by hand. Building one costs more than the wait it saves.

Only writers wait. Reads never take the lock, so `list`, `trace` and the
desktop's file watcher are unaffected. A waiting writer gives up after
`LOCK_WAIT` (5 s) with `Error::Locked`, before anything is written, and can
retry. Measured with a release build on dk-server-1 (2026-09-26), a warm-cache
`link` of a genealogy edge took about 0.1 s at 3,000 nodes, 0.2 s at 10,000
and 1.1 s at 30,000. Corpora today hold hundreds. Scope: the cycle checks in
`ops::link` and `ops::new_node` (`refuse_cycle`). Revisit at 10,000 nodes, or
sooner if a writer reports `Error::Locked` behind a `link` or `new`. That is
also when "Full scan instead of an index" above comes due.

## `review`'s cut notice is part of the document

STD-01@2 §R12 sends pagination hints to stderr. The full `review` report keeps
its `- _… and K more; raise --limit for more_` line inside the markdown, one
line per cut section, because that report is a document rather than a list.
It is the markdown that goes into a review file, `--out` writes it to a file
that has no stderr, and it is read later by someone who never saw the command
run. A section that lost findings without saying so would read as complete,
the failure STD-01's §R34 exists to prevent. The line appears only when
`--limit` cut something, and never without `--limit`.

Scope: the full `review` markdown only. `review --short` is a list, and
ORB-13190 moves its cut notice to stderr as §R12 asks. ORB-13190 also repeats
the full report's notice on stderr. The machine signal belongs to `--json`, not
to this line: ORB-13191 gives `review --limit --json` the `{total, truncated}`
envelope. Reverses if the review stops being written to a file for later
reading.

## Reports are not tables

STD-01@2 §R14's table layout applies to list-shaped human output: one header
row, two-space gutters, one line per record (ORB-13193). These verbs are
reports, not lists, and keep their own layouts:

- **`show`** is one record's detail view, key by key. STD-01's §R16 already
  permits this layout for `show`.
- **`trace`** is a tree on a terminal. The shape of the lineage is the content,
  and a table would flatten it. ORB-13190 gives its piped form one
  tab-separated line per node.
- **`impact`** groups what it reaches under a heading per relation
  (`descends from`, `contradicts`).
- **`check`** is a verdict. It gives one finding per line with its severity,
  rule number and node, then a tally, and the exit code carries the result.
  Of the five, it is closest to a list. It is kept as a report because each
  finding's message is free prose that no column width suits.
- **`review`** is the markdown document of the decision above.

What is given up: these views cannot be cut or awk'd by column. Each one has a
`--json` form carrying the same payload, and that is the form for a program.
Reverses for any of them that turns out to be read as a list, which becomes a
table.

## The `--json` contract: every field, one author shape, capped lists say so

Core's types serialise for two readers that want absence left out: the YAML
frontmatter, which stores the human's authorship and every empty field by
omission, and the desktop's generated TypeScript. A script reading
`neb … --json` wants the opposite, so the CLI serialises through a view,
`crates/neb/src/render/json.rs`, rather than changing core's serde
attributes. Every documented field is present, an absent value `null` and an
empty collection `[]` (`STD-01@2 §R11`), and every author label is stated
through `Node::with_authorship_stated`, so a write verb's node has the key set
and the labels `show` gives it (`STD-01@2 §R10`). Each view destructures its
core type in full, so a field added to core does not compile until the view
names it. The node files and `apps/desktop/src/types` are unchanged.

A list a limit cut says so in its payload, as
`{"items": [...], "total": N, "truncated": bool}` (`STD-01@2 §R34`), with
`total` the number that matched before the cut:

- **`near` always**, because it always has a limit (`-k`, default 3). A cut
  also says `K of N shown; raise -k for more` on stderr in every mode. Core
  returns the count beside `Near` (`graph::near_counted`), so the type the
  desktop reads keeps its bare shape.
- **`list`, `inbox`, `review` (full and `--short`) and `trace` only with the
  flag.** Given `--limit` or `--depth`, the answer is the envelope whether or
  not anything was cut, so its shape follows the flag and never the data;
  without the flag it is the bare array `§R34` allows for an unbounded list,
  which keeps `neb list --json | jq '.[]'` in the routine working. For
  `review`, `total` counts every rule's findings before each kept its first N;
  for `trace --depth`, it is the size of the unbounded walk.

0.2.0 is unreleased, so these shape changes are made now, while they cost
nothing; after the release each would be a breaking change (`STD-01@2 §R10`).
Reverses only by the same route: a recorded breaking change to the machine
contract.

## An offset-less inbox stamp is local time

STD-01@2 §R11 requires machine timestamps to be RFC 3339 with an explicit
offset, and the inbox stamp was not: `2026-09-26T08:11`, to the minute, with
no offset, and silently in UTC when the local offset could not be read. From
0.2.0 capture stamps RFC 3339 to the second with the offset it used,
`2026-09-26T08:11:05+02:00`, or `…Z` when it fell back to UTC, so a fallback
never passes for local time; `+00:00` is a local offset of zero that was
actually read.

The inbox files are a persisted format, so the change is compatible
(STD-02@2 §R16). A legacy line still loads and is never rewritten: settling
it strikes the line through with the stamp as written, and ids stay what the
line says, since nothing re-derives an id from its stamp. Its `at`, in
`neb inbox --json`, `drop --json` and the desktop, is interpreted rather than
passed through: local time with the offset this machine has for that
instant (`UtcOffset::local_offset_at`), or `Z` when there is none. That is
what the old writer meant whenever it could read the offset, and it keeps
`at` one type for every consumer. The alternative, the legacy text in `at`
beside a separate normalized field, was rejected because it leaves `at`
failing §R11 for exactly the entries that need interpreting, and makes every
consumer handle two forms. The cost is the ambiguity §R16 warns about: a
legacy stamp written under the silent UTC fallback reads as local time, off
by the machine's offset, and nothing in the line can tell the two apart.
`triage` orders by instant, and an entry's age counts from the date its stamp
was taken on in its own offset, which for a legacy stamp is the date it
shows. Reverses if a legacy stamp is found that was not written in the
machine's local time often enough to mislead, in which case legacy `at`
should pass through as written with a separate field for the reading.

## Owner-only modes are set on Unix only

This departs from STD-05@1 §R8, a MUST, on platforms without Unix modes.
`nebula_core::fs` creates files `0600` and directories `0700` on Unix and sets
nothing elsewhere: the standard's Deviations section says Windows meets the
rule with owner-only ACLs, and none are applied. The rename is still atomic
there. nebula builds, tests and ships on Linux and macOS only, so no supported
platform is affected. Reverses if a Windows build is ever supported; the
helper is the one place an ACL would be set.

## The editor stays in the terminal's process group, with no deadline

This departs from STD-03@2 §R11, which puts every child in a process group of
its own, and from STD-03@2 §R12 and §R22, which give every wait on a child a
deadline ended by SIGTERM, a grace period and SIGKILL of the group. Every git
child goes through the supervised runner in `crates/nebula-core/src/git.rs`,
which meets them. The one other child is the `$VISUAL`/`$EDITOR` that
`neb edit` opens on a node's body (`edit_body` in `crates/neb/src/cli.rs`), and
it stays in `neb`'s own process group, the terminal's foreground group.

An editor is a terminal program. In a group of its own it would be a
background job, stopped by SIGTTIN the first time it read a key, and Ctrl-C,
Ctrl-Z and window resizes would stop reaching it. It has no deadline because
a person drives it, for as long as they like. `neb` does nothing else while it
waits, and no git child is live meanwhile, so the runner's signal handling
never involves it; a Ctrl-C reaches the editor and `neb` together, which is
what a person pressing it means.

What is given up: an editor that hangs holds `neb edit` until someone ends
it. Scope: the editor child of `neb edit`. Reverses if nebula ever opens an
editor with no person at the terminal, such as for an agent or a routine,
which would then need a group and a deadline like git.
