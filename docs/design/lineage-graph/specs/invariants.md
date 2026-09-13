---
title: Invariants
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: What neb check enforces, where each rule is enforced, and which failures block.
tags: [lineage-graph, invariants]
paths: ["src/check/**", "src/corpus/model.rs", "src/corpus/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Invariants

`neb check` is the lock, the same role `check-theory.py` plays in principia. A
schema is a suggestion until something refuses a corpus that violates it.
These are the ten rules as of the 2026-09-12 reduction; see
[docs/design/v0.2/1_spec.md](../../v0.2/1_spec.md) for the model they apply to
and its "What is removed" table for the rules this replaced.

| # | rule | level | enforced at |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link` refuses; `check` proves |
| 2 | `hypothesis` names a non-empty kill condition | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`, `check` |
| 4 | `contradicts` is mutual | error | `link` writes both; `check` |
| 5 | `refuted` carries a closing reason | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status` |
| 7 | A reference carries none of the removed judgement fields | error | parse |
| 8 | Local reference URIs resolve | error | `cite`, `check` |
| 9 | Every reference has a note | warn | `check` |
| 10 | No two tags differ only by case or a trailing `s` | warn | `check` |

## Where a rule lives matters

Three placements, chosen per rule.

**Deserialization**, for rule 7. A reference carrying an unknown field makes
the corpus fail to load rather than producing a finding. `deny_unknown_fields`
on every model type is the mechanism: the schema itself is the enforcement,
and a rule that merely warned would be routed around the moment it was
inconvenient.

**Point of action**, for rules 1, 2, 4, 5 and 6. `neb link` refuses a
cycle-closing edge and writes both halves of a `contradicts` pair. `neb
sharpen`/`neb status` refuse a `hypothesis` or a `refuted` transition that
does not carry what rule 2 or 5 requires. `neb status` refuses to move a
`refuted` node to any other status. Catching these when you act is worth more
than catching them later, because you still remember what you meant.

**The checker**, for everything that needs the whole corpus in view, and as a
backstop for rules also enforced elsewhere, since node files are hand-editable.

## Errors and warnings

Errors mean the corpus is inconsistent and `check` exits non-zero. Warnings mean
something deserves attention without blocking, and are reserved for cases where
the flagged state is sometimes the honest answer: a link you cannot yet
explain but would rather keep than lose, or a tag that might be a legitimate
near-duplicate of another.
