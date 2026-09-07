---
title: Capture Path
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: The five-second capture budget, the inbox format, and why promotion is a separate act.
tags: [lineage-graph, capture]
paths: ["src/corpus/store.rs", "src/commands/inbox.rs"]
related_features: [lineage-graph]
related_artifacts: []
---

# Capture Path

## Budget

Capture must complete in under five seconds and require no decisions. This is the
strictest constraint in the system, and every other feature yields to it.

```sh
neb capture "ranking signal decay looks like a half-life, not a cliff"
```

No parent, no type, no tags, no status. It works before a corpus exists: being
told to run a setup command first is exactly the friction that loses the thought,
so `capture` creates the corpus rather than refusing.

## Inbox format

One append-only file per month, `inbox/YYYY-MM.md`, one entry per line:

```
- [62fe] 2026-09-06T20:48 gravity might be about scarcity, not curvature
```

The short id is a hash of the timestamp and text, retried until unique within the
file. Append-only means capture never reads or rewrites, so it cannot corrupt
what is already there and stays fast as the file grows.

## Settling an entry

An entry leaves the inbox by being promoted or dropped, and either way the line
is struck through in place rather than removed:

```
- ~~[62fe] 2026-09-06T20:48 gravity might be about scarcity, not curvature~~ -> gravity-as-scarcity
- ~~[efbb] 2026-09-06T20:49 a half-formed thing that went nowhere~~ dropped
```

Dropping is a normal outcome, not a failure, and most captures should end there.
Keeping the dropped text costs nothing and records a road not taken, which is
the same reason refuted nodes persist.

Promotion carries the captured text into the new node's prose, so the original
wording of a thought survives the tidying that naming it involves.

## Why promotion is separate

Promotion requires a title, and possibly parents. That is a decision, and
decisions belong after the thought is safe rather than in front of it. Splitting
the two means the expensive step can wait for a moment when you have attention to
spend, and the cheap step is always available.
