# Routine mode

Nobody is watching. An Orbit routine, a cron entry, a sweep — anything that
runs without a human in the loop is routine mode, and so is any session where
you are not sure. The rule is absolute: **read the corpus, then propose; never
mutate the corpus.**

Allowed read-only corpus commands: `check`, `inbox`, `show`, `log`, `list`,
`near`, `trace`, `impact`, `graph`, `open`, `review` (with or without
`--short`), and `tag list`. Do not use corpus-mutating commands, including
`capture`, `promote`, `drop`, `new`, `edit`, `sharpen`, `status`, `link`,
`tag`, `note`, `cite`, `migrate`, `config`, or `init`. `completions` only
generates shell scripts and is not part of a routine corpus review.

`open` is a deprecated alias for `review --short`. Both raise hypotheses
created at least fourteen days ago with no references, seeds untouched for at
least ninety days, and inbox captures waiting at least fourteen days. Full
`review` also reports hypotheses untouched for thirty days by default and
unconfirmed kills; `--since` overrides the stale-hypothesis and untouched-seed
thresholds.

Proposals go in `nebula-review.md`, attached to the current Orbit task with
`orbit.task.artifact.put`. Draft it at `$ORBIT_SCRATCH_DIR/nebula-review.md`,
then attach it; this run scratch directory is outside the corpus. Without an
Orbit task, write proposals to `$HOME/.local/state/nebula/nebula-review.md`
(create its parent directory first, and overwrite it each run). `neb review
--out` writes the diagnostic report, not the proposals; if used, give it a
path outside the corpus. Never put proposals or reports under the corpus root.
A human or session-mode agent reads the proposal and runs only the lines they
agree with.

## The run

```sh
neb check --json                 # if any error finding: report it as the whole review and stop
neb review --json                # review findings; covers everything `review --short` would
neb inbox --json                 # what is waiting
neb list --json                  # for finding parents and duplicates
```

Then, for each item, decide which proposal type it is and write one line.
If you cannot say why in one line, do not propose it.

## `nebula-review.md` format

One `##` heading per proposal type, in this order, omitting empty ones. One
line per item: the exact command, then ` — ` and the one-line reason. Lead
with the check line and the date.

```markdown
# nebula review — 2026-09-13

Orbit task: <task-id>
Orbit run: <run-id>

`neb check`: 4 nodes, 0 errors, 0 warnings.

## promote
- `neb promote a6e8 --title "Review as a weekly Orbit routine" --parent tags-beat-domains --tag design --by agent:nebula-routine` — reads as a claim about unattended review; nearest node by topic and tag

## drop
- `neb drop 3f1c` — "buy more coffee" is an action item, not an idea

## sharpen
- `neb sharpen required-categorical-fields-drift --kill "..." --by human` — seed since 2026-09-12 with no falsifier; the kill is the human's to write

## confirm
- `neb sharpen tags-beat-domains --confirm` — `review` rule unconfirmed-kill; the kill reads as theirs, but only they can say so

## link
- `neb link required-categorical-fields-drift refines tags-beat-domains --by agent:nebula-routine` — its body restates the parent's second sentence

## cite
- `neb cite tags-beat-domains --uri "..." --kind note --note "Source for the categorical-fields claim" --by agent:nebula-routine` — `review` no-references rule for a hypothesis created at least fourteen days ago; the human knows the source

## abandon
- `neb status tags-beat-domains abandoned --why "..."` — aged hypothesis has no reference; the human supplies the closure reason
```

Rules for the lines:

- The command must be runnable as written except for `"..."`, which marks a
  value only the human can supply (a kill condition, a reason, a URI).
- Never propose `refuted`. Refuting asserts a kill condition fired, and that
  is an observation the human makes. Propose `abandoned` and say why.
- Never run `sharpen --confirm` yourself, in either mode: confirming a kill
  condition is the human saying they stand behind it.
- Never propose a new tag without saying `(new tag)` after the reason.
- Never propose `migrate`, `init`, `config` or anything else touching
  `config.yaml`; if `check` fails to open the corpus, that failure is the
  entire review. An observatory-root warning is the one exception worth
  reporting: say which records did not resolve and leave the setting to the
  human.

## Provenance

When attaching an Orbit artifact, include the task and run ids in its header;
omit those lines for a non-Orbit cron. Examples use `--by agent:nebula-routine`
for wording and edges the routine authored; use the deployed routine's agent
label in real proposals. A human supplies fields marked `"..."`; those
commands use `--by human` where the CLI supports authorship. The human running
a proposed line supplies their own `--task` and `--run` when those apply.

## Worked transcript

```sh
$ neb check --json
{ "findings": [], "nodes": 3 }
$ neb review --json
[ { "rule": "no-references", "id": "tags-beat-domains",
    "title": "Tags beat domains", "reason": "no references attached" } ]
$ neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16:42+02:00", "text": "nebula review as a weekly orbit routine" },
  { "id": "3f1c", "at": "2026-09-12T18:17:05+02:00", "text": "buy more coffee" } ]
$ neb list --json | jq -c '.[] | {id, status, tags, kill}'
{"id":"a-single-global-taxonomy","status":"refuted","tags":["design"],"kill":"nobody can keep it current"}
{"id":"required-categorical-fields-drift","status":"seed","tags":["design"],"kill":null}
{"id":"tags-beat-domains","status":"hypothesis","tags":["design","corpus"],"kill":"a corpus of 50+ nodes needs a cross-cutting query that tags cannot answer"}
```

This sample finding represents a hypothesis created at least fourteen days
earlier; newer hypotheses without references are still within the grace period.

Drafted in `$ORBIT_SCRATCH_DIR/nebula-review.md`, then attached to the current
Orbit task as `nebula-review.md`:

```markdown
# nebula review — 2026-09-13

Orbit task: <task-id>
Orbit run: <run-id>

`neb check`: 3 nodes, 0 errors, 0 warnings.

## promote
- `neb promote a6e8 --title "Review as a weekly Orbit routine" --parent tags-beat-domains --tag design --by agent:nebula-routine` — a claim about how the corpus is operated; nearest node by topic, same tag

## drop
- `neb drop 3f1c` — "buy more coffee" is an action item, not an idea

## sharpen
- `neb sharpen required-categorical-fields-drift --kill "..." --by human` — seed with no falsifier; `tags-beat-domains` derives from it, so a kill here matters downstream

## cite
- `neb cite tags-beat-domains --uri "..." --kind note --note "Source for the categorical-fields claim" --by agent:nebula-routine` — `review` no-references for a hypothesis created at least fourteen days ago; the v0.1 domain-list drift is the obvious source
```

Nothing was written inside the corpus, including `nodes/`, `inbox/`, or its
root. The report was read from stdout and the proposals are attached to the Orbit
task. The routine exits 0 whether or not it had anything to propose; an empty
proposal artifact is `# nebula review — <date>` plus the check line.
