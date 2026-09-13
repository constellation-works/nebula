# Invariants and refusals

Ten rules. Each is enforced at one of three strengths — at parse (the file
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
| 6 | `refuted` leaves only via a new node's `reopens` edge | error | `status` |
| 7 | A reference carries no `verdict`/`strength` | error | parse |
| 8 | Local reference URIs resolve, relative to `nodes/` | error | `cite`, `check` |
| 9 | Every reference has a note | warn | `check` |
| 10 | No two tags differ only by case or a trailing `s` | warn | `check` |

Genealogy means the four directed kinds: `derives-from`, `refines`,
`generalizes`, `reopens`. `contradicts` is symmetric and not genealogy, so it
may point anywhere without creating a cycle.

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
| `\`X\` is refuted and cannot simply reopen` | `RefutedCannotReopen` | 6 | Refuted is final. `neb new "..." && neb link <new> reopens X` so the fact that it once died stays visible. Only with the human's say-so. |
| `\`X\` cannot become \`Y\`` | `InvalidTransition` | — | A guard you have not seen. Report it verbatim; do not work around it. |
| `\`../x.md\` does not resolve from .../nodes` | `UnresolvedUri` | 8 | Local URIs are relative to `nodes/`. Fix the path (`../../studies/x.md`) or use a URL/wikilink. |
| `no open inbox entry \`X\`` | `NoSuchInboxEntry` | — | Already promoted or dropped, or the id is wrong. `neb inbox --json`. |
| `node \`X\` already exists` | `NodeExists` | — | A node with that slug exists. Show it; the human decides whether this is a duplicate (drop) or a refinement (`new` with a different title + `refines`). |
| `no corpus at <dir>` | `NoCorpus` | — | The root is wrong. Do **not** `neb init` somewhere new; confirm `NEBULA_ROOT` with the human. |
| `... is schema_version 1, and this build understands 2` | `SchemaMismatch` | — | The corpus needs `neb migrate`. In session mode, run it only on a clean git tree and tell the human it lands as its own commit; in routine mode, propose it. |
| `a reason only applies to refuted or abandoned` | `Corpus(..)` | — | Drop `--why` when moving to an open status. |

## Warnings `check` will raise after your writes

- **Rule 9** — a reference with no note. Add one with the human's reason for
  attaching it; if you cited it, you know why.
- **Rule 10** — `Design` next to `design`, or `study` next to `studies`. Tags
  are normalised on write, so this only arises from hand edits; propose
  `neb tag <id> --remove <bad> --add <good>` and name the node.
