---
title: Lineage Graph — Overview
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: lineage-graph
doc_role: overview
type: design
summary: Why nebula exists, what a node is, and where it sits relative to principia and orbit-research.
tags: [lineage-graph]
paths: ["crates/**", "docs/spec.md"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Lineage Graph — Overview

## The problem

Ideas arrive vague, at random, across unrelated fields of thought, and every existing home
is wrong for that moment. principia's smallest unit is a claim, which already
demands a crisp statement with a kind and a status. orbit-research is stricter
still, since it exists to hold preregistered protocols. So the half-formed
observation goes into a note, a chat, or nowhere, and it is gone.

The second failure is lineage. principia groups claims by `family`, a flat
partition, and every other relation is smuggled into an untyped `links` array
that mixes ancestry with sourcing with sibling references. There is no
"derived from" edge in that schema. Lineage therefore exists only in prose,
which is why a claim cannot be walked back to the thought that started it.

## What nebula is

A graph of inquiry units, and two verbs that matter: put a thought in, and walk
back to where a thought came from.

A node's **status** carries an idea through the stages that can be entered: it
arrives as a `seed`, sharpens into a `hypothesis` by naming a kill condition,
and ends at `refuted`, when the kill condition fires, or `abandoned`, when
interest simply stops — kept distinct on purpose, since one is wrong and the
other is untouched. Sharpening a further idea out of one that already exists
is not a status transition at all: it creates a new node with a `refines` edge
back, and nothing is ever deleted. The full state machine is in
[docs/design/v0.2/1_spec.md](../v0.2/1_spec.md).

## The shape is a DAG

Two observations converge on one hypothesis. A refinement is often prompted by
a sibling branch dying. A node therefore has any number of genealogical
parents, and a node reachable by two paths is a diamond. Diamonds are legal,
expected, and the reason the structure is a directed acyclic graph rather than
a tree. Acyclicity constrains genealogy only: an idea cannot be its own
ancestor.

## Genealogy, and one flat relation beside it

Genealogy (`derives-from`, `refines`, `generalizes`, `reopens`) answers "where
did this come from" and is enforced acyclic. `contradicts` sits beside it: a
symmetric statement that two ideas cannot both be true, written on both nodes
at once by `link`. It is not a graph to walk on its own, just a flag a node
carries. A second, cycle-tolerant relation lived here in v0.1 and was cut in
the 2026-09-12 reduction; see [docs/design/v0.2/1_spec.md](../v0.2/1_spec.md)
("What is removed") for why.

## Boundaries

nebula sits upstream of everything else and hands off by reference rather than
by growing into what it hands off to. A node that reaches a real hypothesis
with a real kill condition, and needs simulation, is picked up by a principia
gate card that cites it. A node needing preregistration and protocols is
picked up the same way by orbit-research. Neither is absorbed here: their
strictness is correct downstream and would destroy capture upstream. A node
picked up downstream stays in the corpus forever, so a trace starting
downstream still reaches the observation that began it.

## Related

- [2_design.md](2_design.md) — how it is built
- [4_decisions.md](4_decisions.md) — the choices and their reasoning
- [specs/capture-path.md](specs/capture-path.md) — the five-second budget
- [specs/invariants.md](specs/invariants.md) — what `neb check` enforces
- [../v0.2/1_spec.md](../v0.2/1_spec.md) — the current node model and verbs
