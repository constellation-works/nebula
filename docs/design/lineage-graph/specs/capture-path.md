---
title: Capture Path
owner: claude
last_updated: 2026-09-26
last_validated: 2026-09-25
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: The five-second capture budget, the inbox format, and why promotion is a separate act.
tags: [lineage-graph, capture]
paths: ["crates/nebula-core/src/store.rs", "crates/nebula-core/src/ops.rs"]
related_features: [lineage-graph, v0.2]
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

Creating one is never silent. A mistyped `--root` or `$NEBULA_ROOT` looks exactly
like a corpus that does not exist yet, and a quiet success would split the corpus
with nothing to show for it. So whenever `capture` creates a corpus it prints one
line on stderr naming the absolute path it created, in text and `--json` modes
alike, and changes nothing on stdout:

```
note: created a new corpus at /home/you/.nebula
```

The path is made absolute lexically, against the working directory, and never
resolved, so a relative typo shows where it landed. The notice asks nothing and
refuses nothing, so the budget holds. A capture into an existing corpus prints
no notice. When the corpus cannot be created, the error names the path it tried:
`creating /nonexistent: Permission denied (os error 13)`.

## Inbox format

One file per month, `inbox/YYYY-MM.md`, with one entry per line. Captures
append; settling strikes the entry through in place:

```
- [62fe] 2026-09-06T20:48 gravity might be about scarcity, not curvature
```

The short id is a hash of the timestamp and text, allocated from a fixed
four-hex-character space across every live entry in the inbox. Capture tries
hash-derived candidates, then the remaining space; if every id is occupied, it
refuses the capture rather than reusing an id. It reads the inbox to enforce
that corpus-wide uniqueness, then appends one new line to the current month's
file; it never rewrites an existing line.

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

Promotion creates a seed node and uses the captured text as its default title.
It can add parents, but `near` only suggests them and never chooses one.
Deciding whether to promote, and whether a suggested parent is defensible, can
wait until the thought is safe rather than in front of it. Splitting the two
means the expensive step can wait for a moment when you have attention to
spend, and the cheap step is always available.
