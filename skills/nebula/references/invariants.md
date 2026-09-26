# Invariants and refusals

The rules below are each enforced at one of three strengths — at parse (the file
will not load), at the point of action (the verb refuses), or by `neb check`
(a finding) — and the strength is deliberate. `error` findings make `check`
exit non-zero; `warn` findings do not.

| # | Rule | Level | Enforced by |
|---|---|---|---|
| 1 | Genealogy is acyclic | error | `link`/`new` refuse; `check` proves |
| 2 | `hypothesis` names a non-empty `kill` | error | `sharpen`/`status`, `check` |
| 3 | Every edge target exists; no self-loop | error | `link`/`new`, `check` |
| 4 | `contradicts` is mutual | error | `link`/`new` write both; `check` |
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

Rule 15 is the one rule `check` cannot hold: an id decides which file a write
lands in, so a corpus whose ids disagree with their file names is one `check`
could not load to report on. A file whose stored id could
not name a node file, and a file whose name and stored id simply disagree,
are both refused by every verb that touches them — `check` included — and the
refusal names the file and the id it stores. Both mean a hand edit, and both
are the human's to resolve: never yours to "repair" by renaming a file or
rewriting an `id`.

Genealogy means the four directed kinds: `derives-from`, `refines`,
`generalizes`, `reopens`. `contradicts` is symmetric and not genealogy, so it
may point anywhere without creating a cycle.

## One writer at a time

Every verb that writes takes an advisory lock on `<root>/.lock` and holds it
until the write — and the commit that records it — is done. Reads take
nothing, so `show`, `list`, `trace`, `impact`, `graph`, `near`, `review`
and `check` never wait and never block anybody.

This matters to you because you are the second writer. A human at a terminal,
the desktop's capture box and your session all run the same verbs against one
corpus, and a write of yours that lands between somebody's load and their save
is an edit of theirs that quietly disappears. The lock is what stops that; the
only thing you see of it is the refusal below when the wait runs out.

`<root>/.lock` is not corpus content. It is never committed, never checked,
and never something to delete or edit.

## What each refusal means and what to do

