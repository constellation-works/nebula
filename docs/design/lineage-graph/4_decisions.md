---
title: Lineage Graph — Decisions
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: decisions
type: design
summary: The choices that shaped nebula, each with the reasoning and the condition that would reverse it.
tags: [lineage-graph]
paths: ["src/**"]
related_features: [lineage-graph]
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
link` refuses the edge that would close a loop. The evidence graph is left
unconstrained, because a cycle in `supports` is circular reasoning: a real
finding worth surfacing, and occasionally an honest description of two ideas that
genuinely lean on each other. It warns and does not fail.

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

## References cannot carry a verdict

Evidence bears on truth and carries a verdict. References give context and carry
none. The separation is enforced by `deny_unknown_fields`, so a reference with a
verdict fails to parse. Had it merely warned, findings would be routed through
references to dodge the discipline evidence demands, and that discipline is the
reason the system exists.

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
enumerate every site to fix.

## One file per node

Reverses cleanly if the workload changes. Splitting evidence into separate files
would buy conflict-free concurrent appends; writes here are infrequent, the
corpus is small, and the maintenance agent proposes rather than writes. Reading
one file shows the whole node with no join and no tooling, and one `git log`
gives its full history. Revisit if frontmatter routinely outgrows prose, or a
second writer appears.

## Full scan instead of an index

An index would be a second source of truth able to drift, bought with time that
is not currently scarce. Revisit when a scan is measurably slow.

## Graduating requires a falsifier

principia and orbit-research both demand a kill condition, so nebula refuses to
graduate a node without one rather than exporting the gap downstream.

## Domains inside a corpus, corpora per trust boundary

Tags were the only thing distinguishing physics from ranking signals, and tags
are open and optional, so a year of typing produces `principia`, `Principia` and
`physics` for one thing and nodes with nothing at all. A required `domain` field
validated against a closed set in `config.yaml` cannot drift or be forgotten.

Making each domain its own corpus was rejected. Edges cannot cross corpora, the
checker would flag any that tried as dangling, and `trace` would stop dead at
the boundary. The graph is the product, and the cross-domain edges are the ones
worth finding later. So principia is a domain inside the personal corpus, not a
corpus of its own. The split that does justify a second corpus is ownership:
work ideas and personal ideas have different owners, backups and legal standing.

The domain is chosen at `promote` or `new`, where a title and parents are
already being decided, and never at `capture`, which must stay decision-free.

## A module tree, not a flat `src/`

The tool is small enough that a flat directory would work today. It is laid
out as a tree anyway, with `main.rs` as the sole file under `src/`, because the
layering is the part worth keeping honest over years: `corpus` must never learn
about the terminal, and `commands` must never learn about clap. A directory per
layer makes a violation visible in an import path. The layering also fixes the
seam for a future library crate without deciding now whether one is needed.
