---
title: "Nebula — Idea Lineage Graph (v0.1 spec)"
type: project
status: active
created: 2026-09-07
updated: 2026-09-07
tags: [research-process, knowledge-graph, constellation, tooling]
related: ["[[principia]]", "[[orbit-research]]"]
---

# Nebula

> This is the original design conversation, kept as the record of why the system
> is shaped this way. The maintained documentation is
> [docs/design/lineage-graph/](design/lineage-graph/1_overview.md) for design and
> [docs/runbooks/](runbooks/corpus-setup.md) for operation. Where they disagree,
> those are right and this is history.

A place to put an idea the moment you have it, and a way to trace where any
idea came from years later.

Working name only. Nebula is diffuse material that collapses under its own
gravity into stars, which is the metaphor. `transit` and `ephemeris` are the
alternates if you want an instrument name instead. The name touches only the
directory and the CLI binary, so it is cheap to change before v0.1 ships.

## Why

Ideas arrive vague, at random, across unrelated domains, and the current
options are all wrong for that moment. principia's smallest unit is a claim,
which already demands a crisp statement with a `kind` and a `status`. There is
nowhere to put "huh, that's odd." orbit-research is stricter still. So the
observation goes into a note, a chat, or nothing, and it is gone.

The second failure is lineage. principia partitions its claims by `family`,
which is a flat ten-way grouping, and every other relation is smuggled into an
untyped `links` array that mixes ancestry, evidence and sibling references in
one list. There is no "derived from" edge in the schema at all. That is the
mechanical reason you cannot pick a claim and walk back to the thought that
started it.

Nebula owns the messy upstream. It is domain agnostic by construction, it
never deletes anything, and its one non-negotiable feature is the lineage
walk.

## Model

### Node

One unit of inquiry. One markdown file with YAML frontmatter in a **flat**
`nodes/` directory, named `<id>.md`. Everything about a node lives in that one
file: its edges, its evidence, its references, its provenance and its prose.

Flat is deliberate. Nesting would encode a single parent into the filesystem,
and a node can have several.

```yaml
---
id: scarcity-wake-retardation        # kebab-case, permanent, never reused
title: "Retardation in the scarcity wake"
status: hypothesis
created: 2026-09-07
updated: 2026-09-07
kill: "If the wake timescale is frame-independent under a boosted source, this is dead."
tags: [physics, gravity]
edges:
  - {type: derives-from, to: gravity-as-scarcity-seed}
  - {type: derives-from, to: moving-source-oddity}
  - {type: depends-on,   to: closed-system-existence}
evidence:
  - id: ev1
    verdict: supports
    strength: suggestive
    source: "../../orrery/lab/sims/scarcity-capped-cumulative-field/"
    date: 2026-09-07
    note: >
      Gradient sign matches the prediction. Only tested for static sources,
      so it says nothing yet about the moving case, which is the whole point.
    origin: {task: ORB-11396, run: jrun-20260906-0537-3, artifact: wake-sweep-summary}
references:
  - id: r1
    kind: study
    uri: "../../principia/studies/gravitational-time-dilation.md"
    title: "Gravitational time dilation"
    note: "The constraint any wake timescale has to survive."
    added: 2026-09-07
  - id: r2
    kind: discussion
    uri: "[[15-discussions/26-08/scarcity-and-retardation]]"
    title: "Where the retardation idea came up"
    note: "Read for the alternatives we discarded; the sharp version is this node."
    added: 2026-09-07
origin:
  task: ORB-11402
  workspace: ws_constellation
  agent: claude
  at: 2026-09-07T10:12:00Z
tasks:
  - {id: ORB-11440, state: open, why: "Boosted-source sim to test the kill condition"}
graduated_to: null
---

Free prose. The argument, the sketch, the thing you actually thought. No
required structure.
```

### One file, and why it holds

Evidence and references are frontmatter arrays living in the node file, not
separate attachment files. Splitting them out would buy cleaner concurrent
appends, and at this scale that is not worth paying for.

Two properties of the actual workload decide it. **Writes are infrequent**, so
the merge conflict that splitting prevents is a rare event rather than a daily
one. **The corpus is small**, so no node accumulates enough evidence to make
its frontmatter unwieldy. A third property is already a rule in this spec: the
maintenance agent proposes and never writes to a node, which means you are
effectively the only writer, editing one node at a time.

