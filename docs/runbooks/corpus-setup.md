---
type: runbook
summary: Create a nebula corpus, point the CLI at it, and put it under version control.
tags: [operations, setup, corpus]
paths: ["crates/nebula-core/src/store.rs", "crates/nebula-core/src/config.rs"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-21
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
worth having on its own. The corpus exists on one disk until it has a remote.

The recommended setup is a **private repository at the corpus root** — `git
init` inside the corpus, not in whatever directory contains it:

```sh
git -C "$NEBULA_ROOT" init
git -C "$NEBULA_ROOT" add -A
git -C "$NEBULA_ROOT" commit -m "corpus"
```

A repository at the root means the containing repository's `.gitignore` is
irrelevant: a corpus kept under a vault or a notes checkout that ignores it
(the usual arrangement, since a corpus should not ride along in someone else's
history) still gets its own history, and `neb` can commit to it. A nested
repository is invisible to the outer one beyond a single untracked directory
entry, which the outer ignore rule already hides.

Use a private remote. The corpus mixes work and personal material, and unlike
this repository it is not safe to publish.

```sh
git -C "$NEBULA_ROOT" remote add origin <private-remote>
git -C "$NEBULA_ROOT" push -u origin HEAD
```

`neb migrate` refuses to run against a dirty git tree, so keep the corpus
committed between sessions — see
[migrate-v1-to-v2.md](migrate-v1-to-v2.md) if you are bringing an older corpus
forward.

## Let neb commit for you

Once the corpus is a repository, `neb` can commit after every write:

```sh
neb config commit on
```

This writes `commit: true` to `config.yaml` (off by default, and absent from
the file until turned on). From then on every mutating verb — `capture`,
`promote`, `drop`, `new`, `sharpen`, `status`, `link`, `tag`, `cite`, `note`,
`migrate`, `config` — ends with one commit of the corpus paths, and prints
`committed <hash>` under its usual output:

```
$ neb capture "tags beat domains"
3f2a
committed 8c1d2e0
$ git -C "$NEBULA_ROOT" log --oneline -1
8c1d2e0 neb capture 3f2a
```

What it does, and does not do:

- The commit stages `nodes/`, `inbox/` and `config.yaml` under the corpus
  root and nothing else. Anything else under the root, and everything outside
  it, is left as it was. The message is `neb <verb> <ids>`.
- It never pushes. Push on your own schedule (`git -C "$NEBULA_ROOT" push`),
  or from a cron job, and the remote is your off-disk copy.
- It needs a git identity, like any commit: `git -C "$NEBULA_ROOT" config
  user.name ...` and `user.email` if your global config has none.
- If something outside the corpus is already staged in the repository (which
  can only happen when the corpus is nested in a larger one), the commit is
  refused so that a `neb` commit is always exactly the corpus. The write
  itself is never rolled back; see
  [corpus-recovery.md](corpus-recovery.md#symptom-a-verb-writes-but-refuses-to-commit).
- If the repository around the corpus ignores it, `neb` says so rather than
  silently commit nothing; the fix is the `git init` at the root above.
- `--no-commit` on any verb skips the commit once; the next verb that commits
  sweeps the earlier write up with its own.
- `neb config commit off` turns it off. That last rewrite of `config.yaml` is
  left uncommitted, because off means off; commit it by hand.

`neb config commit` with no argument reports the setting (`--json` gives
`{ "enabled": true }`).

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
| `$NEBULA_ROOT/config.yaml` | corpus id, schema version, `commit: true` when auto-commit is on, and the observatory root when one is set |

Work and personal ideas belong in separate corpora, each with its own
`NEBULA_ROOT`; topics inside one owner's thinking are tags, not separate
corpora.
