# Verbs: corpus and review

Read this for `init`, `check`, `migrate`, `config`, commits, and the `review`
reports.

Contents:

- [Corpus](#corpus)
- [Maintenance](#maintenance)

## Corpus

| verb | does | flags |
|---|---|---|
| `neb init [PATH]` | create an empty corpus without changing the machine default | `--set-root`, `--force` (requires `--set-root`) |
| `neb check` | run the rules in [invariants-rules.md](invariants-rules.md); exit non-zero on any error | — |
| `neb migrate` | v1 → v2 in place; idempotent; refuses on a dirty git tree | — |
| `neb config observatory-root [DIR]` | read or set where the Observatory checkout is on this machine | `--drop-legacy` |
| `neb config commit [on\|off]` | read or set whether each write is committed to the corpus's git repository | — |

`config commit` rewrites `config.yaml` whole when the setting changes; `init`,
`migrate`, and `capture` when it initializes a corpus can write it too. The
file stays machine-written and is never hand-edited.

`config observatory-root DIR` writes this machine's
`~/.config/nebula/observatory-root` and nothing in the corpus, so it commits
nothing; `DIR` must be absolute. Without `DIR` it prints the effective root and
which setting supplied it: `$OBSERVATORY_ROOT`, else this machine's setting,
else the legacy `observatory_root` key an older `neb` wrote into `config.yaml`,
else nothing.

The legacy key is deprecated: a verb that resolves a record through it (`show`,
`cite --kind observatory`, `handoff`) warns on stderr, naming it and `neb config
observatory-root <DIR>`. `legacy` names that key whenever the file still carries
it, in force or not.

`--drop-legacy` removes it from `config.yaml` (a corpus write, committed as `neb
config observatory-root` when `commit` is on); run it once every machine that
shares the corpus has its own setting.

It says on stderr, in every mode, what it did: `removed the legacy
observatory_root (<path>) from <root>/config.yaml`, or, when there is no key,
`no legacy observatory_root key in <root>/config.yaml; nothing removed`, and
then the file is not rewritten and nothing is committed.

```json
// neb init /Users/you/.nebula --json
{ "root": "/Users/you/.nebula" }
```

`--root` and `PATH` name the same thing, so `neb --root A init B` with two
different directories is refused before either is created
(`root_and_path_differ`, exit 2); the same directory given both ways is fine.

Plain `init` never writes `~/.config/nebula/root`. For a primary non-default
corpus, pass `--set-root`; it refuses to replace a setting that names another
corpus unless `--force` is also explicit. Scratch corpora use `--root` and
never `--set-root`. The setting must hold one absolute path: an empty or
relative one is refused by every command that reads it
(`empty_root_setting`, `relative_root_setting`), and `--set-root` replaces
such a setting without `--force`, since it names no corpus.

```json
// neb config observatory-root --json     (source: env | machine | config | unset)
{ "root": "/Users/you/workspace/observatory", "source": "machine", "legacy": null }
```

```json
// neb config commit --json
{ "enabled": true }
```

With `commit` on (off by default) and the corpus root inside a git work tree, a
changed corpus write can end with one commit of `nodes/`, `inbox/`,
`config.yaml` and the generated `.gitignore`. A commit is named `neb <verb>
<ids>` and prints `committed <hash>` on stderr in text mode (nothing extra in
`--json`). It never pushes and never touches a path outside the corpus root.

The commit names those paths, so anything else staged in the repository — before
the verb or while it runs — is left staged and out of it. With `commit` on and
no git repository at or above the root, the verb succeeds and says `note: not
committed: <root> is not inside a git work tree` on stderr, in every mode.

When git fails — a hook, a repository it cannot read — the verb exits non-zero
with a `git` refusal **after** its write has landed: the write is never rolled
back because of git; report it rather than retry the write. `--no-commit` skips
the commit for one invocation.

`check` writes `corpus: <root>` and its findings on stdout, and its tally on
stderr: `N nodes, E errors, W warnings, U unreadable files` (each noun is
singular when its count is 1). `N` counts readable nodes; `E` includes
unreadable files as well as error-level findings. Any unreadable file or error
finding means exit 1. Under `--json`, stdout is the report below and there is no
tally on stderr.

`unreadable` contains `{path, code, message}` for each unreadable node file; the
remaining nodes are still checked.

```json
// neb check --json          (findings[] carries {rule, level, node, message})
{ "root": "/Users/you/.nebula", "unreadable": [], "findings": [], "nodes": 3 }
```

## Maintenance

| verb | does | flags |
|---|---|---|
| `neb review` | the weekly report: stale hypotheses (≥ 30 days), untouched seeds (≥ 90), hypotheses created ≥ 14 days ago with no references, hypotheses whose kill nobody human wrote, inbox waiting ≥ 14 | `--since <DAYS>`, `--out <FILE>`, `--limit <N>` (per section) |
| `neb review --short` | the quick glance, one `ID WHY` row each: hypotheses created ≥ 14 days ago with no references, seeds untouched ≥ 90 days, inbox entries waiting ≥ 14 days | `--tag <TAG>`×, `--limit <N>` (lines) |

Both forms leave the corpus read-only by the spec's hard rule. `--short` refuses `--since`
and `--out`, and `--tag` needs `--short`.
Notes are reasoning, not context: adding a note does not count as adding a
reference and does not close the no-references finding after the grace period.

```json
// neb review --json     (rule: stale-hypothesis | untouched-seed | no-references | unconfirmed-kill | stale-inbox)
[
  { "rule": "no-references", "id": "tags-beat-domains",
    "title": "Tags beat domains", "reason": "no references attached" }
]
```

`review --out <FILE>` requires an existing parent directory outside the resolved
corpus root. Destinations inside the corpus, including aliases through symlinks
or `..`, are refused (`report_in_corpus`, exit 1); a final symlink is never
followed (`not_regular_file` if it points outside or is dangling). The external
report is replaced atomically with mode `0600`. `NEBULA_READ_ONLY=1` refuses
**every** `--out` destination (`read_only`, exit 1); use stdout review instead.

`neb review` without `--json` prints five `##` sections in that order, each
`_none_` or a `- \`id\` Title — reason` list; `--out review.md` writes it to a
file (the `--json` report too, under `--json`) and says `wrote review.md` on
stderr, leaving stdout empty.

`--limit <N>` keeps the first N findings under each heading, so a crowded
section cannot push a short one out, and a cut section ends `- _… and K more;
raise --limit for more_`, which is part of the report and so of the `--out`
file; under `--json` it keeps the first N items of each `rule`, in the
[capped-list](verbs-query.md#capped-lists) envelope whose `total` counts every rule's
findings.

Either way stderr says how many findings were cut (`2 findings not shown; raise
--limit for more`). With `--short` it keeps the first N lines, and stderr says
`N of M shown; raise --limit for more`; with nothing to report, stdout is empty
and stderr says `nothing needs attention`.

```json
// neb review --short --json
[
  { "id": "an-idea", "why": "hypothesis with no references" }
]
```

`neb review --short --json` is an array of `{id, why}` — narrower than full
`review`'s items, with no `rule`, `title`, or `reason`.
