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

`config.yaml` at the corpus root carries a `schema_version`, an opaque
`corpus_id`, the closed list of `domains`, and an optional `default_domain`.
`neb init` writes it; `neb domain` edits it; a corpus without one loads as a
single `general` domain so nothing predating the file breaks.

## Domains

Every node names exactly one domain, and the set of domains is declared in
`config.yaml` rather than inferred from use. Two things follow.

A domain is a view, not a wall. Edges cross domains without restriction and
`trace` and `impact` always walk the whole corpus, because an observation in one
field feeding a hypothesis in another is precisely the link the graph exists to
surface. Only `list` and `open` scope, and only when more than one domain is
declared: they narrow to `default_domain`, take `--domain` to pick another, and
`--all` to cross. A single-domain corpus never sees any of this.

The domain is assigned at `promote` and `new`, never at `capture`. Capture is
the five-second path and choosing a domain is a decision; a seed often does not
have one yet, which is part of what makes it a seed.

The boundary that does need separate storage is trust, not topic. Work and
personal ideas belong to different owners, so they are separate corpora with
separate configs, and a lineage that genuinely crosses that line is recorded as
a reference rather than an edge.

## Modules

`src/main.rs` is the only file directly under `src/`; everything else is a
module directory. Dependencies point downward only.

| module | holds |
|---|---|
| `cli` | the clap tree and dispatch; the only module that knows clap |
| `commands` | one function per verb, grouped by subject: `inbox`, `node`, `attach`, `read`, `domain`, `check` |
| `check` | `mod` runs the whole-corpus rules; `rules` the per-node ones; `graph` cycle detection |
| `render` | terminal output; `tree` draws ancestry |
| `corpus` | the data layer: `model` (schema, parse, atomic write), `store` (location, load, save, inbox), `config` (`config.yaml`) |

`corpus`, `check` and `render` know nothing about the layers above them. That
is deliberate: if a second consumer appears, an MCP server or orbit-research
reading the graph directly, those three become a library crate by adding
`lib.rs` and leaving `cli` and `commands` in the binary.

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
