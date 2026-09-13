---
title: Lineage Graph — Vision
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Draft
feature: lineage-graph
doc_role: vision
type: design
summary: The maintenance loop that decides whether the corpus survives, and what deliberately stays out.
tags: [lineage-graph]
paths: ["src/**", "skills/**", "apps/desktop/**"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Lineage Graph — Vision

## The half that decides survival

The schema is the easy part. What killed the previous attempts was not a
missing field, it was that curation is work nobody does. Promotion, linking
and pruning all cost effort at a moment when the interesting thing is the
idea, not the filing. A corpus that depends on that effort decays into a
folder of markdown.

So an agent owns the maintenance, in one of two modes, never both at once for
a given run. **Session mode**, a human present and directing: the agent acts —
triage the inbox, sharpen, link, cite, move status — and runs `neb check`
after every write. **Routine mode**, unattended, run as an Orbit routine: `neb
review` and an inbox read produce proposals only, written to `review.md`, and
never a write to `nodes/`. The full split, including the review-file format,
is in [docs/design/v0.2/2_architecture.md](../v0.2/2_architecture.md)
("skills/nebula").

**The hard rule, unchanged since v0.1: unattended running proposes and never
writes to a node.** All routine-mode output goes to a single `review.md` that a
human skims and accepts in one pass. The moment routine mode edits nodes on
its own, the graph stops being trustworthy, and an untrusted graph is worse
than no graph. `--json` on every read command exists to make that loop cheap
for the agent to build on.

## Later, maybe — now in flight

Two things this document once filed under "maybe" are now scoped, tracked
tasks rather than hypotheticals:

- A desktop app that draws the whole corpus as a graph and captures from a
  global shortcut, so lineage can be read and added to without a terminal.
- An agent skill that teaches a session-mode or routine-mode agent the verbs,
  their `--json` shapes, and the invariants, so the maintenance loop above is
  actually run rather than merely specified.

Both are scoped in [docs/design/v0.2/2_architecture.md](../v0.2/2_architecture.md)
and staged in [docs/design/v0.2/3_plan.md](../v0.2/3_plan.md). Similarity
search for proposing links remains a real "maybe": nothing today scopes it.

## Deliberately out of scope

No automatic linking without review. No multi-user or sync beyond git. No
per-node categorical field, ever, since the same shape has to hold regardless
of what the ideas are about — tags are the only thing that vary by subject.

Nothing above the capture path should be built before roughly fifty real nodes
exist. Until then every claim in this document is a guess about habits that
have not been observed.
