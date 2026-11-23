# Verbs: flags, output and refusals

Read this for the flags every verb shares, what stdout and stderr carry, the
`--json` refusal envelope with its exit codes, and `--by` authorship.

Contents:

- [stdout and stderr](#stdout-and-stderr)
- [Refusals under `--json`](#refusals-under---json)
- [Authorship](#authorship)

`ORBIT_TASK_ID` and `ORBIT_RUN_ID` fill absent provenance flags on `new`,
`promote`, `cite` and `handoff`. A nonempty `ORBIT_RUN_ID` requires explicit
`--by` for new authored words (`by_required`) and refuses `sharpen --confirm`
(`human_only`). `NEBULA_READ_ONLY=1` refuses every write (`read_only`).

Every corpus verb below takes `--root <DIR>` and `--json`, and those flags may
appear anywhere on the line — before the verb, after it, or after the free text
of `capture`, `note` and `near`. Every verb that can commit also takes
`--no-commit`, after the verb. `init` creates a corpus but never commits, so
like a read-only verb such as `show`, `list` or `trace` it refuses `--no-commit`
there as an unknown argument.

The older spelling before the verb is deprecated: before a verb that can commit
it still skips the commit and warns on stderr, and before any other verb —
including `init` — it is refused (`usage`, exit 2). Under `--json`, each emits
one JSON value on stdout.

Two commands are the exceptions: `completions` always emits a shell script, and
refuses `--json` and `--root` rather than ignore them (exit 2; `$NEBULA_ROOT` is
not read), and `triage`, which takes its decisions from a person one key at a
time, refuses `--json` (`Interactive`) and names the scriptable verbs instead.

Ids are slugs of the title (`"Tags beat domains"` → `tags-beat-domains`); inbox
ids are four hex chars. A slug over 60 characters is cut at the last `-` at or
before the limit, never mid-word.

`promote` without `--title` or `--id` keeps the captured sentence as the title
but, for a capture over five words, mints the id from its first five words after
dropping stop-words (`gravity might be a scarcity gradient in some shared
resource` → `gravity-scarcity-gradient-shared-resource`); if that id belongs to
a different idea it adds one more such word at a time, then falls back to the
full slug.

A node already titled with the same text is refused with `NodeExists` naming it,
never duplicated, as is a capture whose every candidate is taken. `promote` and
`new` take `--id <SLUG>` to choose the id explicitly instead — useful for a long
title, since ids are frozen by invariant once written.

An explicit id follows the same slug rules (lowercase words joined by single
dashes, 60 characters or fewer) and is refused, as a typed error, if it breaks
those rules or collides with an existing node. The `--json` excerpts below are
real output from a three-node fixture corpus.

Every field a `--json` payload documents is present in it: an absent value is
`null` and an empty list `[]`, never a missing key, so one kind of record has
one key set whichever verb returned it.

A node is always `{id, title, title_by, status, created, updated, kill, kill_by,
tags, edges, references, closed, origin}`, and every author label is stated,
`"human"` where nobody passed `--by`, from a write verb exactly as from `show`.
A list that a limit cut says so: see [capped lists](verbs-query.md#capped-lists).

## stdout and stderr

stdout carries the result and nothing else: records, ids, the report. So `neb
list | wc -l` counts nodes, `grep` and `cut` see only records, and `E=$(neb
capture -q …)` holds the entry id alone. Everything about the result goes to
stderr, one line each: counts (`3 of 4 nodes`), what `--limit` or `--depth` left
out, the line saying a query found nothing, hints, and `committed <hash>`.

Under `--json` the line saying nothing matched and the line saying a bound cut
the result are still written to stderr; counts, hints and `committed` are not.

A list is a table. On a terminal it has one header row, then one line per
record, never wrapped or cut; columns are two spaces apart, a count is
right-aligned under its header, and an empty cell reads `-`. Piped or
redirected, the same records are one tab-separated line each, with no
header and no colour, and every column is there (`-` when empty), so `cut
-f3` is always the same field:

| verb | columns |
|---|---|
| `list` | `STATUS ID TAGS TITLE`, the tags comma-joined in one cell |
| `inbox` | `ID AT TEXT` |
| `near` | `BAND STATUS ID TITLE LINKED` |
| `tag list` | `TAG COUNT` |
| `review --short` | `ID WHY` |
| `log` | `HASH DATE MESSAGE` |

```
// neb list --tag design        (on a terminal)
STATUS      ID                        TAGS           TITLE
abandoned   a-single-global-taxonomy  design         A single global taxonomy
hypothesis  tags-beat-domains         design,corpus  Tags beat domains

// neb list --tag design | cat
abandoned	a-single-global-taxonomy	design	A single global taxonomy
hypothesis	tags-beat-domains	design,corpus	Tags beat domains
```

An empty list prints nothing on stdout, header included. `show`, `trace`,
`impact`, `check` and `review` are reports, not lists, and keep their own
layouts.

Colour is decided once, per stream: text is coloured only for a terminal,
never when that stream is piped or redirected, when `TERM=dumb`, or when
`NO_COLOR` is set to a non-empty value. `CLICOLOR_FORCE` never colours a
pipe. Each colour stands for a role (a live seed, an owed hypothesis, a dead
node, an error) and is never the only thing saying so. `--help` is never
coloured and never re-wrapped to the terminal's width.

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
| `code` | string | The refusal's stable `snake_case` name. For a core refusal it is the `nebula-core` variant's name in `snake_case`: `no_such_node`, `cycle`, `self_loop`, `needs_kill`, `refuted_needs_why`, `refuted_cannot_reopen`, `seed_with_kill`, `unknown_reference_kind`, `unresolved_uri`, `absolute_uri`, `schema_mismatch`, `missing_config`, `corpus_ignored`, `git`, `git_timed_out`, `locked`, `edit_conflict`, `input_too_large`, `io_at` (an I/O failure, naming the path), `io_stdin` (standard input could not be read), `stdin_not_utf8`, `empty_capture` (an empty `capture`), `empty_note`, `reason_on_open_status`, `no_kill_to_confirm`, `uri_required`, `invalid_author_label`, `invalid_at`, `not_a_status`, `not_an_edge_type`, `malformed_frontmatter`, `unreadable_nodes` (a strict graph load found unreadable node files), `yaml` (YAML could not be parsed or rendered), `invalid_read_only_environment` (the read-only switch is neither empty nor `1`), `invalid_origin_environment` (non-UTF-8 Orbit provenance), `current_schema_unreadable` (migration refuses an unreadable current-schema node), `inbox_entry_missing`, `inbox_entry_changed`, `inbox_entry_foreign`, `inbox_ids_exhausted`, `nodes_symlink`, `inbox_symlink`, `not_regular_file` (a node file, `config.yaml`, `.lock` or another corpus file that is a symlink, a FIFO, a device or a directory), `home_unset`, `home_not_unicode`, `empty_root_setting`, `relative_root_setting`, `dirty_tree`, `not_a_v1_status`, `not_a_v1_edge_type`, `migrated_node_unreadable`, `malformed_history`, `no_free_keep_name`, `temp_cleanup_failed`, `no_free_temp_name`, and the rest in the `invariants-refusals-` files ([nodes](invariants-refusals-nodes.md), [corpus](invariants-refusals-corpus.md), [inbox](invariants-refusals-inbox.md), [writes](invariants-refusals-writes.md)). The CLI adds its own: `usage` (arguments that parse but ask for nothing, such as `graph` with no format, `near` with no text, or `--no-commit` before a verb that never commits), `editor_not_configured` (neither editor variable names an editor), `editor_invalid_command` (empty command or unmatched quotes), `editor_start` (the editor could not start), `editor_unsuccessful` (the editor exited unsuccessfully), `report_in_corpus` (`review --out` resolves inside the corpus; choose an external file), `triage_key` (a key `triage` does not know), `triage_title_lost` (input ended while a `triage` title was waiting to be used), `stdout` (writing standard output failed; a broken pipe exits quietly instead) and `json` (JSON could not be parsed or rendered, with context). Core also reports `notes_changed` when an edit removes, reorders or changes an existing Notes section. |
| `hint` | string or `null` | What to do about it, as the text output words it: often a command to run, such as `neb sharpen <id> --kill "..."`. `null` when the CLI has nothing to add. |

Key order is not significant. A new refusal arrives with its own `code` and
the same three fields; new fields may be added, and existing ones will not
change meaning. `SchemaMismatch` covers both directions: the hint says
whether to `neb migrate` (the corpus is older) or upgrade `neb` (newer).

Exit codes, with or without `--json`:

| code | means | stdout | stderr under `--json` |
|---|---|---|---|
| 0 | success | the payload | empty, bar a `warning:` or `note:` line (from `init`, `capture`, or a commit that could not happen), or the line saying a query found nothing or a bound cut the result |
| 1 | a refusal | empty, **except** when the write landed and its commit was refused (`CorpusIgnored`, `Git`, `GitTimedOut`): then it holds the write's payload | the envelope |
| 1 | `check` found an `error`-level finding | the report | empty |
| 2 | a usage error `neb` raised: an argument no corpus could accept, whatever it holds (`usage`, `interactive`, `self_loop`, `parent_and_reopens`, `empty_kill`, `refuted_needs_why`, `absolute_uri`, `unusable_title`, `unknown_reference_kind`, `invalid_observatory_id`, `invalid_id`, `empty_root`, `root_and_path_differ`, `empty_capture`, `empty_note`, `reason_on_open_status`, `uri_required`, `invalid_at`, `invalid_author_label`, `not_a_status`, `not_an_edge_type`, and `relative_observatory_root` for a path given on the command line); nothing was written | empty | the envelope |
| 2 | clap rejected the command line: unknown flag, missing argument, bad value | empty | clap's prose, **not** JSON: it is raised before `neb` knows `--json` was asked for |

A refusal of a line read by `triage` from piped input exits 1 whatever its
code: the command line was fine. Without `--json`, every refusal prints
`error:`, the message, and the hint after a blank line.

## Authorship

`--by <LABEL>` records who wrote the text a verb authors. The label is free text
— a session id, a crew name — and defaults to `human`, so an unattributed write
reads as the human's own. It is stored per field rather than per node:
`title_by`, `kill_by`, `by` on each edge and each reference, and, for a note,
inline in the line it writes (`- YYYY-MM-DD (agent:crew-alpha): text`; the
human's line names nobody).

The human is stored by omission, so files written before this existed are
already correct and `neb migrate` has nothing to do; `show --json` and `list
--json` state the default outright. A label cannot contain parentheses, a
newline or `: `, since a note line has to parse back.

`--by` is not `--task`/`--run`: those say which Orbit run produced a write,
not who wrote the words. `check` enforces nothing about authorship.
