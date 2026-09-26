---
title: v0.2 — Reduced model
owner: claude
last_updated: 2026-09-22
last_validated: 2026-09-22
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

This document records the maintained v0.2 contract. `docs/spec.md` is the
original v0.1 design record; more focused documents under
`docs/design/lineage-graph/` remain authoritative where they explicitly name
v0.2 behavior.

## What is removed

| removed | why |
|---|---|
| `domain` (field, `config.yaml` list, `neb domain`, `--domain`/`--all`) | One more decision per node. Tags do the job and keep everything in one place. |
| `evidence` (field, `neb evidence`, `neb weigh`, `cite --promote`, `Verdict`, `Strength`) | The verdict/strength ladder was precision the corpus never earned. A reference with a good note carries the same information. |
| `tasks` (field, `neb task`) | Orbit provenance stays in `origin`; forward links to work are a reference. |
| `graduate` (verb, `graduated` status, `graduated_to`) | Downstream hand-off happens by the downstream artifact referencing the node, not by a status here. For Observatory, `handoff` records the nebula side in one write: a reference to the record, and the node closed as `abandoned` (see [Hand-off](#hand-off)). |
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

- `id`: derived from the title by lowercasing Unicode letters and digits and
  joining runs with a single `-`; other characters are separators. It is at
  most 60 characters and is cut only at a word boundary. The same rule
  validates an explicit `--id`, so non-ASCII scripts are preserved.
  `promote` with neither `--title` nor `--id` titles the node with the whole
  captured sentence, and an id is permanent, so it does not slug the whole
  sentence: a capture of more than five words drops stop-words (articles,
  auxiliaries, pronouns, common prepositions; never a negation such as `not`)
  and keeps the first five words left, so `gravity might be a scarcity
  gradient in some shared resource` becomes
  `gravity-scarcity-gradient-shared-resource`. If that id belongs to a
  different idea, the next one adds one more significant word at a time, then
  falls back to the full slug. A node already titled with the same text is
  that thought promoted before, so the promotion is refused as `NodeExists`
  for its id rather than duplicated, as it is when every candidate is taken. A
  capture of five words or fewer, an explicit `--title`, and an explicit
  `--id` keep the rules above unchanged, and an existing id is never
  rewritten.
- `status`: `seed → hypothesis → refuted | abandoned`. `hypothesis` requires a
  non-empty `kill`. `refuted` requires `closed.why` (you are asserting the kill
  condition fired; the reference that convinced you goes in `references`).
  `abandoned` accepts `closed.why` but does not require it. A `refuted` node
  never returns to `seed`/`hypothesis`; reviving it means a **new** node with a
  `reopens` edge to it. A node with a `kill` never returns to `seed` either,
  since the kill stays; it reopens as a `hypothesis`.
- `tags`: free strings, lowercase kebab-case enforced on write (`Physics` →
  `physics`), using the same Unicode letter-and-digit rule as ids. No declared
  list. A write that introduces a tag differing from one in use only by case
  or a trailing `s` succeeds with a note on stderr, and `check` warns on every
  such pair, naming the nodes carrying each, so drift is visible without a
  wall.
- `edges`: genealogy `derives-from | refines | generalizes | reopens`, enforced
  acyclic; `contradicts`, symmetric, written on both nodes by `link`.
- `references`: `id, kind, uri, title, note, added`; `kind` is one of `paper |
  study | article | note | discussion | book | dataset | thread | observatory |
  other`. `cite` lowercases the kind it is given and refuses anything still
  outside the list; `check` warns about unexpected values in
  an existing corpus so older or hand-edited files still load. `note` is the
  field that matters; `check` warns when it is empty.
  A `verdict` or `strength` key is a parse error (`deny_unknown_fields`).
- `origin`: unchanged from v0.1. Recorded, never typed by hand.
- `schema_version: 2` in `config.yaml`. It also keeps the stable `corpus_id`
  and the optional machine-written `commit` setting. Use `neb config commit`
  rather than editing it by hand. Where the Observatory checkout is is a
  machine setting, not a corpus one; see [Machine settings](#machine-settings).

## Inbox

Unchanged: `inbox/YYYY-MM.md`, one timestamped line per capture, entries are
not nodes, `promote` and `drop` settle them. `promote` can override the
generated title or id, append body text, add parents and tags, record author
and Orbit provenance, and suppress nearest-node suggestions; run
`neb promote --help` for the complete flag list.

## Verbs

| verb | does |
|---|---|
| `init [<path>]` | create an empty corpus |
| `config commit [on|off]` / `config observatory-root [<dir>]` | read or set the commit policy or machine-specific Observatory root |
| `completions <shell>` | generate shell completion scripts |
| `check` | check the corpus invariants |
| `migrate` | bring a v1 corpus forward to v2, in place |
| `capture <text>` | the five-second path |
| `inbox [--limit ..]` | list unsettled captures |
| `promote <ref> [--title ..] [--body ..] [--parent ..] [--tag ..] [--id ..] [--by ..] [--task ..] [--run ..] [--quiet]` | inbox entry becomes a seed node |
| `drop <ref>` | settle an inbox entry without a node |
| `triage [--by ..]` | interactive: each waiting entry, oldest first, with its age and numbered `near` candidates; one key promotes it as a root (`p`), under a candidate (`1`–`3`), titles it (`t`), drops (`d`), skips (`s`) or stops (`q`). Each decision is `promote` or `drop`, commit included; no `--json` |
| `new <title> [--parent ..] [--reopens ..] [--contradicts ..] [--tag ..] [--kill ..]` | create a node directly, with its edges; `--contradicts` is written on both nodes |
| `edit <id>` | edit a node's body in `$VISUAL` or `$EDITOR` |
| `sharpen <id> --kill "..."` | seed becomes hypothesis |
| `status <id> <status> [--why ..]` | move status under the rules above; reopening requires a new node and a `reopens` edge (`new --reopens <id>`) |
| `link <from> <type> <to>` | add an edge; refuses a genealogy cycle |
| `tag <id> [--add ..] [--remove ..]` / `tag list` | edit tags; list tags with counts |
| `note <id> <text>` | append a dated paragraph of reasoning to a node |
| `cite <id> --kind --uri --note [--title]` | attach a reference |
| `handoff <id> <record> [--note ..] [--by ..] [--task ..] [--run ..]` | hand the node off to an Observatory record: one `observatory` reference and `abandoned` with `why: handed off to <record>`, in one write; see [Hand-off](#hand-off) |
| `show <id>` / `list [--tag ..] [--status ..] [--limit ..]` | read |
| `log <id>` | list the commits that changed a node |
| `near <text-or-id> [--limit ..]` | rank existing nodes by word overlap with text or a node; suggestions never create links |
| `trace <id> [--down] [--depth ..]` | ancestry walk, or descent |
| `impact <id>` | what `contradicts` or descends from this |
| `graph` | `--json` emits `{nodes: [...], edges: [...]}`; `--mermaid [--from <id>]` emits a diagram |
| `open [--tag ..]` | deprecated alias for `review --short`: hypotheses created at least 14 days ago with no references; seeds untouched for at least 90 days; inbox entries waiting at least 14 days |
| `review [--since ..] [--out ..] [--limit ..]` | weekly report: stale hypotheses (default 30 days), untouched seeds (default 90 days), hypotheses created at least 14 days ago with no references, unconfirmed kills, and inbox entries waiting at least 14 days; proposes, never mutates |
| `review --short [--tag ..] [--limit ..]` | quick glance: hypotheses created at least 14 days ago with no references; seeds untouched for at least 90 days; inbox entries waiting at least 14 days |

Every read verb keeps `--json`. The JSON shape **is** the core library's
return type serialised, with every field present (an absent value `null`, an
empty list `[]`) and every author label stated; see
[2_architecture.md](2_architecture.md). `--limit` and `--depth` bound the
output and default to everything; under `--json`, given either flag, the list
is `{items, total, truncated}`, `total` counting the matches before the cut,
and without it the bare array. `near` always has a limit, so it always
answers in that envelope. Every verb that writes takes `--no-commit`; a
read-only verb does not offer it.

## Invariants

| # | rule | level | enforced at |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link`/`new` refuse; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`/`new`, `check` |
| 4 | `contradicts` is mutual | error | `link`/`new` write both; `check` |
| 5 | `refuted` carries `closed.why` | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status`, `handoff` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Non-discussion references have a URI; local URIs resolve relative to `nodes/` and are never absolute | error; warn for an absolute path already in the corpus | `cite` refuses both; `check` reports both |
| 9 | An `observatory` reference's record resolves under the configured root | warn | `check` (the id's shape is refused at `cite`; `handoff` refuses a record that does not resolve under a set root) |
| 10 | Every reference has a note | warn | `check` |
| 11 | No two tags differ only by case or a trailing `s` | warn | `check`; noted at `new`/`promote`/`tag` |
| 12 | `closed` is set only on a `refuted`/`abandoned` node, never an open one | error | `check` |
| 13 | A `seed` does not carry a `kill` condition | warn | `status` refuses the move to `seed`; `check` |
| 14 | `created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created` | error | `check` |
| 15 | A node's `id` names one file under `nodes/`, and is the id its file name names | error | parse (the shape); every read and write (the agreement) |
| 16 | Every reference kind belongs to the documented vocabulary | warn | `cite` refuses new values; `check` reports existing ones |

## Hand-off

`neb handoff <id> <record> --note "..."` is the nebula side of a node
becoming an Observatory record. In one write it adds an `observatory`
reference to the record (a bare id, normalised up, with the note) and moves
the node to `abandoned` with `closed: {why: "handed off to <record>"}`.

- It refuses, writing nothing: an unknown node (`NoSuchNode`); a node that is
  already closed (`AlreadyClosed`), since a refuted node's verdict is final
  (revive it with `new --reopens` and hand that off) and an abandoned one's
  reason would be replaced; a record id of the wrong shape
  (`InvalidObservatoryId`); and, when this machine has an observatory root,
  a record that does not resolve under it (`UnresolvedObservatoryRecord`).
- With no root set, the id is accepted on its shape alone and the verb says
  it cannot be located, as `cite --kind observatory` does; `check` keeps
  warning (rule 9) until this machine has a root.
- A node is read as handed off when both halves are present: it is
  `abandoned`, its `closed.why` is exactly `handed off to <record>`, and it
  carries an `observatory` reference to that record. No field records it.
  `show` prints where the record is under the `closed:` line, and `trace`'s
  tree appends `handed off to <record>` to the node's line; both `--json` forms
  carry `handed_off_to`, `null` when the node was not handed off.

Decision (ORB-13077): nebula owns this verb; back-links from Observatory to
the node (`nebula:<id>`) are Observatory's to write. The hand-off stays an
existing reference plus an existing status, so the schema grows nothing and
the two-verb form (`cite --kind observatory`, then `status abandoned --why
"handed off to <record>"`) reads the same. Refusing an unresolved record
only when a root is set keeps a machine without a checkout usable, as
`cite` does.

## Machine settings

The corpus travels between machines (synced, and committed under
`commit: true`), so nothing machine-specific belongs in it. Where the
Observatory checkout is, which `observatory` references resolve against, is
therefore read per machine, first match wins:

1. `$OBSERVATORY_ROOT`;
2. `~/.config/nebula/observatory-root`, beside `~/.config/nebula/root`,
   written by `neb config observatory-root <DIR>` (an absolute path; nothing
   under the corpus changes);
3. legacy: an `observatory_root` key in `config.yaml`, which earlier builds
   wrote there. It is still read, so a corpus that carries one keeps
   resolving, but never written. `check` warns (rule 9) whenever the file
   carries it, saying whether it is in force here or outranked, and
   `neb config observatory-root --drop-legacy` removes it once every machine
   has its own setting.

`neb config observatory-root` without a directory reports the effective root
and which of these supplied it (`source: env | machine | config | unset`),
plus the legacy key when the file still carries one.

Decision (ORB-13050): the real corpus, synced between two machines, carried
one machine's absolute path in `config.yaml`, so every record resolved on
that machine and warned on the other. Moving the setting to the machine and
keeping the key as a read-only fallback fixes that without breaking any
corpus that has the key; reconsider only if a corpus-wide Observatory
location ever becomes meaningful, such as a path relative to the corpus.

## Migration (`neb migrate`)

One-shot, idempotent, refuses on a dirty git tree in the corpus, bumps
`config.yaml` to `schema_version: 2`. Reads nodes with a lenient v1 model and
writes them back in v2 form:

- `domain: X` → appended to `tags` if absent; field removed.
- each `evidence[]` entry → a reference `{kind: other, uri: <source>, title: <note first line>, note: "[<verdict>/<strength>] <note>", added: <date>}`; ids continue the `r<n>` sequence.
- each `tasks[]` entry → a reference `{kind: other, uri: "orbit:<id>", title: <id>, note: <why>}`.
- `status: testing | supported` → `hypothesis`.
- `status: graduated` → `abandoned` with `closed: {why: "graduated to <graduated_to>"}`.
- `config.yaml` loses `domains` and `default_domain`; existing
  `observatory_root` (thereafter the legacy fallback above) and `commit`
  settings are preserved.

Nothing is dropped; the mapping is a lossless re-labelling into references,
which is the whole point of keeping references and cutting the rest.

The lenient read is scoped to corpora that declare an older schema. A
`config.yaml` already at `schema_version: 2` has nothing left to re-label, so
every node is read with the strict current model first, over the whole corpus
and before any file is written; a node that will not parse refuses the run
with the bytes untouched. Leniency there could only delete a key this build
does not know and report the node as migrated, and a refusal that came per
node instead would leave the corpus half rewritten. A future schema is
refused the same way, at the config.

## Out of scope for v0.2

Full-text indexing, semantic search, or network search; `near` is a word-overlap
query over existing nodes. Sync beyond git. Evidence weighting in any form.
Multi-user. Automatic linking without review.
