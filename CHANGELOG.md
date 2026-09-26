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
  Downstream hand-off happens by the downstream artifact referencing the node,
  or in one step with `neb handoff` for an Observatory record.
- Statuses `testing` and `supported`; `Status` is now exactly `seed`,
  `hypothesis`, `refuted`, `abandoned`.
- Edge types `supports`, `undermines`, `depends-on`; `EdgeType` is now exactly
  `derives-from`, `refines`, `generalizes`, `reopens`, `contradicts`.
- `check --online`, the Orbit resolver, and invariants 3, 9, 10, 12, 13 of the
  v0.1 numbering. The v0.2 rules are listed in
  [the spec](docs/design/v0.2/1_spec.md#invariants); local reference URIs now
  resolve as an **error** and are refused at `cite`.
- `new --status`. `new --kill "..."` starts a hypothesis; without it, a seed.
- The quick glance (`open` in v0.1, now `review --short`) no longer reports
  references without a note; that is `check` rule 10.

### Added

- `closed: {why, at}` on nodes. `neb status <id> refuted --why "..."` is
  required and writes it; `neb status <id> abandoned [--why "..."]` writes it
  when given. Moving back to an open status clears it. Refuted is final: the
  only way back is a new node with a `reopens` edge.
- `new --reopens <id>` and a repeatable `new --contradicts <id>` write those
  edges with the node, under `link`'s guards: every end must exist, no edge
  twice, no genealogy loop, and `contradicts` recorded on both nodes. Any
  refusal writes nothing. Naming one node as both `--parent` and `--reopens`
  is refused, since `reopens` is already genealogy. Trying to reopen a
  refuted node with `status` now hints the one command that does it,
  `neb new "..." --reopens <id>`.
- `new` and `promote` take `--id <slug>` to choose the id rather than derive
  it from the title, which suits a long title, since an id never changes. It
  follows the rules a derived id does (lowercase words joined by single
  dashes, 60 characters or fewer) and is refused when it does not
  (`invalid_id`) or when a node already has it (`node_exists`).
- `neb note <id> <text>...` appends a dated paragraph of reasoning to a
  node's body, `- YYYY-MM-DD: <text>` under a `## Notes` section at the end,
  opening one when the body has none or other prose has closed the last. The
  earlier prose, status, edges and tags are left as they are, `updated` is
  bumped, and text over several lines is joined onto one. `show --json` and
  `note --json` carry `notes: [{at, text, by}]`, read from every `## Notes`
  section, oldest first. A note is reasoning rather than context, so it does
  not count as a reference for `review`.
- `neb edit <id>` opens the node's prose body, never its frontmatter, in
  `$VISUAL`, else `$EDITOR`, and saves what the editor leaves once it exits
  successfully. The value is split into words as a shell would, so
  `code --wait` works. Unmatched quotes or no words are refused
  (`editor_invalid_command`), neither variable set is refused naming both
  (`editor_not_configured`), and an editor that fails leaves the node
  unchanged (`editor_unsuccessful`). Removing, moving or changing any existing
  `## Notes` section is refused (`notes_changed`), since notes are appended
  with `note`. `--json` is the `show --json` view. `new --body <TEXT>` sets
  the prose at creation, `promote --body <TEXT>` appends it after the captured
  line, and `--body -` reads it from standard input.
- `--by <label>` on `new`, `promote`, `sharpen`, `link`, `note`, `cite`,
  `handoff` and `triage` records who wrote the words, per field: `title_by`,
  `kill_by`, and `by` on each edge and reference, while a note carries it in
  its line, `- YYYY-MM-DD (agent:crew): text`. It defaults to `human`, stored
  by omission, so files written earlier need no migration; `show --json` and
  `list --json` state it outright. A label cannot hold parentheses, a newline
  or `: `. `edit` validates `--by` but stores nothing, since a body has no
  author field. `sharpen <id> --confirm` adopts the kill condition already on
  a node as the human's own and changes nothing else; it is what takes a node
  off `review`'s unconfirmed-kill list. `--by` is not `--task`/`--run`, which
  record the Orbit run behind a write.
- Tags are normalised to lowercase kebab-case on every write path (`new`,
  `promote`, `tag`, `migrate`). `neb tag <id> --add <tag> --remove <tag>`
  edits them; `neb tag list` shows every tag with its node count; `--tag` on
  `list` and `review --short` is repeatable and requires every tag named.
- `check` rule 11 warns when two tags differ only by case or a trailing `s`,
  and names the nodes carrying each variant. `new`, `promote` and `tag --add`
  catch it at write time: a tag new to the corpus that is that close to one
  in use prints `note: tag physic is close to physics (2 nodes)` on stderr,
  in every output mode. It is a note, never a refusal; the write has landed.
- `check` rules for what only a hand edit produces: a `closed` block on a
  seed or hypothesis (rule 12, error); a seed carrying a kill condition
  (rule 13, warning); and a `created`, `updated` or reference `added` that is
  not a real calendar date in `YYYY-MM-DD` form, or an `updated` earlier than
  `created` (rule 14, error). Rule 16 warns on a reference kind outside the
  vocabulary (`paper`, `study`, `article`, `note`, `discussion`, `book`,
  `dataset`, `thread`, `observatory`, `other`), which `cite` refuses for a
  new reference (`unknown_reference_kind`).
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
  `schema_version: 2` with only `corpus_id` and `schema_version`, plus
  `observatory_root` and `commit` when they were set. Prints a
  per-node summary of what changed; a second run rewrites nothing. Before it
  writes anything it refuses a `config.yaml` at a schema it does not know
  (`schema_mismatch`), and a corpus already at schema 2 that holds a node the
  current model cannot read (`current_schema_unreadable`, naming the file),
  which the lenient v1 reading would have rewritten without its unknown keys.
- `config.yaml` is `schema_version: 2`; a v1 corpus refuses to open with a
  hint to run `neb migrate`.
- `cite --kind observatory --uri Q002`: a portable link to an Observatory
  record. The `uri` is the bare record id (`Q###`, `H###`, `T###`, `R###`),
  normalised to upper case and refused at `cite` when it is not one; nothing
  machine-specific reaches the corpus. Where the record is comes from the
  machine: `$OBSERVATORY_ROOT`, else `~/.config/nebula/observatory-root`,
  written by the new `neb config observatory-root DIR`. That verb requires an
  absolute path and writes nothing in the corpus, so it commits nothing; with
  no directory it prints the effective root and which setting supplied it
  (`--json`: `{root, source, legacy}`, `source` one of `env`, `machine`,
  `config`, `unset`). `check` rule 9 *warns* rather than errors when the
  root is unset or the id does not resolve, since that judges the machine
  rather than the corpus. `show` prints the resolved path, and `show --json`
  carries `observatory: [{reference, record, path}]`. Earlier 0.2 builds
  stored the root as `observatory_root` in `config.yaml`, one machine's path
  in a file every machine shares. That key is still read, as the last
  fallback, and `migrate` keeps it, but `check` warns (rule 9) while it is
  there, and `neb config observatory-root --drop-legacy` removes it (a corpus
  write, committed when `commit` is on). Run that once every machine sharing
  the corpus has its own setting.
- A `discussion` reference may omit its URI:
  `neb cite <id> --kind discussion --note "..."` records a conversation with
  nowhere to link to. Every other kind still needs `--uri`, and an empty or
  blank one counts as missing, refused at `cite` and an error under `check`
  rule 8.
- `neb handoff <id> <record> --note "..."`: the one-step form of "this idea
  became Observatory record `H012`". One save adds an `observatory` reference
  to the record and closes the node `abandoned` with
  `closed.why: handed off to <record>`. It refuses, writing nothing, an
  unknown node, a node already refuted or abandoned (`already_closed`), a
  record id of the wrong shape, and, when this machine has an observatory
  root, a record that does not resolve under it
  (`unresolved_observatory_record`); with no root set it accepts the id and
  says the record cannot be located. `show` prints the record's location
  under `closed:`, `trace` ends the node's line with `handed off to <record>`,
  and both `--json` forms carry `handed_off_to`, omitted for every other
  node. `handoff --json` is `{doc, reference, record, from}`.
- One writer at a time. Every verb that writes, and the desktop's capture and
  settle actions, hold an advisory lock on `<root>/.lock` for the whole write
  and the commit that records it, so a terminal, an agent session and the
  desktop app can no longer interleave one's load with another's save, leave
  a `contradicts` edge on one side only, or strike the wrong inbox line.
  Reads never take it. A writer waits up to five seconds and then refuses
  before reading anything (`locked`), so the same command is safe to run
  again; the lock goes with the process holding it, so there is never a
  stale one to delete. `config.yaml` is re-read under the lock before a
  setting is rewritten or a commit decided, so two writers no longer erase
  each other's settings. `init` adds `/.lock` to the corpus's `.gitignore`,
  which a `commit` stages with the corpus. Running `init` again on an older
  corpus adds the rule and changes nothing else; a `.gitignore` that is a
  symlink, which git does not read, becomes a regular file with the same
  rules, its target untouched.
- `neb init <dir> --set-root` makes that corpus the machine default, writing
  its path to `~/.config/nebula/root`. The path must be absolute, and a
  setting that already names another corpus is replaced only with `--force`
  as well (`root_config_conflict`); both are refused before anything is
  created. Plain `init` never touches the setting: it prints the
  `--set-root` command when there is none, and warns on stderr when the
  setting names a different corpus from the one it just made. `init` on an
  existing corpus opens it rather than resetting it, and refuses without
  changing anything a `config.yaml` there that will not parse or is at
  another schema.
- `neb config commit on|off`: with `commit: true` in `config.yaml` (off by
  default, absent from the file until set) and the corpus root inside a git
  work tree, every verb that writes (`capture`, `promote`, `drop`, `triage`,
  `new`, `edit`, `sharpen`, `status`, `link`, `tag`, `note`, `cite`,
  `handoff`, `migrate`, `config`) ends with one commit of `nodes/`, `inbox/`,
  `config.yaml` and the generated `.gitignore` under the root, as
  `neb <verb> <ids>`, and prints `committed <hash>`; `triage` makes one per
  decision. Never pushes, never stages a path outside the corpus. Something
  staged elsewhere in the repository is a typed refusal (`StagedElsewhere`)
  that leaves the write in place: a write is never rolled back because of
  git. A corpus the containing repository ignores is refused too
  (`CorpusIgnored`), with the fix — a repository at the corpus root — in the
  hint. `--no-commit` after any of those verbs skips the commit once. Only
  their `--help` offers it, and a verb that only reads refuses it as an
  unknown argument (exit 2); written before the verb, the spelling from when
  the flag was global, it is still honoured but hidden. `neb config commit`
  with no argument reads the setting (`--json`: `{ "enabled": bool }`) and,
  like every `config` that changes nothing in the corpus, commits nothing;
  `migrate` preserves it and commits itself. With `commit` on, a node whose
  file name is not ASCII commits like any other, where git's quoting of the
  name used to make it look staged elsewhere. The runbooks now recommend a
  private repository at the corpus root and cover restoring from it.
- `neb near <TEXT>... | <NODE>` (`--limit K`/`-k`, default 3): the existing
  nodes closest to free text, or to a node (left out of its own answer), best
  first. Word overlap only — BM25 over title, tags and body, title and tags
  weighted up, plurals and `-ing` folded — with no embeddings, no network and
  no new dependency. The score is uncalibrated (a word-for-word copy of a node
  scores about 0.35 to 0.6 against it), so text output prints a band instead:
  `strong` from 0.25, `some` from 0.07, `weak` below. Given a node, a
  neighbour already joined to it by an edge, either way round, ends
  `linked: parent (<kinds>)`, `linked: child (<kinds>)` or
  `linked: contradicts`. Nodes sharing no word are left out, so `[]` means
  the thought is unlike anything in the corpus. `capture` prints the same top
  three under the entry id, and `promote` without `--parent` prints them and
  proceeds as a root; `--quiet`/`-q` on either leaves them out. Suggestion
  only: nothing here ever writes an edge, by the v0.2 rule against automatic
  linking. `--json` shapes: `near` is a bare list of
  `{id, title, status, tags, score, band, linked}`, where `linked` is a list
  of `{from, type, to}` or `null`, and always `null` for free text; `capture`
  is `{entry, near}`; `promote` is `{doc, path, near}`, `near` omitted when
  empty. The skill's triage heuristic now runs `near`, picks a parent only
  when one is defensible, and otherwise promotes as a root.
- `neb capture -` reads the thought from standard input. Text over several
  lines, piped or quoted, is joined onto one inbox line, each line break a
  single space and blank lines dropped; only whitespace-only text is refused.
  The joining is in `nebula-core`, so the desktop capture box stores the same
  line.
- `neb triage`: the inbox worked through at a terminal, oldest entry first.
  Each entry is shown with its age and its `near` candidates numbered 1 to 3,
  and one key decides it: `p` promotes it as a root, `1`–`3` under that
  candidate, `t` titles the promotion that follows, `d` drops it, `s` leaves
  it waiting, `q` stops, `?` lists the keys. Nothing is linked unless a
  number is chosen. Every decision is the single `promote` or `drop`, with
  the same refusals and `--by`, and the session ends with a tally. With
  standard input piped, keys are read one per line and the first refusal ends
  the session with exit 1. `--json` is refused (`interactive`), and the hint
  names the scriptable verbs.
- `impact` reports every descendant (reverse genealogy) plus the node's
  `contradicts` neighbours.
- `review`'s stale-hypothesis section is "untouched for N days" rather than
  "no evidence for N days". The no-references section counts only hypotheses
  created fourteen or more days ago, and a new section lists agent-authored
  kill conditions no human has confirmed. Day thresholds are inclusive (`>=`)
  throughout.
- `--limit N` on `list`, `inbox` and `review` (per section, or lines with
  `--short`) and `trace --depth N` bound long output; without them everything
  is printed, as before. Text says what was left out (`2 of 5 nodes shown;
  raise --limit for more`, `(1 more beyond --depth)`); under `--json` the
  arrays are cut to N (per `rule` for `review`) in the same shape, with no
  marker.
- `--json` covers the writes as well as the reads: each verb that writes
  prints the value it changed. `new` is `{doc, path}`; `sharpen` and `tag` a
  `Doc`; `status` `{doc, from}`; `link` an array, since `contradicts` changes
  both nodes; `cite` `{doc, reference}`; `drop` the settled entry; `note` and
  `edit` the `show --json` view; `init` `{root}`.
- Refusals under `--json` are data: one line on stderr,
  `{"error": "...", "code": "...", "hint": ...}`. `error` is the message,
  `code` a stable `snake_case` name (for a core refusal, the `nebula-core`
  variant's name: `no_such_node`, `needs_kill`, `staged_elsewhere`, …), and
  `hint` the remedy or `null`. Stdout stays empty, except when a write landed
  and only its commit was refused. A refusal exits 1 and a command line clap
  rejects exits 2, with clap's prose even under `--json`. Without `--json`
  every refusal prints what it printed before. An unknown `cite` kind is now a
  typed refusal (`unknown_reference_kind`). `skills/nebula/references/verbs.md`
  documents the envelope and the exit codes.
- `neb --help` groups are Corpus (`init`, `check`, `migrate`, `config`,
  `completions`), Inbox (`capture`, `inbox`, `promote`, `drop`, `triage`),
  Nodes (`new`, `edit`, `sharpen`, `status`, `link`, `tag`, `note`),
  References (`cite`, `handoff`), Query, Maintenance.
- `neb graph --json`: the whole corpus as `{nodes, edges}`, for a tool that
  draws it. `neb graph --mermaid` prints a Mermaid `graph BT` block to paste
  instead: nodes styled by status, edges labelled by kind, each `contradicts`
  pair one dotted undirected line, and titles escaped (`&`, `<`, `>`, `"`,
  `[`, `]`) so they stay label text. `--from <id>` narrows it to that node,
  its ancestors and its descendants. `--mermaid` and `--from` conflict with
  `--json` whether it is written before the verb or after it (exit 2), so
  JSON is always the whole corpus. With neither format, `graph` prints a hint
  and exits 2.
- `neb log <id>` lists the commits that changed a node, newest first: short
  hash, date and message, or `no commits touched this node` (`--json`:
  `[{hash, date, message}]` with the full hash, or `[]`). `neb show <id> --at
  <HASH|YYYY-MM-DD>` prints the node as it was then, in `show`'s own text and
  JSON; a date means the last commit that day. Both read the history
  `commit on` writes and are read-only. A corpus outside a git work tree is
  refused (`not_git_work_tree`), a revision before the node existed is
  `no_node_at_revision`, and an `--at` that is neither a date nor a hex
  revision is refused naming the value.
- `skills/nebula/`: the agent skill. `SKILL.md` states the two modes (session:
  act and `neb check` after every write; routine: read-only, proposals into
  `review.md`), corpus resolution, capture-from-conversation, provenance and
  triage heuristics; `references/` carry every verb's flags and real `--json`
  shape, the rules in invariants.md with each typed refusal and its remedy, and a
  worked transcript per mode. `make skill-link` symlinks it into
  `~/.claude/skills/nebula`.
- `apps/desktop`: a menu-bar app (Tauri 2, React, TypeScript) on
  `nebula-core` directly, no sidecar. The tray shows the unsettled inbox
  count; `Alt+Space` (configurable in the app's `settings.json`) opens a
  floating capture box that writes the same line `neb capture` writes, and
  commits it when `commit` is on; a multi-line paste is joined with spaces.
  The window has an Inbox view and a Graph view. The Inbox view is a capture
  box and the unsettled entries, each with *Promote as root* and *Drop*,
  which run the same core operations, lock and commit as `neb promote` and
  `neb drop`; a refusal stays on the entry, and entries waiting fourteen days
  or more are marked stale. A `notify` watcher on `nodes/` and `inbox/`
  refreshes both on outside changes. A missing corpus shows the path tried
  and a reload button. `make desktop-dev`, `make desktop` (unsigned `.app`
  and `.dmg`, with install steps in `apps/desktop/README.md`) and
  `make desktop-check`; CI gains a `desktop` job (types drift, tsc, vitest,
  clippy). The workspace's `rust-version` moves to 1.88 for the desktop's
  dependency tree.
- `Corpus::node_path` is public, so a consumer that hands a node file to the
  OS asks for the path rather than re-deriving the layout. It returns a
  `Result`: an id becomes a path there, so one that is not a single file name
  is refused rather than joined.
- The desktop's Graph view: the whole corpus from one `graph()` call as a
  layered DAG (`elkjs`, in a web worker, plain SVG; genealogy edges layer the
  drawing, `contradicts` is dashed and drawn afterwards). Status is the card
  colour; tag chips with `+n`; a marker on nodes that `reopens`. Drag or
  scroll to pan, Ctrl+wheel to zoom, click to select and light ancestry and
  descent in two tints, Escape to clear, double-click to open the file. A
  title search (applied once typing pauses) and a multi-select tag filter
  apply before layout. The right-hand panel shows the node read-only:
  frontmatter, `closed.why`, the body via `react-markdown` + `remark-gfm`
  with raw HTML shown as text and images loaded only from `https:` URLs,
  edges as two clickable lists, references as a table whose `http(s)`/`mailto`
  URIs open through the opener plugin. A non-human `by` is named beside the
  title, kill condition, edges and references, and an Observatory reference
  shows where its record is on this machine. `corpus-changed` refetches and
  re-lays out without losing the selection or the viewport, and so does
  switching to the Inbox tab and back.

### Changed

- The repository is a Cargo workspace. `crates/nebula-core` is the corpus as
  a library (model, store, graph queries, ops, check, migrate) with a typed
  `Error` and no terminal, clap or `anyhow` dependency; `crates/neb` is the
  CLI, and each verb is one core call plus rendering. Every value core
  returns is `Serialize`, so `--json` and the desktop app's IPC share one
  schema. `cargo test -p nebula-core --features ts` (`make types`) generates
  `apps/desktop/src/types/*.ts` from those types; CI fails if they are stale.
  Observable CLI behaviour, messages and exit codes are unchanged.
- The version is written once, in `[workspace.package]`;
  `scripts/release-check.sh` reads it there.
- A node's id is checked before it becomes a path, and a node file's name and
  the id it stores have to agree (rule 15). A hand-edited `id: ../../escaped`
  used to load, and the next verb wrote `escaped.md` outside the corpus root;
  a hand-edited id naming *another* node used to overwrite that node, since
  `save` derives its destination from the stored id. Both now refuse, as do a
  caller-supplied traversal or absolute id (`Error::UnsafeId`,
  `Error::IdMismatch`). Ids are judged as written and nothing is
  canonicalized, so a symlinked root behaves as before, and the rule is about
  path structure rather than alphabet: `ünïcode-título-ok` is still an id.
  Name and id agree only when they are one directory entry, as the
  filesystem answers it: a byte-for-byte copy of another node is refused
  like any other mismatch, and so is a second name for a node under
  `nodes/`, a hard link or a symlink, which is named as the alias to remove
  (`id_mismatch`). A name stored in another Unicode normalization is still
  the same node.
- A title's id keeps its Unicode letters and digits, lowercased
  (`Ünïcode título → ok` is `ünïcode-título-ok`, and `시간은 프레임의 수다`
  keeps its words); they used to be dropped, leaving a fragment of the title
  or no id at all. Tags are normalised by the same rule. An id over 60
  characters is cut at the last `-` at or before the limit rather than
  mid-word.
- `sharpen --kill` no longer replaces a kill condition already on an open
  node. It refuses (`kill_already_set`) and prints the one in place: a
  different falsifier is a different idea, and belongs on a new node. On a
  refuted node, `sharpen --kill`, `sharpen --confirm` and a second
  `status <id> refuted` are all refused as a status change is
  (`refuted_cannot_reopen`); the last used to overwrite `closed.why` and its
  date. An abandoned node is not a verdict, so `sharpen` still works on one
  and a second `status <id> abandoned --why` replaces the reason. When
  `sharpen` leaves the status where it was, text output says
  `<id> kill condition set; status remains abandoned` rather than claiming a
  move.
- `capture`, `note` and `near` still take the remaining words as their
  text, but a flag after the text is a flag: `neb capture an idea --quiet`
  quiets and stores `an idea`, and `--no-commit`, `--by` or `--limit` there
  take effect instead of being stored. A dash-leading word that belongs to
  the thought goes in quotes or after `--`.
- `neb open` is merged into `neb review --short`, which prints what `open`
  did, one line each: hypotheses created fourteen or more days ago with no
  references, seeds untouched for ninety days or more, and inbox entries
  waiting fourteen days or more (`--json`: an array of `{id, why}`).
  `--short` takes `--tag` and `--limit`, and refuses `--since` and `--out`.
  `open` still works in this release as a hidden alias that forwards to
  `review --short` and prints `` warning: `neb open` is deprecated; use
  `neb review --short` `` on stderr; the release after this one removes it.
  The skill, spec, design docs and runbooks name `review` only.
- The corpus is found from the working directory. Resolution is `--root`,
  else `$NEBULA_ROOT`, else the nearest corpus at or above the current
  directory, else `~/.config/nebula/root`, else `~/.nebula`. A corpus is a
  `nodes/` directory beside a `config.yaml` that names a `corpus_id`, and
  only an existing one is found, so `capture` never creates a corpus because
  of where it ran. The innermost of nested corpora wins, and the walk follows
  `$PWD` as written, without resolving symlinks.
- `promote` with neither `--title` nor `--id` keeps the captured sentence as
  the title but, for a capture over five words, mints the id from its first
  five words after dropping stop-words (never negations): `gravity might be a
  scarcity gradient in some shared resource` becomes
  `gravity-scarcity-gradient-shared-resource`. An id held by a different idea
  takes one more such word at a time, then falls back to the full slug; a
  node already titled with the same text is refused (`node_exists`) rather
  than duplicated. Explicit `--title`/`--id` and short captures are
  unchanged.
- A capture that repeats a thought still waiting in the inbox (equal after
  folding case and whitespace) is still captured, and stderr says
  `note: same as <id>, still waiting`; stdout and the `--json` payload are
  unchanged. `promote` and `drop` on an entry already settled say how:
  `` `<id>` was already promoted to `<node>` `` or
  `` `<id>` was already dropped `` (`inbox_entry_settled`).
- `trace` and `trace --down` name, on every line below the start, the
  genealogy edge kind(s) joining it to the line above. Two edges between the
  same pair (`derives-from` plus a later `reopens`) draw as one line naming
  both, instead of a second line ending `(shown above)`; a true diamond keeps
  that marker. `trace --json` gains `via: {from, kinds}` per node (`null` for
  the start), and `parents` names each parent once.
- `show` opens with a header: the status and id; the title, followed by
  `(<label>)` when a non-human author wrote it; `tags: a, b`; and
  `created: <date>  updated: <date>`. The kill condition, each edge and each
  reference likewise end in `(<label>)` when their author is not `human`.
  Counts in text output agree with their nouns (`1 node`, `2 nodes`).
- `cite` lowercases `--kind` before checking it, so `--kind Paper` stores
  `paper`; anything still outside the vocabulary is refused, naming the kind
  as given.
- `check` numbers its rules 1 to 16, each once, from one list in
  `nebula-core`. The number `check` prints, and `check --json` carries as
  `rule`, is the one in the spec's table and the skill's `invariants.md`.
  Earlier 0.2 builds gave two rules the number 11 and reported Observatory
  resolution as rule 8.
- The repository adopts the constellation engineering standards, vendored
  read-only under `docs/standards/` with a managed block in `AGENTS.md`:
  STD-01 (CLI surface), STD-02 (Rust architecture and errors) and STD-03
  (concurrency and process safety) at version 2, and STD-04 (testing and
  verification) and STD-05 (security boundaries) at version 1. A deviation
  is recorded as a design decision, never as an edit to the copies.
  `make standards-check` (`sh docs/standards/check.sh`) fails on any edit to
  them and runs in `make ci` and GitHub CI; `make help` now lists what
  `make ci` runs.

### Fixed

- `capture` at a root with no corpus still creates one rather than refuse,
  but now says so on stderr, `note: created a new corpus at <absolute path>`,
  in text and `--json` alike, so a mistyped `--root` or `$NEBULA_ROOT` shows.
  A root that cannot be created, or a lock file that cannot be opened, is
  refused naming the path it tried (`io_at`).
- `status <id> seed` refuses a node that names a kill condition
  (`seed_with_kill`). It used to move the node and keep the kill, leaving a
  seed that `check` rule 13 blamed on a hand edit. `status <id> hypothesis`
  reopens such a node, from `abandoned` too.
- `cite` refuses an absolute path or `file:` URI (`absolute_uri`) even when
  it exists on this machine, with a hint at a path relative to `nodes/` or an
  Observatory id. `check` reports one already in the corpus as a rule-8
  warning instead of resolving it.
- An exported but empty `$NEBULA_ROOT` is treated as unset rather than as
  the current directory, where `capture` would have made a corpus. An empty
  `--root ""` is refused naming the flag (exit 2), and a caller of the
  library that passes an empty root gets `empty_root`.
- A node file with missing or unterminated frontmatter, and a node or
  `config.yaml` that cannot be read, are refused naming the file (`io_at` for
  an operating-system error), where a bare message used to leave the whole
  corpus unreadable with nothing to go and fix.
- A corpus written by a newer `neb` says to upgrade this build, not to run
  `neb migrate`, which could only fail the same way.
- Unknown keys inside an edge or an `origin` block are a parse error, as
  unknown keys elsewhere in a node already were, instead of being dropped by
  the next write.
- The inbox no longer loses or revives entries. Settling one rewrites its
  month file atomically, and refuses when the line has moved. A capture after
  a month file whose last line has no newline starts a line of its own
  instead of joining it. Only `inbox/YYYY-MM.md` files are read, so a
  leftover temporary file cannot bring a settled entry back. An entry id is
  unique among every waiting entry in every month, and a capture is refused
  only when all 65,536 ids are waiting. A corpus with `nodes/` but no
  `config.yaml` has the one it is given written, so its `corpus_id` stays
  the same from one run to the next.
- Every file `neb` replaces goes through a temporary file it creates
  exclusively, under a fresh name (`<file>.<pid>-<n>-<nanos>.tmp`): a symlink
  planted at the temporary path is never written through, and a temporary
  left by a killed process never blocks the next write.
- Desktop: text typed into the capture box while an earlier capture is still
  saving is kept, and the floating window stays open for it. The node panel
  clears the previous node when the selection changes or a load fails, and
  overlapping inbox or graph refreshes keep the newest result. The
  missing-corpus screen says to run `neb init` at the path shown, or to set
  `NEBULA_ROOT` and restart the app.
- Desktop: a capture no longer freezes the window while another writer holds
  the corpus lock. Every corpus command runs off the webview thread, and a
  capture waits at most 150 ms for the lock, retries twice, then says
  `Corpus busy. Press Enter to retry.` The watcher ignores the app's own
  reads, forwarding only creations, modifications and removals, and refreshes
  at least once a second during a continuous run of changes. A capture
  shortcut that cannot be registered, or a `settings.json` that cannot be
  read or parsed, is shown in the tray menu and as a startup warning in the
  main window, naming the shortcut or the file, rather than only on stderr.
  A failure to open a file or link, load the graph or load the inbox is shown
  where it happened; the graph's offers a Reload button and points at
  `neb check`, and an inbox error no longer replaces the whole window.

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
- `check` — the corpus invariants, exiting non-zero on any error.
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
