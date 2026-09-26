# Verbs

Every corpus verb below takes `--root <DIR>` and `--json`, and those flags may
appear anywhere on the line — before the verb, after it, or after the free
text of `capture`, `note` and `near`. Every verb that writes also takes
`--no-commit`, after the verb (or, as an older spelling, before it); a
read-only verb such as `show`, `list` or `trace` refuses it as an unknown
argument. Under `--json`, each emits
one JSON value on stdout. Two commands are the exceptions: `completions`
always emits a shell script, and `triage`, which takes its decisions from a
person one key at a time, refuses `--json` (`Interactive`) and names the
scriptable verbs instead. Ids are slugs of the title
(`"Tags beat domains"` → `tags-beat-domains`); inbox ids are four hex chars. A
slug over 60 characters is cut at the last `-` at or before the limit, never
mid-word. `promote` without `--title` or `--id` keeps the captured sentence
as the title but, for a capture over five words, mints the id from its first
five words after dropping stop-words (`gravity might be a scarcity gradient
in some shared resource` → `gravity-scarcity-gradient-shared-resource`); if
that id belongs to a different idea it adds one more such word at a time, then
falls back to the full slug. A node already titled with the same text is
refused with `NodeExists` naming it, never duplicated, as is a capture whose
every candidate is taken.
`promote` and `new` take `--id <SLUG>` to choose the id explicitly
instead — useful for a long title, since ids are frozen by invariant once
written. An explicit id follows the same slug rules (lowercase words joined
by single dashes, 60 characters or fewer) and is refused, as a typed error,
if it breaks those rules or collides with an existing node.
The `--json` excerpts below are real output from a three-node fixture corpus.

## Refusals under `--json`

Under `--json` a refusal is data too: one JSON object on one line, the last
line on **stderr**. Stdout stays the payload's alone. Match on `code`, never
on `error`:

```json
// neb show nope --json      (stderr; stdout is empty; exit 1)
{"code":"no_such_node","error":"no node `nope`","hint":"List what exists with:  neb list"}
```

