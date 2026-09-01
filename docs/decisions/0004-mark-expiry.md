# 0004 — Expiring an un-renewed mark at its last heartbeat

## Status

Implemented.

## Context

Four agent marks were found open 69–114 hours after a weekend, three of them
with no heartbeat file at all, and nothing had warned in five days. The work
time had to be reconstructed by hand from the activity ledgers.

Two things combined to produce that silence.

`audit::covered_by_mark` treated an open mark as covering `mark.start → now`
for its project, so `audit::unaccounted` considered every session window for
that project accounted for. The warning surfaces built to catch untracked work
— `tt agent audit`, the `Stop` hook's activity check, the TUI's unaccounted
section — were all suppressed by the very thing that had gone wrong.

Nothing aged a mark. `Mark` held only a start, and `open_marks_in` deliberately
reads no heartbeat: its callers refresh on the mark directory's mtime, which an
append inside `beats/` does not change. Liveness therefore cannot be
stamp-gated; it has to be read at the moment it is judged.

The automatic half of the heartbeat was also unreliable. `touch_all_in` beat
every open mark in every project, so an unrelated project's subagent stop kept
an abandoned mark's last-seen fresh, and beats only arrived on `SubagentStop`.

## Decision

A **lease** is one open mark plus the instant it was last seen — the beats
file's last line. It expires at that instant plus `agent.max_gap_minutes`, or at
`mark.start` plus `agent.max_unvouched_minutes` when the file holds no beat at
all: no evidence is treated as no evidence.

A beat line carries its provenance. `tt agent touch` — the model's own vouch —
writes a bare `<epoch>`; the hooks write `<epoch> hook`. One file, two meanings,
and the four readers of it differ deliberately:

- **Measurement** (`Phase::ended`, what `end` bills) anchors on the last **bare**
  line wherever it sits in the file: tagged lines are invisible to measurement
  entirely, and `end` falls back to measuring to now only when the file holds no
  bare line at all. Reading the anchor off the final line alone instead would
  discard a model vouch the moment a hook beat followed it — a phase touched at
  minute 20 and hook-beaten at 21 would bill 61 minutes at a close an hour later.
- **Gap detection** (`Phase::beats`, `gaps_over`) counts every line: an
  automatic beat is still evidence the session was there.
- **Liveness** (`Lease::last_seen`) takes the last line whatever tag follows it —
  an automatic beat is precisely the evidence staleness exists to see.
- **The unvouched threshold** (`Phase::vouched`) asks whether any *bare* line
  exists, so `max_unvouched_minutes` keeps meaning "the model never vouched"
  rather than "the file is empty", and a hook-only phase is not dropped onto the
  shorter gap threshold.

`audit::unaccounted` subtracts each same-project lease's covered interval
(`mark.start → min(now, expiry)`) from the session window and flags whatever
remains, so an expired mark stops vouching for the stretch it did not cover and
every existing warning surface starts firing on its own. Clamping the bound
alone was not enough: the any-overlap predicate around it still answered
"covered" for a still-open session whose mark expired at its head.
`tt agent list` grows a last-seen column, a `[stale]`
marker, and one indented line per stale mark holding the exact `tt agent end`
line that logs the work and clears it — `--trim` for a mark with a heartbeat to
measure to, the explicit-minutes form for one without, since `--trim` there
reads the whole span as one gap and logs the 5-minute floor.

The thresholds are the two that already exist, and are the same pair `tt agent
end` judges a close by, so there is no new knob and no second vocabulary for
the same question.

Renewal rides the hooks that already fire. `touch_all_in` becomes
`touch_project_in`, `Stop` and `SubagentStop` pass the project they resolved,
and `UserPromptSubmit` gains a ledger-silent `activity prompt <project>`.
Together they renew a live session's marks at every turn boundary, and only the
marks of the project the beating session resolved; no resolved project means no
beat.

`tt agent end`'s measurement, its thresholds and `gaps_over` are unchanged; the
beat line's optional tag is the only format change.

## Alternatives considered

- **An implicit beat at `begin`.** This was the bug report's own suggestion.
  Rejected: `end`'s threshold switch is whether the model vouched, so a beat at
  `begin` collapses the distinction between an unmeasured phase and a beaten
  one — and the common `begin` → work 20m → `end` flow with no touches, which
  today logs 20 minutes by measuring to now, would start logging the 5-minute
  floor.
- **A bulk `tt agent sweep` that closes stale marks.** Rejected: `end --trim`
  already implements the reclamation, and a printed command keeps the operator
  in the loop about work only they can summarise, Vim-swap-file style.
- **Stamping the owning session id into the mark to prove abandonment rather
  than infer it.** A real improvement in attribution, but it needs a mark-file
  format change and buys nothing a timestamp does not already give.
- **Adding last-seen to `Mark`.** Rejected: it would either rot behind the
  TUI's directory-stamp refresh or force a beats read on every frame.
