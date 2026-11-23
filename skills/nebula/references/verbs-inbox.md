# Verbs: inbox

Read this for `capture`, `inbox`, `promote`, `drop` and `triage`.

Contents:

- [Inbox](#inbox)

## Inbox

| verb | does | flags |
|---|---|---|
| `neb capture <TEXT\|->...` | append a thought as one inbox line; prints the entry id, then the three nearest nodes; works on a corpus that does not exist yet, and then says so on stderr (`note: created a new corpus at <absolute path>`); a thought already waiting in the inbox is still captured, and stderr says `note: same as <id>, still waiting` | `--quiet`/`-q` (before or after the text): the entry id alone |
| `neb inbox` | live entries (not promoted, not dropped), oldest first | `--limit <N>` |
| `neb promote <ENTRY>` | inbox entry → seed node; without `--parent`, prints the three nearest nodes and proceeds as a root | `--title`, `--body <TEXT\|->`, `--parent <ID>`×, `--tag <TAG>`×, `--id <SLUG>`, `--by <LABEL>`, `--task`, `--run`, `--quiet`/`-q`: the node id alone |
| `neb drop <ENTRY>` | strike an entry through; never deleted | — |
| `neb triage` | walk the waiting entries oldest first and decide each with one key, through `promote` and `drop` | `--by <LABEL>` |

```json
// neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16:42+02:00", "text": "nebula review as a weekly orbit routine" } ]
```

`at` is RFC 3339 to the second, with the offset the capture was stamped at,
and `Z` when the local offset could not be read. An entry captured before
0.2.0 has a stamp with no offset (`2026-09-12T18:16` in the inbox file); it
lists as local time with the offset this machine has for that instant, and
its line is never rewritten.

`triage` is for a human at a terminal. It snapshots the inbox, then shows
each entry in turn, oldest capture first, with its position, stamp and age
and its `near` candidates numbered from 1. One line decides it:

| key | does | the same as |
|---|---|---|
| `p` | promote as a root | `neb promote <entry>` |
| `1`–`3` | promote under that numbered candidate | `neb promote <entry> --parent <id>` |
| `t <title>`, or `t` then the title on the next line | title the promotion that follows; a blank title goes back to the captured text | `--title` |
| `d` | drop | `neb drop <entry>` |
| `s` | leave it waiting and move on | — |
| `q` | stop; everything undecided stays waiting | — |
| `?` | list the keys | — |

Nothing is linked unless a number is chosen. Each promote or drop is the single
verb, with its refusals and `--by`, and with `commit` on it makes the single
verb's commit — `neb promote <entry> <id>`, `neb drop <entry>` — one per
decision; `--no-commit` waives them for the session. The session ends with
`promoted N, dropped N, skipped N; N still waiting`.

On a terminal a refused key or decision is reported and the same entry is asked
about again.

With standard input piped, keys are read one per line with no prompt, and the
first refusal ends the session with exit 1, because the lines after it were
written for an entry that did not move; end of input stops as `q` does, except
while a title is waiting to be used (after `t`, before the `p` or number it was
for), which is a refusal, exit 1, naming the title and the `neb promote` that
would use it.

An agent does not use `triage`: run `inbox`, `near`, `promote` and `drop` with
`--json`, which is what it is made of.

`capture`, `note` and `near` take remaining words as the thought, so a flag
after the text is still a flag: `neb capture "an idea" --quiet` quiets and
stores `an idea`; `neb note <id> "a thought" --no-commit` skips the commit
and stores `a thought`. A thought that itself contains a token starting with
`-` is one quoted argument, or sits after `--`
(`neb capture -- --quiet is the idea`).

A lone `-` reads the thought from standard input, up to 64 KiB (65536
bytes); more is refused as `input_too_large` before the corpus is opened for
writing, so nothing is captured. Text over several lines,
piped or quoted, is joined onto one line: each line break becomes a single
space and blank lines vanish, so `printf 'a\nb\n' | neb capture -` stores
`a b`. Only whitespace-only text is refused (`nothing to capture`, code
`empty_capture`), before a missing corpus is created for it.

`capture` prints the entry id on its own line, then — when any node shares a
word with the text — a `near:` block of up to three lines, `<band> <status> <id>
<title>`, best first (bands under [`near`](verbs-query.md#query)). `promote` prints `<id>
<path>` and, when no `--parent` was given, the same block for the title plus
captured text.

Both are the [`near`](verbs-query.md#query) query run for you: a suggestion for the triage
step, never an edge. `promote` writes the node as a root whatever it lists, and
with `--parent` lists nothing, since that decision is made. `--quiet` prints the
id alone; `promote --json` carries the path. No block at all means nothing in
the corpus shares a word with it — promote as a root or drop.

The suggestions are read after the capture is written and committed, with the
write lock released. They are a side channel: a node file that will not parse
costs the suggestions — `warning: suggestions unavailable: …` on stderr, naming
the file — and never the capture, which still exits 0 with its id (or, under
`--json`, its entry with `near: []`) on stdout. `--quiet` does not read `nodes/`
at all.

The duplicate check compares the text with every entry still waiting, after
folding case and collapsing whitespace, and names the earliest match. Settled
entries do not count, and a reworded thought is not caught. The notice
is on stderr in both modes, so the id, or the `--json` payload, is unchanged.

`promote` and `drop` on an entry that was already settled say how it was,
from its struck-through line: `` `<id>` was already promoted to `<node>` ``
or `` `<id>` was already dropped `` (`InboxEntrySettled`). An id nothing
recorded is still `no open inbox entry` (`NoSuchInboxEntry`).

`promote --body <TEXT>` keeps the captured line as the first paragraph and
appends `TEXT` after it. With `--body -`, the appended prose is read from
standard input, up to 1 MiB (1048576 bytes); more is refused as
`input_too_large` before anything is written.

```json
// neb capture --json "domains drift when a field is required"
{
  "entry": { "id": "f1ca", "at": "2026-09-21T01:57:08+02:00", "text": "domains drift when a field is required" },
  "near": [
    { "id": "required-categorical-fields-drift", "title": "Required categorical fields drift",
      "status": "seed", "tags": ["design"], "score": 0.555, "band": "strong", "linked": null },
    { "id": "tags-beat-domains", "title": "Tags beat domains",
      "status": "hypothesis", "tags": ["design", "corpus"], "score": 0.177, "band": "some", "linked": null }
  ]
}

// neb promote --json f1ca --title "Required fields drift domains"
{
  "doc": {
    "node": { "id": "required-fields-drift-domains", "title": "Required fields drift domains",
              "title_by": "human", "status": "seed", "created": "2026-09-21", "updated": "2026-09-21",
              "kill": null, "kill_by": null, "tags": [], "edges": [], "references": [],
              "closed": null, "origin": null },
    "body": "domains drift when a field is required"
  },
  "path": "/Users/you/.nebula/nodes/required-fields-drift-domains.md",
  "near": [
    { "id": "required-categorical-fields-drift", "title": "Required categorical fields drift",
      "status": "seed", "tags": ["design"], "score": 0.619, "band": "strong", "linked": null },
    { "id": "tags-beat-domains", "title": "Tags beat domains",
      "status": "hypothesis", "tags": ["design", "corpus"], "score": 0.114, "band": "some", "linked": null }
  ]
}
```

```json
// neb drop 5572 --json
{ "id": "5572", "at": "2026-09-21T02:56:31+02:00", "text": "duplicate thought" }
```

`near` is `[]` in both when it would be empty, so `--quiet`, a `--parent`,
and a thought unlike anything in the corpus all read the same way:
`"near": []`.
