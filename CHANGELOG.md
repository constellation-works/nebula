# Changelog

## 0.1.0 — unreleased

First working version.

- `capture`, `inbox`, `promote`, `drop` — the five-second path and its triage.
- `new`, `sharpen`, `link`, `status`, `graduate` — node lifecycle, with the
  transition guards enforced at the point of action.
- `evidence`, `cite`, `weigh` — findings, context, and promoting one to the other.
- `trace`, `impact`, `open`, `show`, `list` — reading the graph.
- `domain list|add|default|set` — declared domains inside one corpus, with
  `--domain`/`--all` scoping on `list` and `open`. Edges cross domains; only
  the views narrow.
- `check` — fifteen invariants, exiting non-zero on any error.
- `src/` laid out as one module directory per layer, `main.rs` the only file
  at the top.
- `--json` on every read command, for the maintenance loop.
- `show --json` now includes the node's trimmed prose body alongside its metadata.
