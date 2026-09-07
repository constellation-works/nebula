---
title: Lineage Graph — Overview
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: overview
type: design
summary: Why nebula exists, what a node is, and where it sits relative to principia and orbit-research.
tags: [lineage-graph]
paths: ["src/**", "docs/spec.md"]
related_features: [lineage-graph]
related_artifacts: []
---

# Lineage Graph — Overview

## The problem

Ideas arrive vague, at random, across unrelated domains, and every existing home
is wrong for that moment. principia's smallest unit is a claim, which already
demands a crisp statement with a kind and a status. orbit-research is stricter
still, since it exists to hold preregistered protocols. So the half-formed
observation goes into a note, a chat, or nowhere, and it is gone.

The second failure is lineage. principia groups claims by `family`, a flat
partition, and every other relation is smuggled into an untyped `links` array
that mixes ancestry with evidence with sibling references. There is no
"derived from" edge in that schema. Lineage therefore exists only in prose,
which is why a claim cannot be walked back to the thought that started it.

## What nebula is

A graph of inquiry units, and two verbs that matter: put a thought in, and walk
back to where a thought came from.

An idea passes through four stages. It arrives vague. It sharpens into something
that could be wrong. Evidence accumulates for and against. It is then refined or
killed. Those four are the node's **status**, not sub-structure inside it, which
is what makes the corpus queryable. The fourth is not a stage at all: refining
creates a new node with a `refines` edge back, and killing flips a status.
Nothing is ever deleted.

## The shape is a DAG

Two observations converge on one hypothesis. A refinement is often prompted by a
sibling branch dying. A node therefore has any number of genealogical parents,
and a node reachable by two paths is a diamond. Diamonds are legal, expected,
and the reason the structure is a directed acyclic graph rather than a tree.
Acyclicity constrains genealogy only: an idea cannot be its own ancestor.

## Two graphs over one node set

Genealogy answers "where did this come from" and is enforced acyclic. Evidence
and dependency answer "what breaks if this dies" and may contain cycles, since a
cycle in `supports` is circular reasoning, which is a finding worth surfacing
rather than a state worth forbidding. Conflating these two relations into one
link list is what made principia's graph opaque.

## Boundaries

nebula sits upstream of everything else and hands off rather than growing into
it. A node that reaches a real hypothesis with a real falsifier, and needs
simulation, graduates to a principia gate card. A node needing preregistration
and protocols graduates to orbit-research. Neither is absorbed here: their
strictness is correct downstream and would destroy capture upstream. Graduated
nodes stay in the corpus forever with a link out, so a trace starting downstream
still reaches the observation that began it.

## Related

- [2_design.md](2_design.md) — how it is built
- [4_decisions.md](4_decisions.md) — the choices and their reasoning
- [specs/capture-path.md](specs/capture-path.md) — the five-second budget
- [specs/invariants.md](specs/invariants.md) — what `neb check` enforces
