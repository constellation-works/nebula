# Invariants

Read this for the 17 rules and their strengths, the single-writer lock,
`.pending` and temporary files, and the warnings `check` raises after your
writes.

Contents:

- [One writer at a time](#one-writer-at-a-time)
- [Warnings `check` will raise after your writes](#warnings-check-will-raise-after-your-writes)

The rules below are each enforced at one of three strengths — at parse (the file
will not load), at the point of action (the verb refuses), or by `neb check`
(a finding) — and the strength is deliberate. `error` findings make `check`
exit non-zero; `warn` findings do not.

| # | Rule | Level | Enforced by |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link`/`new` refuse; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`/`new`, `check` |
| 4 | `contradicts` is mutual | error | `link`/`new` write both, and re-running `link` writes a missing half; `check` |
| 5 | `refuted` carries `closed.why` | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status`/`sharpen`/`handoff` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Non-discussion references have a URI; local URIs resolve relative to `nodes/` and are never absolute | error; warn for an absolute path already in the corpus | `cite` refuses both; `check` reports both |
| 9 | An `observatory` reference's record resolves under the configured root | warn | `check` (the id's shape is refused at `cite`; `handoff` refuses a record that does not resolve under a set root) |
| 10 | Every reference has a note | warn | `check` |
| 11 | No two tags differ only by case or a trailing `s` | warn | `check`; noted at `new`/`promote`/`tag` |
| 12 | `closed` is set only on a `refuted`/`abandoned` node, never an open one | error | `check` |
| 13 | A `seed` does not carry a `kill` condition | warn | `status` refuses the move to `seed`; `check` |
| 14 | `created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created` | error | `check` |
| 15 | A node's `id` names one file under `nodes/`, and is the id its file name names | error | parse (the shape); every read and write (the agreement) |
| 16 | Every reference kind belongs to the documented vocabulary | warn | `cite` refuses new values; `check` reports existing ones |
| 17 | No write is left half-done: no stray temporary file and no pending-write record | warn; error for a record this build cannot read | the next write settles the record; `check` reports both and deletes neither |

Rule 15 protects where a write lands. A file whose stored id cannot name a
node file, or whose name and stored id disagree, is unreadable. Strict graph
queries refuse the whole load with `unreadable_nodes`; `check` instead lists
each file with its underlying code (`unsafe_id`, `id_mismatch`, or a parse
error) and checks the readable nodes. It exits 1 if any file is unreadable.
Report the paths and causes to the human; never repair them by renaming a
file or rewriting an `id`.

Genealogy means the four directed kinds: `derives-from`, `refines`,
`generalizes`, `reopens`. `contradicts` is symmetric and not genealogy, so it
may point anywhere without creating a cycle.

## One writer at a time

Every verb that writes takes an advisory lock on `<root>/.lock` and holds it
until the write — and the commit that records it — is done, and for nothing
else: not while `edit`'s editor is open, not while standard input is read,
and not across the suggestions, close-tag notes or `--json` views a verb
prints after its write. Reads take
nothing, so `show`, `list`, `trace`, `impact`, `graph`, `near`, `review`
and `check` never wait and never block anybody.

This matters to you because you are the second writer. A human at a terminal,
the desktop's capture box and your session all run the same verbs against one
corpus, and a write of yours that lands between somebody's load and their save
is an edit of theirs that quietly disappears. The lock is what stops that; the
only thing you see of it is the refusal below when the wait runs out.

`<root>/.lock` is not corpus content. It is never committed, never checked,
and never something to delete or edit. While a writer holds the lock, the file
holds one line naming it — its PID, when it took the lock, and a label such as
`neb edit a-node` or `desktop capture` — which the refusal below reads out. The
line only explains a wait: the kernel lock is what decides who holds it, and
the line can be stale or empty.

Nor is `<root>/.pending`. `promote` writes it before the node and removes it
after striking the inbox line, so it outlives the verb only when a crash fell
between the two. The next verb that writes settles it before doing its own work
— strikes the line when the node was written, discards the record when it was
not — and until then `inbox` does not list an entry whose node exists. `check`
names it (rule 17).

Never delete or edit it, and never `drop` the entry it names: run your next
write, or `neb promote <entry>`, which settles it and then reports the entry as
already promoted.

A `*.tmp` file under `nodes/` or `inbox/` is what a write killed before its
rename leaves. `check` warns about each one with the `rm` that removes it.
That is the human's call, not yours: report it, and never delete it.

## Warnings `check` will raise after your writes

- **Rule 10** — a reference with no note. Add one with the human's reason for
  attaching it; if you cited it, you know why.
- **Rule 9, observatory** — the record does not resolve, or no root is set.
  Never "fix" this by rewriting the reference as a path. Tell the human to run
  `neb config observatory-root <DIR>` or export `$OBSERVATORY_ROOT`; if the
  root is right, the checkout simply does not carry that record yet.

  The same rule, with no node, flags a legacy `observatory_root` key in `config.yaml`:
  one machine's path in the shared corpus. Leave it to the human, who sets
  each machine's own and then runs `neb config observatory-root --drop-legacy`.
- **Rule 8, absolute path** — a reference whose URI is an absolute path or a
  `file:` URI, written by hand or carried over by `neb migrate`. It may
  resolve here and nowhere else. Propose rewriting it relative to `nodes/`, or
  as an Observatory id if that is what it points at; the new reference goes in
  with `neb cite`, since there is no verb that edits one in place.
- **Rule 11** — `Design` next to `design`, or `study` next to `studies`. Writes
  normalise case, so a case pair is a hand edit; a plural is usually a write,
  which printed `note: tag X is close to Y (N nodes)` on stderr when it
  happened. The finding names the nodes carrying each variant: propose
  `neb tag <id> --remove <bad> --add <good>` for each node on the minority
  variant, and name them.
