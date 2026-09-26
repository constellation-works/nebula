---
title: Invariants
owner: claude
last_updated: 2026-09-26
last_validated: 2026-09-26
status: Accepted
feature: lineage-graph
doc_role: spec
type: design
summary: What neb check enforces, where each rule is enforced, and which failures block.
tags: [lineage-graph, invariants]
paths: ["crates/nebula-core/src/check.rs", "crates/nebula-core/src/model.rs", "crates/nebula-core/src/ops.rs", "crates/nebula-core/src/store.rs", "crates/nebula-core/src/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
---

# Invariants

`neb check` is the lock, the same role `check-theory.py` plays in principia. A
schema is a suggestion until something refuses a corpus that violates it.
The original v0.2 rules were later extended to catch hand edits that leave a
node's lifecycle fields or dates inconsistent, and to stop an edited `id`
from turning an ordinary verb into a write outside the corpus. See
[docs/design/v0.2/1_spec.md](../../v0.2/1_spec.md) for the model they apply to
and its "What is removed" table for the rules this replaced.

| # | rule | level | enforced at |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link` refuses; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`, `check` |
| 4 | `contradicts` is mutual | error | `link` writes both; `check` |
| 5 | `refuted` carries `closed.why` | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Non-discussion references have a URI; local URIs resolve relative to `nodes/` and are never absolute | error; warn for an absolute path already in the corpus | `cite` refuses both; `check` reports both |
| 9 | An `observatory` reference's record resolves under the configured root | warn | `check` (the id's shape is refused at `cite`) |
| 10 | Every reference has a note | warn | `check` |
| 11 | No two tags differ only by case or a trailing `s` | warn | `check` |
| 12 | `closed` is set only on a `refuted`/`abandoned` node, never an open one | error | `check` |
| 13 | A `seed` does not carry a `kill` condition | warn | `status` refuses the move to `seed`; `check` |
| 14 | `created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created` | error | `check` |
| 15 | A node's `id` names one file under `nodes/`, and is the id its file name names | error | parse (the shape); every read and write (the agreement) |
| 16 | Every reference kind belongs to the documented vocabulary | warn | `cite` refuses new values; `check` reports existing ones |

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
than catching them later, because you still remember what you meant. The
seed-with-kill rule, 13, has a point-of-action refusal too, described
below.

**Deserialization and the store**, for rule 15, which is the one rule the
checker cannot hold: see below.

**The checker**, for everything that needs the whole corpus in view, and as a
backstop for rules also enforced elsewhere, since node files are hand-editable.

## Rule 9 and Observatory records

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

## Rule 8 and absolute paths

