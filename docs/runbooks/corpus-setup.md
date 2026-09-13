---
type: runbook
summary: Create a nebula corpus, point the CLI at it, and put it under version control.
tags: [operations, setup, corpus]
paths: ["crates/nebula-core/src/store.rs", "crates/nebula-core/src/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-12
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

`neb migrate` refuses to run against a dirty git tree, so keep the corpus
committed between sessions — see
[migrate-v1-to-v2.md](migrate-v1-to-v2.md) if you are bringing an older corpus
forward.

## Tag as you go

A fresh corpus has no declared structure to set up: tags are free-form,
normalised to lowercase kebab-case on write, and there is nothing to
initialize before using them.

```sh
neb new "Shear law from scarcity" --tag principia --kill "..."
neb tag shear-law-from-scarcity --add orrery
neb tag list
neb list --tag principia --tag orrery
```

Use a second corpus only for a second owner — see "Where things are" below.

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
| `$NEBULA_ROOT/nodes/<id>.md` | one node, with its edges, references and prose |
| `$NEBULA_ROOT/inbox/YYYY-MM.md` | captures, append-only, struck through when settled |
| `$NEBULA_ROOT/config.yaml` | corpus id and schema version, nothing else |

Work and personal ideas belong in separate corpora, each with its own
`NEBULA_ROOT`; topics inside one owner's thinking are tags, not separate
corpora.
