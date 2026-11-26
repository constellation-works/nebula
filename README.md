# nebula

Capture a half-formed thought in under five seconds. Trace where any idea came
from years later.

An idea arrives vague, at random, in whatever field you happen to be thinking
about. Nebula gives it a home before it is good enough for anywhere else, then
records how it descended into whatever it became. Ideas branch, merge and die,
so the structure is a directed acyclic graph rather than a tree, and dead
branches are kept forever because they are what stops you re-treading ground.

The v0.2 model is in [docs/design/_archived/v0.2/1_spec.md](docs/design/_archived/v0.2/1_spec.md);
[docs/spec.md](docs/spec.md) is the original design record, kept as history.

## Repository boundary

This repository holds **the tool only**. It never holds the corpus.

The corpus spans work and personal thinking across unrelated fields, so it
lives outside, configured by path, and it stays private regardless of where
this repository ends up. Keeping them apart means this code can be published
without a decision about the notes, and the notes can be backed up without
dragging a build tree along.

Every command finds the corpus through `--root`, else `$NEBULA_ROOT`, else the
nearest corpus at or above the current directory (holding `nodes/` plus positive
corpus evidence: a `config.yaml` with a top-level `schema_version` or
`corpus_id` key, an unparseable or unreadable config, or legacy `inbox/` or
`.lock` markers; opening refuses a damaged or missing config rather than falling
back, while a lone `nodes/` falls through), else the path in
`~/.config/nebula/root`, else `~/.nebula`. Only an existing corpus is found from
the current directory, so `neb capture` can create one only at an explicit or
configured root, and it says so when it does.

```
nebula   = the CLI, checker and desktop app           (this repo)
corpus   = nodes/ and inbox/                          (elsewhere, private)
```

## Environment variables

<!-- env-vars:begin -->
| Variable | Meaning | Default | Overridden by |
| --- | --- | --- | --- |
| `NEBULA_ROOT` | Corpus root directory path ahead of working-directory discovery; empty counts as unset. | Nearest corpus above cwd, else `~/.config/nebula/root`, else `~/.nebula` | `--root <DIR>` |
| `OBSERVATORY_ROOT` | Path to the Observatory checkout for resolving `observatory` references; empty counts as unset. | `~/.config/nebula/observatory-root`, else legacy `config.yaml` key, else unset | - |
| `ORBIT_TASK_ID` | Orbit task identifier recorded as `origin.task` on `new`, `promote`, `cite`, and `handoff`; empty counts as unset. | Unset (no task provenance) | `--task <TASK>` |
| `ORBIT_RUN_ID` | Orbit run identifier recorded as `origin.run`; when non-empty, authored writes require `--by` and `sharpen --confirm` is refused. | Unset (no run provenance) | `--run <RUN>` |
| `NEBULA_READ_ONLY` | Set to `1` to refuse all corpus and machine-setting writes while keeping reads available; other non-empty values are refused. | Unset or empty (writes enabled) | - |
| `GIT_CEILING_DIRECTORIES` | Colon-separated directories where `git` repository discovery stops for both `neb` and supervised `git` children. | Unset (no discovery ceiling) | - |
| `HOME` | Home directory for `~/.config/nebula` settings, `~/.nebula` default corpus, and `~/.local/state/nebula/edits`. | None (required when machine paths are needed) | - |
| `PWD` | Shell's logical working directory spelling, used when absolute, clean, and matching the OS cwd by identity. | OS current working directory | - |
| `XDG_STATE_HOME` | Base state directory for kept edits (`$XDG_STATE_HOME/nebula/edits`); relative paths are ignored. | `$HOME/.local/state` | - |
| `NO_COLOR` | Disables ANSI colour output on stdout and stderr when non-empty. | Unset or empty (colour enabled on terminals) | - |
| `TERM` | Terminal type; `dumb` disables ANSI colour output on stdout and stderr. | Unset or non-`dumb` (colour enabled on terminals) | - |
| `VISUAL` | Shell-quoted editor command for `neb edit`, preferred over `EDITOR`; empty counts as unset. | Falls back to `EDITOR` | - |
| `EDITOR` | Fallback shell-quoted editor command for `neb edit` when `VISUAL` is unset or empty. | None (`neb edit` refuses if both are unset) | - |
| `NEBULA_TEST_GIT_DEADLINE_MS` | Debug builds only: supervised `git` deadline in ms for timeout tests; non-integers are ignored. | 30000 ms (120000 ms for `git commit`) | - |
<!-- env-vars:end -->

