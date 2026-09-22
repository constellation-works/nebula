---
title: Invariants
owner: claude
last_updated: 2026-09-22
last_validated: 2026-09-22
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: What neb check enforces, where each rule is enforced, and which failures block.
tags: [lineage-graph, invariants]
paths: ["crates/nebula-core/src/check.rs", "crates/nebula-core/src/model.rs", "crates/nebula-core/src/ops.rs", "crates/nebula-core/src/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Invariants

`neb check` is the lock, the same role `check-theory.py` plays in principia. A
schema is a suggestion until something refuses a corpus that violates it.
Ten of these are the rules as of the 2026-09-12 reduction; rules 11 and 12
were added afterward to catch a hand edit that leaves a node's lifecycle
fields or dates inconsistent with each other. See
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
| 8 | An `observatory` record resolves under the configured root — the same rule, softer, because it judges the machine rather than the corpus | warn | `check` (shape at `cite`) |
| 9 | Every reference has a note | warn | `check` |
| 10 | No two tags differ only by case or a trailing `s` | warn | `check` |
| 11 | `closed` is set only on a `refuted` or `abandoned` node, never an open one | error | `check` |
| 11 | A `seed` does not carry a `kill` condition — a sign status changed by hand | warn | `check` |
| 12 | `created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created` | error | `check` |

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

## Rule 8 and Observatory records

A reference of kind `observatory` carries a bare record id (`Q002`, `H007`,
`T003`, `R012`) rather than a location, and where that record is comes from
`observatory_root` in `config.yaml`, else `$OBSERVATORY_ROOT`. The split
follows from what each half means. The id's *shape* is the corpus's business,
so `cite` refuses anything that is not one of the four letters followed by
digits — a path stored there would never resolve, and the mistake is obvious
now and cryptic in a year. Whether the record is *on this machine* is not the
corpus's business at all: an unset root or a checkout without the record says
the machine is missing something, not that the citation is wrong, so `check`
warns. Erroring would make one portable corpus fail on every machine that
does not happen to have Observatory checked out.

## Rules 11 and 12: nothing a verb writes, only what a hand edit leaves

Every other checker rule backstops something a verb also refuses at the
point of action. Rules 11 and 12 do not: no verb takes `closed`, `created`,
`updated` or a reference's `added` as free-form input, so there is no place
for a matching refusal to live. `ops.rs` only ever produces a `closed` block
on `refuted`/`abandoned`, a `kill` together with a move to `hypothesis`, and
a date from `store::today()`. Any other value has to have gotten there by
hand — `check` is the only place these are ever seen, and it never repairs
them, only reports.

The seed-with-kill case is a warning rather than an error: it is not wrong by
itself, only unusual, since the node has not yet been sharpened through the
guard that would move its status too.

## Errors and warnings

Errors mean the corpus is inconsistent and `check` exits non-zero. Warnings mean
something deserves attention without blocking, and are reserved for cases where
the flagged state is sometimes the honest answer: a link you cannot yet
explain but would rather keep than lose, or a tag that might be a legitimate
near-duplicate of another.
