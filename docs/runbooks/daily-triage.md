---
type: runbook
summary: Clear the inbox, find hypotheses with nothing running, and keep the corpus honest.
tags: [operations, triage, routine]
paths: ["src/commands/**"]
related_features: [lineage-graph]
related_artifacts: []
last_validated: 2026-09-07
---

# Triage the Corpus

Capture is cheap by design, so the inbox fills. This is the routine that keeps
that from becoming a second pile of unprocessed notes.

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

With several domains declared this narrows to the default one and says so in
its last line; `neb open --all` crosses, and `--domain <name>` picks another.

The finding that matters most is a hypothesis with no evidence and no task
running against it. That is an idea you committed to testing and then did not.
Close the gap by filing work and recording it:

```sh
neb task ranking-decay-half-life ORB-11440 --why "measure decay against a control period"
```

Two cautions when filing the Orbit task itself. Filing triggers dispatch, so file
only when the node is genuinely ready. And `orbit.task.add` silently ignores a
`dependencies` field, so wire dependencies with a follow-up `task.update`.

## Sharpen a seed

A seed becomes a hypothesis by naming what would kill it, and refuses to become
one otherwise.

```sh
neb sharpen gravity-as-scarcity --kill "if the effect survives with the coupling off"
```

Write the falsifier before you look for evidence. That ordering is the whole
mechanism: a kill condition written afterwards is a rationalisation.

## Record a finding

```sh
neb evidence gravity-as-scarcity --verdict undermines --strength strong \
  --source "../orrery/lab/sims/coupling-off-control/" \
  --note "Effect persists with coupling off, which is what the kill condition named."
```

When the verdict is `undermines`, the tool prints the node's kill condition back
at you. Compare them honestly, and if it fired, say so:

```sh
neb status gravity-as-scarcity refuted
```

A refuted node cannot be quietly reopened later. Reviving the idea takes a new
node with a `reopens` edge, which keeps the fact that it once died visible.

## Before you stop

```sh
neb check
```

Non-zero exit means the corpus is inconsistent. Warnings are advisory and often
worth leaving alone.
