# Linking to Observatory

Read this before you cite an Observatory record, hand an idea off to one, or
act on an observatory-root warning from `check`. The verbs' flags and `--json`
shapes are in [verbs-references.md](verbs-references.md); the refusals are in
[invariants-refusals-nodes.md](invariants-refusals-nodes.md).

## Cite a record

A node that became an Observatory record is cited by the record's id, never
by a path: `neb cite <node> --kind observatory --uri <record-id> --note "why"`.
Record ids are `Q###` (questions), `H###` (hypotheses), `T###` (theories) and
`R###` (research).

The path is reconstructed per machine: `$OBSERVATORY_ROOT`, else this
machine's setting (`neb config observatory-root <DIR>`, stored in
`~/.config/nebula/observatory-root`, never in the corpus), else a legacy
`observatory_root` key in `config.yaml`.

An absolute path in a reference is a bug: it may resolve here and breaks
everywhere else. `cite` refuses one, and `check` warns about any already in the
corpus. An unresolved record is a warning about this machine, not something to
rewrite.

A `check` warning that `config.yaml` carries `observatory_root`, or the
`warning:` a verb prints on stderr while that key is the root in effect, means
an older `neb` stored one machine's path in the shared corpus. Leave it to the
human to set each machine's own and then run
`neb config observatory-root --drop-legacy`.

## Hand an idea off

When the human says an idea has become an Observatory record, hand it off in
one step: `neb handoff <node> <record-id> --note "why"`.

- It cites the record and closes the node as `abandoned` with
  `why: handed off to <record-id>` in one write, and `show` and `trace` then
  say where the idea went.
- It refuses a node that is already closed and, when this machine has an
  observatory root, a record the checkout does not carry.
- Never split it back into `cite` plus `status` to get past either refusal.
