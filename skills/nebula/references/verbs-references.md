# Verbs: references and hand-off

Read this for `cite`, Observatory records and `handoff`.

Contents:

- [References](#references)
  - [Observatory records](#observatory-records)
  - [Hand-off](#hand-off)

## References

| verb | does | flags |
|---|---|---|
| `neb cite <NODE> [--uri <URI>]` | attach context | `--kind` (paper, study, article, note, discussion, book, dataset, thread, observatory, other), `--title`, `--note`, `--by <LABEL>`, `--task`, `--run` |
| `neb handoff <NODE> <RECORD>` | hand the node off to an Observatory record: cite it and close the node, in one write | `--note`, `--by <LABEL>`, `--task`, `--run` |

`--kind` is lowercased before it is checked (`--kind Paper` stores `paper`);
anything still outside the list is refused. `--uri` may be omitted only with
`--kind discussion`; every other kind requires
it. A local URI is resolved relative to `nodes/` and refused if it does not
exist; an absolute path or `file:` URI is refused even when it does, because
it resolves on this machine only. Always pass `--note`: it is the only field
that matters in a year.

Reference counters use a fixed unsigned 64-bit range on every machine.
`reference_ids_exhausted` means an existing numeric reference suffix is at
or beyond `18446744073709551615`; `cite` and `handoff` refuse before changing
the node. The references remain readable. Do not remove or renumber them
to make room, since that can reuse an ID; continue the idea in a new node.

`cite --json` returns the changed `doc`, the newly allocated reference id, and
`observatory`, which is `null` for every kind but `observatory` (see below):

```json
// neb cite tags-beat-domains --kind article --uri https://example.org/folksonomy --note "the drift argument" --json
{
  "doc": {
    "node": {
      "id": "tags-beat-domains", "title": "Tags beat domains", "title_by": "human",
      "status": "hypothesis", "created": "2026-09-21", "updated": "2026-09-21",
      "kill": "a corpus of 50 nodes needs a query tags cannot answer", "kill_by": "human",
      "tags": ["design", "corpus"],
      "edges": [{ "type": "contradicts", "to": "a-single-taxonomy", "by": "human" }],
      "references": [
        { "id": "r1", "kind": "article", "uri": "https://example.org/folksonomy",
          "title": null, "note": "the drift argument", "added": "2026-09-21",
          "by": "human", "origin": null }
      ],
      "closed": null, "origin": null
    },
    "body": ""
  },
  "reference": "r1",
  "observatory": null
}
```

### Observatory records

`--kind observatory` links a node to an Observatory record, and its `--uri` is
the bare record id — `Q<nnn>`, `H<nnn>`, `T<nnn>` or `R<nnn>` — not a path.
That is what makes the citation portable: nothing machine-specific reaches the
corpus. Case is normalised up (`q<nnn>` stores `Q<nnn>`), and anything that is
not one of `Q`,
`H`, `T`, `R` followed by digits is a typed refusal at `cite`.

Where the record is comes from the machine, not the reference or the corpus:
`$OBSERVATORY_ROOT`, else this machine's setting (`neb config observatory-root
<DIR>`), else the legacy `observatory_root` key in `config.yaml`. The id is
matched by prefix inside the directory its
letter names — `questions/`, `hypotheses/`, `theories/`, `research/` — so
`Q<nnn>` finds `questions/Q<nnn>-<slug>.md` and `R<nnn>` finds the
`research/R<nnn>-<slug>/` directory.

`check` warns, and never errors, when the root is unset or the id does not
resolve: the citation is still true, and the machine is merely missing or behind
the checkout. `show` prints the resolved path under the reference, and `show
--json` carries an `observatory` array of `{reference, record, path}` (`path` is
`null` when it does not resolve).

`cite --kind observatory` prints the path it resolved to, and `cite --json`
carries the same `{reference, record, path}` for the new reference as
`observatory`, `path` `null` with no root or no match.

```json
// neb cite proper-time-is-a-count --kind observatory --uri Q<nnn> --note "the question this became"
// then: neb show proper-time-is-a-count --json
{
  "node": {
    …,
    "references": [
      { "id": "r1", "kind": "observatory", "uri": "Q<nnn>", "title": null,
        "note": "the question this became", "added": "2026-09-21", "by": "human",
        "origin": null }
    ],
    …
  },
  "observatory": [
    { "reference": "r1", "record": "Q<nnn>",
      "path": "/Users/you/workspace/observatory/questions/Q<nnn>-<slug>.md" }
  ]
}
```

### Hand-off

`neb handoff <NODE> <RECORD> --note "why"` is the one-step form of "this idea
became Observatory record `RECORD`". In one write it adds an `observatory`
reference to the record, with the note, and moves the node to `abandoned`
with `closed.why: handed off to <RECORD>`. The record id follows the same
rules as `cite --kind observatory`: a bare id, case normalised up.

It refuses, writing nothing: an unknown node (`NoSuchNode`); a node already
refuted or abandoned (`AlreadyClosed`), including one handed off before; a
record id of the wrong shape (`InvalidObservatoryId`); and, when this machine
has an observatory root, a record that does not resolve under it
(`UnresolvedObservatoryRecord`).

With no root set, the id is accepted and the verb prints the same `No
observatory root set` line `cite` does, on stderr. The record's path, when it
resolves, is part of the result on stdout; a missing `--note` is nudged on
stderr.

```
// neb handoff scarcity-wake H<nnn> --note "the hypothesis this became"
scarcity-wake seed -> abandoned, handed off to H<nnn> (r1)
/Users/you/workspace/observatory/hypotheses/H<nnn>-<slug>.md
```

`--json` returns the node as written, the new reference's id, the record as
stored, the status it left, and where the record is, as `observatory`
(`{reference, record, path}`, `path` `null` with no root):

```json
// neb handoff scarcity-wake H<nnn> --note "the hypothesis this became" --json
{
  "doc": {
    "node": {
      "id": "scarcity-wake", "title": "Scarcity wake", "title_by": "human",
      "status": "abandoned", "created": "2026-09-26", "updated": "2026-09-26",
      "kill": null, "kill_by": null, "tags": [], "edges": [],
      "references": [
        { "id": "r1", "kind": "observatory", "uri": "H<nnn>", "title": null,
          "note": "the hypothesis this became", "added": "2026-09-26", "by": "human",
          "origin": null }
      ],
      "closed": { "why": "handed off to H<nnn>", "at": "2026-09-26" },
      "origin": null
    },
    "body": ""
  },
  "reference": "r1",
  "record": "H<nnn>",
  "from": "seed",
  "observatory": { "reference": "r1", "record": "H<nnn>",
                   "path": "/Users/you/workspace/observatory/hypotheses/H<nnn>-<slug>.md" }
}
```

A node reads as handed off when it is `abandoned`, its `closed.why` is
exactly `handed off to <RECORD>`, and it carries an `observatory` reference
to that record, so the two-verb form reads the same. `show` then prints the
record's location under its `closed:` line, `trace` appends
`handed off to <RECORD>` to its line, and both `--json` forms carry
`"handed_off_to": "<RECORD>"`, and `null` for every other node.
