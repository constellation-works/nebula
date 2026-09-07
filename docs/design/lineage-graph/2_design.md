---
title: Lineage Graph — Design
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: design
type: design
summary: Storage layout, module boundaries, traversal, and how the corpus stays separate from the tool.
tags: [lineage-graph]
paths: ["src/**"]
related_features: [lineage-graph]
related_artifacts: []
---

# Lineage Graph — Design

## Repository boundary

This repository holds the tool. It never holds the corpus.

The corpus spans work and personal thinking across unrelated domains, so it is
found at runtime through `--root`, else `NEBULA_ROOT`, else `~/.nebula`. Keeping
them apart means the code can be public while the notes stay private, and the
notes can be backed up without dragging a build tree along.

## Storage

One markdown file per node, in a flat `nodes/` directory, named `<id>.md`.
Everything about a node lives in that file: edges, evidence, references,
provenance and the prose argument. Captures append to `inbox/YYYY-MM.md`.

Flat is deliberate. Nesting nodes would encode a single parent into the
filesystem, and a node can have several.

Evidence and references are frontmatter arrays rather than separate files.
Splitting them would buy cleaner concurrent appends, which is not worth paying
for here: writes are infrequent, the corpus is small, and the maintenance agent
proposes rather than writes, so there is effectively one writer. Revisit if a
node's frontmatter routinely outgrows its prose, or if a second writer appears.

Every node write is rendered to a sibling temporary file and renamed, so an
interrupted write cannot leave half a node behind.

## Modules

| module | holds |
|---|---|
| `model` | the schema, parse and render, atomic write |
| `store` | corpus location, load, save, inbox, dates, slugs |
| `check` | the invariants, and cycle detection over both graphs |
| `render` | terminal output, status colour, tree drawing |
| `commands` | one function per verb |
| `main` | the clap surface and dispatch |

## Traversal

`load_all` reads and parses every node on each invocation. This is a full scan by
choice: the corpus is small and writes are rare, so an index would be a second
source of truth able to drift, in exchange for time that is not currently scarce.
Revisit when a scan is measurably slow, not before.

`trace` walks genealogy upward by default and downward with `--down`. `impact`
walks `depends-on` and `supports` in reverse. The tree renderer prints a node at
every place it appears, so a diamond is visible as a diamond, but expands it only
once, so the output stays finite on a graph with many shared ancestors.

## Enforcement placement

Some invariants are enforced by the type system rather than the checker. A
reference carrying a `verdict`, or a node with an unknown field, fails to
deserialize, so the corpus refuses to load rather than reporting a finding. That
is deliberate for the reference case: the separation between context and evidence
is the discipline the system exists to impose, and a rule that merely warns would
be routed around.

Others are enforced at the point of action. `neb link` refuses a genealogy edge
that would close a cycle, rather than accepting it and letting `check` find it
later, because the moment of linking is when you still remember what you meant.
