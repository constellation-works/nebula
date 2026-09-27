---
type: runbook
summary: Diagnose a corpus that will not load, repair hand-edited nodes, and recover from an interrupted write.
tags: [operations, recovery, debugging]
paths: ["crates/nebula-core/src/model.rs", "crates/nebula-core/src/check/", "crates/nebula-core/src/store/", "crates/nebula-core/src/pending.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-26
---

# Recover a Corpus

## Symptom: every command fails with a parse error

`neb check` reports the offending file and field:

```
error: in /corpus/nodes/an-idea.md: parsing frontmatter: references[0]: unknown field `uri_kind`
```

`neb check` reports each unreadable or malformed node file as an error finding,
with its path and cause, and checks the nodes it can load. Its summary counts
unreadable files
separately from checked nodes, and it exits nonzero when any file is unreadable.
A broken Observatory setting is an error finding while other node checks still
run. An unreadable Observatory directory is reported as unreadable rather than
as a missing record.

Graph queries such as `list`, `show`, `trace`, `graph`, `impact`, `near`, and
`review` refuse as a whole, naming every bad file. A partially loaded graph
would give wrong lineage answers while looking complete.

Common causes, all from hand-editing:

| message | cause | fix |
|---|---|---|
| `unknown field` on a reference | a field that never existed, or one from an older schema | correct the spelling, or run `neb migrate` if the corpus is still at schema 1 |
| `unknown field` on a node | a typo, or a field from a newer version | correct the spelling |
| `missing YAML frontmatter` | the leading `---` line was lost | restore it |
| `frontmatter is not terminated` | the closing `---` was lost | restore it |

If `config.yaml` names `schema_version: 1`, or exists with no
`schema_version` key, the corpus is refused before any node is read
(`schema_mismatch`), and the hint names `neb migrate`. If there is no
`config.yaml` at all, every verb but `migrate` and `init` refuses with
`missing_config` and writes nothing: `neb migrate` brings forward a corpus from
before the file existed, and a file deleted by mistake is better restored from
git, which keeps the corpus's id and `commit` setting. A node in the v1 shape
under a config that says `schema_version: 2` is `v1_node_under_current_schema`,
and the repair is to set `schema_version: 1` in `config.yaml` and run
`neb migrate`. All three are in [migrate-v1-to-v2.md](migrate-v1-to-v2.md).

Prefer the CLI over hand-editing. Every command that mutates a node writes valid
frontmatter by construction.

## Symptom: check warns about a `.tmp` file

```
warn  [17] corpus `nodes/an-idea.md.1f2e-0-18d8.tmp` is a temporary file left by a write that did not finish; ...
```

Every file is written to a sibling temporary file and renamed into place, so
a crash mid-write leaves the original intact and a nonce-named
`<file>.<pid>-<counter>-<nanos>.tmp` beside it. Nothing reads it, `neb`
never commits it (its commits exclude `*.tmp`, and `neb init` git-ignores
it), and nothing in `neb` deletes it: its name says a write made it, not that
its bytes are nothing you want. Inspect it if you suspect the write was one
you wanted, then remove it with the `rm` the warning prints. To list them
all:

```sh
find "$NEBULA_ROOT/nodes" "$NEBULA_ROOT/inbox" -name '*.tmp'
```

## Symptom: a capture you promoted is still in the inbox

`promote` writes the node and then strikes the inbox line, and records what
it is doing in `$NEBULA_ROOT/.pending` first. A crash between the two leaves
that record, and `check` names it:

```
warn  [17] corpus a promotion of inbox entry `1f2e` did not finish, as /corpus/.pending records; `an-idea` was written, so the next write strikes the entry `-> an-idea`
```

Do nothing by hand. The next `neb` verb that writes — any of them, and the
desktop's too — settles it before its own work: the line is struck
`-> <node>` when the node was written, and the record is discarded, with the
entry left waiting, when it was not. Until then `neb inbox` does not offer
an entry whose node exists. Running `neb promote <entry>` again settles it
and then says the entry was already promoted. Do not `neb drop` it: that
records a promoted thought as dropped.

If every writing verb refuses with `records an unfinished write that this
build cannot read`, the record was edited by hand or written by a newer
`neb`. Read it, make sure the node it names and that entry's inbox line say
what you want, then remove it:

```sh
cat "$NEBULA_ROOT/.pending"
rm "$NEBULA_ROOT/.pending"
```

## Symptom: check reports a one-sided contradiction

```
ERROR [4] alpha contradicts `beta`, which does not contradict back; record the other half with `neb link alpha contradicts beta`
```

`link` saves the two ends one after the other, so a crash between them, or a
hand edit, leaves the claim on one node only. Run the command the finding
names: with the edge already on `alpha`, it writes only the missing half on
`beta`, credited to whoever made the first, and refuses as `that edge already
exists` once both are there.

## Symptom: check reports a genealogy cycle

`neb link` refuses cycle-closing edges, so a cycle means a node file was edited
by hand. The message names the whole path:

```
ERROR [1] corpus genealogy cycle: a -> b -> a
```

Remove whichever edge is wrong. `derives-from`, `refines`, `generalizes` and
`reopens` are genealogy edges and can create a cycle; `contradicts` cannot,
since it is symmetric rather than directional.

## Symptom: an edge points at a node that does not exist

```
ERROR [3] some-node edge `derives-from` points at missing node `ghost`
```

Nodes are never deleted through the CLI, so this means a file was removed
manually or an id was mistyped. Restore the file from git, or correct the id.

## Symptom: `neb migrate` refuses with uncommitted changes

```
error: /corpus has uncommitted changes, and the migration must be its own commit; nothing was changed

Commit or stash them, then run it again.
```

This is deliberate: see [migrate-v1-to-v2.md](migrate-v1-to-v2.md). Commit or
stash, then run `neb migrate` again. A `git rev-parse failed` or `git status
failed` refusal instead means git could not say whether the tree is clean, so
nothing was rewritten; repair the repository first.

## Symptom: a verb writes but refuses to commit

With `neb config commit on`, a verb prints its usual result and then a git
refusal, for example:

```
error: git rev-parse failed in /corpus: fatal: not a git repository (or any of the parent directories): .git
```

The write happened — the node or inbox line is on disk — and only the commit
did not. `rev-parse failed` means there is a `.git` at or above the corpus
root that git cannot read: a corrupt `HEAD`, a half-finished edit of `.git`,
or a repository owned by another user that `safe.directory` does not trust.
`neb` reports that rather than treat the corpus as unversioned. Repair the
repository (`git -C "$NEBULA_ROOT" status` shows git's own complaint), then
either run any verb (its commit sweeps up the earlier write) or catch up by
hand:

```sh
git -C "$NEBULA_ROOT" add -A -- nodes inbox config.yaml .gitignore
git -C "$NEBULA_ROOT" diff --cached --name-only --relative -z -- nodes/ inbox/ config.yaml .gitignore \
  | git -C "$NEBULA_ROOT" commit -m "neb" --pathspec-from-file=- --pathspec-file-nul
```

The commit gets its path list from the staged corpus files, so an empty
`nodes/` or `inbox/` is not passed as an unmatched pathspec. The add and diff
are rooted at the corpus even when it sits inside a larger worktree; other
staged paths stay staged. The explicit add also leaves runtime files such as
`.pending/` records and `*.tmp` debris out of this recovery commit.

`--no-commit` on a verb skips its commit once, if you need to keep working
before sorting the repository out.

If the `git` executable is missing from `PATH` or not installed, the verb exits
1 with code `git` and prints a start refusal:

```
error: git start failed in /corpus: No such file or directory (os error 2)
```

Under `--json`, stderr carries the refusal envelope (`code: "git"`, exit 1)
and stdout retains the write's payload (such as the new node or inbox entry).
The write is already on disk. **Do not retry the write**: retrying `neb new`
or `neb promote` with the same id will fail with `node_exists`. To recover
safely:

1. Restore or install `git` so it is on `PATH`.
2. Catch up the uncommitted write: either run your next writing command
   (its commit sweeps up earlier uncommitted writes) or commit manually:

```sh
git -C "$NEBULA_ROOT" add -A -- nodes inbox config.yaml .gitignore
git -C "$NEBULA_ROOT" diff --cached --name-only --relative -z -- nodes/ inbox/ config.yaml .gitignore \
  | git -C "$NEBULA_ROOT" commit -m "neb" --pathspec-from-file=- --pathspec-file-nul
```

This uses only staged corpus paths as commit pathspecs, so empty `nodes/` or
`inbox/` directories are harmless. Since both commands run from
`$NEBULA_ROOT`, the recipe also works when the corpus is nested in a larger
worktree. Other staged paths remain staged, and the path-limited add excludes
runtime `.pending/` records and `*.tmp` debris.

3. If this host is not meant to use git, turn commits off with
   `neb config commit off`, or pass `--no-commit` for one invocation.

Work staged elsewhere in the repository never blocks a `neb` commit and never
rides in one: the commit names the corpus paths (`nodes/`, `inbox/`,
`config.yaml`, and the generated `.gitignore`), so anything else stays staged
for you.

If instead a verb succeeds with `note: not committed: /corpus is not inside a
git work tree`, there is no repository at or above the root: set one up as
[corpus-setup.md](corpus-setup.md#put-it-under-git) describes, or turn the
setting off with `neb config commit off`.

A different refusal, `is ignored by the git repository that contains it`,
means the corpus sits under an outer repository whose `.gitignore` hides it,
so there is nothing git would ever record. Make the corpus its own
repository — `git -C "$NEBULA_ROOT" init` — as
[corpus-setup.md](corpus-setup.md#put-it-under-git) recommends, or turn the
setting off with `neb config commit off`.

Any other git failure (`git commit failed in /corpus: ...`) is reported with
git's own message and, again, never undoes the write.

## Restoring from the corpus repo

The corpus should be a git repository, ideally its own one at the corpus
root with a private remote (see
[corpus-setup.md](corpus-setup.md#put-it-under-git)). With `neb config
commit on`, each changed write can make one commit named `neb <verb> <ids>`,
so the history reads as a log of what you did and any state is addressable.

Since nothing is ever deleted in normal operation, almost any damage is a
checkout away:

```sh
git -C "$NEBULA_ROOT" status
git -C "$NEBULA_ROOT" log --oneline -- nodes/an-idea.md
git -C "$NEBULA_ROOT" checkout -- nodes/an-idea.md
```

To undo one verb entirely — a promotion that should have been a drop, a
`link` that was wrong — revert its commit rather than editing files, so the
mistake stays in the history too:

```sh
git -C "$NEBULA_ROOT" log --oneline -5
git -C "$NEBULA_ROOT" revert <hash>
```

If the disk is gone, the remote is the corpus:

```sh
git clone <private-remote> ~/corpus/nebula
export NEBULA_ROOT=$HOME/corpus/nebula
neb check
```

`check` should report the node count you remember and zero errors. Anything
written after the last push is lost, which is the argument for pushing
often; `neb` commits but never pushes, so a cron job or a habit has to.
