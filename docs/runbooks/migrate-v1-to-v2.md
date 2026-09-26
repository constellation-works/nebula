---
type: runbook
summary: Bring a v1 corpus (domain, evidence, tasks, graduate) forward to the v2 schema with `neb migrate`.
tags: [operations, migration, corpus]
paths: ["crates/nebula-core/src/migrate.rs", "crates/nebula-core/src/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-26
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

A corpus with no git repository at or above it is migrated as is — put it
under git first if you want the safety net (see
[corpus-setup.md](corpus-setup.md)). A repository git cannot read is not the
same thing: when git cannot say whether the tree is clean (`git rev-parse
failed` or `git status failed`), `neb migrate` refuses and rewrites nothing.
Repair the repository, then run it again.

## The already-at-v2 refusal

Running it on a corpus that is already at `schema_version: 2` is a no-op, and
that is the point: there is nothing left to re-label. So before writing
anything, `migrate` reads every node with the strict current model — the same
one `neb check` uses — and refuses the whole run if one will not parse:

```
error: the corpus already declares schema_version 2, and in /corpus/nodes/current.md:
parsing frontmatter: unknown field `future_field`, expected one of `id`, `title`, ...;
migration will not rewrite a node it cannot read, so nothing was changed
```

Without that, the lenient v1 read below would swallow the unrecognised key,
write the node back without it, and report it as migrated. The check runs over
the whole corpus first rather than node by node, so a bad node late in the
sequence cannot leave the earlier ones rewritten.

Fix the node by hand — `neb check` names the same problem — then run
`neb migrate` again. Nothing on disk changed, so there is nothing to undo.

## A corpus stamped v2 over v1 nodes

An older `neb` wrote a fresh `config.yaml` whenever it opened a corpus that had
none, and stamped it `schema_version: 2`. A v0.1 corpus from before the file
existed could come out of that declaring v2 over nodes still in the v1 shape.
Nothing can tell that apart from a hand edit for certain, so `migrate` still
refuses it, but when the unreadable node reads as v1 and carries a key only v1
had (`domain`, `evidence`, `tasks` or `graduated_to`), `migrate` and every
verb that reads the nodes say so (`v1_node_under_current_schema`):

```
error: /corpus/nodes/an-idea.md is a v1 node (it carries `domain`), but
/corpus/config.yaml declares schema_version 2
```

If the corpus was never migrated, repair it by hand and migrate:

1. In `<root>/config.yaml`, set `schema_version: 1`. This keeps the
   `corpus_id` and the `commit` setting. Deleting the file works too, but
   `migrate` then mints a new `corpus_id` and leaves `commit` off.
2. Run `neb migrate`.

If the key was added by hand to a corpus that really is v2, remove it from the
node instead.

## A corpus with no `config.yaml`

Every verb except `migrate` and `init` refuses a corpus root whose `nodes/` has
no `config.yaml` beside it (`missing_config`), and none of them writes one:
which schema the files follow, the corpus's id and its `commit` setting are all
unknown. `neb migrate` reads the absence as a corpus from before the file
existed, so it converts the nodes as v1, writes the config, and reports that it
minted a `corpus_id`:

```
config.yaml schema_version -> 2
config.yaml minted corpus_id neb-1a2b3c (there was none to keep)
```

If the file was deleted from a corpus that had one, restore it instead, for
example with `git -C "$NEBULA_ROOT" checkout -- config.yaml`: that keeps the
corpus's own id and settings. A `config.yaml` that is there but cannot be read,
such as a symlink to a missing file, is reported as that path's I/O error, not
as a missing config.

## What changes

Per node, `neb migrate` reads the old, lenient shape and converts it to the
new one, printing a line per node changed and a note per thing it did. Every
node is converted in memory before any file is written, so a node that cannot
be converted (`status \`bogus\` is not a v1 status`, naming its file) refuses
the whole run with nothing rewritten. Fix that node and run it again. Then the
nodes are written one by one, and `config.yaml` is written last: until it
says `schema_version: 2`, every other verb refuses the corpus, so no reader
treats a half-written corpus as current.

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
- `config.yaml` is rewritten with `corpus_id` and `schema_version: 2`, while
  preserving `observatory_root` when set and the `commit` setting. A preserved
  `observatory_root` is read only as a legacy fallback; set each machine's own
  with `neb config observatory-root <DIR>`, then remove the key with
  `neb config observatory-root --drop-legacy`. The
  declared-domain list and default are dropped along with the field they
  supported.

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
second run rewrites nothing and no node's `updated` timestamp moves. The same
property makes a run that stopped part way safe to repeat: the config still
declares v1, the nodes already written convert to themselves, and the rerun
finishes the rest.

## Verify

```sh
neb check
```

should report the same node count as before the migration, zero new errors,
and only the ordinary warnings (an empty reference note, a tag that drifted).
If a reference note reads oddly, that is the point where a human should look
at it and clean it up by hand — `neb migrate` re-labels, it does not judge.
