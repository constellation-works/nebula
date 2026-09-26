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

Where it is run never decides that. The working directory resolves to a corpus
only when one already exists at or above it (`nodes/` beside a `config.yaml`
naming a `corpus_id`); anywhere else resolution carries on to the configured
root. So a corpus is only ever created at a root that was named: `--root`,
`$NEBULA_ROOT`, `~/.config/nebula/root`, or the `~/.nebula` default.

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

## Text over several lines

A thought often arrives as more than one line: pasted from a chat, piped from
another command, or typed with a stray newline. Capture takes it as it comes
and stores it as one line:

```sh
printf 'gravity might be about\nscarcity, not curvature\n' | neb capture -
neb capture "gravity might be about
scarcity, not curvature"
```

A lone `-` reads the thought from standard input, as `--body -` does
elsewhere; a dash among other words is part of the thought. Either way, every
line break (`\n` or `\r`) becomes a single space together with the whitespace
around it, blank lines vanish, and the ends are trimmed, so both commands above
store `gravity might be about scarcity, not curvature`. Whitespace inside a
line is left alone. Only text that is nothing but whitespace is refused
(`nothing to capture`), and it is refused before anything, a corpus included,
is created.

The joining is `nebula_core::store::capture_line`, applied inside the core
capture itself, so the CLI, the desktop capture box and any other caller store
the same line for the same text.

This reverses the earlier refusal. When capture first met a newline (#62), the
inbox's one-entry-per-line format was protected by refusing the capture
(`capture text must fit on one line`). That kept the format but lost the
thought, at the moment the budget above exists to protect, and made
`echo "thought" | neb capture` impossible. Joining keeps the format just as
strictly, since the stored line can never hold a line break, and asks nothing
of the person capturing. What it gives up is the line structure of the
original text, which an inbox entry never had room for; prose that needs it
belongs in a node's body, through `promote --body -`.

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
