---
title: Lineage Graph — Decisions
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: lineage-graph
doc_role: decisions
type: design
summary: The choices that shaped nebula, each with the reasoning and the condition that would reverse it.
tags: [lineage-graph, v0.2]
paths: ["src/**"]
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
there is exactly one graph, and it is acyclic.

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
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md), "What is removed").

principia and orbit-research both demand a kill condition, so nebula refused to
move a node to a graduated status without one rather than exporting the gap
downstream. That protection is no longer needed once graduating is not a
status at all: a downstream artifact that cites a node either names what it is
relying on or it does not, and that is a property of the downstream artifact,
not something nebula can enforce by refusing a transition.

## A module tree, not a flat `src/`

The tool is small enough that a flat directory would work today. It is laid
out as a tree anyway, with `main.rs` as the sole file under `src/`, because the
layering is the part worth keeping honest over years: `corpus` must never learn
about the terminal, and `commands` must never learn about clap. A directory per
layer makes a violation visible in an import path. The layering also fixed the
seam for a future library crate without deciding at the time whether one was
needed — see "A core-library workspace" below for the point it got cashed in.

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
