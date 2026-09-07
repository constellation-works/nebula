---
type: runbook
summary: Create a nebula corpus, point the CLI at it, and put it under version control.
tags: [operations, setup, corpus]
paths: ["src/corpus/store.rs", "src/corpus/config.rs"]
related_features: [lineage-graph]
related_artifacts: []
last_validated: 2026-09-07
---

# Set Up a Corpus

The corpus holds the nodes and the inbox. It lives outside this repository and is
the only irreplaceable part of the system.

## Create it

```sh
neb init ~/corpus/nebula
```

Then make every command find it:

```sh
echo 'export NEBULA_ROOT=$HOME/corpus/nebula' >> ~/.zshrc
```

A single command can override it with `--root`, which is how the tests run
against throwaway corpora.

## Put it under git

The corpus is append-mostly text, so git is the right backup and the history is
worth having on its own.

```sh
git -C "$NEBULA_ROOT" init
git -C "$NEBULA_ROOT" add -A
git -C "$NEBULA_ROOT" commit -m "corpus"
```

Use a private remote. The corpus mixes work and personal material, and unlike
this repository it is not safe to publish.

## Declare domains

A fresh corpus has one domain, `general`, and never asks about it. Declare more
once the corpus spans areas you want to look at separately:

```sh
neb domain add principia
neb domain add economics
neb domain default principia
```

With several domains declared, `new` and `promote` take `--domain` and fall
back to the default; `list` and `open` narrow to the default and take `--all`
to cross. `trace` and `impact` always cross, since edges between domains are
the point.

Use a second corpus only for a second owner. Work and personal ideas belong in
separate corpora, each with its own `NEBULA_ROOT`; principia and economics
belong in one.

## Shell completions

For zsh, generate the completion function and load it from your zsh startup:

```sh
mkdir -p ~/.zfunc
neb completions zsh > ~/.zfunc/_neb
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

For bash:

```sh
neb completions bash > ~/.neb-completion.bash
source ~/.neb-completion.bash
```

For fish:

```sh
neb completions fish > ~/.config/fish/completions/neb.fish
```

## Verify

```sh
neb capture "checking the setup works"
neb inbox
neb check
```

`check` on an empty corpus reports zero nodes, zero errors, and exits zero.

## Where things are

| path | holds |
|---|---|
| `$NEBULA_ROOT/nodes/<id>.md` | one node, with its edges, evidence and prose |
| `$NEBULA_ROOT/inbox/YYYY-MM.md` | captures, append-only, struck through when settled |
| `$NEBULA_ROOT/config.yaml` | corpus id, declared domains, default domain |
