# Routine mode

Nobody is watching. An Orbit routine, a cron entry, a sweep — anything that
runs without a human in the loop is routine mode, and so is any session where
you are not sure. The rule is absolute: **read, then propose; never mutate.**

Allowed: `inbox`, `open`, `review`, `list`, `show`, `trace`, `impact`,
`graph --json`, `check`, `tag list`. Forbidden: everything else, including
`capture`, `migrate` and `init`.

The output is one file, `review.md`, in the corpus root (beside `nodes/`).
Overwrite it each run; it is a proposal, not a log. A human or a session-mode
agent reads it and runs the lines they agree with.

## The run

```sh
neb check --json                 # if any error finding: report it as the whole review and stop
neb review --json                # the four rules
neb inbox --json                 # what is waiting
neb open --json                  # what needs attention
neb list --json                  # for finding parents and duplicates
```

Then, for each item, decide which proposal type it is and write one line.
If you cannot say why in one line, do not propose it.

## `review.md` format

One `##` heading per proposal type, in this order, omitting empty ones. One
line per item: the exact command, then ` — ` and the one-line reason. Lead
with the check line and the date.

```markdown
# nebula review — 2026-09-13

`neb check`: 4 nodes, 0 errors, 0 warnings.

## promote
- `neb promote a6e8 --title "Review as a weekly Orbit routine" --parent tags-beat-domains --tag design` — reads as a claim about unattended review; nearest node by topic and tag

## drop
- `neb drop 3f1c` — "buy more coffee" is an action item, not an idea

## sharpen
- `neb sharpen required-categorical-fields-drift --kill "..."` — seed since 2026-09-12 with no falsifier; the kill is the human's to write

## link
- `neb link required-categorical-fields-drift refines tags-beat-domains` — its body restates the parent's second sentence

## contradicts
- `neb link tags-beat-domains contradicts a-single-global-taxonomy` — already recorded both ways; nothing to do

## cite
- `neb cite required-categorical-fields-drift --uri "..." --note "..."` — `review` rule no-references; the human knows the source

## abandon
- `neb status stale-idea abandoned --why "..."` — hypothesis untouched 30 days with no reference; propose closing, reason is theirs
```

Rules for the lines:

- The command must be runnable as written except for `"..."`, which marks a
  value only the human can supply (a kill condition, a reason, a URI).
- Never propose `refuted`. Refuting asserts a kill condition fired, and that
  is an observation the human makes. Propose `abandoned` and say why.
- Never propose a new tag without saying `(new tag)` after the reason.
- Never propose `migrate`, `init` or anything touching `config.yaml`; if
  `check` fails to open the corpus, that failure is the entire review.

## Provenance

The routine's Orbit ids go in the file header, not on the proposed commands:
the human running a line is the author of that write, and they will pass
their own `--task`/`--run` if they have one.

## Worked transcript

```sh
$ neb check --json
{ "findings": [], "nodes": 3 }
$ neb review --json
[ { "rule": "no-references", "id": "required-categorical-fields-drift",
    "title": "Required categorical fields drift", "reason": "no references attached" } ]
$ neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16", "text": "nebula review as a weekly orbit routine" } ]
$ neb open --json
[]
$ neb list --json | jq -c '.[] | {id, status, tags, kill}'
{"id":"a-single-global-taxonomy","status":"refuted","tags":["design"],"kill":"nobody can keep it current"}
{"id":"required-categorical-fields-drift","status":"seed","tags":["design"],"kill":null}
{"id":"tags-beat-domains","status":"hypothesis","tags":["design","corpus"],"kill":"a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer"}
```

Written to `$NEBULA_ROOT/review.md`:

```markdown
# nebula review — 2026-09-13

`neb check`: 3 nodes, 0 errors, 0 warnings.

## promote
- `neb promote a6e8 --title "Review as a weekly Orbit routine" --parent tags-beat-domains --tag design` — a claim about how the corpus is operated; nearest node by topic, same tag

## sharpen
- `neb sharpen required-categorical-fields-drift --kill "..."` — seed with no falsifier; `tags-beat-domains` derives from it, so a kill here matters downstream

## cite
- `neb cite required-categorical-fields-drift --uri "..." --note "..."` — `review` no-references; the v0.1 domain-list drift is the obvious source
```

Nothing was written to `nodes/` or `inbox/`. The routine exits 0 whether or
not it had anything to propose; an empty review is `# nebula review — <date>`
plus the check line.
