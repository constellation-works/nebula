# Verbs

Every verb takes `--root <DIR>` and `--json`. Ids are slugs of the title
(`"Tags beat domains"` → `tags-beat-domains`); inbox ids are four hex chars.
The `--json` excerpts below are real output from a three-node fixture corpus.

## Corpus

| verb | does | flags |
|---|---|---|
| `neb init [PATH]` | create an empty corpus | — |
| `neb check` | run the ten invariants; exit non-zero on any error | — |
| `neb migrate` | v1 → v2 in place; idempotent; refuses on a dirty git tree | — |

```json
// neb check --json          (findings[] carries {rule, level, node, message})
{ "findings": [], "nodes": 3 }
```

## Inbox

| verb | does | flags |
|---|---|---|
| `neb capture <TEXT>...` | append a thought; prints the entry id; works on a corpus that does not exist yet | — |
| `neb inbox` | live entries (not promoted, not dropped) | — |
| `neb promote <ENTRY>` | inbox entry → seed node | `--title`, `--parent <ID>`×, `--tag <TAG>`×, `--task`, `--run` |
| `neb drop <ENTRY>` | strike an entry through; never deleted | — |

```json
// neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16", "text": "nebula review as a weekly orbit routine" } ]
```

`promote` prints `<id> <path>`; `capture` prints the entry id alone.

## Nodes

| verb | does | flags |
|---|---|---|
| `neb new <TITLE>` | create a node directly | `--parent <ID>`×, `--kill`, `--tag <TAG>`×, `--task`, `--run` |
| `neb sharpen <NODE> --kill <KILL>` | seed → hypothesis by naming the falsifier | — |
| `neb status <NODE> <STATUS>` | `seed`, `hypothesis`, `refuted`, `abandoned`, with guards | `--why` (required for refuted, optional for abandoned) |
| `neb link <FROM> <KIND> <TO>` | `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts` | — |
| `neb tag <NODE>` | edit tags; normalised to lowercase kebab-case | `--add <TAG>`×, `--remove <TAG>`× |
| `neb tag list` | every tag with its node count | — |

`new --kill "..."` starts the node as a hypothesis; without it, a seed.
`contradicts` is written on both nodes. `link` prints `<from> <kind> <to>`.

```json
// neb tag list --json
[ { "tag": "corpus", "count": 1 }, { "tag": "design", "count": 3 } ]
```

## References

| verb | does | flags |
|---|---|---|
| `neb cite <NODE> --uri <URI>` | attach context | `--kind` (paper, study, article, note, discussion, book, dataset, thread, other), `--title`, `--note`, `--task`, `--run` |

A local `--uri` is resolved relative to `nodes/` and refused if it does not
exist. Always pass `--note`: it is the only field that matters in a year.

## Query

| verb | does | flags |
|---|---|---|
| `neb show <NODE>` | one node in full, plus its body | — |
| `neb list` | every node | `--status <S>`, `--tag <TAG>`× (every tag must match) |
| `neb trace <NODE>` | ancestry, nearest first, each node once | `--down` for descendants |
| `neb impact <NODE>` | descendants plus `contradicts` neighbours | — |
| `neb graph` | the whole corpus as `{nodes, edges}` | `--json` only; without it, a hint and exit 2 |

```json
// neb show tags-beat-domains --json
{
  "node": {
    "id": "tags-beat-domains",
    "title": "Tags beat domains",
    "status": "hypothesis",
    "created": "2026-09-12",
    "updated": "2026-09-12",
    "kill": "a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer",
    "tags": ["design", "corpus"],
    "edges": [
      { "type": "derives-from", "to": "required-categorical-fields-drift" },
      { "type": "contradicts", "to": "a-single-global-taxonomy" }
    ],
    "references": [
      { "id": "r1", "kind": "article", "uri": "https://example.org/folksonomy",
        "title": "Folksonomies", "note": "the drift argument, made for web tagging",
        "added": "2026-09-12" }
    ]
  },
  "body": "tags beat domains because a category you must pick is a decision you skip"
}
```

`neb list --json` is an array of the same `node` objects (no `body`). A closed
node carries `"closed": { "why": "...", "at": "2026-09-12" }`; optional fields
(`kill`, `closed`, `origin`, empty lists) are omitted.

```json
// neb trace tags-beat-domains --json
[
  { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
    "parents": ["required-categorical-fields-drift"] },
  { "id": "required-categorical-fields-drift", "title": "Required categorical fields drift",
    "status": "seed", "parents": [] }
]

// neb impact required-categorical-fields-drift --json     (via: descends | contradicts)
[ { "id": "tags-beat-domains", "via": "descends" } ]

// neb graph --json
{
  "nodes": [
    { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
      "tags": ["design", "corpus"], "created": "2026-09-12", "updated": "2026-09-12" }
  ],
  "edges": [
    { "from": "tags-beat-domains", "type": "derives-from", "to": "required-categorical-fields-drift" },
    { "from": "tags-beat-domains", "type": "contradicts", "to": "a-single-global-taxonomy" },
    { "from": "a-single-global-taxonomy", "type": "contradicts", "to": "tags-beat-domains" }
  ]
}
```

## Maintenance

| verb | does | flags |
|---|---|---|
| `neb open` | hypotheses with no references, seeds untouched ≥ 90 days, inbox entries waiting ≥ 14 days | `--tag <TAG>`× |
| `neb review` | the weekly report: stale hypotheses (≥ 30 days), untouched seeds (≥ 90), nodes with no references, inbox waiting ≥ 14 | `--since <DAYS>`, `--out <FILE>` |

Both are read-only by the spec's hard rule.

```json
// neb review --json     (rule: stale-hypothesis | untouched-seed | no-references | inbox-waiting)
[
  { "rule": "no-references", "id": "required-categorical-fields-drift",
    "title": "Required categorical fields drift", "reason": "no references attached" }
]
```

`neb review` without `--json` prints four `##` sections in that order, each
`_none_` or a `- \`id\` Title — reason` list; `--out review.md` writes it to a
file. `neb open --json` is an array of `{id, title, status, reason}`.
