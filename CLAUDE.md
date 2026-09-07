# nebula — Claude-specific guide

Read [AGENTS.md](AGENTS.md) first; it carries the rules every provider needs.

The single hard constraint: this repository is the tool, never the corpus. The
corpus is private and lives outside. Do not commit node or inbox content here,
and do not create fixture corpora inside the checkout.

When touching `src/check/`, note that invariants live in three places by
design: deserialization, the point of action, and the checker. Moving a rule
between them changes its strength. The reasoning per rule is in
[docs/design/lineage-graph/specs/invariants.md](docs/design/lineage-graph/specs/invariants.md).