## Tags

Nodes carry free-form tags, normalised to lowercase kebab-case on every
write. There is no declared list; a write that introduces a tag differing from
one in use only by case or a trailing `s` goes through with a note on stderr
(`note: tag physic is close to physics (2 nodes)`), and `neb check` warns on
any such pair, naming the nodes that carry each. Use a second corpus only for a second owner, which
is what keeps work and personal material apart.

```sh
neb new "Shear law from scarcity" --tag principia --kill "..."
neb tag shear-law-from-scarcity --add orrery
neb tag list
neb list --tag principia --tag orrery
neb status shear-law-from-scarcity refuted --why "..."
neb completions zsh > ~/.zfunc/_neb
```

New title-derived IDs are slugged after Unicode NFC normalization: `Café`
and `Café` produce the same `café` ID. Capture duplicate comparisons also
use NFC. Existing IDs and stored titles are left as written; there is no ID
migration, and references to existing IDs continue to work.

A corpus written by v0.1 is brought forward in place with `neb migrate`.

## CLI results

`neb --help` lists the shipped verbs. Read commands include `show`, `list`,
`inbox`, `near`, `trace`, `impact`, `graph`, `log`, `review` and `check`; writes
include `capture`, `promote`, `drop`, `new`, `edit`, `sharpen`, `status`,
`link`, `tag`, `note`, `cite` and `handoff`. Use `neb review --short` for the
quick glance; `neb open` is a deprecated alias. Put `--no-commit` after a
write verb; the before-verb form is deprecated.

Stdout holds the result alone. List-shaped text is an aligned table on a
terminal and headerless TSV when piped; notices and commit hashes go to
stderr. Typed refusals under `--json` emit `{error, code, hint}` on stderr;
clap parser errors stay prose. Exit 2 means
an argument no corpus could accept (including clap errors); exit 1 means a
state refusal or an error-level `check` finding; exit 0 means success.
Unsafe node or inbox IDs such as `../x` are `unsafe_id` argument refusals
(exit 2), on both reads and writes. `near` accepts arbitrary search text as
well as existing IDs; text that is not an existing ID remains a search query.
`not_regular_file`, `locked`, `io_at` and `unknown_revision` are state
refusals. The [skill's verb references](skills/nebula/SKILL.md#references)
(`verbs-*.md`) list command flags, JSON shapes and error codes.

With `neb config commit on`, a changed corpus write commits only corpus
paths, leaving unrelated staged work alone. If no git work tree contains the
corpus, the write succeeds and stderr says it was not committed. If git
refuses a commit, the write remains on disk; resolve the git problem before
making another corpus change.

## Status

v0.2: the reduced model — four statuses, five edge kinds, references with
notes, tags, and a checker-enforced invariant set. Everything past that is a
guess until roughly fifty real nodes exist.

## Three ways in

`neb` is the CLI documented above. The agent skill in
[skills/nebula/](skills/nebula/SKILL.md) teaches a session-directed or
unattended agent the verbs, their `--json` shapes, the invariants and the two
operating modes — `make skill-link` symlinks it into `~/.claude/skills`. A
desktop app draws the whole corpus as a graph and captures from a global
shortcut on the same `nebula-core`, as documented in
[docs/design/_archived/v0.2/](docs/design/_archived/v0.2/2_architecture.md). All three read and write the same `nodes/` and
`inbox/` — there is one corpus underneath, never three.

## Headless Linux tests

To run the core and CLI tests without building the desktop app:

```sh
cargo test -p nebula-core -p neb --all-targets
```

`make hostile-env-test` runs the same suites under a hostile `HOME`, with
`TMPDIR` inside a git repository and with `GIT_DIR` exported, and checks that
no test touched the host; it includes the desktop crate where WebKitGTK is
installed.

`make test` runs `cargo test --workspace --all-targets`, which includes the
Tauri desktop crate. On Linux, building that crate needs the desktop system
development libraries, including GTK, WebKitGTK, and D-Bus.
