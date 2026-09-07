---
title: Glossary
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: reference
type: design
summary: Terms used across nebula's docs and code, with the distinctions that matter.
tags: [lineage-graph, glossary]
paths: ["src/model.rs"]
related_features: [lineage-graph]
related_artifacts: []
---

# Glossary

**Node.** One unit of inquiry, from a vague observation to a supported claim. One
markdown file. Never deleted.

**Capture.** A line of text in the inbox. Not a node. Most captures never become
one.

**Genealogy.** The edges answering "where did this come from": `derives-from`,
`refines`, `generalizes`, `reopens`. Enforced acyclic.

**Diamond.** A node reachable from an ancestor by two distinct paths, created
when a node has two or more genealogical parents. Legal, expected, and the reason
the structure is a DAG rather than a tree.

**Evidence.** Something bearing on whether a node is true. Carries a verdict and
a strength. Only evidence can move a node to `supported` or `refuted`.

**Reference.** Context that situates an idea without bearing on its truth.
Carries no verdict, and the schema rejects one.

**Weigh.** Promoting a reference into evidence, once you have read it closely
enough to say which way it cuts. The reference stays, marked, so the reading
history survives.

**Kill condition.** What would falsify a node, written when the hypothesis is
stated and before any evidence arrives. Pre-registering it is what keeps the
later verdict honest rather than retroactive.

**Refuted.** The kill condition fired. Distinct from **abandoned**, which means
you stopped caring. Both persist forever.

**Graduated.** Handed downstream to principia or orbit-research, with a link out.
The node stays here, so lineage crosses the boundary.

**Corpus.** The nodes and inbox, living outside this repository.
