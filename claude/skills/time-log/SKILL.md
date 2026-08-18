---
name: time-log
description: "Record work time to the `tt` time tracker. Use when dispatching work to a skill or subagent (mark it), when that work reports back (close it), when the user asks \"log this\", \"how long have I worked\", \"what did I do today/this week\", or at session start when open marks may be left over. Covers planning and review time too, not just work that reaches a commit."
---

# Time log (orchestrator)

Records work against `tt`. **You are the only writer** — subagents report back and
you record it. Not for safety (concurrent writes are serialized under a lock and
nothing is lost) but for the **unit**: one entry per completed delegation, so an
issue reads as one row rather than twenty.

**The convention itself lives in [`AGENTS.md`](../../../AGENTS.md)** at the repo
root — summaries, the three tags, the gap-refusal contract, rounding, project
resolution. Read it once and follow it; this file only maps Claude's workflow onto
it, and deliberately does not restate it, because two copies of a convention drift.

## Which phase

| Workflow point | Phase |
|---|---|
| `epic` / `detailed-plan` | `plan` |
| implementer agent → commits | `impl` |
| `code-review` / `simplify` | `review` |
| qa-queue item resolving | `qa` |
| `document` | `docs` |
| `researcher`, investigation with no artifact | `spike` |
| tooling, config, environment | `ops` |

## Steps

1. **At dispatch**, before handing work to a skill or subagent:

   ```sh
   tt agent begin <project> <issue|-> <phase>
   ```

   Marks are files keyed per project/issue/phase, so parallel phases across projects
   never collide and the start time survives context compaction. Do **not** hold a
   start time in your own context.

2. **Whenever work is confirmed still happening** — a subagent reports back, a commit
   lands, a QA round finishes:

   ```sh
   tt agent touch <project> <issue|-> <phase>
   ```

   `end` measures start → last touch, so idle time *after* the work finished is never
   counted, and the heartbeats are what let a long session log without a question.
   Touch often: an unbeaten stretch over the threshold is what `end` refuses on.

3. **When the phase is done:**

   ```sh
   tt agent end <project> <issue|-> <phase> "<summary>"
   ```

   Never write tags yourself — `tt` applies them. Summary rules are in `AGENTS.md`.

## Branches

- **`end` refused with exit 65** — a silent gap. **Ask the user about the gap it
  named**; do not choose between `--full` and `--trim` yourself and do not default to
  either. `AGENTS.md` has the full contract and the precedence rules.
- **Exit 64** — no mark, or no summary. For a missing mark, log directly with a
  duration you can justify: `tt agent item <project> <issue|-> <phase> "<summary>"
  <minutes>`. If you cannot justify one, ask.
- **Work abandoned** — `tt agent cancel <project> <issue|-> <phase>` drops the mark
  without logging.
- **Session start** — check `tt agent list`, but **only act on marks for the current
  project.** An open mark for *this* project means a phase was never closed out:
  surface it to the user rather than silently ending it. Marks for **other** projects
  are none of this session's business — work runs in parallel across repos, so an
  open mark elsewhere means another session is mid-flight, not that something broke.
  Never ask about it, never end it, never cancel it.

## Reporting

`tt report` for today, `--week`, `--project <name>`, `--json` to parse. **Never parse
`tt list`** — it is emoji-decorated text for humans.

Watch the overlap counter. `tt log` back-dates from now, so entries written in a
batch at day's end claim overlapping slots — totals stay right, the timeline does
not. A rising count means logging is drifting away from the milestones it should be
attached to.

The TUI's **Agents** panel (`Shift-A`) shows the same open phases.
