# Invariants and refusals

Twelve rules. Each is enforced at one of three strengths — at parse (the file
will not load), at the point of action (the verb refuses), or by `neb check`
(a finding) — and the strength is deliberate. `error` findings make `check`
exit non-zero; `warn` findings do not.

| # | Rule | Level | Enforced by |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link` refuses; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`, `check` |
| 4 | `contradicts` is mutual | error | `link` writes both; `check` |
| 5 | `refuted` carries `closed.why` | error | `status`, `check` |
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status`/`sharpen` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Local reference URIs resolve, relative to `nodes/` | error | `cite`, `check` |
| 8 | An `observatory` reference's record resolves under the configured root | warn | `check` (the id's shape is refused at `cite`) |
| 9 | Every reference has a note | warn | `check` |
| 10 | No two tags differ only by case or a trailing `s` | warn | `check` |
| 11 | `closed` is set only on a `refuted`/`abandoned` node, never an open one | error | `check` |
| 11 | A `seed` does not carry a `kill` condition | warn | `check` |
| 12 | `created`, `updated` and every reference's `added` parse as `YYYY-MM-DD`, and `updated` is not earlier than `created` | error | `check` |

Genealogy means the four directed kinds: `derives-from`, `refines`,
`generalizes`, `reopens`. `contradicts` is symmetric and not genealogy, so it
may point anywhere without creating a cycle.

## One writer at a time

Every verb that writes takes an advisory lock on `<root>/.lock` and holds it
until the write — and the commit that records it — is done. Reads take
nothing, so `show`, `list`, `trace`, `impact`, `graph`, `near`, `open`,
`review` and `check` never wait and never block anybody.

This matters to you because you are the second writer. A human at a terminal,
the desktop's capture box and your session all run the same verbs against one
corpus, and a write of yours that lands between somebody's load and their save
is an edit of theirs that quietly disappears. The lock is what stops that; the
only thing you see of it is the refusal below when the wait runs out.

`<root>/.lock` is not corpus content. It is never committed, never checked,
and never something to delete or edit.

## What each refusal means and what to do

Refusals are typed. The message is what the CLI prints; the variant is what
`nebula-core` returns to the desktop app. **Do not retry the same command.**

| Message | Variant | Rule | Do instead |
|---|---|---|---|
| `that edge would make \`X\` its own ancestor` | `Cycle {from, to}` | 1 | The edge is backwards, or the relation is really `contradicts`. Run `neb trace X` and `neb trace Y --down` to see the existing line; propose the reverse edge or none. |
| `a node cannot link to itself` | `SelfLoop` | 3 | You passed the same id twice. Check the ids with `neb list --json`. |
| `no node \`X\`` | `NoSuchNode` | 3 | The id is wrong. Ids are title slugs; `neb list --json \| jq '.[].id'`. Never `neb new` a node to satisfy a link you meant for an existing one. |
| `that edge already exists` | `DuplicateEdge` | — | Nothing to do; it is already recorded. |
| `\`hypothesis\` needs a kill condition first` | `NeedsKill(hypothesis)` | 2 | `neb sharpen <id> --kill "..."` — it moves the status for you. Ask the human for the falsifier if you do not have one; do not invent it. |
| `a kill condition cannot be empty` | `EmptyKill` | 2 | Same: write the falsifier. |
| `refuted needs --why: say how the kill condition fired` | `RefutedNeedsWhy` | 5 | `neb status <id> refuted --why "..."`. The reason is the human's; quote them. |
| `\`X\` is refuted and cannot simply reopen` | `RefutedCannotReopen` | 6 | Refuted is final, including its kill condition: `sharpen --kill` and `sharpen --confirm` refuse too, because rewriting the falsifier would orphan `closed.why`. A verdict is part of the record, so `status <id> refuted --why ...` on an already-refuted node is refused too, even with a new `--why`: that would silently replace `closed.why` and its date rather than leaving the recorded verdict alone. `neb new "..." && neb link <new> reopens X` so the fact that it once died stays visible. Only with the human's say-so. Abandoned is not a verdict and is revivable: `sharpen` on an abandoned node is allowed, and `status <id> abandoned --why ...` on an already-abandoned node is allowed too, replacing the reason. |
| `\`X\` cannot become \`Y\`` | `InvalidTransition` | — | A guard you have not seen. Report it verbatim; do not work around it. |
| `duplicate node id \`X\`` | `DuplicateId` | — | The corpus has two documents claiming one id, so a verb cannot safely choose one. Report both paths; do not overwrite either document or retry the verb. |
| `parent \`X\` does not exist` | `MissingParent` | — | The parent id is wrong or has not been created. Check `neb list --json`; use an existing parent, or create the intended parent only with the human's approval. |
| `title \`X\` does not reduce to a usable id` | `UnusableTitle` | — | Give `neb new` or `neb promote` a title containing letters or numbers, or provide a valid `--id`. Do not retry the same unusable title. |
| `\`X\` is not a valid id: ids are lowercase words joined by single dashes, 60 characters or fewer` | `InvalidId` | — | Use a lowercase slug of single-dash-separated words, at most 60 characters, for `neb new` or `neb promote --id`. Do not retry the invalid id. |
| `\`../x.md\` does not resolve from .../nodes` | `UnresolvedUri` | 8 | Local URIs are relative to `nodes/`. Fix the path (`../../studies/x.md`) or use a URL/wikilink. |
| `\`X\` is not an Observatory record id` | `InvalidObservatoryId` | 8 | `--kind observatory` takes the bare id (`Q002`, `H007`, `T003`, `R012`), never a path or a slug. Never fall back to `--uri <absolute path>`: it breaks on every other machine. |
| `no open inbox entry \`X\`` | `NoSuchInboxEntry` | — | Already promoted or dropped, or the id is wrong. `neb inbox --json`. |
| `node \`X\` already exists` | `NodeExists` | — | A node with that slug exists. Show it; the human decides whether this is a duplicate (drop) or a refinement (`new` with a different title + `refines`). |
| `no corpus at <dir>` | `NoCorpus` | — | The root is wrong. Do **not** `neb init` somewhere new; confirm `NEBULA_ROOT` with the human. |
| `... is schema_version 1, and this build understands 2` | `SchemaMismatch` | — | The corpus needs `neb migrate`. In session mode, run it only on a clean git tree and tell the human it lands as its own commit; in routine mode, propose it. |
| `<root> has staged changes outside the corpus (<paths>); the write is in place and nothing was committed` | `StagedElsewhere { root, paths }` | — | **The write already landed. Do not retry the verb.** Report the staged paths; commit or unstage them, then catch up the corpus with `git -C <root> add nodes inbox config.yaml && git -C <root> commit -m "neb"`, or use `--no-commit` next time. |
| `another nebula writer is holding <root>; nothing was written` | `Locked { root }` | — | Another `neb`, an agent session, or the desktop app was mid-write and still had the corpus lock after a five-second wait. **Nothing was written, so the same command is safe to run again** — unlike every other refusal in this table, this one is worth retrying, once, after a pause. Do not delete `<root>/.lock`: the lock goes with the writer's process, so there is never a stale one to clear. If it keeps refusing, say so and name the root; something is holding the corpus open. |
| `<root> is ignored by the git repository that contains it; nothing can be committed` | `CorpusIgnored` | — | The write landed but cannot be committed there. Run `git -C <root> init` to make the corpus its own repository, or turn commits off with `neb config commit off`; do not retry the write. |
| `git <context> failed in <root>: <stderr>` | `Git { root, context, stderr }` | — | The write is in place; git is what failed. Report the command and stderr, fix the git problem, then catch up the corpus with a separate commit. Do not retry the verb. |
| `a reason only applies to refuted or abandoned` | `Corpus(..)` | — | Drop `--why` when moving to an open status. |

## Warnings `check` will raise after your writes

- **Rule 9** — a reference with no note. Add one with the human's reason for
  attaching it; if you cited it, you know why.
- **Rule 8, observatory** — the record does not resolve, or no root is set.
  Never "fix" this by rewriting the reference as a path. Tell the human to run
  `neb config observatory-root <DIR>` or export `$OBSERVATORY_ROOT`; if the
  root is right, the checkout simply does not carry that record yet.
- **Rule 10** — `Design` next to `design`, or `study` next to `studies`. Tags
  are normalised on write, so this only arises from hand edits; propose
  `neb tag <id> --remove <bad> --add <good>` and name the node.
