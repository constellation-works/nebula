# Session mode

A human is present and directing. You run the verbs; they decide. The loop
for every write is the same:

1. Say what you are about to run and why, in one line.
2. Run it.
3. Run `neb check`.
4. Report the changed ids and the check line (`N nodes, E errors, W warnings`).

If `check` reports an error after your write, stop and show it; do not write
again until the human has seen it. If it reports a warning you caused (rule 9,
a reference without a note), fix it in the same turn.

## Start of session

```sh
neb check                 # root resolves, schema is v2, corpus is clean
neb inbox --json          # what is waiting
neb open --json           # what needs attention
```

Report all three in two or three lines. If the inbox is non-empty, offer to
triage it; do not start without being asked.

## Triage

For each inbox entry, in order:

- Find the nearest existing node: `neb list --json`, then `neb trace <id>`
  on the candidates. Read titles and `kill` fields, not just ids.
- Say which of **promote** (with the proposed `--title`, `--parent`, `--tag`)
  or **drop** (with the reason: duplicates `<id>`, or an action item) you
  recommend, and wait for the human unless they have said "just do it".
- Promote with the parent's tags. A new tag is a decision: name it as one.
- After promoting, ask for the falsifier. If the human gives one, `sharpen`
  in the same breath. If not, leave it a seed — a seed is honest.

## Capture

"Capture that" / "that's a seed" → `neb capture "<their sentence>"`. Their
words. If they said three things, ask which one. If they did not flag it,
ask before capturing. `capture` works even when nothing else does; it is the
five-second path and must stay one.

## Linking, citing, closing

- `link` only when the human has stated the relation or it is unambiguous
  from what they said. `derives-from` is the default for descent; the other
  kinds are claims, and claims are theirs.
- `cite --note "<why>"` always. Under Orbit add `--task`/`--run`.
- `status <id> refuted --why "<how the kill fired>"`: quote the human. Refuted
  is final. `abandoned` may carry `--why` and should.
- Never move a `refuted` node. Reviving it is `new` + `link <new> reopens <old>`.

## End of session

Propose, do not run, up to five captures from the discussion, each a ready
line:

```sh
neb capture "…"
```

Then `neb check` one last time and report it.

## Worked transcript

Fixture: three nodes, one hypothesis with a kill and a reference, one seed,
one refuted; one live inbox entry.

> **human:** let's clear the inbox

```sh
$ neb check
3 nodes, 0 errors, 0 warnings
$ neb inbox --json
[ { "id": "a6e8", "at": "2026-09-12T18:16", "text": "nebula review as a weekly orbit routine" } ]
$ neb list --json | jq -c '.[] | {id, status, tags}'
{"id":"a-single-global-taxonomy","status":"refuted","tags":["design"]}
{"id":"required-categorical-fields-drift","status":"seed","tags":["design"]}
{"id":"tags-beat-domains","status":"hypothesis","tags":["design","corpus"]}
```

> **agent:** One entry. "nebula review as a weekly orbit routine" is about
> operating the corpus, closest to `tags-beat-domains` (both are about how
> the tool is used, tag `design`). It reads as an action item more than an
> idea, so I'd **drop** it — unless you mean the claim that unattended review
> is safe as long as it only proposes. Promote as that, or drop?

> **human:** promote it as the claim, parent tags-beat-domains

```sh
$ neb promote a6e8 --title "Review as a weekly Orbit routine" --parent tags-beat-domains --tag design --task DANI-10348 --run jrun-20260913-0113-c3
review-as-a-weekly-orbit-routine /…/nodes/review-as-a-weekly-orbit-routine.md
$ neb check
4 nodes, 0 errors, 0 warnings
```

> **agent:** Promoted `review-as-a-weekly-orbit-routine` (seed, parent
> `tags-beat-domains`, tag `design`). Check clean, 4 nodes. What would kill it?

> **human:** if a routine ever has to write to fix something. actually no —
> that belongs in the skill, not the graph. abandon it.

```sh
$ neb status review-as-a-weekly-orbit-routine abandoned --why "belongs in the skill, not the graph"
review-as-a-weekly-orbit-routine seed -> abandoned
$ neb check --json
{ "findings": [], "nodes": 4 }
```

> **agent:** Abandoned with your reason; it stays in the graph so the question
> is not re-asked. Check clean. Two captures from this conversation, if you
> want them:
>
> ```sh
> neb capture "a routine that has to write to fix something is a skill bug, not a graph fact"
> neb capture "operating rules for the corpus belong in the skill, ideas about the corpus belong in the graph"
> ```
