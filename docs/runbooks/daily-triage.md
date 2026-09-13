---
type: runbook
summary: Clear the inbox, find hypotheses with nothing running, and keep the corpus honest.
tags: [operations, triage, routine]
paths: ["crates/nebula-core/src/ops.rs", "crates/nebula-core/src/graph.rs", "crates/neb/src/cli.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-12
---

# Triage the Corpus

Capture is cheap by design, so the inbox fills. This is the routine that keeps
that from becoming a second pile of unprocessed notes. It describes the
commands directly; if an agent is doing this for you, see
[agent-triage.md](agent-triage.md) for the session-mode flow.

## Clear the inbox

```sh
neb inbox
```

Each entry gets exactly one of three outcomes.

Promote it, when the thought deserves a node of its own:

```sh
neb promote 62fe --title "Gravity as scarcity" --parent information-density-hunch
```

Drop it, which is the normal outcome for most captures:

```sh
neb drop efbb
```

Or leave it, when you genuinely cannot tell yet. Entries do not expire.

Dropping loses nothing. The line stays in the inbox file struck through, so a
thought you discarded is still findable by grep a year later.

## Find what needs work

```sh
neb open
```

This reports hypotheses with no references, seeds untouched for ninety days or
more, and inbox entries waiting fourteen days or more; the inbox finding is
listed first. Narrow it to a slice of the corpus with `--tag` (repeatable,
every one required):

```sh
neb open --tag ranking
```

The finding that matters most is a hypothesis with no references at all. That
is an idea you committed to running down and then did not attach anything to.
Close the gap by citing whatever you find, or by moving its status once you
actually know the answer:

```sh
neb cite ranking-decay-half-life --kind study --uri "../orrery/lab/sims/coupling-off-control/" \
  --note "Effect persists with the coupling off, which is what the kill condition named."
```

## Sharpen a seed

A seed becomes a hypothesis by naming what would kill it, and refuses to become
one otherwise.

```sh
neb sharpen gravity-as-scarcity --kill "if the effect survives with the coupling off"
```

Write the falsifier before you look for anything that bears on it. That
ordering is the whole mechanism: a kill condition written afterwards is a
rationalisation.

## Move a node's status

```sh
neb status gravity-as-scarcity refuted --why "Effect persists with coupling off, which is what the kill condition named."
```

`refuted` requires `--why`; `abandoned` takes it optionally. A refuted node
cannot be quietly reopened later. Reviving the idea takes a new node with a
`reopens` edge, which keeps the fact that it once died visible.

```sh
neb new "Gravity as scarcity, take two" --parent gravity-as-scarcity --kill "..."
neb link gravity-as-scarcity-take-two reopens gravity-as-scarcity
```

## Before you stop

```sh
neb check
```

Non-zero exit means the corpus is inconsistent. Warnings are advisory and often
worth leaving alone.
