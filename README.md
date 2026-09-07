# nebula

Capture a half-formed thought in under five seconds. Trace where any idea came
from years later.

An idea arrives vague, at random, in whatever domain you happen to be thinking
about. Nebula gives it a home before it is good enough for anywhere else, then
records how it descended into whatever it became. Ideas branch, merge and die,
so the structure is a directed acyclic graph rather than a tree, and dead
branches are kept forever because they are what stops you re-treading ground.

The design is in [docs/spec.md](docs/spec.md).

## Repository boundary

This repository holds **the tool only**. It never holds the corpus.

The corpus spans work and personal thinking across unrelated domains, so it
lives outside, configured by path, and it stays private regardless of where
this repository ends up. Keeping them apart means this code can be published
without a decision about the notes, and the notes can be backed up without
dragging a build tree along.

```
nebula   = the CLI, the checker, the index builder   (this repo)
corpus   = nodes/ and inbox/                          (elsewhere, private)
```

## Status

v0.1 is deliberately small: `capture`, `promote`, `cite`, `trace`, `check`.
Everything else in the spec is a guess until roughly fifty real nodes exist.