What one file buys is worth more under those conditions. Reading the file shows
the entire node with no join and no tooling, which matters most on the day the
tool is broken or you are looking at the repository from a phone. One `git log`
gives the node's whole history in order. A grep hit lands in a file that
already carries its own context.

Revisit only if a node's frontmatter routinely outgrows its prose, or if
something other than you starts writing node files directly. Neither is true
now, and designing for either would be speculative.

### Status

The four stages you described are the node's status, not sub-branches inside
it. Flattening them is what makes the graph queryable.

| status | meaning | required to enter |
|---|---|---|
| `seed` | vague idea, observation, corollary | one line of text |
| `hypothesis` | sharpened into something that could be wrong | non-empty `kill` |
| `testing` | evidence being gathered | at least one evidence entry |
| `supported` | evidence favours it, for now | evidence with `supports` verdict |
| `refuted` | the kill condition fired | evidence with `undermines` verdict |
| `abandoned` | lost interest, not worth the effort | nothing |
| `graduated` | promoted downstream | `graduated_to` URI |

`supported` is never terminal and never certain. `refuted` and `abandoned`
are different states on purpose: one is wrong, the other is untouched. Both
stay in the graph forever.

### Stage four is an edge, not a stage

Refinement creates a **new** node with a `refines` edge to the old one. The
old node keeps its status and its content. Killing flips a status and changes
nothing else. Nodes are never edited into unrecognisability and never
deleted, because the dead branches are the part that stops you re-treading
ground you already covered. principia has this instinct already in its
refuted wall, which forbids claim ids from quietly vanishing. Nebula applies
that rule to everything from the start.

### Two graphs, one node set

Conflating these is what made principia's `links` field opaque.

**Genealogy** answers "where did this come from." It must be acyclic, and the
checker enforces that.

| edge | meaning |
|---|---|
| `derives-from` | this was prompted by that |
| `refines` | this is a sharpened successor to that |
| `generalizes` | this is the broader form of that |
| `reopens` | this revives a refuted node, and needs an explicit flag |

A node with two or more `derives-from` parents is a merge. That is the
diamond, it is legal, and it is the case where one idea branched into several
lines that later informed a single synthesis. There is no separate merge edge
type; multiple parents is the whole mechanism.

**Dependency and evidence** answers "what breaks if this dies."

| edge | meaning |
|---|---|
| `supports` | this being true makes that more likely |
| `undermines` | this being true makes that less likely |
| `depends-on` | if that dies, this dies with it |
| `contradicts` | these cannot both be true, symmetric |

This graph may contain cycles, and that is not an error to reject. A cycle in
`supports` is circular reasoning, which is a finding worth surfacing rather
than forbidding. The checker warns on it.

### Evidence

Attached to a node, never a node of its own. Coarse on purpose.

- `verdict`: `supports` | `undermines` | `inconclusive`
- `strength`: `anecdote` | `suggestive` | `strong`
- `source`: any URI or path. A paper, a sim, a study note, a screenshot, a memory.
- `note`, `date`

Three strength levels, no weighting scheme, no Bayesian arithmetic. Rigor is
downstream's job and pretending otherwise here would be false precision.

### References

Context, not evidence. A reference explains or situates the idea. It never
bears on whether the idea is true, it carries no verdict, and it can never
move a node's status.

That separation is load-bearing. If references could carry weight you would
route everything through them and quietly skip the verdict that `evidence`
demands, which is exactly the discipline the whole system exists to enforce.
The schema rejects a `verdict` key on a reference for that reason.

- `kind`: `paper` | `study` | `article` | `note` | `discussion` | `book` | `dataset` | `thread` | `other`
- `uri`: a URL, a DOI, a repo path, or an almanac wikilink such as `[[15-discussions/26-08/...]]`
- `title`, `added`
- `note`: **why this is attached**

The note is the only field that matters in a year. A bare link is how these
collections rot, so `check` warns on any reference without one. It warns
rather than fails, because a link you cannot yet explain is still better
captured than lost.

Vault paths are first-class. Distilled discussions in `15-discussions/`,
sourced notes in `10-notes/` and `30-references/`, and principia studies are
all legitimate reference targets, which means the almanac becomes the reading
layer under the graph instead of a parallel pile.

