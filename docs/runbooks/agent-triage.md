---
type: runbook
summary: Direct an agent through a session-mode triage of the corpus, and know what to check afterward.
tags: [operations, triage, agent, session-mode]
paths: ["skills/nebula/SKILL.md"]
related_features: [lineage-graph, v0.2]
related_artifacts: []
last_validated: 2026-09-12
---

# Direct an Agent Through Triage

This is the human side of session mode: what to say to an agent, what it
should do in response, and what to check before you walk away. The agent side
— the skill that teaches an agent the verbs, the `--json` shapes and the
refusals — is a separate piece; see `skills/nebula/SKILL.md` once it exists.

## The one rule

**Session mode means a human is present and directing.** The agent runs verbs
directly on your say-so and reports back; it never runs unattended in this
mode, and it never proposes into a review file instead of acting, because you
are already there to decide. The opposite mode — unattended, run as an Orbit
routine — only ever proposes and never writes to a node; if you are reading
this, you are not in that mode.

## Starting a session

Point the agent at the corpus and ask it to confirm the corpus is healthy
before touching anything:

> Triage my nebula corpus at `~/corpus/nebula`. Run `neb check` first and tell
> me if anything is already broken.

A healthy corpus reports zero errors. If it does not, stop and fix that first
— see [corpus-recovery.md](corpus-recovery.md).

## Directing the work

Say what you want in plain language; the agent maps it onto verbs. Some
starting points:

> Clear the inbox. For each entry, tell me what you'd promote, what you'd
> drop, and why, before you do either.

> Look at what `neb open` flags and tell me the single most actionable one.

> Sharpen `gravity-as-scarcity` — the falsifier is that the effect survives
> with the coupling turned off.

> Cite `[source]` on `gravity-as-scarcity`, and say in one line why it belongs
> there.

The agent should narrate the command it is about to run before running it, so
you can stop it. In this mode, that is a courtesy, not a safety mechanism —
you can always inspect the corpus's git history afterward.

## After every write

The agent should run `neb check` after every mutating command and report the
result along with the ids of whatever changed. If it does not, ask it to. A
clean check after each step is what keeps a triage session from compounding a
mistake across ten commands before anyone notices.

## Ending a session

Ask for a one-line summary of what changed, and skim `git log` in the corpus
if it is under version control. A good agent also proposes a short list of
things worth capturing from the conversation itself, without capturing them
unasked — accept or decline each one explicitly rather than letting it decide.

## When to stop directing and let it run unattended

Only once you trust the corpus is in a state where a proposal-only pass adds
value without you standing over it — for example, a nightly pass over the
inbox that leaves a report for you to read in the morning rather than acting
on your behalf. That is the other mode, and it is a different runbook.