| field | type | what it is |
|---|---|---|
| `error` | string | What is wrong, in the words the text output uses before its hint. |
| `code` | string | The refusal's stable `snake_case` name. For a core refusal it is the `nebula-core` variant's name in `snake_case`: `no_such_node`, `cycle`, `self_loop`, `needs_kill`, `refuted_needs_why`, `refuted_cannot_reopen`, `seed_with_kill`, `unknown_reference_kind`, `unresolved_uri`, `absolute_uri`, `schema_mismatch`, `staged_elsewhere`, `corpus_ignored`, `locked`, and the rest in [invariants.md](invariants.md#what-each-refusal-means-and-what-to-do). The CLI adds its own: `usage` (arguments that parse but ask for nothing, such as an empty `capture`), `editor_not_configured`, `editor_invalid_command`, `editor_start`, `editor_unsuccessful`, `notes_changed`, `triage_key` (a key `triage` does not know), `json` and `io_at`. |
| `hint` | string or `null` | What to do about it, as the text output words it: often a command to run, such as `neb sharpen <id> --kill "..."`. `null` when the CLI has nothing to add. |

Key order is not significant. A new refusal arrives with its own `code` and
the same three fields; new fields may be added, and existing ones will not
change meaning. `SchemaMismatch` covers both directions: the hint says
whether to `neb migrate` (the corpus is older) or upgrade `neb` (newer).

Exit codes, with or without `--json`:

| code | means | stdout | stderr under `--json` |
|---|---|---|---|
| 0 | success | the payload | empty, bar a `warning:` line from `init` or `capture` |
| 1 | a refusal | empty, **except** when the write landed and its commit was refused (`StagedElsewhere`, `CorpusIgnored`, `Git`): then it holds the write's payload | the envelope |
| 1 | `check` found an `error`-level finding | the report | empty |
| 2 | clap rejected the command line: unknown flag, missing argument, bad value | empty | clap's prose, **not** JSON: it is raised before `neb` knows `--json` was asked for |

`neb graph` with neither `--json` nor `--mermaid` also exits 2, with its hint
on stdout; under `--json` that cannot happen. Without `--json`, every refusal
prints exactly what it printed before the envelope existed: `error:`, the
message, and the hint after a blank line.

## Corpus

| verb | does | flags |
|---|---|---|
| `neb init [PATH]` | create an empty corpus without changing the machine default | `--set-root`, `--force` (requires `--set-root`) |
| `neb check` | run the rules in [invariants.md](invariants.md); exit non-zero on any error | — |
| `neb migrate` | v1 → v2 in place; idempotent; refuses on a dirty git tree | — |
| `neb config observatory-root [DIR]` | read or set where the Observatory checkout is on this machine | `--drop-legacy` |
| `neb config commit [on\|off]` | read or set whether each write is committed to the corpus's git repository | — |

`config commit` rewrites `config.yaml` whole when the setting changes; `init`,
`migrate`, and `capture` when it initializes a corpus can write it too. The
file stays machine-written and is never hand-edited.

`config observatory-root DIR` writes this machine's
`~/.config/nebula/observatory-root` and nothing in the corpus, so it commits
nothing; `DIR` must be absolute. Without `DIR` it prints the effective root and
which setting supplied it: `$OBSERVATORY_ROOT`, else this machine's setting,
else the legacy `observatory_root` key an older `neb` wrote into `config.yaml`,
else nothing. `legacy` names that key whenever the file still carries it, in
force or not. `--drop-legacy` removes it from `config.yaml` (a corpus write,
committed as `neb config observatory-root` when `commit` is on); run it once
every machine that shares the corpus has its own setting.

```json
// neb init /Users/you/.nebula --json
{ "root": "/Users/you/.nebula" }
```

Plain `init` never writes `~/.config/nebula/root`. For a primary non-default
corpus, pass `--set-root`; it refuses to replace a setting that names another
corpus unless `--force` is also explicit. Scratch corpora use `--root` and
never `--set-root`.

```json
// neb config observatory-root --json     (source: env | machine | config | unset)
{ "root": "/Users/you/workspace/observatory", "source": "machine", "legacy": null }
```

```json
// neb config commit --json
{ "enabled": true }
```

With `commit` on (off by default) and the corpus root inside a git work
tree, every mutating verb ends with one commit of `nodes/`, `inbox/` and
`config.yaml`, named `neb <verb> <ids>`, and prints `committed <hash>` in
text mode (nothing extra in `--json`). It never pushes and never touches a
path outside the corpus root. If something outside the corpus is already
staged, the verb exits non-zero with `staged changes outside the corpus`
**after** its write has landed — the write is never rolled back because of
git; report it rather than retry the write. `--no-commit` skips the commit
for one invocation.

```json
// neb check --json          (findings[] carries {rule, level, node, message})
{ "findings": [], "nodes": 3 }
```

## Inbox

| verb | does | flags |
|---|---|---|
| `neb capture <TEXT\|->...` | append a thought as one inbox line; prints the entry id, then the three nearest nodes; works on a corpus that does not exist yet, and then says so on stderr (`note: created a new corpus at <absolute path>`); a thought already waiting in the inbox is still captured, and stderr says `note: same as <id>, still waiting` | `--quiet`/`-q` (before or after the text) |
| `neb inbox` | live entries (not promoted, not dropped), oldest first | `--limit <N>` |
| `neb promote <ENTRY>` | inbox entry → seed node; without `--parent`, prints the three nearest nodes and proceeds as a root | `--title`, `--body <TEXT\|->`, `--parent <ID>`×, `--tag <TAG>`×, `--id <SLUG>`, `--by <LABEL>`, `--task`, `--run`, `--quiet`/`-q` |
| `neb drop <ENTRY>` | strike an entry through; never deleted | — |
| `neb triage` | walk the waiting entries oldest first and decide each with one key, through `promote` and `drop` | `--by <LABEL>` |

```json
// neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16", "text": "nebula review as a weekly orbit routine" } ]
```

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

Nothing is linked unless a number is chosen. Each promote or drop is the
single verb, with its refusals and `--by`, and with `commit` on it makes the
single verb's commit — `neb promote <entry> <id>`, `neb drop <entry>` — one
per decision; `--no-commit` waives them for the session. The session ends
with `promoted N, dropped N, skipped N; N still waiting`. On a terminal a
refused key or decision is reported and the same entry is asked about again.
With standard input piped, keys are read one per line with no prompt, and the
first refusal ends the session with exit 1, because the lines after it were
written for an entry that did not move; end of input stops as `q` does. An
agent does not use `triage`: run `inbox`, `near`, `promote` and `drop` with
`--json`, which is what it is made of.

`capture`, `note` and `near` take remaining words as the thought, so a flag
after the text is still a flag: `neb capture "an idea" --quiet` quiets and
stores `an idea`; `neb note <id> "a thought" --no-commit` skips the commit
and stores `a thought`. A thought that itself contains a token starting with
`-` is one quoted argument, or sits after `--`
(`neb capture -- --quiet is the idea`).

A lone `-` reads the thought from standard input. Text over several lines,
piped or quoted, is joined onto one line: each line break becomes a single
space and blank lines vanish, so `printf 'a\nb\n' | neb capture -` stores
`a b`. Only whitespace-only text is refused (`nothing to capture`).

`capture` prints the entry id on its own line, then — when any node shares a
word with the text — a `near:` block of up to three lines, `<band> <status>
<id> <title>`, best first (bands under [`near`](#query)). `promote` prints `<id> <path>` and, when no
`--parent` was given, the same block for the title plus captured text. Both
are the [`near`](#query) query run for you: a suggestion for the triage
step, never an edge. `promote` writes the node as a root whatever it lists,
and with `--parent` lists nothing, since that decision is made. `--quiet`
prints the id (and path) alone. No block at all means nothing in the corpus
shares a word with it — promote as a root or drop. The id is printed before
`nodes/` is read, so a node file that will not parse fails the suggestions
(non-zero, after the id, like a refused commit) and never the capture;
`--quiet` does not read `nodes/` at all.

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
standard input.

```json
// neb capture --json "domains drift when a field is required"
{
  "entry": { "id": "f1ca", "at": "2026-09-21T01:57", "text": "domains drift when a field is required" },
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
              "status": "seed", "created": "2026-09-21", "updated": "2026-09-21" },
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
{ "id": "5572", "at": "2026-09-21T02:56", "text": "duplicate thought" }
```

`near` is omitted from both when it would be empty, so `--quiet`, a
`--parent`, and a thought unlike anything in the corpus all read the same
way: no `near` key.

## Nodes

| verb | does | flags |
|---|---|---|
| `neb new <TITLE>` | create a node directly | `--body <TEXT\|->`, `--parent <ID>`×, `--reopens <ID>`, `--contradicts <ID>`×, `--kill`, `--tag <TAG>`×, `--id <SLUG>`, `--by <LABEL>`, `--task`, `--run` |
| `neb edit <NODE>` | open the body, without frontmatter, in `$VISUAL` or `$EDITOR` | `--by <LABEL>` |
| `neb sharpen <NODE> --kill <KILL>` | seed → hypothesis by naming the falsifier | `--by <LABEL>`, or `--confirm` instead of `--kill` |
| `neb status <NODE> <STATUS>` | `seed`, `hypothesis`, `refuted`, `abandoned`, with guards | `--why` (required for refuted, optional for abandoned) |
| `neb link <FROM> <KIND> <TO>` | `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts` | `--by <LABEL>` |
| `neb tag <NODE>` | edit tags; normalised to lowercase kebab-case | `--add <TAG>`×, `--remove <TAG>`× |
| `neb tag list` | every tag with its node count | — |
| `neb note [--by <LABEL>] <NODE> <TEXT>...` | append a dated paragraph of reasoning to the body | `--by <LABEL>` (before or after the text) |

`new`, `promote` and `tag --add` print `note: tag physic is close to physics
(2 nodes)` on stderr, in every output mode, when a tag they introduce to the
corpus differs from one in use only by case or a trailing `s`. The write has
already succeeded. Reuse the existing tag unless the difference is deliberate:
`neb tag <NODE> --remove physic --add physics`.

`new --body <TEXT>` sets the prose at creation; `--body -` reads it from
standard input. `new --kill "..."` starts the node as a hypothesis; without
it, a seed.
`new --reopens <ID>` revives a refuted node as a new one with a single
`reopens` edge; that edge is genealogy, so naming the same node as `--parent`
too is refused. `new --contradicts <ID>` writes the edge on both nodes, as
`link` does. Every edge is checked before anything is written: a missing
node, the same edge twice, or a genealogy loop refuses the whole `new`.
`sharpen --kill` refuses to replace a kill condition already present on an open
node and prints the existing falsifier. A changed falsifier is a changed idea:
create a new node and relate it to the old one instead of overwriting the
record. On a refuted node, the stricter final-verdict refusal still applies.
`sharpen --confirm` takes no text: it adopts the kill condition already on the
node as the human's own, changing nothing else and appending nothing.
`status <id> seed` refuses a node that names a kill condition
(`SeedWithKill`), because the kill would stay and leave a seed carrying one;
reopen it with `status <id> hypothesis` instead, including from `abandoned`.
`contradicts` is written on both nodes. `link` prints `<from> <kind> <to>`.
`note` creates a `## Notes` section at the end of the body if needed, then
appends `- YYYY-MM-DD: <text>`. Repeated notes accumulate in order; earlier
body text, status, edges and tags are left as they are. `updated` is bumped.
`edit` writes only the prose body to a temporary file, preferring `$VISUAL`
over `$EDITOR`, then saves the result after the editor exits successfully.
Frontmatter is never exposed. If the node already has a `## Notes` section,
removing, moving or changing it is refused; append reasoning with `note`
instead. A body can hold more than one such section, because `note` opens a
fresh one rather than reach back into a section other prose has closed, and
every one of them is protected. `--by` is accepted and validated, but records nothing because the
body has no per-field author in the current schema. With neither environment
variable set, `edit` refuses and names both variables.
Unknown nodes are refused (`NoSuchNode`). `--json` is the same `NodeView` as
`show --json`: `notes` is a list of `{at, text, by}`, oldest first, omitted
when empty.

The write verbs return the core value they changed. `new` returns `{doc,
path}`; `sharpen` (including `--confirm`) and `tag` return a `Doc`; `status`
returns `{doc, from}`; and `link` returns an array because `contradicts`
changes both nodes.

```json
// neb new "Tags beat domains" --tag design --json
{
  "doc": {
    "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "seed",
              "created": "2026-09-21", "updated": "2026-09-21", "tags": ["design"] },
    "body": ""
  },
  "path": "/Users/you/.nebula/nodes/tags-beat-domains.md"
}
```

```json
// neb sharpen tags-beat-domains --kill "a corpus of 50 nodes needs a query tags cannot answer" --json
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
            "created": "2026-09-21", "updated": "2026-09-21",
            "kill": "a corpus of 50 nodes needs a query tags cannot answer", "tags": ["design"] },
  "body": ""
}
```

```json
// neb status a-single-taxonomy abandoned --why "tags preserve the useful cross-cuts" --json
{
  "doc": {
    "node": { "id": "a-single-taxonomy", "title": "A single taxonomy", "status": "abandoned",
              "created": "2026-09-21", "updated": "2026-09-21",
              "closed": { "why": "tags preserve the useful cross-cuts", "at": "2026-09-21" } },
    "body": ""
  },
  "from": "seed"
}
```

```json
// neb link tags-beat-domains contradicts a-single-taxonomy --json
[
  { "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
                "created": "2026-09-21", "updated": "2026-09-21",
                "edges": [{ "type": "contradicts", "to": "a-single-taxonomy" }] }, "body": "" },
  { "node": { "id": "a-single-taxonomy", "title": "A single taxonomy", "status": "seed",
                "created": "2026-09-21", "updated": "2026-09-21",
                "edges": [{ "type": "contradicts", "to": "tags-beat-domains" }] }, "body": "" }
]
```

```json
// neb tag tags-beat-domains --add corpus --json
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
            "created": "2026-09-21", "updated": "2026-09-21",
            "tags": ["design", "corpus"] },
  "body": ""
}
```

```json
// neb --json note tags-beat-domains "folksonomy is the argument, not a taxonomy with extra steps"
{
  "node": { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis" },
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

`cite --json` returns the changed `doc` and the newly allocated reference id:

```json
// neb cite tags-beat-domains --kind article --uri https://example.org/folksonomy --note "the drift argument" --json
{
  "doc": {
    "node": {
      "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
      "created": "2026-09-21", "updated": "2026-09-21",
      "references": [
        { "id": "r1", "kind": "article", "uri": "https://example.org/folksonomy",
          "note": "the drift argument", "added": "2026-09-21" }
      ]
    },
    "body": ""
  },
  "reference": "r1"
}
```

### Observatory records

`--kind observatory` links a node to an Observatory record, and its `--uri` is
the bare record id — `Q002`, `H007`, `T003`, `R012` — not a path. That is what
makes the citation portable: nothing machine-specific reaches the corpus. Case
is normalised up (`q002` stores `Q002`), and anything that is not one of `Q`,
`H`, `T`, `R` followed by digits is a typed refusal at `cite`.

Where the record is comes from the machine, not the reference or the corpus:
`$OBSERVATORY_ROOT`, else this machine's setting (`neb config observatory-root
<DIR>`), else the legacy `observatory_root` key in `config.yaml`. The id is
matched by prefix inside the directory its
letter names — `questions/`, `hypotheses/`, `theories/`, `research/` — so
`Q002` finds `questions/Q002-is-proper-time-a-count….md` and `R012` finds the
`research/R012-arc/` directory.

`check` warns, and never errors, when the root is unset or the id does not
resolve: the citation is still true, and the machine is merely missing or
behind the checkout. `show` prints the resolved path under the reference, and
`show --json` carries an `observatory` array of
`{reference, record, path}` (`path` omitted when it does not resolve).

```json
// neb cite proper-time-is-a-count --kind observatory --uri Q002 --note "the question this became"
// then: neb show proper-time-is-a-count --json
{
  "node": {
    "references": [
      { "id": "r1", "kind": "observatory", "uri": "Q002",
        "note": "the question this became", "added": "2026-09-21", "by": "human" }
    ]
  },
  "observatory": [
    { "reference": "r1", "record": "Q002",
      "path": "/Users/you/workspace/observatory/questions/Q002-is-proper-time-a-count-of-snapshots-along-a-worldline.md" }
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
(`UnresolvedObservatoryRecord`). With no root set, the id is accepted and the
verb prints the same `No observatory root set` line `cite` does.

```
// neb handoff scarcity-wake H012 --note "the hypothesis this became"
scarcity-wake seed -> abandoned, handed off to H012 (r1)
/Users/you/workspace/observatory/hypotheses/H012-scarcity-wake.md
```

`--json` returns the node as written, the new reference's id, the record as
stored, and the status it left:

```json
// neb handoff scarcity-wake H012 --note "the hypothesis this became" --json
{
  "doc": {
    "node": {
      "id": "scarcity-wake", "title": "Scarcity wake", "status": "abandoned",
      "created": "2026-09-26", "updated": "2026-09-26",
      "references": [
        { "id": "r1", "kind": "observatory", "uri": "H012",
          "note": "the hypothesis this became", "added": "2026-09-26" }
      ],
      "closed": { "why": "handed off to H012", "at": "2026-09-26" }
    },
    "body": ""
  },
  "reference": "r1",
  "record": "H012",
  "from": "seed"
}
```

A node reads as handed off when it is `abandoned`, its `closed.why` is
exactly `handed off to <RECORD>`, and it carries an `observatory` reference
to that record, so the two-verb form reads the same. `show` then prints the
record's location under its `closed:` line, `trace` appends
`handed off to <RECORD>` to its line, and both `--json` forms carry
`"handed_off_to": "<RECORD>"`, omitted for every other node.

## Authorship

`--by <LABEL>` records who wrote the text a verb authors. The label is free
text — a session id, a crew name — and defaults to `human`, so an
unattributed write reads as the human's own. It is stored per field rather
than per node: `title_by`, `kill_by`, `by` on each edge and each reference,
and, for a note, inline in the line it writes
(`- YYYY-MM-DD (agent:crew-alpha): text`; the human's line names nobody). The
human is stored by omission, so files written before this existed are already
correct and `neb migrate` has nothing to do; `show --json` and `list --json`
state the default outright. A label cannot contain parentheses, a newline or
`: `, since a note line has to parse back.

`--by` is not `--task`/`--run`: those say which Orbit run produced a write,
not who wrote the words. `check` enforces nothing about authorship.

## Query

| verb | does | flags |
|---|---|---|
| `neb show <NODE>` | one node in full, plus its body, currently or at a historical revision | `--at <HASH\|YYYY-MM-DD>` |
| `neb log <NODE>` | commits that changed the node, newest first | — |
| `neb list` | every node | `--status <S>`, `--tag <TAG>`× (every tag must match), `--limit <N>` |
| `neb near <QUERY>...` | the existing nodes closest to free text, or to a node (left out of its own answer), best first, each banded `strong`/`some`/`weak`; for a node, marks neighbours already linked to it | `--limit <K>`/`-k` (default 3); flags may follow the query |
| `neb trace <NODE>` | ancestry as a tree, each line naming its edge kind(s) | `--down` for descendants, `--depth <N>` |
| `neb impact <NODE>` | descendants plus `contradicts` neighbours | — |
| `neb graph` | the whole corpus as `{nodes, edges}` or a Mermaid diagram | `--json`, or `--mermaid [--from <ID>]`; without a format, a hint and exit 2 |

```json
// neb show tags-beat-domains --json
{
  "node": {
    "id": "tags-beat-domains",
    "title": "Tags beat domains",
    "title_by": "human",
    "status": "hypothesis",
    "created": "2026-09-12",
    "updated": "2026-09-12",
    "kill": "a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer",
    "kill_by": "human",
    "tags": ["design", "corpus"],
    "edges": [
      { "type": "derives-from", "to": "required-categorical-fields-drift", "by": "human" },
      { "type": "contradicts", "to": "a-single-global-taxonomy", "by": "human" }
    ],
    "references": [
      { "id": "r1", "kind": "article", "uri": "https://example.org/folksonomy",
        "title": "Folksonomies", "note": "the drift argument, made for web tagging",
        "added": "2026-09-12", "by": "human" }
    ]
  },
  "body": "tags beat domains because a category you must pick is a decision you skip"
}
```

In text, `show` opens with a header: the status and id; the title, followed
by `(<label>)` when an agent wrote it; `tags: a, b` when there are any; and
`created: <date>  updated: <date>`. The kill condition, each edge and each
reference likewise end in `(<label>)` when their author is not `human`.

```
// neb show tags-beat-domains
hypothesis tags-beat-domains
Tags beat domains
tags: design, corpus
created: 2026-09-12  updated: 2026-09-12

kill: a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer

…the body, then the edges and references
```

`neb show <NODE> --at <HASH|YYYY-MM-DD>` has exactly the same text and JSON
shape as current `show`; a date means the final commit on that date. A date
before the node existed is refused. `neb log <NODE>` prints short hash, date,
and message. When no commit has touched the node, text output says `no commits
touched this node`; its JSON stays `[]`. Otherwise its JSON keeps the full hash:

```json
// neb log tags-beat-domains --json
[
  { "hash": "be82c52c9420bbd5bf50b284ad07a050575524f1", "date": "2026-09-21",
    "message": "neb sharpen tags-beat-domains" },
  { "hash": "9cfa9bf3e11b0ad32899d84bf1049511f060f19a", "date": "2026-09-20",
    "message": "neb new tags-beat-domains" }
]
```

Both historical verbs require the corpus to be inside a git work tree and are
read-only.

`neb list --json` is an array of the same `node` objects (no `body`). A closed
node carries `"closed": { "why": "...", "at": "2026-09-12" }`; optional fields
(`kill`, `closed`, `origin`, empty lists) are omitted.

`--limit <N>` on `list`, `inbox` and `review` bounds the output; without it
everything is printed, as before. The text ends by saying what was left out
(`2 of 3 matching nodes shown, of 4 in all; raise --limit for more`). Under
`--json` the array is simply cut to N: same shape, no marker, so compare its
length with N to know whether there may be more.

`near` is word overlap — BM25 over title, tags and body, title and tags
weighted up, plurals and `-ing` folded, no embeddings and no network — with
the score normalised so `1` would saturate every word of the query. Nothing
does: a word-for-word copy of a node scores about `0.35`–`0.6` against it,
so the number is not a percentage and text output does not print it. It
prints a band instead, and `--json` carries both `score` and `band`:

| band | score | reads as |
|---|---|---|
| `strong` | `0.25` and up | shares most of the query's distinctive words: a possible duplicate, or the obvious parent |
| `some` | `0.07` to `0.25` | shares a few distinctive words: read it before deciding |
| `weak` | under `0.07` | a word or two in passing |

The cut-offs come from a synthetic corpus of sixteen varied nodes: copies
and promoted copies scored `0.44`–`0.59` (one in a real corpus scored
`0.36`), sentences reusing most of a node's key words `0.28`–`0.53`, a
sentence sharing one incidental word `0.09`–`0.20`, and a whole node against
the others `0.07`–`0.13` for its topical neighbours and under `0.07` for
nearly everything else. A short query scores
higher for the same overlap (one word is half of a two-word query), so a
one-word match on a two-word query can read `strong`. The band never
reorders anything; the order is the score's.

Given a node, each neighbour's `linked` lists the edges already joining it
to that node, either way round, as `{from, type, to}` in declaration order;
text appends `linked: parent (<kinds>)`, `linked: child (<kinds>)` or
`linked: contradicts`. It is `null` when there is no such edge, and always
for free text (and so for the `near` of `capture` and `promote`). A linked
neighbour is a link that exists, not one to make.

Nodes sharing no word are left out, so an empty answer (`[]`; in text,
`nothing near: no node shares a word with this`) is a real finding: the
thought is unlike anything in the corpus. It ranks candidates for a human to
read; it never writes anything, and passing its first line straight to
`--parent` unread is the automatic linking the spec rules out.

```json
// neb near --json "one global taxonomy for every domain"
[
  { "id": "a-single-global-taxonomy", "title": "A single global taxonomy",
    "status": "abandoned", "tags": ["design"], "score": 0.301, "band": "strong", "linked": null },
  { "id": "tags-beat-domains", "title": "Tags beat domains",
    "status": "hypothesis", "tags": ["design", "corpus"], "score": 0.138, "band": "some", "linked": null }
]

// neb near tags-beat-domains        (the node's own text is the query; it is not in the answer)
strong abandoned  a-single-global-taxonomy A single global taxonomy  linked: contradicts
some   seed       required-categorical-fields-drift Required categorical fields drift  linked: parent (derives-from)
```

`neb trace` prints a tree. Every line below the start names the genealogy
edge kind(s) joining it to the line above. The kinds are always the
descendant's edges, so they read the same both ways: walking up, the line
above declares them to this one; walking down, this one declares them to the
line above. Two edges between the same pair (`derives-from` plus a later `reopens`)
are one relation stated twice, so they draw as one line naming both kinds. A
node reached along two different paths is a true diamond: it is drawn on each
path but expanded only once, and later copies end in `(shown above)`. A node
handed off to Observatory ends its line with `handed off to <RECORD>`.

```
// neb trace retardation-in-the-wake
hypothesis retardation-in-the-wake Retardation in the wake
├─ derives-from, reopens  refuted    scarcity-wake-retardation Scarcity wake retardation
│  └─ derives-from  seed       gravity-as-scarcity Gravity as scarcity
└─ refines  seed       gravity-as-scarcity Gravity as scarcity  (shown above)
```

`--depth <N>` stops the walk N steps out (`1` is the parents, or the children
with `--down`; `0` the node alone), in the tree and in `--json` alike. A line
whose branches it cut ends in `(K more beyond --depth)`. A node within N steps
along any path is kept, even when the first path the walk took reached it
further out. Without `--depth` the whole walk is printed, as before.

`--json` lists each node once, in walk order. `parents` names each genealogical
parent once, however many edges reach it. `via` is the step that first reached
the node: `from` is the node it was reached from, and `kinds` lists every edge
kind between the two, in declared order. It is `null` for the start.
`handed_off_to` names the Observatory record a node was handed off to, and is
omitted for every other node.

```json
// neb trace tags-beat-domains --json
[
  { "id": "tags-beat-domains", "title": "Tags beat domains", "status": "hypothesis",
    "parents": ["required-categorical-fields-drift"], "via": null },
  { "id": "required-categorical-fields-drift", "title": "Required categorical fields drift",
    "status": "seed", "parents": [],
    "via": { "from": "tags-beat-domains", "kinds": ["derives-from"] } }
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

`neb graph --mermaid` emits a `graph BT` block with every node styled by
status and every edge labelled by kind. Symmetric `contradicts` edges become
one dotted, undirected line. Add `--from <ID>` to include only that node, its
ancestors and its descendants; siblings and unrelated components stay out.
`--from` requires `--mermaid`, and both conflict with `--json` whether it is
written before the verb or after it (exit 2), so JSON always exports the whole
corpus rather than silently ignoring a requested lineage.
Titles are escaped so quotes and brackets remain label text.

```mermaid
graph BT
  tags-beat-domains["Tags beat domains"]:::hypothesis
  required-categorical-fields-drift["Required categorical fields drift"]:::seed
  tags-beat-domains -->|derives-from| required-categorical-fields-drift
  classDef seed fill:#dbeafe,stroke:#2563eb,color:#172554
  classDef hypothesis fill:#fef3c7,stroke:#d97706,color:#451a03
  classDef refuted fill:#fee2e2,stroke:#dc2626,color:#450a0a
  classDef abandoned fill:#f3f4f6,stroke:#6b7280,color:#374151
```

## Maintenance

| verb | does | flags |
|---|---|---|
| `neb review` | the weekly report: stale hypotheses (≥ 30 days), untouched seeds (≥ 90), hypotheses created ≥ 14 days ago with no references, hypotheses whose kill nobody human wrote, inbox waiting ≥ 14 | `--since <DAYS>`, `--out <FILE>`, `--limit <N>` (per section) |
| `neb review --short` | the quick glance, one line each: hypotheses created ≥ 14 days ago with no references, seeds untouched ≥ 90 days, inbox entries waiting ≥ 14 days | `--tag <TAG>`×, `--limit <N>` (lines) |

Both forms are read-only by the spec's hard rule. `--short` refuses `--since`
and `--out`, and `--tag` needs `--short`.
Notes are reasoning, not context: adding a note does not count as adding a
reference and does not close the no-references finding after the grace period.

```json
// neb review --json     (rule: stale-hypothesis | untouched-seed | no-references | unconfirmed-kill | stale-inbox)
[
  { "rule": "no-references", "id": "tags-beat-domains",
    "title": "Tags beat domains", "reason": "no references attached" }
]
```

`neb review` without `--json` prints five `##` sections in that order, each
`_none_` or a `- \`id\` Title — reason` list; `--out review.md` writes it to a
file. `--limit <N>` keeps the first N findings under each heading, so a
crowded section cannot push a short one out, and a cut section ends
`- _… and K more; raise --limit for more_`; under `--json` it keeps the first
N items of each `rule`. With `--short` it keeps the first N lines and ends
`… and K more; raise --limit for more`.

```json
// neb review --short --json
[
  { "id": "an-idea", "why": "hypothesis with no references" }
]
```

`neb review --short --json` is an array of `{id, why}` — narrower than full
`review`'s items, with no `rule`, `title`, or `reason`.
