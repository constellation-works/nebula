# nebula — agent guide

This repository contains `neb`, a Rust CLI for capturing ideas and tracing their lineage in a Markdown corpus.

## Rules

- The corpus may contain personal and work material. Never commit node or inbox content in this repository.
- Before changing node shape or capture behavior, read [design decisions](docs/design/lineage-graph/4_decisions.md) and the [invariants spec](docs/design/lineage-graph/specs/invariants.md#where-a-rule-lives-matters); they state schema constraints, reversal conditions and where each invariant is enforced. The [vision](docs/design/lineage-graph/3_vision.md#deliberately-out-of-scope) says when to expand beyond capture.
- Resolve corpus location once at each surface boundary and pass it through. See [corpus setup](docs/runbooks/corpus-setup.md) for resolution order and setup.

## Gates

- Run `make ci-fast` before handoff. Run `make test` for corpus or CLI behavior changes, `make hostile-env-test` for process or environment boundary changes, and `make types` plus `make types-check` when changing core types exported to the desktop.
- After dependency changes, run `make audit` and follow [STD-05 §R23](docs/standards/STD-05-security-boundaries/STD-05.md#supply-chain-and-consent) for pinned inputs. Group minor and patch updates by ecosystem; ship major updates with their required changes or a reasoned ignore. For an npm advisory without a fix, record an ignore reason and review date in `apps/desktop/pnpm-workspace.yaml`.

## Code

- Keep corpus reads and writes in `nebula-core`; each surface builds `Locations` at startup, and core never reads the environment or working directory. The [crate design](docs/design/lineage-graph/2_design.md#crates) defines the dependency boundary; [location handling](crates/nebula-core/src/locations.rs) defines the resolver.
- Product writes go through [the core filesystem helpers](crates/nebula-core/src/fs.rs), and entries below the corpus root use their regular-file readers. A root may be symlinked, but entries below it may not; keep paths as supplied. See [filesystem boundaries](docs/design/lineage-graph/4_decisions.md#the-root-as-spelled-every-entry-below-it-judged-physically).
- Take the machine-settings lock before any corpus lock; [lock.rs](crates/nebula-core/src/lock.rs) documents the order.

## Tests

- Tests use temporary corpora and isolate host state. Create children only through fixture command helpers, which isolate the environment and own each child through a bounded guard; set test-specific environment on the built command. See [core test support](crates/nebula-core/tests/support/mod.rs) and the [CLI harness](crates/neb/tests/cli/harness.rs).

## Orbit integration

- `origin` records what produced a node; represent work it spawns as a reference. Follow the skill's [provenance and authorship rules](skills/nebula/SKILL.md#provenance-and-authorship), including its rule against hardcoding an agent family.
- `orbit.task.add` ignores `dependencies`; wire task dependencies with a follow-up `orbit.task.update`.

<!-- constellation-standards:begin -->
<!-- Managed by the constellation's operations/scripts/sync-standards.sh; edits inside this block are overwritten. -->
## Constellation standards

This repository adopts these constellation standards, vendored read-only in `docs/standards/` (each a directory: `STD-nn.md` holds the rules, `why.md` and `checks.md` the reasons and gates):

- `STD-01@2` — [docs/standards/STD-01-cli-surface/STD-01.md](docs/standards/STD-01-cli-surface/STD-01.md)
- `STD-02@4` — [docs/standards/STD-02-rust-architecture-and-errors/STD-02.md](docs/standards/STD-02-rust-architecture-and-errors/STD-02.md)
- `STD-03@2` — [docs/standards/STD-03-concurrency-and-process-safety/STD-03.md](docs/standards/STD-03-concurrency-and-process-safety/STD-03.md)
- `STD-04@2` — [docs/standards/STD-04-testing-and-verification/STD-04.md](docs/standards/STD-04-testing-and-verification/STD-04.md)
- `STD-05@1` — [docs/standards/STD-05-security-boundaries/STD-05.md](docs/standards/STD-05-security-boundaries/STD-05.md)

Follow them; they are normative. To deviate from a rule, record a decision in `docs/design/<feature>/4_decisions.md` citing `STD-nn@<version> §Rn`; never edit `docs/standards/` (`sh docs/standards/check.sh` enforces this).
Reviewers check every change against the adopted standards and report violations as `STD-nn §Rn` with file:line evidence.
<!-- constellation-standards:end -->
