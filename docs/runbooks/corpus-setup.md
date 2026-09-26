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
neb init ~/corpus/nebula --set-root
```

`--set-root` writes this non-default location to
`~/.config/nebula/root`, so every command finds it even from shells that do
not load your shell profile. The path must be absolute: a relative path is
refused before either the corpus or the setting is created because it would
name a different directory after changing shells or working directories.
Absolute paths are stored as written, including symlink spellings. Plain
initialization (`neb init` without `--set-root`) never changes that machine-wide
setting: when it is absent, the command prints the opt-in command above. If the
file already names a different corpus, `--set-root` refuses to replace it;
review the two paths and pass `--force` only when redirecting the machine is
intentional.
The resolver checks `--root`, `$NEBULA_ROOT`, the current directory, that
config file, then `~/.nebula` in that order. The current directory counts when
it is a corpus or anywhere under one: the nearest directory at or above it that
holds `nodes/` beside a `config.yaml` naming a `corpus_id` is used, the way git
finds a repository, and the innermost of two nested corpora wins. The walk
follows the directory as the shell spells it (`$PWD`) and never resolves a
symlink. Only an existing corpus is found this way, so `neb capture` never
creates one because of where it was run.

An exported-but-empty `$NEBULA_ROOT` is treated as unset, not as the current
directory itself, so it falls through to the steps after it; an explicit
`--root ""` is refused outright rather than resolving to the current directory. You can still export the environment
variable when you want a shell-specific override:

```sh
echo 'export NEBULA_ROOT=$HOME/corpus/nebula' >> ~/.zshrc
```

A single command can override it with `--root`, which is how tests and agents
initialize throwaway corpora. Those commands omit `--set-root` so a scratch
corpus can never claim the machine default.

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

`neb init` writes `/.lock` to the corpus `.gitignore`, so the advisory runtime
lock never appears in `git status` or gets swept up by `git add -A`. For a
corpus created by an older `neb`, run `neb init "$NEBULA_ROOT"` once; it keeps
the corpus and existing ignore rules intact while adding the missing rule.
Git does not honor a symlink at `.gitignore`, so `neb init` replaces one with
an effective regular file in the corpus: a readable target's bytes are copied
before `/.lock` is added when needed, while a dangling link becomes a local
file containing only `/.lock`. The external target is never changed. If the
link cannot be read for another reason, initialization fails instead of
claiming the lock is ignored.

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
`promote`, `drop`, `new`, `sharpen`, `status`, `link`, `tag`, `cite`,
`handoff`, `note`, `migrate`, `config` — ends with one commit of the corpus
paths (`triage` makes one per promote or drop it carries out), and prints
`committed <hash>` on stderr under its usual output, so `E=$(neb capture -q …)`
still holds the id alone:

```
$ neb capture "tags beat domains"
3f2a
committed 8c1d2e0
$ git -C "$NEBULA_ROOT" log --oneline -1
8c1d2e0 neb capture 3f2a
```

What it does, and does not do:

- The commit stages `nodes/`, `inbox/`, `config.yaml` and the generated
  `.gitignore` under the corpus root and nothing else. Anything else under the
  root, and everything outside it, is left as it was. The message is
  `neb <verb> <ids>`.
- It never pushes. Push on your own schedule (`git -C "$NEBULA_ROOT" push`),
  or from a cron job, and the remote is your off-disk copy.
- It needs a git identity, like any commit: `git -C "$NEBULA_ROOT" config
  user.name ...` and `user.email` if your global config has none.
- If something outside the managed corpus paths (`nodes/`, `inbox/`,
  `config.yaml`, and the generated `.gitignore`) is already staged in the
  repository, the commit is refused so that a `neb` commit is always exactly
  the corpus. This includes an unrelated file in a repository rooted at the
  corpus as well as a change elsewhere in a larger enclosing repository. The
  write itself is never rolled back; see
  [corpus-recovery.md](corpus-recovery.md#symptom-a-verb-writes-but-refuses-to-commit).
- If the repository around the corpus ignores it, `neb` says so rather than
  silently commit nothing; the fix is the `git init` at the root above.
- `--no-commit` on any verb that writes skips the commit once; the next verb that commits
  sweeps the earlier write up with its own.
- `neb config commit off` turns it off. That last rewrite of `config.yaml` is
  left uncommitted, because off means off; commit it by hand.

`neb config commit` with no argument reports the setting (`--json` gives
`{ "enabled": true }`).

## Tag as you go

A fresh corpus has no declared structure to set up: tags are free-form,
normalised to lowercase kebab-case on write, and there is nothing to
initialize before using them. A new tag that differs from one already in use
only by case or a trailing `s` is still written, with a note on stderr
naming the existing one; reuse that one unless the difference is meant.

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
| `$NEBULA_ROOT/config.yaml` | corpus id, schema version, and `commit: true` when auto-commit is on (a corpus written by an older `neb` may also carry a legacy `observatory_root`) |
| `~/.config/nebula/root` | this machine's default corpus, written by `neb init <DIR> --set-root` |
| `~/.config/nebula/observatory-root` | this machine's Observatory checkout, written by `neb config observatory-root <DIR>`; `$OBSERVATORY_ROOT` outranks it |
| `$NEBULA_ROOT/.lock`, `~/.config/nebula/.lock` | the advisory write locks; runtime state, never committed, never to be deleted by hand |

Every file `neb` writes is created `0600` and every directory it creates
`0700`, whatever your umask, because a corpus mixes work and personal
thinking. A directory that already existed, such as a corpus root you made
yourself, keeps the mode you gave it.

Work and personal ideas belong in separate corpora, each with its own
`NEBULA_ROOT`; topics inside one owner's thinking are tags, not separate
corpora.
