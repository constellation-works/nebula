# Refusals: inbox

Refusals are typed. The message is what the CLI prints; the variant is what
`nebula-core` returns to the desktop app, and its name in `snake_case` is the
`code` that `--json` reports: `NeedsKill` is `needs_kill`, `IoAt` is `io_at`
(the envelope is in [verbs-common.md](verbs-common.md#refusals-under---json)).
**Do not retry the same command.**

Read this for a refusal from `capture`, `promote`, `drop` or `triage`.

| Message | Variant | Rule | Do instead |
|---|---|---|---|
| `no open inbox entry \`X\`` | `NoSuchInboxEntry` (`no_such_inbox_entry`) | — | The id is wrong: no inbox line, waiting or settled, carries it. `neb inbox --json`. |
| `\`X\` was already promoted to \`N\`` / `\`X\` was already dropped` | `InboxEntrySettled { id, settlement }` (`inbox_entry_settled`) | — | The entry is settled and the verb has nothing to do. Promoted: work on node `N` (`neb show N`). Dropped: it stays dropped; if the human wants it back, capture it again. Do not retry. |
| `there is no candidate N; this entry has ...` | `NoSuchCandidate {number, shown}` (`no_such_candidate`) | — | `triage` was given a number its current entry was not shown. Nothing was written; pick a listed number, `p` for a root, or `s`. |
| `\`triage\` is interactive and has no JSON form` | `Interactive` | — | `triage` is for a human at a terminal. Script the same decisions with `neb inbox --json`, `neb near --json`, then `neb promote` or `neb drop` per entry. |
| `nothing to capture` | `EmptyCapture` (`empty_capture`) | — | The text was only whitespace. Nothing was written, and no corpus was created for it. Capture the thought itself. |
| `inbox entry \`X\` was on line N of <file>, which now ends before it; nothing was written` | `InboxEntryMissing { id, file, line }` (`inbox_entry_missing`) | — | The month file got shorter between reading the entry and settling it: a hand edit or another writer. Nothing was written. Read `neb inbox --json` again before deciding anything about `X`. |
| `line N of <file> no longer holds inbox entry \`X\`; nothing was written` | `InboxEntryChanged { id, file, line }` (`inbox_entry_changed`) | — | The line now holds other text. Same as above: nothing was written; read the inbox again. |
| `inbox entry \`X\` is in <file>, not in this corpus's inbox <dir>` | `InboxEntryForeign { id, file, inbox }` (`inbox_entry_foreign`) | — | A library caller handed one corpus an entry read from another. Settle it through the corpus it came from. |
| `inbox id namespace exhausted; nothing captured` | `InboxIdsExhausted` (`inbox_ids_exhausted`) | — | All 65,536 inbox ids are held by waiting entries. Nothing was captured; the inbox needs triage before anything else goes in. Tell the human. |
