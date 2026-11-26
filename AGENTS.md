# nebula — agent guide

nebula captures half-formed ideas and traces their lineage. It is a Rust CLI
(`neb`) over a corpus of markdown files.

## Rules

- Keep corpus data private and outside this public repository; the [repository boundary](README.md#repository-boundary) documents root discovery and commit behavior.
- Build, CI and install inputs follow [STD-05 §R23](docs/standards/STD-05-security-boundaries/STD-05.md#supply-chain-and-consent).

## Gates

- Before handoff, run `make ci-fast`, `make test` and `make hostile-env-test`; `make ci-fast` is the pre-handoff gate and includes formatting, structural, dependency-direction and Clippy checks.
- When core serialized types change, run `make types` to regenerate desktop types, then `make types-check`.
- When dependency inputs change, run `make audit`; desktop advisory handling is in [pnpm-workspace.yaml](apps/desktop/pnpm-workspace.yaml).

## Code

- Corpus reads and writes belong in core. Each surface resolves one `Locations` and passes it down; core never reads the environment or working directory.
- A write is one call into `nebula_core::verb`, which holds the corpus lock across the write and its commit; a read opens `Corpus` and runs one query. See the [verb boundary](crates/nebula-core/src/verb.rs).
- Take the machine-settings lock before any corpus lock, never after; see the [lock implementation](crates/nebula-core/src/lock.rs).
- Use [`nebula_core::fs`](crates/nebula-core/src/fs.rs) for corpus writes and named file reads.
- Keep corpus paths as supplied; do not canonicalize them. The [root-path decision](docs/design/lineage-graph/4_decisions.md#the-root-as-spelled-every-entry-below-it-judged-physically) explains the symlinked-root behavior.
- Invariants live in deserialization, at the point of action and in the checker; moving a rule between them changes its strength. See [invariant placement](docs/design/lineage-graph/specs/invariants.md#where-a-rule-lives-matters).

## Tests

- Tests never touch host state: use fixture isolation before test threads start, set needed environment variables on the command after its builder, and create child processes only through guarded fixture helpers. See [shared test support](crates/nebula-core/tests/support/mod.rs) and the [CLI harness](crates/neb/tests/cli/harness.rs).

## Schema

- Before changing schema, read the [lineage-graph decisions](docs/design/lineage-graph/4_decisions.md).
- Genealogy is a DAG and diamonds are legal. Do not add a `parent` field or nest nodes in directories; see the [DAG decision](docs/design/lineage-graph/4_decisions.md#a-dag-not-a-tree).
- Do not add schema until roughly fifty real nodes exist to justify it.
- The [persistence decision](docs/design/lineage-graph/4_decisions.md#stage-four-is-an-edge-not-a-stage) explains why nodes are never deleted.
- The [reference decision](docs/design/lineage-graph/4_decisions.md#references-cannot-carry-a-verdict-as-it-applied-to-evidence) defines the verdict-free reference shape.
- The [unknown-field decision](docs/design/lineage-graph/4_decisions.md#unknown-fields-are-refused-never-dropped) documents how unsupported schema fields are handled.
- The [capture decision](docs/design/lineage-graph/4_decisions.md#capture-has-a-five-second-budget) sets the capture time and decision budgets.

## Orbit integration

- See the [Origin model](crates/nebula-core/src/model.rs) and [glossary](docs/design/lineage-graph/references/glossary.md#glossary) for producer provenance and spawned-work references.
- Follow the [agent skill's provenance and authorship rules](skills/nebula/SKILL.md#provenance-and-authorship), including its guidance on agent-family names.
- Set Orbit task dependencies with a follow-up `orbit.task.update`; `orbit.task.add` ignores a `dependencies` field.

<!-- constellation-standards:begin -->
<!-- Managed by the constellation's operations/scripts/sync-standards.sh; edits inside this block are overwritten. -->
## Constellation standards

This repository adopts these constellation standards, vendored read-only in `docs/standards/` (each a directory: `STD-nn.md` holds the rules, `why.md` and `checks.md` the reasons and gates):

- `STD-01@4` — [docs/standards/STD-01-cli-surface/STD-01.md](docs/standards/STD-01-cli-surface/STD-01.md): CLI surface — command grammar, inputs and effects, output modes, errors, help, and golden-tested public surfaces
- `STD-02@8` — [docs/standards/STD-02-rust-architecture-and-errors/STD-02.md](docs/standards/STD-02-rust-architecture-and-errors/STD-02.md): Rust architecture and errors — layering, visibility, typed errors, failure semantics, idiom and naming, logging, and…
- `STD-03@2` — [docs/standards/STD-03-concurrency-and-process-safety/STD-03.md](docs/standards/STD-03-concurrency-and-process-safety/STD-03.md): Concurrency and process safety — locks, channels, durable writes, state evolution, subprocess lifecycles, destructive…
- `STD-04@2` — [docs/standards/STD-04-testing-and-verification/STD-04.md](docs/standards/STD-04-testing-and-verification/STD-04.md): Testing and verification — test design, hermetic tests, honest gates, docs as contracts, and evidence-based…
- `STD-05@1` — [docs/standards/STD-05-security-boundaries/STD-05.md](docs/standards/STD-05-security-boundaries/STD-05.md): Security boundaries — authority, filesystem containment, child environments, redaction, local network surfaces, supply…
- `STD-11@3` — [docs/standards/STD-11-agent-context/STD-11.md](docs/standards/STD-11-agent-context/STD-11.md): Agent context files — skills and instruction files loaded by an agent harness

Follow them; they are normative. To deviate from a rule, record a decision in `docs/design/<feature>/4_decisions.md` citing `STD-nn@<version> §Rn`; never edit `docs/standards/` (`sh docs/standards/check.sh` enforces this).
Reviewers check every change against the adopted standards and report violations as `STD-nn §Rn` with file:line evidence.
<!-- constellation-standards:end -->
