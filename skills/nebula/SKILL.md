---
name: nebula
description: Operates a nebula corpus, the idea lineage graph, through the `neb` CLI — captures, triages, sharpens, links, cites, closes and hands off ideas, and answers where an idea came from (`neb trace`) or whether it was thought before (`neb near`). Use when a human says "capture that" or "that's a nebula seed", asks to triage the inbox, runs `neb review`, or when an Orbit routine reads a nebula corpus unattended. Covers the corpus location, every verb and its `--json` shape, the refusals and their rules, and the two operating modes. Not for Orbit tasks, runs or artifacts (use the `orbit` skill); a routine only attaches its proposal to an Orbit task.
---

# nebula

A corpus of ideas as a directed acyclic graph: seeds become hypotheses by
naming what would kill them, and die as `refuted` or `abandoned` — never
deleted. The point of the tool is `neb trace`: years later, where did this
come from? The agent's job is to keep the machinery honest so the human can
think.

## Common path

1. Decide the mode (table below). If you cannot tell, it is routine mode.
2. Read [session-mode.md](references/session-mode.md) or
   [routine-mode.md](references/routine-mode.md), whichever the mode names,
   before acting: before you run any `neb` command.
3. Resolve the corpus: run `neb check` before the first write and compare the
   printed `corpus:` path with the intended corpus.
4. Run the verbs with `--json` for anything you will reason over, and `--by
   agent:<session-or-crew>` on every write whose text is yours.
5. After every write run `neb check`; report the changed node ids and the
   check result.

## Rules

**The human is the author; the agent is the hands.**

### Which mode you are in

| | Session mode | Routine mode |
|---|---|---|
| When | A human is present and directing | Unattended: an Orbit routine, a cron, a sweep |
| Corpus writes | Yes — run the verbs directly | **Never.** Read only |
| After every write | `neb check`; report the result and the changed ids | — |
| Output | The changed node ids and the check result | `nebula-review.md` attached to the Orbit task, or an explicit path outside the corpus for other routine jobs |

If you cannot tell, you are in routine mode. A mutating verb in routine mode
is the one thing this skill forbids outright.

### Where the corpus is

- The root is `--root <dir>`, else `$NEBULA_ROOT`, else the nearest corpus at
  or above the working directory (holding `nodes/` plus positive corpus
  evidence; the innermost wins), else the one-line path in
  `~/.config/nebula/root`, else `~/.nebula`.
- A shell that has `cd`'d into a corpus uses that corpus even when another is
  configured; pass `--root` when the working directory is not the corpus you
  mean.
- Evidence means `config.yaml` has a top-level `schema_version` or `corpus_id`
  key (even with an invalid value), or cannot be parsed or read.
- When config is absent, a real `inbox/` directory or regular `.lock` file
  alongside `nodes/` identifies a legacy or damaged corpus.
- A lone `nodes/`, `nodes/` with another tool's parsable config, or a config
  without `nodes/` falls through to the configured or default corpus.
- The corpus is private and **never inside the nebula repository**
  (`codebases/nebula`); never commit `nodes/`, `inbox/` or `config.yaml` there.
- Before the first write of a session run `neb check` — compare the printed
  `corpus:` path (the `root` field in `--json`) with the intended corpus, then
  check the findings. A successful check confirms the schema is v2 and reports
  any warnings.
- If it resolves to `~/.nebula`, stop and ask rather than write unless the
  human has confirmed that is the corpus.
- A corpus at schema 1 refuses to open until `neb migrate`, and so does one
  with `nodes/` and legacy markers (`inbox/` or `.lock`) but no `config.yaml`
  (`missing_config`).
- A malformed or unreadable config also refuses at that root; discovery
  identifies the target and never skips a damaged corpus for another corpus.
  Never create that file yourself; see
  [invariants-refusals-corpus.md](references/invariants-refusals-corpus.md).
- Plain `neb init` never changes `~/.config/nebula/root`. A human setting up
  the machine's primary corpus can opt in with `neb init <dir> --set-root`;
  replacing a different configured root also requires `--force`.
