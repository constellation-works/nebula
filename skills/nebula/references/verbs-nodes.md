# Verbs: nodes

Read this for `new`, `edit`, `sharpen`, `status`, `link`, `tag` and `note`.

Contents:

- [Nodes](#nodes)

## Nodes

| verb | does | flags |
|---|---|---|
| `neb new <TITLE>` | create a node directly | `--body <TEXT\|->`, `--parent <ID>`×, `--reopens <ID>`, `--contradicts <ID>`×, `--kill`, `--tag <TAG>`×, `--id <SLUG>`, `--by <LABEL>`, `--task`, `--run` |
| `neb edit <NODE>` | open the body, without frontmatter, in `$VISUAL` or `$EDITOR` | `--by <LABEL>` (required under an Orbit run) |
| `neb sharpen <NODE> --kill <KILL>` | seed → hypothesis by naming the falsifier | `--by <LABEL>`, or `--confirm` instead of `--kill` |
| `neb status <NODE> <STATUS>` | `seed`, `hypothesis`, `refuted`, `abandoned`, with guards | `--why` (required for refuted, optional for abandoned) |
| `neb link <FROM> <KIND> <TO>` | `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts` | `--by <LABEL>` |
| `neb tag <NODE>` | edit tags; normalised to lowercase kebab-case | `--add <TAG>`×, `--remove <TAG>`× |
| `neb tag list` | every tag with its node count | — |
| `neb note [--by <LABEL>] <NODE> <TEXT>...` | append a dated paragraph of reasoning to the body | `--by <LABEL>` (before or after the text) |

`tag` notes on stderr, in every output mode, each tag that was not on the
node to `--remove` (`` `absent` is not a tag of <id>; nothing to remove ``) or
was already on it to `--add` (`` `physics` is already a tag of <id>; nothing to
add ``). When that leaves the tags as they were, the node is not written:
`updated` stays, nothing is committed, and stderr ends
`no change; <id> not written`. The exit code is 0 either way.

`new`, `promote` and `tag --add` print `note: tag physic is close to physics
(2 nodes)` on stderr, in every output mode, when a tag they introduce to the
corpus differs from one in use only by case or a trailing `s`. The write has
already succeeded. Reuse the existing tag unless the difference is deliberate:
`neb tag <NODE> --remove physic --add physics`.

`new --body <TEXT>` sets the prose at creation; `--body -` reads it from
standard input, up to 1 MiB (1048576 bytes), refusing more as `input_too_large`
before anything is written. `new --kill "..."` starts the node as a hypothesis;
without it, a seed. `new --reopens <ID>` revives a refuted node as a new one
with a single `reopens` edge; that edge is genealogy, so naming the same node as
`--parent` too is refused.

`new --contradicts <ID>` writes the edge on both nodes, as `link` does. Every
edge is checked before anything is written: a missing node, the same edge twice,
or a genealogy loop refuses the whole `new`. `sharpen --kill` refuses to replace
a kill condition already present on an open node and prints the existing
falsifier.

A changed falsifier is a changed idea: create a new node and relate it to the
old one instead of overwriting the record. On a refuted node, the stricter
final-verdict refusal still applies. `sharpen --confirm` takes no text: it
adopts the kill condition already on the node as the human's own, changing
nothing else and appending nothing.

`status <id> seed` refuses a node that names a kill condition (`SeedWithKill`),
because the kill would stay and leave a seed carrying one; reopen it with
`status <id> hypothesis` instead, including from `abandoned`. `contradicts` is
written on both nodes. `link` prints `<from> <kind> <to>`. `note` creates a `##
Notes` section at the end of the body if needed, then appends `- YYYY-MM-DD:
<text>`.

Repeated notes accumulate in order; earlier body text, status, edges and tags
are left as they are. `updated` is bumped. `edit` writes only the prose body to
a temporary file, preferring `$VISUAL` over `$EDITOR`, then saves the result
after the editor exits successfully. No lock is held while the editor is open,
so captures and other writes go on meanwhile; the lock is taken only to save.

If the body changed in the meantime (a `note`, another `edit`), the save is
refused as `edit_conflict` rather than erase that change; a change to the
frontmatter alone (a `tag`, a `status`) is kept and the edit lands on top of it.

Whenever a save is refused after the editor exits — `edit_conflict`,
`notes_changed`, `locked`, a failed write — the text you typed is kept first, as
a new owner-only `<id>-<UTC stamp>.md` under `$XDG_STATE_HOME/nebula/edits/`
(default `~/.local/state/nebula/edits/`), outside the corpus, and the refusal
(and its `--json` `error`) ends `your edited text is kept at <path>`.

Look at the node with `neb show <id>`, then `neb edit <id>` again and carry the
text over. Frontmatter is never exposed. If the node already has a `## Notes`
section, removing, moving or changing it is refused; append reasoning with
`note` instead.

A body can hold more than one such section, because `note` opens a fresh one
rather than reach back into a section other prose has closed, and every one of
them is protected. A body left as it was (outer whitespace aside) is not
written: `updated` stays, nothing is committed, stderr says `no change; <id> not
written`, and `--json` still prints the node.

`edit` accepts `--by` and requires it under an Orbit run, although the body has
no per-field author in the current schema. With neither environment variable
set, `edit` refuses and names both variables. Unknown nodes are refused
(`NoSuchNode`). `--json` is the same `NodeView` as `show --json`: `notes` is a
list of `{at, text, by}`, oldest first, and `[]` when there are none.

The write verbs return the core value they changed. `new` returns `{doc,
path, near}`, `near` always `[]`; `sharpen` (including `--confirm`) and `tag` return a `Doc`; `status`
returns `{doc, from}`; and `link` returns an array because `contradicts`
changes both nodes.

```json
// neb new "Tags beat domains" --tag design --json
{
  "doc": {
    "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "title_by": "human",
              "status": "seed", "created": "2026-09-21", "updated": "2026-09-21",
              "kill": null, "kill_by": null, "tags": ["design"], "edges": [], "references": [],
              "closed": null, "origin": null },
    "body": ""
  },
  "path": "/Users/you/.nebula/nodes/tags-beat-domains.md",
  "near": []
}
```

```json
// neb sharpen tags-beat-domains --kill "a corpus of 50 nodes needs a query tags cannot answer" --json
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "title_by": "human",
            "status": "hypothesis", "created": "2026-09-21", "updated": "2026-09-21",
            "kill": "a corpus of 50 nodes needs a query tags cannot answer", "kill_by": "human",
            "tags": ["design"], "edges": [], "references": [], "closed": null, "origin": null },
  "body": ""
}
```

```json
// neb status a-single-taxonomy abandoned --why "tags preserve the useful cross-cuts" --json
{
  "doc": {
    "node": { "id": "a-single-taxonomy", "title": "A single taxonomy", "title_by": "human",
              "status": "abandoned", "created": "2026-09-21", "updated": "2026-09-21",
              "kill": null, "kill_by": null, "tags": [], "edges": [], "references": [],
              "closed": { "why": "tags preserve the useful cross-cuts", "at": "2026-09-21" },
              "origin": null },
    "body": ""
  },
  "from": "seed"
}
```

```json
// neb link tags-beat-domains contradicts a-single-taxonomy --json
[
  { "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "title_by": "human",
              "status": "hypothesis", "created": "2026-09-21", "updated": "2026-09-21",
              "kill": "a corpus of 50 nodes needs a query tags cannot answer", "kill_by": "human",
              "tags": ["design"],
              "edges": [{ "type": "contradicts", "to": "a-single-taxonomy", "by": "human" }],
              "references": [], "closed": null, "origin": null }, "body": "" },
  { "node": { "id": "a-single-taxonomy", "title": "A single taxonomy", "title_by": "human",
              "status": "seed", "created": "2026-09-21", "updated": "2026-09-21",
              "kill": null, "kill_by": null, "tags": [],
              "edges": [{ "type": "contradicts", "to": "tags-beat-domains", "by": "human" }],
              "references": [], "closed": null, "origin": null }, "body": "" }
]
```

```json
// neb tag tags-beat-domains --add corpus --json
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "title_by": "human",
            "status": "hypothesis", "created": "2026-09-21", "updated": "2026-09-21",
            "kill": "a corpus of 50 nodes needs a query tags cannot answer", "kill_by": "human",
            "tags": ["design", "corpus"],
            "edges": [{ "type": "contradicts", "to": "a-single-taxonomy", "by": "human" }],
            "references": [], "closed": null, "origin": null },
  "body": ""
}
```

```json
// neb --json note tags-beat-domains "folksonomy is the argument, not a taxonomy with extra steps"
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis", … },
  "body": "tags beat domains because a category you must pick is a decision you skip\n\n## Notes\n\n- 2026-09-21: folksonomy is the argument, not a taxonomy with extra steps",
  "notes": [
    { "at": "2026-09-21", "text": "folksonomy is the argument, not a taxonomy with extra steps",
      "by": "human" }
  ]
}
```

```json
// neb tag list --json
[ { "tag": "corpus", "count": 1 }, { "tag": "design", "count": 3 } ]
```
