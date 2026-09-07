---
type: runbook
summary: Diagnose a corpus that will not load, repair hand-edited nodes, and recover from an interrupted write.
tags: [operations, recovery, debugging]
paths: ["src/model.rs", "src/check.rs"]
related_features: [lineage-graph]
related_artifacts: []
last_validated: 2026-09-07
---

# Recover a Corpus

## Symptom: every command fails with a parse error

`neb check` reports the offending file and field:

```
error: in /corpus/nodes/an-idea.md: parsing frontmatter: references[0]: unknown field `verdict`
```

The corpus refuses to load as a whole rather than skipping the bad node, because
a partially loaded graph would give wrong answers to `trace` and `impact` while
looking like it worked.

Common causes, all from hand-editing:

| message | cause | fix |
|---|---|---|
| `unknown field \`verdict\`` on a reference | a finding filed as context | move it to `evidence`, or use `neb weigh` |
| `unknown field` on a node | a typo, or a field from a newer version | correct the spelling |
| `missing YAML frontmatter` | the leading `---` line was lost | restore it |
| `frontmatter is not terminated` | the closing `---` was lost | restore it |

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

Remove whichever edge is wrong. If both look right, one of them is probably a
`supports` relation mislabelled as `derives-from`: an idea can support its own
ancestor, but it cannot descend from its own descendant.

## Symptom: an edge points at a node that does not exist

```
ERROR [4] some-node edge `derives-from` points at missing node `ghost`
```

Nodes are never deleted through the CLI, so this means a file was removed
manually or an id was mistyped. Restore the file from git, or correct the id.

## Restoring from git

The corpus should be a git repository. Since nothing is ever deleted in normal
operation, almost any damage is a checkout away:

```sh
git -C "$NEBULA_ROOT" status
git -C "$NEBULA_ROOT" checkout -- nodes/an-idea.md
```
