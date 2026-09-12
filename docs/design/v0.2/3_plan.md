---
title: v0.2 — Delivery plan
owner: claude
last_updated: 2026-09-12
last_validated: 2026-09-12
status: Accepted
feature: v0.2
doc_role: plan
type: design
summary: The ordered tasks that deliver v0.2, their dependencies, and what each must leave behind.
tags: [v0.2]
paths: []
related_features: [v0.2]
related_artifacts: []
---

# v0.2 — Delivery plan

Serial where files collide, parallel where they do not. Each task is an Orbit
task in `ws_nebula` tagged `v0.2`; ids are recorded here once filed.

| # | task | depends on | crew |
|---|---|---|---|
| A | Schema cut + `neb migrate` ([1_spec.md](1_spec.md) "What is removed", "Migration") | — | opus |
| B | Cargo workspace: `crates/nebula-core` + `crates/neb`, typed errors, `graph` module, `neb graph --json` ([2_architecture.md](2_architecture.md)) | A | opus |
| C | Docs rewrite: `docs/spec.md`, `docs/design/lineage-graph/*`, runbooks, README, CHANGELOG to v0.2 | A | sonnet |
| D | `skills/nebula/SKILL.md` + references | A | sonnet |
| E | Desktop scaffold: Tauri 2 + React, `src-tauri` on core, tray, global-shortcut capture, inbox view, CI for the frontend | B | opus |
| F | Desktop graph view: elkjs layout, node panel, tag filter, file watcher | E | opus |

Corpus migration of the private corpus (`observatory/knowledgebase/lineage`) is
run by the orchestrator after A merges; it is not a task in this repository.

Definition of done for the milestone: `cargo test --workspace` and clippy clean
on both CI platforms; `neb check` clean on the migrated corpus; the desktop app
captures to that corpus and draws it; the skill runs a session-mode triage
end to end.
