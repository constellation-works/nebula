---
title: v0.2 — Reduced model
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: v0.2
doc_role: spec
type: design
summary: The node model, verbs and invariants after the 2026-09-12 reduction. Where this disagrees with docs/spec.md or docs/design/lineage-graph/, this is right.
tags: [v0.2, lineage-graph]
paths: ["crates/**", "src/**", "apps/**", "skills/**"]
related_features: [lineage-graph, desktop, skill]
related_artifacts: []
---

# v0.2 — Reduced model

Five days after v0.1 the corpus held eight nodes and an empty inbox. The
diagnosis: the machinery was heavier than the habit it was meant to serve, and
a terminal is not where a thought arrives. v0.2 cuts the model to what a
person actually uses, hands the machinery to an agent, and puts the capture
path and the graph on screen.

This document is the authoritative target for every v0.2 task. `docs/spec.md`
and `docs/design/lineage-graph/` describe v0.1 until they are rewritten.

## What is removed

| removed | why |
|---|---|
| `domain` (field, `config.yaml` list, `neb domain`, `--domain`/`--all`) | One more decision per node. Tags do the job and keep everything in one place. |
| `evidence` (field, `neb evidence`, `neb weigh`, `cite --promote`, `Verdict`, `Strength`) | The verdict/strength ladder was precision the corpus never earned. A reference with a good note carries the same information. |
| `tasks` (field, `neb task`) | Orbit provenance stays in `origin`; forward links to work are a reference. |
| `graduate` (verb, `graduated` status, `graduated_to`) | Downstream hand-off happens by the downstream artifact referencing the node, not by a status here. |
| statuses `testing`, `supported` | They were evidence states. Without evidence they cannot be entered honestly. |
| edges `supports`, `undermines`, `depends-on` | The dependency graph was the evidence layer in edge form. `contradicts` survives because it is a relation between ideas, not a claim about truth. |
| `check --online` and rules 3, 9, 10, 12, 13 | They validated fields that no longer exist. |

Nothing about the *corpus* discipline changes: one file per node, flat
`nodes/`, nothing is ever deleted, genealogy is a DAG, capture makes no
decisions.

## Node

```yaml
---
id: retarded-scarcity-wake          # kebab-case, permanent, never reused
title: "Retardation in the scarcity wake"
status: hypothesis                  # seed | hypothesis | refuted | abandoned
created: 2026-09-07
updated: 2026-09-12
kill: "If the wake timescale is frame-independent under a boosted source, this is dead."
tags: [physics, orrery]
edges:
  - {type: derives-from, to: gravity-as-scarcity}
  - {type: contradicts,  to: network-force}
references:
  - id: r1
    kind: study
    uri: "../../principia/studies/gravitational-time-dilation.md"
    title: "Gravitational time dilation"
    note: "The constraint any wake timescale has to survive."
    added: 2026-09-07
closed:                             # present only on refuted / abandoned
  why: "Boosted-source sim showed frame dependence."
  at: 2026-09-12
origin:                             # optional, all fields optional
  task: DANI-10297
  workspace: ws_nebula
  agent: claude
  at: 2026-09-07T10:12:00Z
---

Free prose.
```

Field rules:

- `status`: `seed → hypothesis → refuted | abandoned`. `hypothesis` requires a
  non-empty `kill`. `refuted` requires `closed.why` (you are asserting the kill
  condition fired; the reference that convinced you goes in `references`).
  `abandoned` accepts `closed.why` but does not require it. A `refuted` node
  never returns to `seed`/`hypothesis`; reviving it means a **new** node with a
  `reopens` edge to it.
- `tags`: free strings, lowercase kebab-case enforced on write (`Physics` →
  `physics`). No declared list. `check` warns on two tags that differ only by
  case or a trailing `s`, so drift is visible without a wall.
- `edges`: genealogy `derives-from | refines | generalizes | reopens`, enforced
  acyclic; `contradicts`, symmetric, written on both nodes by `link`.
- `references`: `id, kind, uri, title, note, added`; `kind` as in v0.1
  (`paper | study | article | note | discussion | book | dataset | thread |
  other`). `note` is the field that matters; `check` warns when it is empty.
  A `verdict` or `strength` key is a parse error (`deny_unknown_fields`).
- `origin`: unchanged from v0.1. Recorded, never typed by hand.
- `schema_version: 2` in `config.yaml`. `config.yaml` keeps `corpus_id` and
  `schema_version` only.

## Inbox

Unchanged: `inbox/YYYY-MM.md`, one timestamped line per capture, entries are
not nodes, `promote` and `drop` settle them. `promote` takes `--parent` (0..n)
and `--tag` (0..n); nothing else.

## Verbs

| verb | does |
|---|---|
| `capture <text>` | the five-second path |
| `inbox` | list unsettled captures |
| `promote <ref> [--parent ..] [--tag ..]` | inbox entry becomes a seed node |
| `drop <ref>` | settle an inbox entry without a node |
| `new <title> [--parent ..] [--tag ..] [--kill ..]` | create a node directly |
| `sharpen <id> --kill "..."` | seed becomes hypothesis |
| `link <from> <type> <to>` | add an edge; refuses a genealogy cycle |
| `cite <id> --kind --uri --note [--title]` | attach a reference |
| `status <id> <status> [--why ..] [--reopen]` | move status under the rules above |
| `tag <id> [--add ..] [--remove ..]` / `tag list` | edit tags; list tags with counts |
| `trace <id> [--down]` | ancestry walk, or descent |
| `impact <id>` | what `contradicts` or descends from this |
| `open [--tag ..]` | hypotheses with no references; seeds untouched ≥ 90 d; inbox entries ≥ 14 d |
| `show <id>` / `list [--tag ..] [--status ..]` | read |
| `review [--since] [--out]` | weekly maintenance report; proposes, never mutates |
| `graph` | `--json` only: `{nodes: [...], edges: [...]}` for the whole corpus, one call |
| `check` | the invariants |
| `migrate` | v1 corpus → v2, see below |
| `completions <shell>` | unchanged |

Every read verb keeps `--json`. The JSON shape **is** the core library's
return type serialised; see [2_architecture.md](2_architecture.md).

## Invariants

| # | rule | level | enforced at |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link` refuses; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`, `check` |
| 4 | `contradicts` is mutual | error | `link` writes both; `check` |
| 5 | `refuted` carries `closed.why` | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Local reference URIs resolve | error | `cite`, `check` |
| 9 | Every reference has a note | warn | `check` |
| 10 | No two tags differ only by case or a trailing `s` | warn | `check` |

## Migration (`neb migrate`)

One-shot, idempotent, refuses on a dirty git tree in the corpus, bumps
`config.yaml` to `schema_version: 2`. Reads nodes with a lenient v1 model and
writes them back in v2 form:

- `domain: X` → appended to `tags` if absent; field removed.
- each `evidence[]` entry → a reference `{kind: other, uri: <source>, title: <note first line>, note: "[<verdict>/<strength>] <note>", added: <date>}`; ids continue the `r<n>` sequence.
- each `tasks[]` entry → a reference `{kind: other, uri: "orbit:<id>", title: <id>, note: <why>}`.
- `status: testing | supported` → `hypothesis`.
- `status: graduated` → `abandoned` with `closed: {why: "graduated to <graduated_to>"}`.
- `config.yaml` loses `domains` and `default_domain`.

Nothing is dropped; the mapping is a lossless re-labelling into references,
which is the whole point of keeping references and cutting the rest.

## Out of scope for v0.2

Search beyond `list`. Sync beyond git. Evidence weighting in any form.
Multi-user. Automatic linking without review.
