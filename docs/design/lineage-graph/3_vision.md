---
title: Lineage Graph — Vision
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Draft
feature: lineage-graph
doc_role: vision
type: design
summary: The maintenance loop that decides whether the corpus survives, and what deliberately stays out.
tags: [lineage-graph]
paths: ["src/**"]
related_features: [lineage-graph]
related_artifacts: []
---

# Lineage Graph — Vision

## The half that decides survival

The schema is the easy part. What killed the previous attempts was not a missing
field, it was that curation is work nobody does. Promotion, linking and pruning
all cost effort at a moment when the interesting thing is the idea, not the
filing. A corpus that depends on that effort decays into a folder of markdown.

So an agent owns the maintenance, run as an Orbit routine.

**Nightly.** Read the inbox. For each entry propose exactly one of: promote as a
new seed, attach to an existing node, or drop, with one line of reasoning.

**Weekly.** Flag hypotheses with no evidence for thirty days. Flag seeds
untouched for ninety days and propose abandoning them. Flag nodes whose stated
kill condition looks satisfied by evidence that landed since. Propose
`contradicts` pairs found by similarity. Propose references for nodes that have
none, drawn from the almanac and from principia studies.

**On task completion.** When an Orbit task named in a node's `tasks` finishes,
reconcile its state and propose attaching its artifacts as evidence with a
suggested verdict. This closes the loop: a hypothesis names what would kill it,
spawns work to find out, and the answer returns attached to the node that asked.

**The hard rule: the agent proposes and never writes to a node.** Output lands in
one review file accepted in a pass. The moment it edits nodes on its own you stop
trusting the graph, and an untrusted graph is worse than no graph. `--json` on
every read command exists to make that loop cheap.

## Later, maybe

A read-only browser over the corpus, in the shape of orbit-research's static
export, so lineage can be read on a phone. Similarity search for proposing links.
An `orbit.research.*` tool surface so agents reach nebula the way they reach
`orbit.task.*`, rather than by shelling out.

## Deliberately out of scope

No evidence weighting or Bayesian arithmetic: three coarse strength levels, and
rigor belongs downstream. No multi-user or sync beyond git. No per-domain schema,
ever, since the same shape has to hold for physics and for ranking signals. No
automatic linking without review.

Nothing above the capture path should be built before roughly fifty real nodes
exist. Until then every claim in this document is a guess about habits that have
not been observed.
