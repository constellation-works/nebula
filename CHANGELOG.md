# Changelog

## 0.2.0 — unreleased

The reduced model of `docs/design/v0.2/1_spec.md`. Five days after v0.1 the
corpus held eight nodes and an empty inbox: the machinery was heavier than the
habit. This cut keeps what a person actually uses.

### Removed

- `domain`: the node field, `config.yaml`'s `domains` and `default_domain`,
  `neb domain list|add|default|set`, and `--domain`/`--all` on `list` and
  `open`. Tags do the job and keep everything in one place.
- `evidence`: the node field, `Verdict`, `Strength`, `neb evidence`,
  `neb weigh`, and `promoted_to` on references. A reference with a good note
  carries the same information.
- `tasks`: the node field, `TaskLink`, and `neb task`. Orbit provenance stays
  in `origin`; forward links to work are a reference.
- `graduate`: the verb, the `graduated` status, and `graduated_to`.
  Downstream hand-off happens by the downstream artifact referencing the node.
- Statuses `testing` and `supported`; `Status` is now exactly `seed`,
  `hypothesis`, `refuted`, `abandoned`.
- Edge types `supports`, `undermines`, `depends-on`; `EdgeType` is now exactly
  `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts`.
- `check --online`, the Orbit resolver, and invariants 3, 9, 10, 12, 13 of the
  v0.1 numbering. The checker now implements the ten rules of the v0.2 spec
  under the spec's numbering: 1 acyclic genealogy, 2 hypothesis names a kill,
  3 edge targets exist and no self-loops, 4 `contradicts` is mutual, 5 refuted
  carries `closed.why`, 6 refuted leaves only via `reopens` (enforced at
  `status`), 7 references carry no verdict (enforced at parse), 8 local
  reference URIs resolve (now an **error**, and refused at `cite`), 9 every
  reference has a note (warn), 10 tag drift (warn).
- `new --status`. `new --kill "..."` starts a hypothesis; without it, a seed.
- `open` no longer reports references without a note; that is `check` rule 9.

### Added

- `closed: {why, at}` on nodes. `neb status <id> refuted --why "..."` is
  required and writes it; `neb status <id> abandoned [--why "..."]` writes it
  when given. Moving back to an open status clears it. Refuted is final: the
  only way back is a new node with a `reopens` edge.
- Tags are normalised to lowercase kebab-case on every write path (`new`,
  `promote`, `tag`, `migrate`). `neb tag <id> --add <tag> --remove <tag>`
  edits them; `neb tag list` shows every tag with its node count; `--tag` on
  `list` and `open` is repeatable and requires every tag named.
- `check` rule 10 warns when two tags differ only by case or a trailing `s`.
- `neb migrate`: one-shot, idempotent conversion of a v1 corpus. Refuses when
  the corpus is a git repository with a dirty tree. Reads nodes with a lenient
  private v1 model and re-labels losslessly: `domain` becomes a tag; each
  `evidence[]` entry becomes a `kind: other` reference whose note is prefixed
  `[<verdict>/<strength>]`; each `tasks[]` entry becomes a reference at
  `orbit:<id>` with the state and reason in its note; `supports`,
  `undermines` and `depends-on` edges become references at `neb:<id>`;
  `testing`/`supported` become `hypothesis`; `graduated` becomes `abandoned`
  with `closed.why: "graduated to <uri>"`; a v1 `refuted` node gains a
  `closed` block saying no reason was recorded. `config.yaml` is rewritten to
  `schema_version: 2` with only `corpus_id` and `schema_version`. Prints a
  per-node summary of what changed; a second run rewrites nothing.
- `config.yaml` is `schema_version: 2`; a v1 corpus refuses to open with a
  hint to run `neb migrate`.
- `open` reports hypotheses with no references, seeds untouched for ninety
  days or more, and inbox entries waiting fourteen days or more.
- `impact` reports every descendant (reverse genealogy) plus the node's
  `contradicts` neighbours.
- `review`'s stale-hypothesis section is "untouched for N days" rather than
  "no evidence for N days"; the other three sections are unchanged. Day
  thresholds are inclusive (`>=`) throughout.
- `neb --help` groups are Corpus (`init`, `check`, `migrate`, `completions`),
  Inbox, Nodes (`new`, `sharpen`, `status`, `link`, `tag`), References
  (`cite`), Query, Maintenance.

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
