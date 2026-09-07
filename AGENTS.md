# nebula — agent guide

nebula captures half-formed ideas and traces their lineage. It is a Rust CLI
(`neb`) over a corpus of markdown files.

## The one rule that matters

**This repository holds the tool. It never holds the corpus.**

The corpus mixes work and personal thinking, and this repository is public.
Never commit node or inbox content here. The corpus is found at runtime through
`--root`, else `NEBULA_ROOT`, else `~/.nebula`, and `/corpus` is gitignored as a
backstop. Fixtures for tests belong in a temporary directory, which is what
`tests/cli.rs` does.

## Working here

```sh
cargo test            # 22 end-to-end tests over throwaway corpora
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI runs all three on Linux and macOS. Both matter: the checker walks paths, and
macOS temporary directories sit under a symlink, so path handling that resolves
or canonicalizes will pass on one platform and fail on the other. Use paths as
given.

## Before changing the schema

Read [docs/design/lineage-graph/4_decisions.md](docs/design/lineage-graph/4_decisions.md)
first. Most of the schema's shape is a deliberate choice with a stated condition
that would reverse it. In particular:

- Genealogy is a DAG. Diamonds are legal and expected. Do not add a `parent`
  field or nest nodes in directories.
- Nothing is ever deleted. Refuted, abandoned and dropped things persist.
- References may not carry a verdict. This is enforced by `deny_unknown_fields`,
  not by a check, and that is deliberate.
- Capture must stay under five seconds and require no decisions.

v0.1 is intentionally minimal. Do not add schema until roughly fifty real nodes
exist to justify it.

## Orbit integration

`origin` records what produced a node; `tasks` records work it spawned. Two
hazards, both learned elsewhere in the constellation:

- Never hardcode an agent family. orbit-research pinned `codex` into its task
  lookup and the crew names changed underneath it.
- `orbit.task.add` silently ignores a `dependencies` field. Wire dependencies
  with a follow-up `task.update`.
