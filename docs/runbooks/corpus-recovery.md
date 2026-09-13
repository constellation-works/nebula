---
type: runbook
summary: Diagnose a corpus that will not load, repair hand-edited nodes, and recover from an interrupted write.
tags: [operations, recovery, debugging]
paths: ["crates/nebula-core/src/model.rs", "crates/nebula-core/src/check.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-12
---

# Recover a Corpus

## Symptom: every command fails with a parse error

`neb check` reports the offending file and field:

```
error: in /corpus/nodes/an-idea.md: parsing frontmatter: references[0]: unknown field `uri_kind`
```

The corpus refuses to load as a whole rather than skipping the bad node, because
a partially loaded graph would give wrong answers to `trace` and `impact` while
looking like it worked.

Common causes, all from hand-editing:

| message | cause | fix |
|---|---|---|
| `unknown field` on a reference | a field that never existed, or one from an older schema | correct the spelling, or run `neb migrate` if the corpus is still at schema 1 |
| `unknown field` on a node | a typo, or a field from a newer version | correct the spelling |
| `missing YAML frontmatter` | the leading `---` line was lost | restore it |
| `frontmatter is not terminated` | the closing `---` was lost | restore it |

If the config names `schema_version: 1` (or has no `schema_version` at all),
every node fails to load and the error names `neb migrate` — see
[migrate-v1-to-v2.md](migrate-v1-to-v2.md).

Prefer the CLI over hand-editing. Every command that mutates a node writes valid
frontmatter by construction.

## Symptom: a stray `.md.tmp` file

Node writes render to a sibling temporary file and rename, so a crash mid-write
leaves the original intact and a `.md.tmp` beside it. The temporary file is
never read. Delete it, or inspect it first if you suspect the write was the one
you wanted:

```sh
find "$NEBULA_ROOT/nodes" -name '*.md.tmp'
```

## Symptom: check reports a genealogy cycle

`neb link` refuses cycle-closing edges, so a cycle means a node file was edited
by hand. The message names the whole path:

```
ERROR [1] corpus genealogy cycle: a -> b -> a
```

Remove whichever edge is wrong. `derives-from`, `refines` and `generalizes` are
the only edges that can create one; `contradicts` cannot, since it is symmetric
rather than directional.

## Symptom: an edge points at a node that does not exist

```
ERROR [3] some-node edge `derives-from` points at missing node `ghost`
```

Nodes are never deleted through the CLI, so this means a file was removed
manually or an id was mistyped. Restore the file from git, or correct the id.

## Symptom: `neb migrate` refuses with uncommitted changes

```
error: /corpus has uncommitted changes; commit or stash them so the migration is its own commit
```

This is deliberate: see [migrate-v1-to-v2.md](migrate-v1-to-v2.md). Commit or
stash, then run `neb migrate` again.

## Restoring from git

The corpus should be a git repository. Since nothing is ever deleted in normal
operation, almost any damage is a checkout away:

```sh
git -C "$NEBULA_ROOT" status
git -C "$NEBULA_ROOT" checkout -- nodes/an-idea.md
```
