---
type: runbook
summary: Declare domains, move nodes between them, and place nodes in a corpus that predates domains.
tags: [operations, domains, corpus]
paths: ["src/corpus/config.rs", "src/commands/domain.rs"]
related_features: [lineage-graph]
related_artifacts: []
last_validated: 2026-09-07
---

# Domains

A domain is the one declared area a node belongs to. It narrows what you look
at; it never restricts what you can link. Declared domains live in
`$NEBULA_ROOT/config.yaml`, and `neb domain` is the only thing that should edit
that file.

## See what is declared

```sh
neb domain list
```

Each domain prints with its node count and the default is marked. A trailing
line reports nodes that name no declared domain, which is the migration case
below.

## Declare one

```sh
neb domain add ranking
```

Names are lowercase letters, digits and dashes. Adding a second domain to a
corpus that had one changes what bare `list` and `open` show: nothing, until a
default is set, in the sense that they show everything and say so.

```sh
neb domain default personal
```

## Create nodes in a domain

```sh
neb new "Decay is a half-life" --domain ranking
neb promote 3f2a --domain ranking
```

Without `--domain`, both fall back to the default. `capture` never takes a
domain: that is the five-second path, and the domain is decided at promotion.

## Look at one domain, or all of them

```sh
neb list                  # the default domain, when several are declared
neb list --domain ranking
neb list --all
neb open --all
```

`trace`, `impact` and `check` always cover the whole corpus. Scoping them
would hide exactly the cross-domain edges the graph exists to surface.

## Move a node

```sh
neb domain set decay-is-a-half-life economics
```

Edges are untouched. A node's domain says where it sits in your attention, not
what it descends from.

## Place nodes in a corpus that predates domains

A corpus created before `config.yaml` existed loads with a single `general`
domain and nodes that name none. `neb check` reports each as rule 15, and
`neb domain list` counts them. Place them:

```sh
neb list --all --json | jq -r '.[] | select(.domain == null) | .id' \
  | xargs -I{} neb domain set {} general
```

Then declare the domains you actually want and move nodes into them.

## When to use a second corpus instead

Only for a second owner. Work and personal ideas have different owners, backups
and legal standing, so they are separate corpora pointed at by separate
`NEBULA_ROOT` values. Topics inside one owner's thinking are domains. A lineage
that genuinely crosses the ownership line is recorded as a reference on the
receiving node, described in enough words to be findable and no more.
