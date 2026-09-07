---
title: Invariants
owner: claude
last_updated: 2026-09-07
last_validated: 2026-09-07
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: What neb check enforces, where each rule is enforced, and which failures block.
tags: [lineage-graph, invariants]
paths: ["src/check/**", "src/corpus/model.rs", "src/corpus/config.rs"]
related_features: [lineage-graph]
related_artifacts: []
---

# Invariants

`neb check` is the lock, the same role `check-theory.py` plays in principia. A
schema is a suggestion until something refuses a corpus that violates it.

| # | rule | level | enforced at |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link` refuses; `check` proves |
| 2 | `hypothesis` and later name a kill condition | error | `status`, `check` |
| 3 | A verdict rests on evidence with the matching verdict | error | `status`, `check` |
| 4 | Every edge target exists, and no edge is a self-loop | error | `link`, `check` |
| 5 | `contradicts` is mutual | error | `link` writes both sides; `check` |
| 7 | A refuted node reopens only via a `reopens` edge | error | `status` |
| 8 | A graduated node names where it went | error | `graduate`, `check` |
| 9 | A cycle in `supports` is circular reasoning | warn | `check` |
| 10 | A reference carries no verdict or strength | error | deserialization |
| 11 | A reference explains why it is attached | warn | `cite`, `check` |
| 12 | Attachment ids are unique and never reused | error | `check` |
| 13 | Task ids are well-formed | error | `check` |
| 14 | Local paths resolve | warn | `check` |
| 15 | Every node names a declared domain | error | `new`, `promote`, `domain set`; `check` |

## Where a rule lives matters

Three placements, chosen per rule.

**Deserialization**, for rule 10. A reference carrying a verdict makes the corpus
fail to load rather than producing a finding. The separation between context and
evidence is the discipline the system exists to impose, and a rule that merely
warned would be routed around the moment it was inconvenient.

**Point of action**, for rules 1, 5, 7 and 15. `neb link` refuses a cycle-closing
edge and writes both halves of a `contradicts` pair. `neb status` refuses to
reopen a refuted node. Catching these when you act is worth more than catching
them later, because you still remember what you meant.

**The checker**, for everything that needs the whole corpus in view, and as a
backstop for rules also enforced elsewhere, since node files are hand-editable.

## Errors and warnings

Errors mean the corpus is inconsistent and `check` exits non-zero. Warnings mean
something deserves attention without blocking, and are reserved for cases where
the flagged state is sometimes the honest answer: two ideas that genuinely lean
on each other, a link you cannot yet explain but would rather keep than lose.
