---
type: runbook
summary: Bring a v1 corpus (domain, evidence, tasks, graduate) forward to the v2 schema with `neb migrate`.
tags: [operations, migration, corpus]
paths: ["src/commands/migrate.rs", "src/corpus/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-12
---

# Migrate a v1 Corpus to v2

`neb migrate` brings a corpus written by v0.1 forward to the reduced v0.2
schema in place: one shot, idempotent, and lossless. Read
[docs/design/v0.2/1_spec.md](../design/v0.2/1_spec.md) ("Migration") for the
full mapping this runbook operationalizes.

## Before you run it: the dirty-tree refusal

```sh
neb migrate
```

If `$NEBULA_ROOT` is a git repository with uncommitted changes, this refuses:

```
error: /corpus has uncommitted changes; commit or stash them so the migration is its own commit
```

That is deliberate. The migration should land as its own commit, so that
`git diff` shows exactly what changed and a bad conversion is a `git revert`
away. Commit or stash first:

```sh
git -C "$NEBULA_ROOT" add -A
git -C "$NEBULA_ROOT" commit -m "pre-migration snapshot"
neb migrate
```

A corpus that is not a git repository at all is migrated as is — put it under
git first if you want the safety net (see
[corpus-setup.md](corpus-setup.md)).

## What changes

Per node, `neb migrate` reads the old, lenient shape and writes the new one
back, printing a line per node changed and a note per thing it did:

- `domain: X` is appended to `tags` (normalised to kebab-case) if not already
  present, and the field is dropped.
- Each `evidence[]` entry becomes a reference: `kind: other`, `uri` from the
  evidence `source`, `title` from the first line of its note, and
  `note: "[<verdict>/<strength>] <original note>"` so the old judgement is
  legible rather than silently dropped. It keeps its `origin` block if it had
  one.
- Each `tasks[]` entry becomes a reference with `uri: "orbit:<id>"`,
  `title: <id>`, and `note: "[<state>] <why>"`.
- The removed dependency-and-evidence edges (the ones that were not
  genealogy and not `contradicts`) become a reference on the source node
  pointing at `neb:<target-id>`, carrying the old edge kind and target in the
  note, so the claim survives even though the edge kind does not.
- `status: testing | supported` becomes `hypothesis`.
- `status: graduated` becomes `abandoned`, with `closed.why` recording where
  it graduated to (or a generic note if `graduated_to` was empty).
- A v1 `status: refuted` node that never had a reason recorded gets a
  `closed.why` synthesized to say exactly that — nothing here invents a real
  reason, it records that the old record did not have one.
- `config.yaml` is rewritten to hold only `corpus_id` and
  `schema_version: 2`; the declared-domain list and default are dropped along
  with the field they supported.

Nothing is deleted. Everything the v0.1 schema could express ends up either
unchanged (genealogy edges, `contradicts`, existing references) or re-labelled
into a reference with enough of the original note that a human can still tell
what it meant.

## Idempotence

Run it again and nothing changes:

```
already at schema 2; nothing changed
```

A node already in v2 form renders back to exactly the bytes on disk, so a
second run rewrites nothing and no node's `updated` timestamp moves.

## Verify

```sh
neb check
```

should report the same node count as before the migration, zero new errors,
and only the ordinary warnings (an empty reference note, a tag that drifted).
If a reference note reads oddly, that is the point where a human should look
at it and clean it up by hand — `neb migrate` re-labels, it does not judge.
