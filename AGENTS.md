# nebula — agent guide

nebula captures half-formed ideas and traces their lineage. It is a Rust CLI
(`neb`) over a corpus of markdown files.

## The one rule that matters

**This repository holds the tool. It never holds the corpus.**

The corpus mixes work and personal thinking, and this repository is public.
Never commit node or inbox content here. The corpus is found at runtime through
`--root`, else `NEBULA_ROOT`, else `~/.nebula`, and `/corpus` is gitignored as a
backstop. Fixtures for tests belong in a temporary directory, which is what
`crates/neb/tests/cli.rs` and `crates/nebula-core/tests/core.rs` do.

## Working here

```sh
make test             # every crate: core unit tests + end-to-end CLI tests
make clippy           # --workspace --all-targets --all-features -D warnings
make fmt-check
make types            # regenerate apps/desktop/src/types from nebula-core
```

The workspace is `crates/nebula-core` (the corpus as a library: no terminal,
no clap, typed errors, `Serialize` returns) and `crates/neb` (the CLI: one
core call per verb, plus rendering). Anything that reads or writes the corpus
goes in core; the desktop app and the agent skill must see what the CLI sees.
CI runs all four on Linux and macOS, and fails if the generated TypeScript is
stale. Both matter: the checker walks paths, and
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

v0.2 is intentionally minimal — it *removed* schema. Do not add any until
roughly fifty real nodes exist to justify it.

## Orbit integration

`origin` records what produced a node; work it spawned is a reference. Two
hazards, both learned elsewhere in the constellation:

- Never hardcode an agent family. orbit-research pinned `codex` into its task
  lookup and the crew names changed underneath it.
- `orbit.task.add` silently ignores a `dependencies` field. Wire dependencies
  with a follow-up `task.update`.
