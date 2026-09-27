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
between the two. The next verb that writes settles it before doing its own
work — strikes the line when the node was written, discards the record when
it was not — and until then `inbox` does not list an entry whose node exists.
`check` names it (rule 17). Never delete or edit it, and never `drop` the
entry it names: run your next write, or `neb promote <entry>`, which settles
it and then reports the entry as already promoted.

A `*.tmp` file under `nodes/` or `inbox/` is what a write killed before its
rename leaves. `check` warns about each one with the `rm` that removes it.
That is the human's call, not yours: report it, and never delete it.

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
| `the edge \`X\` <kind> \`Y\` already exists` | `DuplicateEdge { from, kind, to }` | — | From `link`, nothing to do; it is already recorded (`neb show X` lists it). A `contradicts` link that `check` reports as one-sided is not a duplicate: re-running it writes the missing half. From `new`, a flag named the same node twice and nothing was written: name it once. |
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
| `<path> is a symlink, not a regular file; nothing was read or written through it` (or `a FIFO`, `a device`, `a directory`, `a socket`) | `NotRegularFile { path, found }` (`not_regular_file`) | — | An entry nebula reads or writes by name — a node file, `config.yaml`, `.lock`, `.pending`, an inbox month file, or a non-regular `.gitignore` target — is not a regular file. A node symlink to another node beside it reports `id_mismatch` instead; `init` copies a regular `.gitignore` symlink target into a local file without changing the target. Other corpus symlinks are refused without reading through them, and pipes or devices are never opened for content. Report the path to the human; do not remove, replace or follow it yourself, and do not retry. The corpus root itself may be reached through a symlink; ordinary corpus entries beneath it may not. A `.lock` that is not a regular file can simply be removed (the next writer makes a new one), but that is the human's call. |
| `\`../x.md\` does not resolve from .../nodes` | `UnresolvedUri` | 8 | Local URIs are relative to `nodes/`. Fix the path (`../../studies/x.md`) or use a URL/wikilink. |
| `\`X\` is an absolute local path; local references are relative to nodes/` | `AbsoluteUri` | 8 | An absolute path or `file:` URI names nothing on any other machine the corpus is synced to, so it is refused even when it exists here. Rewrite it relative to `nodes/` (`../../studies/x.md`), cite an Observatory record by its id with `--kind observatory`, or use a URL. |
| `\`X\` is not an Observatory record id` | `InvalidObservatoryId` | 9 | `--kind observatory` takes the bare id (`Q<nnn>`, `H<nnn>`, `T<nnn>`, `R<nnn>`), never a path or a slug. Never fall back to `--uri <absolute path>`: it breaks on every other machine. |
| `Observatory record \`X\` does not resolve under <root>` | `UnresolvedObservatoryRecord { record, root }` | 9 | From `handoff`, with an observatory root set: closing a node for a record this checkout does not carry would point its lineage at nothing. Check the id with the human; if it is right, the checkout is behind, so tell the human rather than work around it with `cite` and `status`. Nothing was written. |
| `\`X\` is not an accepted reference kind; accepted kinds: ...` | `UnknownReferenceKind` | 16 | `--kind` takes one of the listed kinds, in any case. Pick the closest (`other` when none fits); never invent a kind. |
| `the observatory root must be an absolute path, not \`X\`` | `RelativeObservatoryRoot { root, setting }` | 9 | The setting is per machine and read from any directory. With `(in <file>)`, this machine's setting file is empty or relative: tell the human, who sets it again with an absolute `neb config observatory-root <DIR>`. Otherwise pass the checkout's absolute path. Never write the path into `config.yaml` instead. |
| `the corpus root setting <file> is empty` | `EmptyRootSetting` (`empty_root_setting`) | — | `~/.config/nebula/root` is there but names nothing, so every command that would fall back to it refuses rather than guess `~/.nebula`. Nothing was read or written. Tell the human, who points it at the corpus with `neb init <DIR> --set-root` (an absolute path; no `--force` needed to replace an invalid setting). Do not write the file yourself. |
| `the corpus root setting <file> must be an absolute path, not \`X\`` | `RelativeRootSetting { setting, root }` (`relative_root_setting`) | — | The setting holds a relative path, which would name a different corpus from every working directory, so every command that falls back to it refuses before creating anything. From `init --set-root`, the path you passed was relative: pass it absolute. Otherwise tell the human, who replaces it with `neb init <DIR> --set-root`; never create a corpus at the relative path to work around it. |
| `no open inbox entry \`X\`` | `NoSuchInboxEntry` | — | The id is wrong: no inbox line, waiting or settled, carries it. `neb inbox --json`. |
| `\`X\` was already promoted to \`N\`` / `\`X\` was already dropped` | `InboxEntrySettled { id, settlement }` | — | The entry is settled and the verb has nothing to do. Promoted: work on node `N` (`neb show N`). Dropped: it stays dropped; if the human wants it back, capture it again. Do not retry. |
| `there is no candidate N; this entry has ...` | `NoSuchCandidate {number, shown}` | — | `triage` was given a number its current entry was not shown. Nothing was written; pick a listed number, `p` for a root, or `s`. |
| `\`triage\` is interactive and has no JSON form` | `Interactive` | — | `triage` is for a human at a terminal. Script the same decisions with `neb inbox --json`, `neb near --json`, then `neb promote` or `neb drop` per entry. |
| `node \`X\` already exists` | `NodeExists` | — | A node with that slug exists. Show it; the human decides whether this is a duplicate (drop) or a refinement (`new` with a different title + `refines`). |
| `no corpus at <dir>` | `NoCorpus` | — | The root is wrong. Do **not** `neb init` somewhere new; confirm `NEBULA_ROOT` with the human. |
| `creating <dir>: <OS reason>` | `IoAt { action: "creating", path, source }` | — | `capture` or `init` could not make a corpus at that path, usually because the root is mistyped or its parent is not writable. Nothing was captured. Confirm `NEBULA_ROOT` or `--root` with the human; do not retry against some other directory. |
| `... is schema_version 1, and this build understands 2` | `SchemaMismatch` | — | The corpus needs `neb migrate`. In session mode, run it only on a clean git tree and tell the human it lands as its own commit; in routine mode, propose it. |
| `<root>/config.yaml does not exist, so the corpus's schema and settings are unknown; nothing was read or written` | `MissingConfig { path }` (`missing_config`) | — | The corpus has `nodes/` but no config, and no verb will guess one. Nothing was written. Do **not** create `config.yaml` by hand and do not run `neb init`. Tell the human: if the file was deleted, restore it from git (`git -C <root> checkout -- config.yaml`), which keeps the corpus's id and `commit` setting; if the corpus predates the file, `neb migrate` brings it forward (session mode, clean tree only; propose it in routine mode). |
| `<node> is a v1 node (it carries \`domain\`), but <root>/config.yaml declares schema_version 2` | `V1NodeUnderCurrentSchema { path, config, version, keys }` | — | The config says v2 but this node is still in the v1 shape, most likely because an older `neb` stamped a config over a v0.1 corpus. Nothing was written. Report it to the human, who either sets `schema_version: 1` in that `config.yaml` and runs `neb migrate`, or, if the key was a hand edit, removes it from the node. Do not edit either file yourself. |
| `<root>/.pending records an unfinished write that this build cannot read: ...; nothing was written` | `PendingWriteUnreadable { path, reason }` | 17 | The record of an interrupted `promote` was edited by hand or written by a newer `neb`, so every write refuses rather than guess how to finish it. **Nothing was written; do not retry, and do not delete the file.** Report the path and the reason to the human, who reads it, checks the node and inbox line it names, and removes it. |
| `another nebula writer is holding <root> (\`<label>\`, pid <pid>, for <age>); nothing was written`, or `... (an unidentified writer); ...` | `Locked { root, holder }` (`locked`), `holder: Option<LockHolder { pid, since, label }>` | — | Another `neb`, an agent session, or the desktop app was mid-write and still had the lock after a five-second wait. **Nothing was written.** The message names the holder: its label says which writer (`neb tag a-node`, `desktop capture`; `nebula` for one that did not say), with its PID and how long it has held the lock. Do not loop on retries. Tell the human which writer holds the corpus; if it is your own session's other command, let that finish first. Once the holder has finished, the same command is safe to run once more, since nothing was written. If it holds on (minutes, not seconds), report the label and PID rather than waiting it out. Never kill that process and never delete `<root>/.lock`: the lock goes with the writer's process, so there is never a stale one to clear. "An unidentified writer" means the holder left no readable record (a writer between taking the lock and recording itself, or an older `neb`); it is still a held lock, so the same advice applies. When `<root>` is `~/.config/nebula`, the lock held is the one on this machine's settings, taken by `init --set-root` and `config observatory-root DIR`. |
| `` `X`'s body changed while it was being edited; nothing was written; your edited text is kept at <path> `` | `EditConflict(X)` (`edit_conflict`) | — | From `edit`: another writer changed the body while it was open in the editor, and saving would have erased that. Your text is in `<path>`, a new owner-only file under `$XDG_STATE_HOME/nebula/edits/` (default `~/.local/state/nebula/edits/`), outside the corpus. `neb show X`, then `neb edit X` again and carry the text over; never copy the kept file into `nodes/`. Every other refusal after the editor exits (`notes_changed`, `locked`, a failed write) keeps the text the same way and names the file. |
| `<what> on standard input is larger than <limit> bytes; nothing was written` | `InputTooLarge { what, limit }` (`input_too_large`) | — | `capture -` takes up to 64 KiB (65536 bytes) and `--body -` up to 1 MiB (1048576 bytes). Refused before the corpus is opened for writing, so nothing was written. A thought that long is not a capture: put the text in a file and `neb cite` it, or split it. |
| `<root> is ignored by the git repository that contains it; nothing can be committed` | `CorpusIgnored` | — | The write landed but cannot be committed there. Run `git -C <root> init` to make the corpus its own repository, or turn commits off with `neb config commit off`; do not retry the write. |
| `<root> is not inside a git work tree` | `NotGitWorkTree` | — | From `log` or `show --at`: there is no git repository at or above the root, so there is no history to read. Say so; do not `git init` on the human's behalf. |
| `no node \`X\` at \`REV\`` | `NoNodeAtRevision { node, revision }` | — | The commit or date is real, and the node did not exist then. That is an answer, not an error to work around. |
| `no commit \`REV\` in the corpus's repository` | `UnknownRevision { revision }` (`unknown_revision`) | — | The hash names no commit here: mistyped, or from another repository. Take a hash from `neb log <id> --json`, or pass a date. |
| `git <context> failed in <root>: <stderr>` | `Git { root, context, stderr }` | — | The write is in place; git is what failed. `git rev-parse failed` means there is a repository at or above the root that git cannot read (a corrupt `HEAD`, a `safe.directory` refusal): report it, never treat the corpus as unversioned. `git status failed` from `migrate` means nothing was rewritten. Report the command and stderr, fix the git problem, then catch up the corpus with a separate commit. Do not retry the verb. A stderr longer than 1 MiB ends in `… [truncated N bytes]`. |
| `git <context> did not finish within <deadline> in <root> and was stopped` | `GitTimedOut { root, context, after }` (`git_timed_out`) | — | git ran past its deadline (120s for `commit`, 30s otherwise), and it and everything it started were stopped. From `commit`, a hook is the likely cause: **the write is in place and uncommitted, so do not retry the verb.** Tell the human which hook (`git -C <root> hook run pre-commit` reproduces it); they fix it, or use `--no-commit` and commit the corpus later. The lock is already free. |
| `a reason only applies to refuted or abandoned, not \`X\`` | `ReasonOnOpenStatus(X)` (`reason_on_open_status`) | — | Drop `--why` when moving to an open status. Nothing was written. |
| `nothing to capture` | `EmptyCapture` (`empty_capture`) | — | The text was only whitespace. Nothing was written, and no corpus was created for it. Capture the thought itself. |
| `a note cannot be empty` | `EmptyNote` (`empty_note`) | — | The note was only whitespace; nothing was written. Pass the note's text. |
| `\`X\` has no kill condition to confirm` | `NoKillToConfirm(X)` (`no_kill_to_confirm`) | 2 | `sharpen --confirm` adopts a falsifier somebody proposed, and `X` has none. Ask the human for one and `neb sharpen X --kill "..."`; do not invent it. |
| `a reference of kind \`K\` needs a URI; only a discussion may have none` | `UriRequired { kind }` (`uri_required`) | — | Pass where the reference lives with `--uri`; only `--kind discussion` may go without. Never invent a URI to get past this. |
| `\`X\` cannot be an author label: no parentheses, newlines or \`: \`` | `InvalidAuthorLabel(X)` (`invalid_author_label`) | — | Pass a `--by` label without those characters, such as your session id or crew name. Nothing was written. |
| `\`X\` is not a YYYY-MM-DD date or a git revision` | `InvalidAt(X)` (`invalid_at`) | — | `show --at` takes a date or a commit hash; `neb log <id>` lists the node's commits. |
| `\`X\` is not a status` / `\`X\` is not an edge type` | `NotAStatus(X)` / `NotAnEdgeType(X)` (`not_a_status`, `not_an_edge_type`) | — | Use one of the listed values: `seed`, `hypothesis`, `refuted`, `abandoned`; `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts`. |
| `in <file>: missing YAML frontmatter ...` / `in <file>: frontmatter is not terminated by a \`---\` line` | `MalformedFrontmatter { path, problem }` (`malformed_frontmatter`) | — | A node file lost a `---` line, usually to a hand edit. Report the file; the fix is in `docs/runbooks/corpus-recovery.md`. Do not rewrite the file yourself. |
| `inbox entry \`X\` was on line N of <file>, which now ends before it; nothing was written` | `InboxEntryMissing { id, file, line }` (`inbox_entry_missing`) | — | The month file got shorter between reading the entry and settling it: a hand edit or another writer. Nothing was written. Read `neb inbox --json` again before deciding anything about `X`. |
| `line N of <file> no longer holds inbox entry \`X\`; nothing was written` | `InboxEntryChanged { id, file, line }` (`inbox_entry_changed`) | — | The line now holds other text. Same as above: nothing was written; read the inbox again. |
| `inbox entry \`X\` is in <file>, not in this corpus's inbox <dir>` | `InboxEntryForeign { id, file, inbox }` (`inbox_entry_foreign`) | — | A library caller handed one corpus an entry read from another. Settle it through the corpus it came from. |
| `inbox id namespace exhausted; nothing captured` | `InboxIdsExhausted` (`inbox_ids_exhausted`) | — | All 65,536 inbox ids are held by waiting entries. Nothing was captured; the inbox needs triage before anything else goes in. Tell the human. |
| `<root>/nodes is a symlink; node operations require a real directory` / `<path> is a symlink; inbox operations require real paths` | `NodesSymlink(path)` / `InboxSymlink(path)` (`nodes_symlink`, `inbox_symlink`) | 15 | A symlink could redirect a write outside the corpus, so nothing was read or written. Report the path; do not replace or follow the symlink yourself. |
| `HOME is not set` / `HOME is not valid UTF-8: \`...\`` | `HomeUnset` / `HomeNotUnicode(value)` (`home_unset`, `home_not_unicode`) | — | There is no usable home directory for this machine's settings or `~/.nebula`. Name the corpus with `--root` or `NEBULA_ROOT`, and tell the human about the environment. |
| `an explicit corpus root cannot be empty` | `EmptyRoot` (`empty_root`) | — | `--root ""` would resolve to the working directory. Pass a directory, or leave `--root` out. |
| `<file> already points to <A>, not <B>` | `RootConfigConflict { path, configured, requested }` (`root_config_conflict`) | — | This machine's default corpus is already another one. Only with the human's say-so, replace it with `--force`. |
| `<root> has uncommitted changes, and the migration must be its own commit; nothing was changed` | `DirtyTree(root)` (`dirty_tree`) | — | With the human, commit or stash those changes, then `neb migrate` again. |
| `in <file>: status \`X\` is not a v1 status` / `in <file>: edge type \`X\` is not a v1 edge type` | `NotAV1Status { path, status }` / `NotAV1EdgeType { path, edge_type }` (`not_a_v1_status`, `not_a_v1_edge_type`) | — | `neb migrate` met a value v1 never had. Nothing was changed. Report the file; the human decides what the value was meant to be. |
| `in <file>: the migration produced a node this build cannot read: ...; nothing was written` | `MigratedNodeUnreadable { path, source }` (`migrated_node_unreadable`) | — | A migration step produced a node the current model refuses, found before any write. Nothing was changed. Report the file and the reason verbatim; do not edit the node to force the migration through. |
| `git log returned a malformed history record for \`nodes/X.md\` in <root>` (or `git rev-list ...`) | `MalformedHistory { root, command, pathspec }` (`malformed_history`) | — | git's output did not split into whole records. Report it verbatim; do not retry. |
| `no free name to keep the edit of \`X\` under in <dir> after N tries` | `NoFreeKeepName { id, dir, attempts }` (`no_free_keep_name`) | — | A refused `edit` could not keep its text under a new name. Report the directory; nothing in the corpus changed. |
| `reading <what> from standard input: <OS reason>` | `IoStdin { what, source }` (`io_stdin`) | — | Standard input could not be read; nothing was written. Pass the text as arguments instead of `-`. |
| `<what> on standard input is not valid UTF-8; nothing was written` | `StdinNotUtf8 { what }` (`stdin_not_utf8`) | — | The piped bytes are not text. Convert them to UTF-8 first, or pass the text as arguments. |
| `writing <file> failed: ...; removing the temporary file <tmp> also failed: ...` | `TempCleanupFailed { path, tmp, .. }` (`temp_cleanup_failed`) | — | The write did not happen, and its temporary file is still beside the target. Report both paths; do not delete anything yourself. |
| `no free temporary name beside <file> after N tries; ...` | `NoFreeTempName { path, attempts }` (`no_free_temp_name`) | — | Something keeps creating files where `neb` writes its temporaries. Nothing was written. Report the path. |

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
