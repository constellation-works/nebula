---
name: nebula
description: nebula — the idea lineage graph and its `neb` CLI. Use when a human says "capture that", "that's a nebula seed", asks to triage the inbox, sharpen, link, cite or close an idea, trace where an idea came from, run `neb review`, or when an Orbit routine reads a nebula corpus unattended. Covers the corpus location, every verb and its `--json` shape, the rules in invariants.md and their refusals, and the two operating modes.
---

# nebula

A corpus of ideas as a directed acyclic graph: seeds become hypotheses by
naming what would kill them, and die as `refuted` or `abandoned` — never
deleted. The point of the tool is `neb trace`: years later, where did this
come from? The agent's job is to keep the machinery honest so the human can
think. **The human is the author; the agent is the hands.**

## Rule one: which mode you are in

| | Session mode | Routine mode |
|---|---|---|
| When | A human is present and directing | Unattended: an Orbit routine, a cron, a sweep |
| Writes | Yes — run the verbs directly | **Never.** Reads only |
| After every write | `neb check`; report the result and the changed ids | — |
| Output | The changed node ids and the check result | Proposals in `<root>/review.md`, one line of reasoning each |

If you cannot tell, you are in routine mode. A mutating verb in routine mode
is the one thing this skill forbids outright. Read
[session-mode.md](references/session-mode.md) or
[routine-mode.md](references/routine-mode.md) before acting.

## Where the corpus is

`--root <dir>`, else `$NEBULA_ROOT`, else the one-line path in
`~/.config/nebula/root`, else `~/.nebula`. The corpus is private and **never
inside the nebula repository** (`codebases/nebula`); never commit `nodes/`,
`inbox/` or `config.yaml` there. Before the first write of a session run `neb
check` — it confirms the root resolves, the schema is v2, and the corpus is
clean. If it resolves to `~/.nebula`, stop and ask rather than write unless the
human has confirmed that is the corpus. A corpus at schema 1 refuses to open
until `neb migrate`.

Plain `neb init` never changes `~/.config/nebula/root`. A human setting up the
machine's primary corpus can opt in with `neb init <dir> --set-root`; replacing
a different configured root also requires `--force`. Agents initializing a
scratch corpus always use `neb --root <scratch-dir> init` and never pass
`--set-root`.

## The verbs

Mutating: `capture`, `promote`, `drop`, `new`, `sharpen`, `status`, `link`,
`tag`, `note`, `cite`, `migrate`, `config`. Read-only: `inbox`, `show`, `list`, `near`, `trace`,
`impact`, `graph --json`, `review` (and `review --short`), `check`, `tag list`. Every corpus
verb listed here (plus `init`) emits JSON on stdout under `--json`;
[verbs.md](references/verbs.md) has each one's flags and its real shape. Shell
`completions` is the one exception. Prefer `--json` for anything you will
reason over. `neb triage` is the human's interactive loop over the inbox, a
key per entry; it refuses `--json`, so an agent triages with `inbox`, `near`,
`promote` and `drop` instead.

## Linking to Observatory

A node that became an Observatory record is cited by the record's id, never
by a path: `neb cite <node> --kind observatory --uri Q002 --note "why"`.
Record ids are `Q###` (questions), `H###` (hypotheses), `T###` (theories) and
`R###` (research). The path is reconstructed per machine: `$OBSERVATORY_ROOT`,
else this machine's setting (`neb config observatory-root <DIR>`, stored in
`~/.config/nebula/observatory-root`, never in the corpus), else a legacy
`observatory_root` key in `config.yaml`. An absolute path in a reference is a
bug: it may resolve here and breaks everywhere else. `cite` refuses one, and
`check` warns about any already in the corpus. An unresolved record is a
warning about this machine, not something to rewrite. A `check` warning that
`config.yaml` carries `observatory_root` means an older `neb` stored one
machine's path in the shared corpus: leave it to the human to set each
machine's own and then run `neb config observatory-root --drop-legacy`.

## Capture from conversation

When the human flags something worth keeping — "capture that", "that's a
seed" — run `neb capture "<one sentence, in their words>"`. Their words, not
your summary; one sentence, no decisions, no parent. Ask before capturing
anything they did not flag. At the end of a substantive session propose (do
not run) up to five `neb capture "..."` lines from the discussion. When the
human refines an idea in conversation, `neb note` the refinement in their words.

## Provenance and authorship

Two different questions, kept apart. **Provenance** says which run produced a
write: under Orbit pass `--task <id>` and `--run <id>` on `promote`, `new` and
`cite`, from `ORBIT_TASK_ID` / `ORBIT_RUN_ID` or the task you were given.

**Authorship** says who wrote the words. `--by <label>` defaults to `human`,
so an unattributed write claims the human wrote it. Pass `--by` with your own
session or crew label — `--by agent:<session-or-crew>` — on every write whose
text is yours: `new`, `sharpen`, `link`, `note`, `cite`, and `promote` when
you also pass `--title`. A capture promoted as it was captured is in the
human's words; leave it that way. Never hardcode an agent family: crews and
model names change underneath the skill. Authorship is stored per field
(`title_by`, `kill_by`, `by` on each edge, reference and note entry) and
`show --json` states it, `human` included.

A kill condition you wrote is a proposal, not the human's claim. `neb review`
lists it under "Agent-authored kills not yet confirmed by a human" until the
human runs `neb sharpen <node> --confirm`. Propose that line; never run it
yourself, because confirming is the human saying they stand behind the
falsifier.

## Triage heuristics

- **Promote** when the entry names something falsifiable, or connects to an
  existing node — `neb near "<text>"` ranks the closest by shared words
  (`capture` and a parentless `promote` print the same three lines); read
  the candidates with `neb show` and `neb trace`, pass one as `--parent`
  only if you can say why in a line, otherwise promote as a root, and reuse
  the parent's tags. `near` suggests and never links; an empty answer means
  a root, not a failure. Never invent a new tag without saying so, and when
  a write notes `tag X is close to Y`, retag to `Y` unless `X` is meant.
- **Drop** when it duplicates a node (say which) or is an action item, not an
  idea. Dropping is a normal outcome, not a failure.
- **Sharpen** the moment a seed has a falsifier: `--kill` is written before
  anything is read, with `--by` when the falsifier is yours rather than theirs.
- **Link** only what you can defend: `derives-from` for descent, `refines`
  for a narrower version, `generalizes` for a wider one, `contradicts` for a
  live conflict, `reopens` for a new node reviving a refuted one.
- **Cite** with a `--note` saying why it is here; a reference without one is a
  link that rots and `check` will warn.

## Refusals

`neb` refuses rather than warns at the point of action. Each refusal is typed
and tells you what to do; do not retry the same command. Under `--json` it is
one line of JSON on stderr, `{"error", "code", "hint"}`, with exit 1: match
on `code` (a `snake_case` name such as `needs_kill`) and act on `hint`
([verbs.md](references/verbs.md#refusals-under---json) has the envelope and
the exit codes).
[invariants.md](references/invariants.md) lists the rules, which verb
enforces each, and the move that resolves it.

## References

| Reference | Read it for |
|---|---|
| [verbs.md](references/verbs.md) | Every verb, its flags, and its `--json` shape from a real corpus. |
| [invariants.md](references/invariants.md) | The rules, which verb refuses what, and what to do instead. |
| [session-mode.md](references/session-mode.md) | The directed flow: triage with the human, sharpen, link, cite, close; check after every write. Worked transcript. |
| [routine-mode.md](references/routine-mode.md) | The unattended flow: read, then write proposals to `review.md`. Format and worked transcript. |
