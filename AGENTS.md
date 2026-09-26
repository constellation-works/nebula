# nebula — agent guide

nebula captures half-formed ideas and traces their lineage. It is a Rust CLI
(`neb`) over a corpus of markdown files.

<!-- constellation-standards:begin -->
<!-- Managed by the constellation's operations/scripts/sync-standards.sh; edits inside this block are overwritten. -->
## Constellation standards

This repository adopts these constellation standards, vendored read-only in `docs/standards/`:

- `STD-01@2` — [docs/standards/STD-01-cli-surface.md](docs/standards/STD-01-cli-surface.md)
- `STD-02@2` — [docs/standards/STD-02-rust-architecture-and-errors.md](docs/standards/STD-02-rust-architecture-and-errors.md)
- `STD-03@2` — [docs/standards/STD-03-concurrency-and-process-safety.md](docs/standards/STD-03-concurrency-and-process-safety.md)
- `STD-04@1` — [docs/standards/STD-04-testing-and-verification.md](docs/standards/STD-04-testing-and-verification.md)
- `STD-05@1` — [docs/standards/STD-05-security-boundaries.md](docs/standards/STD-05-security-boundaries.md)

Follow them; they are normative. To deviate from a rule, record a decision in `docs/design/<feature>/4_decisions.md` citing `STD-nn@<version> §Rn`; never edit `docs/standards/` (`sh docs/standards/check.sh` enforces this).
Reviewers check every change against the adopted standards and report violations as `STD-nn §Rn` with file:line evidence.
<!-- constellation-standards:end -->

## The one rule that matters

**This repository holds the tool. It never holds the corpus.**

The corpus mixes work and personal thinking, and this repository is public.
Never commit node or inbox content here. The corpus is found at runtime through
`--root`, else `$NEBULA_ROOT`, else the nearest corpus at or above the working
directory, else `~/.config/nebula/root`, else `~/.nebula`, and `/corpus` is
gitignored as a backstop. Fixtures for tests belong in a
temporary directory, which is what
`crates/neb/tests/cli.rs` and `crates/nebula-core/tests/core.rs` do.

Tests never touch the host either. `crates/nebula-core/tests/support` gives
each test process a temporary `HOME` and git environment before any test
thread starts, and is the one place a test creates a child: through the
fixture helpers, `neb_command`/`git_command` in `cli.rs`, or
`support::command`. They clear `NEBULA_ROOT`, `OBSERVATORY_ROOT`, the editor,
the Orbit run context, the terminal settings and every `GIT_*`, and stop git
discovery at the fixture's temporary root; a test that needs one of those
sets it on its command after the builder. Each suite fails if a `Command` is
created anywhere else, and every child is owned by a guard that kills it on
drop and waits only up to a deadline. `make hostile-env-test` proves it.

## Working here

```sh
make ci-fast          # pre-handoff gate: fmt, release/standards/terminal/dependency checks, clippy
make test             # every crate: core unit tests + end-to-end CLI tests
make hostile-env-test # the suites under a hostile HOME, TMPDIR and GIT_DIR
make clippy           # --workspace --all-targets --all-features -D warnings
make fmt-check
make types            # regenerate apps/desktop/src/types from nebula-core
```

The workspace is `crates/nebula-core` (the corpus as a library: no terminal,
no clap, typed errors, `Serialize` returns) and `crates/neb` (the CLI: one
core call per verb, plus rendering). Anything that reads or writes the corpus
goes in core; the desktop app and the agent skill must see what the CLI sees.
CI runs all of these on Linux and macOS, and fails if the generated TypeScript
is stale (`make types-check`). Both matter: the checker walks paths, and
macOS temporary directories sit under a symlink, so path handling that resolves
or canonicalizes will pass on one platform and fail on the other. Use paths as
given.

Every file nebula writes goes through `nebula_core::fs`: `write_private_atomic`
(temp file, fsync, rename, directory fsync, `0600`) and `create_private_dir_all`
(`0700`). `clippy.toml` refuses `std::fs::write` and `std::fs::create_dir_all`
elsewhere; test fixtures opt out with an `#[allow]` that gives a reason. Take
the machine-setting lock (`Corpus::lock_machine_settings`) before any corpus
lock, never after one.

Build inputs are pinned (STD-05 §R23). `rust-toolchain.toml` names the exact
toolchain; rustup installs it on first use, and CI's `toolchain:` inputs must
match it. CI actions are pinned by commit SHA, and pnpm by version and sha512
in `apps/desktop/package.json`. Dependabot (`.github/dependabot.yml`) watches
cargo, npm and the actions. `make audit` runs cargo-deny, the pnpm pin check
and `pnpm audit`. Fix an npm advisory with an override in
`apps/desktop/pnpm-workspace.yaml`; one with no fix gets an ignore there with
its reason and a re-review date.

In `crates/nebula-core/src/{model,ops,check}.rs`, invariants live in three
places by design: deserialization, the point of action, and the checker.
Moving a rule between them changes its strength. See
[docs/design/lineage-graph/specs/invariants.md](docs/design/lineage-graph/specs/invariants.md).

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
