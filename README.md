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

```
nebula   = the CLI, the checker, the index builder   (this repo)
corpus   = nodes/ and inbox/                          (elsewhere, private)
```

## Tags

Nodes carry free-form tags, normalised to lowercase kebab-case on every
write. There is no declared list; `neb check` warns when two tags differ only
by case or a trailing `s`. Use a second corpus only for a second owner, which
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

## Status

v0.2: the reduced model — four statuses, five edge kinds, references with
notes, tags, and ten invariants. Everything past that is a guess until
roughly fifty real nodes exist.

## Three ways in

`neb` is the CLI documented above, and the only one that exists today. Two
more consumers are being built on the same corpus, both scoped in
[docs/design/v0.2/](docs/design/v0.2/2_architecture.md): an agent skill that
teaches a session-directed or unattended agent the verbs and their `--json`
shapes, and a desktop app that draws the whole corpus as a graph and captures
from a global shortcut. All three read and write the same `nodes/` and
`inbox/` — there is one corpus underneath, never three.