- An empty or relative `~/.config/nebula/root` makes every command that falls
  back to it refuse (`empty_root_setting`, `relative_root_setting`), naming the
  file: tell the human, and never create a corpus at the relative path.
- Agents initializing a scratch corpus always use
  `neb --root <scratch-dir> init` and never pass `--set-root`.

### The verbs

- Mutating corpus commands: `init`, `migrate`, `config`, `capture`, `promote`,
  `drop`, `new`, `edit`, `sharpen`, `status`, `link`, `tag`, `note`, `cite`,
  and `handoff`.
- Read-only corpus commands: `check`, `inbox`, `show`, `log`, `list`, `near`,
  `trace`, `impact`, `graph`, `open`, and `review` (including `review
  --short`), plus `tag list`. `open` is a deprecated view that takes only
  `--tag`; a capped quick glance is `review --short --limit <N>`.
- Every corpus command listed here emits JSON on stdout under `--json`.
  `completions` generates shell scripts and is not a corpus command.
- `neb triage` is the human's interactive loop over the inbox, a key per
  entry; it refuses `--json`, so an agent triages with `inbox`, `near`,
  `promote` and `drop` instead.

### Observatory records

- Cite a node that became an Observatory record by the record's id, never by a
  path: `neb cite <node> --kind observatory --uri <record-id> --note "why"`.
- When the human says an idea has become an Observatory record, hand it off in
  one step: `neb handoff <node> <record-id> --note "why"`.
- Never split a hand-off back into `cite` plus `status` to get past either
  refusal: the node is already closed, or this machine has an observatory root
  and the checkout does not carry the record.
- Read [observatory.md](references/observatory.md) before you cite a record or
  hand an idea off.

### Capture from conversation

- When the human flags something worth keeping — "capture that", "that's a
  seed" — run `neb capture "<one sentence, in their words>"`. Their words, not
  your summary; one sentence, no decisions, no parent.
- Ask before capturing anything they did not flag.
- At the end of a substantive session propose (do not run) up to five `neb
  capture "..."` lines from the discussion.
- When the human refines an idea in conversation, `neb note` the refinement in
  their words.

### Provenance and authorship

Two different questions, kept apart.

- **Provenance** says which run produced a write: `promote`, `new`, `cite` and
  `handoff` fill absent `--task` and `--run` from `ORBIT_TASK_ID` and
  `ORBIT_RUN_ID`; explicit flags win.
- **Authorship** says who wrote the words. `--by <label>` defaults to `human`,
  so an unattributed write claims the human wrote it.
- Pass `--by agent:<session-or-crew>`, your own label, on every write whose
  text is yours: `new`, `sharpen`, `link`, `note`, `cite`, and `promote` when
  you also pass `--title`. A capture promoted as it was captured is in the
  human's words; leave it that way.
- Never hardcode an agent family: crews and model names change underneath the
  skill.
- Authorship is stored per field (`title_by`, `kill_by`, `by` on each edge,
  reference and note entry) and `show --json` states it, `human` included.
- Under an Orbit run, the core refuses agent-supplied words without explicit
  `--by` (`by_required`). Pass it on `handoff` and `edit` too, and on `promote`
  when supplying `--body`. `--by human` is a deliberate authorship statement.
- A kill condition you wrote is a proposal, not the human's claim. `neb review`
  lists it under "Agent-authored kills not yet confirmed by a human" until the
  human runs `neb sharpen <node> --confirm`.
- Propose that line; never run it yourself, because confirming is the human
  saying they stand behind the falsifier. The core refuses confirmation under
  an Orbit run (`human_only`).
- Routine sessions should set `NEBULA_READ_ONLY=1`; the core then refuses
  corpus writes (`read_only`) while leaving reads available.

### Triage heuristics

- **Promote** when the entry names something falsifiable, or connects to an
  existing node. `neb near "<text>"` ranks the closest by shared words
  (`capture` and a parentless `promote` print the same three lines).
