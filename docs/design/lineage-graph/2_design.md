---
title: Lineage Graph — Design
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: lineage-graph
doc_role: design
type: design
summary: Storage layout, module boundaries, traversal, and how the corpus stays separate from the tool.
tags: [lineage-graph]
paths: ["src/**"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Lineage Graph — Design

## Repository boundary

This repository holds the tool. It never holds the corpus.

The corpus spans work and personal thinking across unrelated fields, so it is
found at runtime through `--root`, else `NEBULA_ROOT`, else `~/.nebula`. Keeping
them apart means the code can be public while the notes stay private, and the
notes can be backed up without dragging a build tree along.

## Storage

One markdown file per node, in a flat `nodes/` directory, named `<id>.md`.
Everything about a node lives in that file: edges, references, provenance and
the prose argument. Captures append to `inbox/YYYY-MM.md`.

Flat is deliberate. Nesting nodes would encode a single parent into the
filesystem, and a node can have several.

References are a frontmatter array rather than a separate file. Splitting them
out would buy cleaner concurrent appends, which is not worth paying for here:
writes are infrequent, the corpus is small, and an agent proposes rather than
writes in routine mode, so there is effectively one writer at a time. Revisit
if a node's frontmatter routinely outgrows its prose, or if a second concurrent
writer appears.

Every node write is rendered to a sibling temporary file and renamed, so an
interrupted write cannot leave half a node behind.

`config.yaml` at the corpus root holds exactly two things: `schema_version` and
an opaque `corpus_id`. Nothing else is configured — tags on the nodes
themselves are how the corpus is partitioned for a view. `neb init` writes it;
`neb migrate` bumps `schema_version` in place.

## Tags, not a declared list

Every node carries free-form tags, normalised to lowercase kebab-case on write.
There is no closed list declared anywhere, unlike v0.1's required categorical
field: a tag is exactly as much structure as a node needs, most nodes need
very little, and a required field validated against a config-file list turned
out to be one more decision standing between a person and capturing the idea. `list` and
`open` narrow with `--tag` (repeatable, every one required); `trace` and
`impact` always walk the whole corpus regardless of tags, because an
observation in one field feeding a hypothesis in another is precisely the link
the graph exists to surface. `neb tag list` and `check`'s drift warning (two
tags differing only by case or a trailing `s`) are what keep free-form tags
from rotting the way a closed list was meant to prevent. See
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md) ("What is removed") for why the
declared list was cut rather than kept alongside tags.

The boundary that still needs separate storage is trust, not topic. Work and
personal ideas belong to different owners, so they are separate corpora with
separate configs, and a lineage that genuinely crosses that line is recorded as
a reference rather than an edge.

## Modules

`src/main.rs` is the only file directly under `src/`; everything else is a
module directory. Dependencies point downward only.

| module | holds |
|---|---|
| `cli` | the clap tree and dispatch; the only module that knows clap |
| `commands` | one function per verb group: `inbox`, `node` (new, sharpen, link, status, tag), `attach` (cite), `read` (show, list, trace, impact, open, review), `check`, `migrate` |
| `check` | `mod` runs the whole-corpus rules; `rules` the per-node ones; `graph` cycle detection |
| `render` | terminal output; `tree` draws ancestry |
| `corpus` | the data layer: `model` (schema, parse, atomic write), `store` (location, load, save, inbox), `config` (`config.yaml`) |

`corpus`, `check` and `render` know nothing about the layers above them. That
is deliberate: a second consumer appearing — the desktop app and the agent
skill both now exist — is exactly the case
[docs/design/v0.2/2_architecture.md](../v0.2/2_architecture.md) plans for, by
splitting this binary into a pure `nebula-core` library and a thin `neb` CLI
that wraps it. That split is a separate task; this module boundary is what
makes it a mechanical move rather than a redesign.

## Traversal

`load_all` reads and parses every node on each invocation. This is a full scan by
choice: the corpus is small and writes are rare, so an index would be a second
source of truth able to drift, in exchange for time that is not currently scarce.
Revisit when a scan is measurably slow, not before.

`trace` walks genealogy upward by default and downward with `--down`. `impact`
walks descendants and `contradicts` neighbours. The tree renderer prints a node
at every place it appears, so a diamond is visible as a diamond, but expands it
only once, so the output stays finite on a graph with many shared ancestors.

## Enforcement placement

Some invariants are enforced by the type system rather than the checker. A
reference carrying an unknown key, or a node with an unknown field, fails to
deserialize, so the corpus refuses to load rather than reporting a finding.
That is deliberate: a rule that merely warned would be routed around the
moment it was inconvenient.

Others are enforced at the point of action. `neb link` refuses a genealogy edge
that would close a cycle, rather than accepting it and letting `check` find it
later, because the moment of linking is when you still remember what you meant.
`neb status` refuses to move a node to `refuted` without `--why`, and refuses
to move a `refuted` node anywhere at all, for the same reason.
