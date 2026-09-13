---
name: nebula
description: nebula — the idea lineage graph and its `neb` CLI. Use when a human says "capture that", "that's a nebula seed", asks to triage the inbox, sharpen, link, cite or close an idea, trace where an idea came from, run `neb review`, or when an Orbit routine reads a nebula corpus unattended. Covers the corpus location, every verb and its `--json` shape, the ten invariants and their refusals, and the two operating modes.
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

`--root <dir>`, else `$NEBULA_ROOT`, else `~/.nebula`. The corpus is private
and **never inside the nebula repository** (`codebases/nebula`); never commit
`nodes/`, `inbox/` or `config.yaml` there. Before the first write of a session
run `neb check` — it confirms the root resolves, the schema is v2, and the
corpus is clean. A corpus at schema 1 refuses to open until `neb migrate`.

## The verbs

Mutating: `capture`, `promote`, `drop`, `new`, `sharpen`, `status`, `link`,
`tag`, `cite`, `migrate`. Read-only: `inbox`, `show`, `list`, `trace`,
`impact`, `graph --json`, `open`, `review`, `check`, `tag list`. Every verb
takes `--json`; [verbs.md](references/verbs.md) has each one's flags and its
real `--json` shape. Prefer `--json` for anything you will reason over.

## Capture from conversation

When the human flags something worth keeping — "capture that", "that's a
seed" — run `neb capture "<one sentence, in their words>"`. Their words, not
your summary; one sentence, no decisions, no parent. Ask before capturing
anything they did not flag. At the end of a substantive session propose (do
not run) up to five `neb capture "..."` lines from the discussion.

## Provenance

Under Orbit, pass `--task <id>` and `--run <id>` on `promote`, `new` and
`cite`, from `ORBIT_TASK_ID` / `ORBIT_RUN_ID` or the task you were given.
Never hardcode an agent family: crews and model names change underneath the
skill.

## Triage heuristics

- **Promote** when the entry names something falsifiable, or connects to an
  existing node — find the closest one with `neb list --json` and `neb trace`,
  pass it as `--parent`, and reuse that parent's tags. Never invent a new tag
  without saying so.
- **Drop** when it duplicates a node (say which) or is an action item, not an
  idea. Dropping is a normal outcome, not a failure.
- **Sharpen** the moment a seed has a falsifier: `--kill` is written before
  anything is read.
- **Link** only what you can defend: `derives-from` for descent, `refines`
  for a narrower version, `generalizes` for a wider one, `contradicts` for a
  live conflict, `reopens` for a new node reviving a refuted one.
- **Cite** with a `--note` saying why it is here; a reference without one is a
  link that rots and `check` will warn.

## Refusals

`neb` refuses rather than warns at the point of action. Each refusal is typed
and tells you what to do; do not retry the same command.
[invariants.md](references/invariants.md) lists the ten rules, which verb
enforces each, and the move that resolves it.

## References

| Reference | Read it for |
|---|---|
| [verbs.md](references/verbs.md) | Every verb, its flags, and its `--json` shape from a real corpus. |
| [invariants.md](references/invariants.md) | The ten rules, which verb refuses what, and what to do instead. |
| [session-mode.md](references/session-mode.md) | The directed flow: triage with the human, sharpen, link, cite, close; check after every write. Worked transcript. |
| [routine-mode.md](references/routine-mode.md) | The unattended flow: read, then write proposals to `review.md`. Format and worked transcript. |
