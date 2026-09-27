# nebula

Capture a half-formed thought in under five seconds. Trace where any idea came
from years later.

An idea arrives vague, at random, in whatever field you happen to be thinking
about. Nebula gives it a home before it is good enough for anywhere else, then
records how it descended into whatever it became. Ideas branch, merge and die,
so the structure is a directed acyclic graph rather than a tree, and dead
branches are kept forever because they are what stops you re-treading ground.

The v0.2 model is in [docs/design/v0.2/1_spec.md](docs/design/v0.2/1_spec.md);
[docs/spec.md](docs/spec.md) is the original design record, kept as history.

## Repository boundary

This repository holds **the tool only**. It never holds the corpus.

The corpus spans work and personal thinking across unrelated fields, so it
lives outside, configured by path, and it stays private regardless of where
this repository ends up. Keeping them apart means this code can be published
without a decision about the notes, and the notes can be backed up without
dragging a build tree along.

Every command finds the corpus through `--root`, else `$NEBULA_ROOT`, else the
nearest corpus at or above the current directory (a `nodes/` beside a
`config.yaml` naming a `corpus_id`, found the way git finds a repository), else
the path in `~/.config/nebula/root`, else `~/.nebula`. Only an existing corpus
is found from the current directory, so `neb capture` can create one only at an
explicit or configured root, and it says so when it does.

`ORBIT_TASK_ID` and `ORBIT_RUN_ID` supply missing provenance for `new`,
`promote`, `cite` and `handoff`; explicit `--task` and `--run` values win.
When `ORBIT_RUN_ID` is nonempty, writes of new words require an explicit
`--by`, and `sharpen --confirm` is reserved for a human outside the run.
Set `NEBULA_READ_ONLY=1` to refuse all corpus writes while keeping reads
available. An empty or unset value leaves writes enabled; other values are
refused.

```
nebula   = the CLI, checker and desktop app           (this repo)
corpus   = nodes/ and inbox/                          (elsewhere, private)
```

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
`not_regular_file`, `locked`, `io_at` and `unknown_revision` are state
refusals. The [skill's verb reference](skills/nebula/references/verbs.md)
lists command flags, JSON shapes and error codes.

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
[docs/design/v0.2/](docs/design/v0.2/2_architecture.md). All three read and write the same `nodes/` and
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