A reference can be promoted to evidence later, once you have actually read it
closely enough to say which way it cuts:

```
neb cite scarcity-wake-retardation --promote r1 --verdict undermines --strength strong
```

Promotion appends a new entry to `evidence` carrying the reference's note and
provenance, and leaves the original in `references` marked `promoted_to`.
Nothing is moved or deleted, so the reading history stays intact, which is the
same rule the node graph follows.

## Orbit provenance

Nodes and evidence rarely appear from nowhere. They come out of a session, a
dispatched task, or a job run that produced an artifact. That context is worth
almost nothing at the time and a great deal two months later, so it gets
recorded automatically and never typed by hand.

An `origin` block may hang off a node, an evidence entry, or a reference.
Every field is optional and an absent block is normal, since plenty of ideas
genuinely do arrive in the shower.

| field | meaning |
|---|---|
| `task` | Orbit task id, e.g. `ORB-11440` |
| `workspace` | workspace selector, copied verbatim, never constructed |
| `run` | job run id, e.g. `jrun-20260906-0537-3` |
| `artifact` | task artifact key holding the output this came from |
| `agent` | `claude`, `codex`, `gemini`, `grok` |
| `at` | timestamp |

`origin` points backward at what produced the node. The `tasks` list points
forward at work the node has spawned:

```yaml
tasks:
  - {id: ORB-11440, state: open, why: "Boosted-source sim to test the kill condition"}
```

That forward link is what turns the graph from a filing cabinet into
something with a work queue attached. `neb open` can then surface the genuinely
actionable gap, which is a hypothesis with a kill condition, no evidence, and
no open task against it. Nothing else in the system tells you that.

Two cautions worth writing down now, both learned the hard way elsewhere in
the constellation:

- **Never hardcode an agent family.** orbit-research pinned `model='codex'` into its task lookup and the crew names have since changed underneath it. Read the value, default to the running agent, and let it be overridden.
- **`orbit.task.add` silently ignores a `dependencies` field.** If a spawned task needs to depend on another, file it first and wire the dependency with a follow-up `task.update`. Filing a task also triggers dispatch, so file it only when the node is genuinely ready for the work.


## Capture

This is the part that decides whether the system lives or dies, so it gets
the strictest constraint in the spec: **capture must take under five seconds
and require no decisions.**

```
neb capture "ranking signal decay looks like it has a half-life, not a cliff"
```

Appends one timestamped line with a short id to `inbox/YYYY-MM.md`. No parent,
no type, no tags, no status. If capture ever asks you to pick a parent, you
will stop capturing, and the whole thing is dead.

Inbox entries are **not nodes**. Promotion is a separate, explicit act, and
most captures should never be promoted. Dropping an entry is a normal
outcome, not a failure.

```
neb promote ab3f --parent gravity-as-scarcity-seed
```

## Commands

| command | does |
|---|---|
| `capture <text>` | the five-second path |
| `inbox` | list unprocessed captures |
| `promote <ref>` | inbox entry becomes a seed node |
| `sharpen <id> --kill "..."` | seed becomes hypothesis |
| `link <from> <type> <to>` | add an edge |
| `evidence <id> --verdict --strength --source` | attach evidence |
| `cite <id> --kind --uri --note` | attach a reference |
| `cite <id> --promote <ref>` | a reference becomes evidence |
| `task <id> --title "..."` | file an Orbit task and link it both ways |
| `trace <id>` | walk ancestry, the headline feature |
| `impact <id>` | reverse walk: what dies if this dies |
| `open` | nodes needing attention |
| `check` | run the invariants |
| `graduate <id> --to <uri>` | hand off downstream |

## Invariants

`check` is the lock, the same role `check-theory.py` plays in principia.