- Read the candidates with `neb show` and `neb trace`. Pass one as `--parent`
  only if you can say why in a line, otherwise promote as a root, and reuse the
  parent's tags.
- `near` suggests and never links; an empty answer means a root, not a failure.
- Never invent a new tag without saying so, and when a write notes `tag X is
  close to Y`, retag to `Y` unless `X` is meant.
- **Drop** when it duplicates a node (say which) or is an action item, not an
  idea. Dropping is a normal outcome, not a failure.
- **Sharpen** the moment a seed has a falsifier: `--kill` is written before
  anything is read, with `--by` when the falsifier is yours rather than theirs.
- **Link** only what you can defend: `derives-from` for descent, `refines`
  for a narrower version, `generalizes` for a wider one, `contradicts` for a
  live conflict, `reopens` for a new node reviving a refuted one.
- **Cite** with a `--note` saying why it is here; a reference without one is a
  link that rots and `check` will warn.

### Refusals

- `neb` refuses rather than warns at the point of action. Each refusal is
  typed and tells you what to do; do not retry the same command.
- Under `--json` it is one line of JSON on stderr, `{"error", "code", "hint"}`,
  with exit 1, or exit 2 when the command line itself is wrong (a usage error).
- Match on `code` (a `snake_case` name such as `needs_kill`) and act on `hint`.
- The envelope and the exit codes are in
  [verbs-common.md](references/verbs-common.md#refusals-under---json).
- The rules, which verb enforces each, and the move that resolves it are in
  [invariants-rules.md](references/invariants-rules.md) and the
  `invariants-refusals-` files below.

## References

Read a reference when its condition holds; each stands alone.

| Reference | Open it when |
|---|---|
| [session-mode.md](references/session-mode.md) | Before acting, when a human is present: triage with them, sharpen, link, cite, close; check after every write. Worked transcript. |
| [routine-mode.md](references/routine-mode.md) | Before acting, when you run unattended: read, then attach proposals as `nebula-review.md` to the Orbit task or save them outside the corpus. Format and worked transcript. |
| [observatory.md](references/observatory.md) | Before citing an Observatory record, handing an idea off to one, or acting on an observatory-root warning from `check`. |
| [verbs-common.md](references/verbs-common.md) | You need the flags every verb shares, what stdout and stderr carry, the `--json` refusal envelope and exit codes, or `--by` authorship. |
| [verbs-corpus.md](references/verbs-corpus.md) | You run `init`, `check`, `migrate`, `config` or `review`, or need the commit behaviour. |
| [verbs-inbox.md](references/verbs-inbox.md) | You run `capture`, `inbox`, `promote`, `drop` or `triage`. |
| [verbs-nodes.md](references/verbs-nodes.md) | You run `new`, `edit`, `sharpen`, `status`, `link`, `tag` or `note`. |
| [verbs-references.md](references/verbs-references.md) | You run `cite` or `handoff`. |
| [verbs-query.md](references/verbs-query.md) | You run `show`, `log`, `list`, `near`, `trace`, `impact` or `graph`, or need a capped list's `--json` shape. |
| [invariants-rules.md](references/invariants-rules.md) | You need the 17 rules and their strengths, the writer lock, `.pending` and temporary files, or a `check` warning after your writes. |
| [invariants-refusals-nodes.md](references/invariants-refusals-nodes.md) | A refusal came from `new`, `link`, `sharpen`, `status`, `cite` or `handoff`. |
| [invariants-refusals-corpus.md](references/invariants-refusals-corpus.md) | A refusal is about where the corpus is, its schema or config, migration, or a node file or symlink. |
| [invariants-refusals-inbox.md](references/invariants-refusals-inbox.md) | A refusal came from `capture`, `promote`, `drop` or `triage`. |
| [invariants-refusals-writes.md](references/invariants-refusals-writes.md) | A refusal is about the writer lock, standard input, `edit`, temporary files, the environment or git. |
