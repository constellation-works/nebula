---
title: Glossary
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: lineage-graph
doc_role: reference
type: design
summary: Terms used across nebula's docs and code, with the distinctions that matter.
tags: [lineage-graph, glossary, v0.2]
paths: ["crates/nebula-core/src/model.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Glossary

**Node.** One unit of inquiry, from a vague observation to a sharpened
hypothesis. One markdown file. Never deleted.

**Capture.** A line of text in the inbox. Not a node. Most captures never
become one.

**Genealogy.** The edges answering "where did this come from": `derives-from`,
`refines`, `generalizes`, `reopens`. Enforced acyclic.

**Diamond.** A node reachable from an ancestor by two distinct paths, created
when a node has two or more genealogical parents. Legal, expected, and the
reason the structure is a DAG rather than a tree.

**Reference.** Context that situates an idea: a paper, a study, a discussion,
a link to work it spawned. Carries a note explaining why it is attached, and
no judgement about whether the idea is true — the schema rejects one.

**Kill condition.** What would falsify a node, written when the hypothesis is
stated and before anything is read to test it. Pre-registering it is what
keeps the later status change honest rather than retroactive.

**Refuted.** The kill condition fired. Distinct from **abandoned**, which means
you stopped caring. Both persist forever.

**Closed.** The block a node carries once it is `refuted` or `abandoned`:
`why` (required for `refuted`, optional for `abandoned`) and `at`, the date it
closed. Absent on every other status.

**Tag.** A free-form label, normalised to lowercase kebab-case on every write.
No declared list; `check` warns when two tags differ only by case or a
trailing `s`. `tag list` shows every tag in the corpus with its node count.

**Graph.** The whole corpus as one `{nodes, edges}` export: what
[docs/design/v0.2/1_spec.md](../../v0.2/1_spec.md) specifies as `neb graph
--json` (not yet built — see [3_plan.md](../../v0.2/3_plan.md), task B) and
what the desktop app's graph view will draw from. The same shape a `trace` or
`impact` walk is computed over, just unfiltered.

**Migrate.** `neb migrate`: a one-shot, idempotent pass that brings a v1
corpus forward to the current schema, re-labelling everything the reduction
removed into references so nothing is lost. See
[docs/design/v0.2/1_spec.md](../../v0.2/1_spec.md) ("Migration").

**Corpus.** The nodes, inbox and `config.yaml`, living outside this
repository. One corpus per owner: work and personal are separate corpora,
since that line is about who owns the material rather than what it is about.
