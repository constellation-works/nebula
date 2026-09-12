---
title: v0.2 — Architecture
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: v0.2
doc_role: design
type: design
summary: Cargo workspace with a pure core library, a thin CLI, a Tauri desktop app and an agent skill, and the layering rules that keep them separable as the software grows.
tags: [v0.2, architecture]
paths: ["Cargo.toml", "crates/**", "apps/**", "skills/**"]
related_features: [v0.2, desktop, skill]
related_artifacts: []
---

# v0.2 — Architecture

Three consumers now sit on one corpus: a CLI a human types at, an agent that
runs the CLI, and a desktop app that draws the graph. The v0.1 layering promise
("`corpus` knows nothing above it, so it could become a library") is cashed
here, because a second consumer has appeared.

## Repository layout

```
nebula/
  Cargo.toml                 # [workspace] members = crates/*, apps/desktop/src-tauri
  crates/
    nebula-core/             # the library: model, store, graph, ops, check, migrate
    neb/                     # the CLI binary: clap + render, nothing else
  apps/
    desktop/                 # Tauri 2 + React/TypeScript/Vite
      src/                   # frontend
      src-tauri/             # Rust shell; depends on nebula-core directly
  skills/
    nebula/SKILL.md          # the agent skill; repo-shipped, symlinked into a checkout's .claude/skills/
  docs/
  scripts/
  Makefile
```

The corpus still never lives here. `NEBULA_ROOT` / `--root` / `~/.nebula`
resolution is unchanged and lives in `nebula-core::store`.

## `nebula-core`

The rule: **core is pure with respect to the outside world's presentation.**
It reads and writes corpus files and nothing else. No terminal, no colour, no
clap, no `println!`, no `anyhow` in the public API, no process exit codes.

```
nebula-core/src/
  lib.rs          # re-exports; the public API is exactly what lib.rs names
  error.rs        # `Error` enum (thiserror); every public fn returns Result<T, Error>
  model.rs        # Node, Status, Edge, EdgeType, Reference, Origin, Closed, InboxEntry
                  # serde with deny_unknown_fields; this is the file format
  store.rs        # Corpus: open/init, load/save nodes, inbox append/settle, config
  graph.rs        # Graph: an indexed snapshot of loaded nodes (by_id, parents, children)
                  # + pure queries: trace, impact, open, review, export
  ops.rs          # mutations: capture, promote, drop, new, sharpen, link, cite,
                  # set_status, tag — each enforces its point-of-action invariants
                  # and returns the changed Node(s)
  check.rs        # rules over a Graph → Vec<Finding { rule, severity, node, message }>
  migrate.rs      # v1 → v2, with its own lenient v1 model kept private to this module
```

Design rules for core:

- **Return data, never text.** Every query and op returns a `serde::Serialize`
  struct. The CLI's `--json` output and the desktop's IPC payload are the same
  value; there is no second schema to drift.
- **Queries are pure over a `Graph`.** `Graph::from(&[Node])` is built once per
  command; `trace`, `impact`, `open`, `review`, `export` take `&Graph` and
  allocate nothing global. This is what lets the desktop keep a `Graph` in
  memory and rebuild it on a file-watch event.
- **Ops take `&Corpus` and typed args, and validate before they write.** A
  caller cannot produce an invalid corpus through core; `check` exists to catch
  hand edits, not core bugs.
- **Typed errors.** `Error::Cycle { from, to }`, `Error::NeedsKill(status)`,
  `Error::NoSuchNode(id)`, `Error::RefutedNeedsWhy`, `Error::Io`, ... The CLI
  maps them to messages and exit codes; the desktop maps them to UI. Neither
  parses strings.
- **`ts` feature.** `#[cfg_attr(feature = "ts", derive(ts_rs::TS))]` on every
  exported type, so `apps/desktop/src/types/` is generated from Rust and the
  frontend never hand-writes a shape.
- No `pub` module internals beyond what `lib.rs` re-exports. A consumer that
  needs more is a signal to add an API, not to reach in.

## `neb`

```
neb/src/
  main.rs         # ExitCode from cli::run()
  cli.rs          # clap tree; the only file that knows clap
  render/         # text rendering of core types: tree, table, badges, colour
```

`cli.rs` dispatch is a table: parse args → call one core fn → either
`serde_json::to_writer` (`--json`) or `render::*`. A command that does anything
else belongs in core. Shell completions stay.

## `apps/desktop`

Tauri 2, React 19, TypeScript, Vite. Menu-bar app, no dock icon, one window
that shows one of two views.

- **`src-tauri`** links `nebula-core` (no sidecar, no shelling out). Exposes
  commands `capture(text)`, `inbox()`, `graph()`, `node(id)`, `open_in_editor(id)`,
  `corpus_path()`. Holds a `notify` watcher on `nodes/` and `inbox/` and emits a
  `corpus-changed` event; the frontend refetches on it. Global shortcut
  (`tauri-plugin-global-shortcut`) toggles a floating capture window; tray icon
  shows the inbox count.
- **Frontend** is read-only except capture. Views: *Inbox* (capture box, list)
  and *Graph* (whole-corpus DAG via `elkjs` layered layout, genealogy edges
  solid, `contradicts` dashed, status → colour, tag filter, click → side panel
  with rendered markdown and the reference table). State is one store keyed
  on the `graph()` export. No triage buttons, no editing forms: the agent does
  those in a session.
- Types under `src/types/` are generated by `cargo run -p nebula-core --features ts --bin gen-types`
  (or the `ts-rs` export test) and checked in; CI fails if they drift.
- Not signed or notarised; `make desktop` produces a local `.app`.

## `skills/nebula`

One `SKILL.md` plus references. It teaches an agent the corpus location, the
verbs and their `--json` shapes, the invariants, and two modes:

- **Session mode** (a human is present and directing): act — triage the inbox,
  sharpen, link, cite, move status — run `neb check` after every write, report
  the result. The human is the author; the agent is the hands.
- **Routine mode** (unattended, an Orbit routine): `neb review` and inbox
  reading produce *proposals only* in `review.md`, one line of reasoning each.
  Never writes to `nodes/`. This preserves the v0.1 trust rule where it matters.

Provenance: always pass `--task`/`--run` when running under Orbit; never
hardcode an agent family.

## Layering, enforced

```
apps/desktop  ─┐
neb           ─┼──▶ nebula-core ──▶ (corpus files)
skills/nebula ─┘   (via neb)
```

Dependencies point down only. `nebula-core` has no dependency on any other
workspace member. The workspace `Cargo.toml` and clippy `-D warnings` are the
enforcement; a `use neb::` inside core is a compile error because core does
not depend on it.

## Why this shape

- A pure core with typed returns is the cheapest thing that lets a CLI, a GUI
  and an agent agree on what the corpus says. Sidecars and `--json` parsing
  would have worked for one more consumer and then rotted.
- Tauri over Electron or SwiftUI: the core is Rust, so the app links it rather
  than talking to it; a webview is the right tool for a DAG with markdown in it.
- Generated TS types over hand-written: the desktop is the consumer most likely
  to drift silently, because nothing runs its shapes against the corpus on CI.
- Skill over UI widgets: the agent already has the CLI; forms would be a second
  implementation of the same verbs with a worse audit trail.
