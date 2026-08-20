# 0001 — Tracking agent activity independently of marks

## Status

Proposed.

## Context

`tt agent begin`/`end` (see `src/marks.rs`, `src/agent.rs`) is the whole tracking
mechanism for agent work, and it exists entirely at the model's discretion:
a mark file is created only because an agent chooses to run `tt agent begin`.
There is no second, independent signal that work happened at all.

This surfaced concretely: an agent (Claude Code, orchestrating no subagents)
did real investigative work — read `marks.rs`/`agent.rs`, ran live diagnostics
against the operator's own mark directory, answered questions about the
system — and never called `tt agent begin`. Nothing caught it. The existing
`Stop` hook only warns about **marks left open**, which requires a mark to
have been created in the first place. A phase that was never begun is
invisible to it.

Two prior instincts for fixing this were considered and rejected on their own:

- **Blame the phase taxonomy.** The work fit `spike` ("investigation that
  produces no artifact") perfectly well; the omission was not a
  classification problem. Adding a distinct `exploration` phase alongside
  `spike` would just split identical work across two rows and make
  `tt report`'s rollups fuzzier, not clearer.
- **Add more rule text** (to `CLAUDE.md` or the skill file) saying "all work
  must be logged." The skill file already implies this — rule 1 ("only the
  orchestrator logs") already means the sole actor logs when there is no
  delegation, and rule 2 already lists investigation-with-no-artifact as
  loggable work. The rule text was already sufficient; it was skipped anyway.
  The skill file's own stated reason for having hooks at all is "prose alone
  gets skipped under context pressure" — restating the same prose more
  bluntly doesn't fix the mechanism that already failed once.

The actual gap is structural: `tt` has no source of truth for "an agent was
active" other than the marks the agent itself chooses to write. Mark
presence and "work happened" are the same signal, so absence of a mark is
indistinguishable from absence of work.

## Decision

Build a second, model-independent ledger of agent activity, written only by
Claude Code hooks (harness-triggered, not model-triggered), and reconcile it
against the existing marks/log data instead of trusting the model to
self-report.

### 1. An `activity/` ledger, written only by hooks

Alongside `marks/`, add an `activity/` directory. Entries are written by:

- **`SessionStart`** — opens `activity/<pid>-<start-epoch>`, holding the
  project (resolved the same way the skill already resolves it: `$TT_PROJECT`,
  else the git toplevel directory name).
- **`Stop`** — appends the end timestamp to that file.
- **`SubagentStop`** — writes a nested entry per `Task` dispatch, inside its
  parent session's window.

These entries carry no phase, no issue, no summary — just "something was
active here, from T1 to T2." That is all a hook can know without the model's
judgment, and it is enough: unlike a mark, it cannot be skipped by the model
deciding not to write it, because it isn't a choice the model makes.

### 2. Reconciliation, not more rules

A new pass (`tt agent audit`, or a second section of `tt agent list`) walks
`activity/` windows and checks whether a mark was open for that project
overlapping any part of the window, or a closed `#agent`-tagged entry covers
it. A window with neither is **unaccounted agent activity** — the concrete,
structural version of "if a mark didn't get created, that's an issue,"
computed from files rather than from the model remembering to report on
itself.

### 3. Surfacing

- The TUI's Agents panel (`src/tui/marks_surface.rs`) gains a second state
  beyond "open marks": *unmarked activity*. Today the panel can only ever
  show what the model chose to write; this adds information the model's
  cooperation was never required for.
- The `Stop` hook prints the warning immediately when its own session's
  window closes unaccounted for, catching the gap in the same session
  rather than on next glance at the TUI.

## Guardrails

- **Roll subagent windows into their parent's before checking.** Rule 1
  ("only the orchestrator logs") stands: a subagent should never be expected
  to own a mark itself. Checking a subagent's window in isolation would flag
  every subagent for lacking its own mark, which contradicts the one-writer
  design already in place. Reconciliation must check the *orchestrating
  session's* window, with nested subagent windows absorbed into it.
- **A floor on window length.** A one-line question should not trip the
  warning. Reuse the existing `agent.max_unvouched_minutes` convention
  (`src/agent.rs`) rather than inventing a new threshold — same shape of
  judgment call the codebase already makes for an unvouched phase.
- **Overlap, not exact match.** Check "any mark existed for this project
  during the window," not "the mark's phase matches the activity." Phase is
  a judgment call only the agent can make; the hook can't and shouldn't
  predict it.

## Consequences

- Adds one more directory and a handful of small, deterministic hook writes;
  no change to the existing mark file format or the `begin`/`touch`/`end`
  contract.
- Makes "an agent ran and never marked it" a detectable, reportable
  condition instead of a silent gap — closing exactly the failure mode that
  prompted this decision.
- Does not fix classification mistakes or genuinely-not-loggable sessions;
  it only catches the case where real activity happened and no tracking
  trail exists for it at all.

## Alternatives considered

- **More prose in `CLAUDE.md` or the skill file.** Rejected: the equivalent
  prose already existed in the skill file and was insufficient on its own,
  by direct demonstration.
- **A new `exploration` phase.** Rejected: `spike` already covers
  investigation producing no artifact; a second phase for the same thing
  would dilute `tt report` rollups rather than sharpen them.
- **A blocking `PreToolUse` hook** that refuses `Edit`/`Write`/`Bash` until a
  mark exists. Considered too aggressive for sessions that are legitimately
  not loggable work (a quick question, a one-line lookup), and out of step
  with the rest of the system, which warns rather than blocks.
