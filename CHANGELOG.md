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
- `neb --help` groups the commands into Corpus, Inbox, Nodes, References,
  Query and Maintenance sections; the `help` subcommand is gone, use `--help`.
- `review [--since <days>] [--out <path>]` — the deterministic half of the weekly
  maintenance pass: stale hypotheses, untouched seeds, nodes with no references,
  and stale inbox entries, as Markdown or `--json`. Read-only: it never edits a
  node, an inbox entry, or the manifest.