1. The genealogy graph is acyclic.
2. `hypothesis` and later requires a non-empty `kill`.
3. `supported` and `refuted` each require at least one evidence entry with the matching verdict.
4. Every edge target resolves to an existing node.
5. `contradicts` is mutual. One-sided declarations fail.
6. No id in the manifest may disappear. Deletion is a hard error.
7. A `refuted` node cannot return to an active status without a `reopens` edge from a new node and an explicit override flag.
8. A `graduated` node carries a resolvable `graduated_to` URI.
9. Warn, do not fail, on a cycle in the `supports` graph.
10. A reference may not carry a `verdict` or a `strength`. Evidence is the only thing that bears on truth.
11. Warn on any reference with an empty `note`.
12. Evidence and reference ids are unique within a node, and are never reused after removal.
13. An `origin.task` or `tasks[].id` must be a well-formed Orbit task id. With `check --online`, verify it resolves and that a `tasks` entry marked open is still open.
14. Every reference and evidence `uri` that points inside the checkout must resolve. External URLs are not fetched by `check`.

## The maintenance loop

The schema is the easy half. The reason principia drifted is that curation is
work nobody does, so an agent owns it. Run as an Orbit routine.

**Nightly.** Read the inbox. For each entry propose exactly one of: promote as
a new seed, attach as evidence or a note to an existing node, or drop. Include
the reasoning in one line.

**Weekly.** Flag hypotheses with no evidence for thirty days. Flag seeds
untouched for ninety days and propose abandoning them. Flag nodes whose stated
kill condition looks satisfied by evidence that has landed since. Propose
`contradicts` pairs found by similarity. Propose references for nodes that have
none, drawn from the almanac and from principia studies.

**On task completion.** When an Orbit task listed in a node's `tasks` finishes,
reconcile its state and propose attaching its artifacts as evidence, with a
suggested verdict and the reasoning. This closes the loop: a hypothesis names
what would kill it, spawns a task to go find out, and the answer comes back
attached to the node that asked the question. Proposed, as always, never
applied.

**The hard rule: the agent proposes, and never mutates a node.** All output
goes to a single `review.md` that you skim and accept in one pass. The moment
it edits nodes on its own you stop trusting the graph, and an untrusted graph
is worse than no graph.

## Boundaries

Nebula sits **upstream** of everything else and hands off rather than growing
into it.

- A node that reaches a real hypothesis with a real kill condition, and needs simulation, graduates to a principia gate card and claim.
- A node that needs preregistration, protocols and hard scientific invariants graduates to orbit-research.
- Neither of those is merged into Nebula. Their strictness is correct downstream and would destroy capture here.
- Graduated nodes stay in Nebula forever with a link out. The lineage does not end at the boundary.

Domain agnostic means one schema for physics, economics, social science and
ranking signals alike. Only `tags` distinguish them. No per-domain fields,
ever.

## Not in v0.1

No web UI, no search beyond grep, no sync beyond git, no evidence weighting,
no automatic linking, no multi-user. A disposable index for fast queries is
allowed, rebuilt from the markdown, never the source of truth.

## Milestone

**Shipped in v0.1**, wider than this document originally planned because the
lifecycle verbs turned out to be unusable without each other: `capture`, `inbox`,
`promote`, `drop`, `new`, `sharpen`, `link`, `evidence`, `cite`, `weigh`,
`status`, `task`, `graduate`, `trace`, `impact`, `open`, `show`, `list`, `check`,
plus `--json` on every read command.

What has not been built is everything above the corpus: no maintenance agent, no
browser, no Orbit tool surface. Those wait on evidence. Live on this for two
weeks, accumulate roughly fifty real nodes, and only then decide whether the rest
of this document survives contact with your actual habits.

## Worked example

The diamond, since it is the shape the model exists to support.

```
neb capture "gravity might be about scarcity of something, not curvature"
neb promote a1 --title "Gravity as scarcity"          -> seed  gravity-as-scarcity-seed

neb capture "a moving source should drag the field, shouldn't it"
neb promote b7 --title "Moving source drag"           -> seed  moving-source-oddity

neb new --title "Retardation in the scarcity wake" \
    --derives-from gravity-as-scarcity-seed \
    --derives-from moving-source-oddity \
    --kill "If the wake timescale is frame-independent, this is dead."
```

```
gravity-as-scarcity-seed ─┐
                          ├─> scarcity-wake-retardation ─> (graduated: principia)
moving-source-oddity ─────┘
```

`neb trace scarcity-wake-retardation` walks back and prints both roots with
their capture dates and original one-line text. That is the feature. Every
other thing in this document exists to keep that walk honest.