A local reference is a path relative to `nodes/`. An absolute path, or a
`file:` URI, is the one local reference that can resolve and still be wrong:
`/etc/hostname` exists on the machine that cited it, so resolving it proves
nothing, and on every other machine the corpus is synced to it names nothing
while leaking this machine's layout into the corpus. So `cite` refuses one
before resolving it, whether or not it exists here, and names the two things
to write instead: a path relative to `nodes/`, or an Observatory record by its
id. "Absolute" is judged as written and alike on every platform — a leading
`/` or `\`, a drive letter, a `file:` scheme — because a path absolute on any
machine is machine layout on all of them.

`check` reports one already in the corpus, hand written or carried over from
a v1 `evidence` source by `migrate`, as a warning rather than an error, and in
place of the resolution error, since whether it resolves is again a fact about
this machine. Warning keeps an older corpus loading and checking cleanly while
leaving each such reference visible until it is rewritten.

## Rules 12–14: nothing a verb writes, only what a hand edit leaves

Rules 12–14 mostly catch states no verb produces: no verb takes `closed`,
`created`, `updated` or a reference's `added` as free-form input, so there is
no place for a matching refusal to live. `ops.rs` only ever produces a
`closed` block on `refuted`/`abandoned`, a `kill` together with a move to
`hypothesis`, and a date from `store::today()`. Any other value has to have
gotten there by hand — `check` is the only place these are ever seen, and it
never repairs them, only reports.

The exception is rule 13, a seed carrying a kill, which a status move could
produce: nothing is deleted, so moving a node that names a kill condition
back to `seed`, from `hypothesis` or from `abandoned`, would keep the kill.
`neb status` refuses that move (`SeedWithKill`) rather than write a state its
own checker would blame on a hand edit; a node with a falsifier reopens as a
`hypothesis` instead. The check still stands as the backstop.

The seed-with-kill case is a warning rather than an error: it is not wrong by
itself, only unusual, since the node has not yet been sharpened through the
guard that would move its status too.

## Rule 15: the id is a path

A node's id is not only a name. `nodes/<id>.md` is where the node is read
from and, through `Corpus::save`, where the next write lands. That makes an
id the one field a hand edit can point at a file the corpus does not own:
`id: ../../escaped` on `nodes/safe.md` loaded happily, and the next `neb
note` wrote `escaped.md` beside the corpus root. An id that was another
node's was the quieter half of the same bug — a valid id, and therefore a
write straight over that node.

So the rule is two halves, and neither belongs in the checker. The *shape*
half — an id is exactly one ordinary file name, with no separator, no `.` or
`..`, no root or drive prefix, no control character — sits in `model::parse`,
beside rule 7, because a `Doc` that exists at all is one every verb
downstream may write from. The *agreement* half — a file's name and the id it
stores are one fact — sits in `store`, at each of the three doors: `load`
refuses a file that claims to be another node, a scan refuses the same
mismatch from the path side, and `save` re-derives its destination from an id
that has passed both. A checker finding would come too late in every one of
these cases, and a corpus whose ids could not be trusted is exactly the
corpus `check` could not load to report on.

The two halves meet in the reporting, by where the id came from rather than
by which check caught it. An id out of a *file* is reported as
`IdMismatch`, naming the path, because the file is what somebody has to go
and look at — a traversal id in a node file is a disagreement with that file
just as surely as another node's id is. An id out of a *caller* — a verb's
argument, an IPC call — is reported as `UnsafeId`, because there is no file
to name.

The shape rule is deliberately weaker than the slug rule `neb new --id`
enforces. Tightening what a *new* id may look like should never make an
existing corpus unreadable, and the escape is about path structure rather
than about which alphabet an idea was named in: `ünïcode-título-ok` and
`시간은-프레임의-수다` are ordinary ids and stay valid.

Nothing here canonicalizes, per the repository's path rule: the components
are judged as written, so a corpus reached through a symlinked root — which
is every corpus under a macOS temporary directory — answers the same on both
platforms. The one place a name is compared against the filesystem, the scan
that matches a file to the id it stores, asks the filesystem rather than
comparing bytes, because a volume may store a name in a different Unicode
normalization than the id it was written from.

What it asks is whether the two names open one *directory entry*, not one
file. A node has exactly one name under `nodes/`; a second name for it is an
alias, and aliases are refused at both doors, before anything is written. A
hard link `nodes/safe.md` to `nodes/victim.md` was once accepted by `load`
and was a split waiting to happen: a write replaces the file the id names
through a rename, the link keeps the old bytes under the other name, and the
next load of either refuses. A symlink `nodes/alias.md` stayed linked across
the write but made a scan read the node twice. Both are now `IdMismatch`,
naming the alias, when the node is reached through the alias or by a scan. A
hard link is refused through the node's own name too, since that write would
split it; a symlink is not, since the write leaves it pointing at the new
file. Only a link a scan would read counts: a backup
hard-linked from outside `nodes/` is not a second name the corpus sees. The
Unicode respelling survives because it is one entry however it is typed.

## Errors and warnings

Errors mean the corpus is inconsistent and `check` exits non-zero. Warnings mean
something deserves attention without blocking, and are reserved for cases where
the flagged state is sometimes the honest answer: a link you cannot yet
explain but would rather keep than lose, or a tag that might be a legitimate
near-duplicate of another.
