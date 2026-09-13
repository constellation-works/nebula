---
title: "Nebula — Idea Lineage Graph (v0.1 spec)"
type: project
status: active
created: 2026-09-07
updated: 2026-09-12
tags: [research-process, knowledge-graph, constellation, tooling]
related: ["[[principia]]", "[[orbit-research]]"]
---

# Nebula

> This is the original design conversation, kept as the record of why the
> system is shaped this way. It described a v0.1 model built around `domain`,
> `evidence`, `weigh` and `graduate`; the 2026-09-12 reduction removed all four
> in favour of tags, references, and hand-off by citation rather than by verb
> (see [docs/design/v0.2/1_spec.md](design/v0.2/1_spec.md), "What is
> removed"). The maintained documentation is
> [docs/design/v0.2/1_spec.md](design/v0.2/1_spec.md) for the current model,
> [docs/design/lineage-graph/](design/lineage-graph/1_overview.md) for the
> design reasoning that survived the reduction, and
> [docs/runbooks/](runbooks/corpus-setup.md) for operation. Where they
> disagree, those are right and this is history.

A place to put an idea the moment you have it, and a way to trace where any
idea came from years later.

Working name only. Nebula is diffuse material that collapses under its own
gravity into stars, which is the metaphor. `transit` and `ephemeris` are the
alternates if you want an instrument name instead. The name touches only the
directory and the CLI binary, so it is cheap to change before v0.1 ships.

## Why

Ideas arrive vague, at random, across unrelated fields, and the current
options are all wrong for that moment. principia's smallest unit is a claim,
which already demands a crisp statement with a `kind` and a `status`. There is
nowhere to put "huh, that's odd." orbit-research is stricter still. So the
observation goes into a note, a chat, or nothing, and it is gone.

The second failure is lineage. principia partitions its claims by `family`,
which is a flat ten-way grouping, and every other relation is smuggled into an
untyped `links` array that mixes ancestry with sourcing and sibling references
in one list. There is no "derived from" edge in the schema at all. That is the
mechanical reason you cannot pick a claim and walk back to the thought that
started it.

Nebula owns the messy upstream. It never deletes anything, and its one
non-negotiable feature is the lineage walk.

## Capture

This is the part that decides whether the system lives or dies, so it gets
the strictest constraint in the spec: **capture must take under five seconds
and require no decisions.**

```
neb capture "ranking signal decay looks like it has a half-life, not a cliff"
```

Appends one timestamped line with a short id to `inbox/YYYY-MM.md`. No parent,
no tags, no status. If capture ever asks you to pick a parent, you will stop
capturing, and the whole thing is dead.

Inbox entries are **not nodes**. Promotion is a separate, explicit act, and
most captures should never be promoted. Dropping an entry is a normal
outcome, not a failure.

```
neb promote ab3f --parent gravity-as-scarcity-seed
```

## One file, and why it holds

References are a frontmatter array living in the node file, not a separate
attachment file. Splitting them out would buy cleaner concurrent appends, and
at this scale that is not worth paying for.

Two properties of the actual workload decide it. **Writes are infrequent**, so
the merge conflict that splitting prevents is a rare event rather than a daily
one. **The corpus is small**, so no node accumulates enough references to make
its frontmatter unwieldy. A third property is already a rule in this spec: an
unattended agent proposes and never writes to a node directly, which means you
are effectively the only writer in that mode, editing one node at a time.

What one file buys is worth more under those conditions. Reading the file shows
the entire node with no join and no tooling, which matters most on the day the
tool is broken or you are looking at the repository from a phone. One `git log`
gives the node's whole history in order. A grep hit lands in a file that
already carries its own context.

Revisit only if a node's frontmatter routinely outgrows its prose, or if
something other than you starts writing node files directly. Neither is true
now, and designing for either would be speculative.

## Stage four is an edge

Refinement creates a **new** node with a `refines` edge to the old one. The
old node keeps its status and its content. Killing flips a status and changes
nothing else. Nodes are never edited into unrecognisability and never
deleted, because the dead branches are the part that stops you re-treading
ground you already covered. principia has this instinct already in its
refuted wall, which forbids claim ids from quietly vanishing. Nebula applies
that rule to everything from the start.

## Boundaries

Nebula sits **upstream** of everything else and hands off rather than growing
into it.

- A node that reaches a real hypothesis with a real kill condition, and needs
  simulation, is picked up by a principia gate card or claim that cites it.
- A node that needs preregistration, protocols and hard scientific invariants
  is picked up the same way by orbit-research.
- Neither of those is merged into Nebula. Their strictness is correct
  downstream and would destroy capture here. The hand-off is the downstream
  artifact citing the node, not a status or a verb here.
- A node picked up downstream stays in Nebula forever. The lineage does not
  end at the boundary.

Tags are the only thing that distinguishes one node's subject from another's.
No per-subject fields, ever.

## The model, today

The node shape, verbs and invariants that followed this point in the original
spec are v0.1 and no longer accurate: they described `domain`, `evidence`,
`verdict`/`strength`, `weigh`, `task` and `graduate`, none of which survive the
2026-09-12 reduction. The current model — four statuses, five edge kinds,
tag-normalised labels, references with notes, and ten invariants enforced by
`neb check` — is specified in full in
[docs/design/v0.2/1_spec.md](design/v0.2/1_spec.md); the architecture that
carries it across a core library, the CLI, a desktop app and an agent skill is
in [docs/design/v0.2/2_architecture.md](design/v0.2/2_architecture.md).

## Not in v0.1

No web UI, no search beyond grep, no sync beyond git, no automatic linking, no
multi-user. A disposable index for fast queries is allowed, rebuilt from the
markdown, never the source of truth.

## Milestone

**Shipped in v0.1**: `capture`, `inbox`, `promote`, `drop`, `new`, `sharpen`,
`link`, `cite`, `status`, `trace`, `impact`, `open`, `show`, `list`, `check`,
plus `--json` on every read command.

v0.1 lived for five days and eight nodes before the reduction specified in
[docs/design/v0.2/1_spec.md](design/v0.2/1_spec.md): the machinery had grown
heavier than the habit it was meant to serve, and a terminal is not where a
thought arrives. That document is now authoritative.

## Worked example

The diamond, since it is the shape the model exists to support.

```
neb capture "gravity might be about scarcity of something, not curvature"
neb promote a1 --title "Gravity as scarcity"          -> seed  gravity-as-scarcity-seed

neb capture "a moving source should drag the field, shouldn't it"
neb promote b7 --title "Moving source drag"           -> seed  moving-source-oddity

neb new "Retardation in the scarcity wake" \
    --parent gravity-as-scarcity-seed \
    --parent moving-source-oddity \
    --kill "If the wake timescale is frame-independent, this is dead."
```

```
gravity-as-scarcity-seed ─┐
                          ├─> scarcity-wake-retardation
moving-source-oddity ─────┘
```

`neb trace scarcity-wake-retardation` walks back and prints both roots with
their capture dates and original one-line text. That is the feature. Every
other thing in this document exists to keep that walk honest.