Refusals are typed. The message is what the CLI prints; the variant is what
`nebula-core` returns to the desktop app, and its name in `snake_case` is what
`--json` reports as the refusal's `code`: `NeedsKill` is `needs_kill`, `IoAt`
is `io_at` (the envelope is in [verbs.md](verbs.md#refusals-under---json)).
**Do not retry the same command.**

| Message | Variant | Rule | Do instead |
|---|---|---|---|
| `that edge would make \`X\` its own ancestor` | `Cycle {from, to}` | 1 | The edge is backwards, or the relation is really `contradicts`. Run `neb trace X` and `neb trace Y --down` to see the existing line; propose the reverse edge or none. |
| `a node cannot link to itself` | `SelfLoop` | 3 | You passed the same id twice. Check the ids with `neb list --json`. |
| `no node \`X\`` | `NoSuchNode` | 3 | The id is wrong. Ids are title slugs; `neb list --json \| jq '.[].id'`. Never `neb new` a node to satisfy a link you meant for an existing one. |
| `that edge already exists` | `DuplicateEdge` | — | From `link`, nothing to do; it is already recorded. From `new`, a flag named the same node twice and nothing was written: name it once. |
| `\`X\` is named as both a parent and the node this reopens` | `ParentAndReopens(X)` | 1 | `reopens` is already genealogy, so drop `--parent X` and keep `--reopens X`. Nothing was written. |
| `\`hypothesis\` needs a kill condition first` | `NeedsKill(hypothesis)` | 2 | `neb sharpen <id> --kill "..."` — it moves the status for you. Ask the human for the falsifier if you do not have one; do not invent it. |
| `a kill condition cannot be empty` | `EmptyKill` | 2 | Same: write the falsifier. |
| `kill condition is already \`X\`; it was not replaced` | `KillAlreadySet(X)` | 2 | An open node's falsifier is content, including after `sharpen --confirm`; `sharpen --kill` never overwrites it, regardless of `--by`. A materially different falsifier is a different idea, so create a new node and link the relationship. |
| `refuted needs --why: say how the kill condition fired` | `RefutedNeedsWhy` | 5 | `neb status <id> refuted --why "..."`. The reason is the human's; quote them. |
| `\`X\` is refuted and cannot simply reopen` | `RefutedCannotReopen` | 6 | Refuted is final, including its kill condition: `sharpen --kill` and `sharpen --confirm` refuse too, because rewriting the falsifier would orphan `closed.why`. A verdict is part of the record, so `status <id> refuted --why ...` on an already-refuted node is refused too, even with a new `--why`: that would silently replace `closed.why` and its date rather than leaving the recorded verdict alone. `neb new "..." --reopens X` so the fact that it once died stays visible. Only with the human's say-so. Abandoned is not a verdict and is revivable: `sharpen` on an abandoned node is allowed, and `status <id> abandoned --why ...` on an already-abandoned node is allowed too, replacing the reason. |
| `\`X\` is already refuted, so it cannot be closed again` / `... already abandoned ...` | `AlreadyClosed { id, status }` | 6 | From `handoff`: only an open node can be handed off. Refuted is a verdict: if the human wants the idea to go on downstream, `neb new "..." --reopens X` and hand that node off. Abandoned (including an earlier hand-off) already has a reason; `neb show X` and report it rather than closing it again. Nothing was written. |
| `\`X\` names a kill condition, so it cannot go back to seed` | `SeedWithKill` | 13 | The kill is content and stays: moving the node to `seed` would leave a seed carrying a falsifier, which `check` reads as a hand edit. A node that names what would kill it reopens as a hypothesis: `neb status X hypothesis`, from `abandoned` too. Never delete or blank the kill to make `seed` succeed. |
| `\`X\` cannot become \`Y\`` | `InvalidTransition` | — | A guard you have not seen. Report it verbatim; do not work around it. |
| `duplicate node id \`X\`` | `DuplicateId` | — | The corpus has two documents claiming one id, so a verb cannot safely choose one. Report both paths; do not overwrite either document or retry the verb. |
| `parent \`X\` does not exist` | `MissingParent` | — | The parent id is wrong or has not been created. Check `neb list --json`; use an existing parent, or create the intended parent only with the human's approval. |
| `title \`X\` does not reduce to a usable id` | `UnusableTitle` | — | Give `neb new` or `neb promote` a title containing letters or numbers, or provide a valid `--id`. Do not retry the same unusable title. |
| `\`X\` is not a valid id: ids are lowercase words joined by single dashes, 60 characters or fewer` | `InvalidId` | — | Use a lowercase slug of single-dash-separated words, at most 60 characters, for `neb new` or `neb promote --id`. Do not retry the invalid id. |
| `\`X\` cannot be a node id: an id names one file under nodes/, ...` | `UnsafeId` | 15 | The id **you passed** holds a path separator, a `.`/`..`, a root, or a control character, so it could not name a node file. Nothing was read or written. Get the real id from `neb list --json`; never rewrite an id into a path, and do not retry. |
| `<file> stores the id \`X\`, which is not the node its file name names` | `IdMismatch {path, id}` | 15 | A node **file**'s name and its stored `id` disagree — because the id is another node's, because it is not a name a file can have at all (`../../escaped`), or because the file is a second name — a hard link or symlink under `nodes/` — for a node whose file is named for its id. A write derives its destination from the stored id, so this would land on some other node or outside the corpus; nothing was read or written. Report the path and the stored id; do not "fix" it by renaming the file or editing the id, and do not retry the verb. Only the human knows which of the two the node really is — or, for an alias, whether the second name should exist at all. |
| `\`../x.md\` does not resolve from .../nodes` | `UnresolvedUri` | 8 | Local URIs are relative to `nodes/`. Fix the path (`../../studies/x.md`) or use a URL/wikilink. |
| `\`X\` is an absolute local path; local references are relative to nodes/` | `AbsoluteUri` | 8 | An absolute path or `file:` URI names nothing on any other machine the corpus is synced to, so it is refused even when it exists here. Rewrite it relative to `nodes/` (`../../studies/x.md`), cite an Observatory record by its id with `--kind observatory`, or use a URL. |
| `\`X\` is not an Observatory record id` | `InvalidObservatoryId` | 9 | `--kind observatory` takes the bare id (`Q002`, `H007`, `T003`, `R012`), never a path or a slug. Never fall back to `--uri <absolute path>`: it breaks on every other machine. |
| `Observatory record \`X\` does not resolve under <root>` | `UnresolvedObservatoryRecord { record, root }` | 9 | From `handoff`, with an observatory root set: closing a node for a record this checkout does not carry would point its lineage at nothing. Check the id with the human; if it is right, the checkout is behind, so tell the human rather than work around it with `cite` and `status`. Nothing was written. |
| `\`X\` is not an accepted reference kind; accepted kinds: ...` | `UnknownReferenceKind` | 16 | `--kind` takes one of the listed kinds, in any case. Pick the closest (`other` when none fits); never invent a kind. |
| `the observatory root must be an absolute path, not \`X\`` | `RelativeObservatoryRoot { root, setting }` | 9 | The setting is per machine and read from any directory. With `(in <file>)`, this machine's setting file is empty or relative: tell the human, who sets it again with an absolute `neb config observatory-root <DIR>`. Otherwise pass the checkout's absolute path. Never write the path into `config.yaml` instead. |
| `no open inbox entry \`X\`` | `NoSuchInboxEntry` | — | The id is wrong: no inbox line, waiting or settled, carries it. `neb inbox --json`. |
| `\`X\` was already promoted to \`N\`` / `\`X\` was already dropped` | `InboxEntrySettled { id, settlement }` | — | The entry is settled and the verb has nothing to do. Promoted: work on node `N` (`neb show N`). Dropped: it stays dropped; if the human wants it back, capture it again. Do not retry. |
| `there is no candidate N; this entry has ...` | `NoSuchCandidate {number, shown}` | — | `triage` was given a number its current entry was not shown. Nothing was written; pick a listed number, `p` for a root, or `s`. |
| `\`triage\` is interactive and has no JSON form` | `Interactive` | — | `triage` is for a human at a terminal. Script the same decisions with `neb inbox --json`, `neb near --json`, then `neb promote` or `neb drop` per entry. |
| `node \`X\` already exists` | `NodeExists` | — | A node with that slug exists. Show it; the human decides whether this is a duplicate (drop) or a refinement (`new` with a different title + `refines`). |
| `no corpus at <dir>` | `NoCorpus` | — | The root is wrong. Do **not** `neb init` somewhere new; confirm `NEBULA_ROOT` with the human. |
| `creating <dir>: <OS reason>` | `IoAt { action: "creating", path, source }` | — | `capture` or `init` could not make a corpus at that path, usually because the root is mistyped or its parent is not writable. Nothing was captured. Confirm `NEBULA_ROOT` or `--root` with the human; do not retry against some other directory. |
| `... is schema_version 1, and this build understands 2` | `SchemaMismatch` | — | The corpus needs `neb migrate`. In session mode, run it only on a clean git tree and tell the human it lands as its own commit; in routine mode, propose it. |
| `<root> has staged changes outside the corpus (<paths>); the write is in place and nothing was committed` | `StagedElsewhere { root, paths }` | — | **The write already landed. Do not retry the verb.** Report the staged paths; commit or unstage them, then catch up the corpus with `git -C <root> add nodes inbox config.yaml && git -C <root> commit -m "neb"`, or use `--no-commit` next time. |
| `another nebula writer is holding <root>; nothing was written` | `Locked { root }` | — | Another `neb`, an agent session, or the desktop app was mid-write and still had the corpus lock after a five-second wait. **Nothing was written, so the same command is safe to run again** — unlike every other refusal in this table, this one is worth retrying, once, after a pause. Do not delete `<root>/.lock`: the lock goes with the writer's process, so there is never a stale one to clear. If it keeps refusing, say so and name the root; something is holding the corpus open. When `<root>` is `~/.config/nebula`, the lock held is the one on this machine's settings, taken by `init --set-root` and `config observatory-root DIR`; the same advice applies. |
| `<root> is ignored by the git repository that contains it; nothing can be committed` | `CorpusIgnored` | — | The write landed but cannot be committed there. Run `git -C <root> init` to make the corpus its own repository, or turn commits off with `neb config commit off`; do not retry the write. |
| `git <context> failed in <root>: <stderr>` | `Git { root, context, stderr }` | — | The write is in place; git is what failed. Report the command and stderr, fix the git problem, then catch up the corpus with a separate commit. Do not retry the verb. A stderr longer than 1 MiB ends in `… [truncated N bytes]`. |
| `git <context> did not finish within <deadline> in <root> and was stopped` | `GitTimedOut { root, context, after }` (`git_timed_out`) | — | git ran past its deadline (120s for `commit`, 30s otherwise), and it and everything it started were stopped. From `commit`, a hook is the likely cause: **the write is in place and uncommitted, so do not retry the verb.** Tell the human which hook (`git -C <root> hook run pre-commit` reproduces it); they fix it, or use `--no-commit` and commit the corpus later. The lock is already free. |
| `a reason only applies to refuted or abandoned` | `Corpus(..)` | — | Drop `--why` when moving to an open status. |

## Warnings `check` will raise after your writes

- **Rule 10** — a reference with no note. Add one with the human's reason for
  attaching it; if you cited it, you know why.
- **Rule 9, observatory** — the record does not resolve, or no root is set.
  Never "fix" this by rewriting the reference as a path. Tell the human to run
  `neb config observatory-root <DIR>` or export `$OBSERVATORY_ROOT`; if the
  root is right, the checkout simply does not carry that record yet. The same
  rule, with no node, flags a legacy `observatory_root` key in `config.yaml`:
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
