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
and two readers that differ deliberately:

- **Judgement** (`Phase`, and through it what `end` bills and refuses) reads
  **bare lines only**. A hook beat is presence, not work, so it enters no part of
  the measurement: `Phase.beats` drops tagged lines, `end` measures to the last
  bare beat wherever it sits, and it falls back to measuring to now only when
  there is no bare beat at all. Anchoring on the final line instead, tag and all,
  would discard a model vouch the moment a hook beat followed it — a phase
  touched at minute 20 and hook-beaten at 21 would bill 61 minutes at a close an
  hour later.
- **Liveness** (`Lease::last_seen`) takes the last line whatever tag follows it —
  an automatic beat is precisely the evidence staleness exists to see.
  `Lease::vouched` says whether any bare line is present, which is what the
  printed close line keys on.

**An unvouched phase is judged exactly as if its beats file were empty**: one
whole-span silence from `begin` to whatever `end` is measuring to, against
`max_unvouched_minutes`. A vouched phase measures to its last bare beat and
judges the holes between bare beats at `max_gap_minutes`. There is no split
between interior and trailing silence.

Judging presence and work by different thresholds in one walk over one list was
tried first and broke in both directions at once: two hook beats could bracket
118 minutes of operator idle inside the unvouched grace and bill it at exit 0,
while a 100-minute unvouched span with one hook beat at minute 5 was refused and
its `--trim` logged the 5-minute floor — a span an *empty* beats file logs
cleanly. Filtering the channel is smaller than tuning the judge, and has no
direction left to break in.

The accepted cost: a phase touched at minute 0 and again at minute 60 with hook
beats in between is refused for that 60-minute hole, because the hook beats do
not shorten it. Under-billing is as bad as over-billing, so a refusal is
preferred to a silent guess; the model touches as the work runs, or passes the
minutes.

This is also what the printed close line keys on: `--trim` for a phase the model
vouched for, the explicit-minutes form otherwise, since whole-span judgement
would trim a hook-only mark to the 5-minute floor. `Lease::is_expired_at` judges
by the same rule `gaps_over` does — integer-floor minutes, strictly greater —
rather than comparing instants, which had a mark one second past its expiry
printing a `--trim` that would trim nothing.

`audit::unaccounted` subtracts each same-project lease's covered interval
(`mark.start → min(now, expiry)`) from the session window and flags whatever
remains, so an expired mark stops vouching for the stretch it did not cover and
every existing warning surface starts firing on its own. Clamping the bound
alone was not enough: the any-overlap predicate around it still answered
"covered" for a still-open session whose mark expired at its head.
`tt agent list` grows a last-seen column, a `[stale]`
marker, and one indented line per stale mark holding the exact `tt agent end`
line that logs the work and clears it.

The thresholds are the two that already exist, and are the same pair `tt agent
end` judges a close by, so there is no new knob and no second vocabulary for
the same question.

Renewal rides the hooks that already fire. `touch_all_in` becomes
`touch_project_in`, `Stop` and `SubagentStop` pass the project they resolved,
and `UserPromptSubmit` gains a ledger-silent `activity prompt <project>`.
Together they renew a live session's marks at every turn boundary, and only the
marks of the project the beating session resolved; no resolved project means no
beat.

`tt agent end`'s thresholds and `gaps_over` itself are unchanged. Two format
changes carry all of this: the beat line's optional tag, and a mark key that
sanitises its three segments before joining them, so a `.` in a project or issue
becomes `-` and every key holds exactly two dots — the project is then the first
segment by construction rather than by a second parser that disagreed with the
first.

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
  than infer it.** It is the only thing that catches a same-project orphan (see
  the limitation below), which no timestamp can. Deferred rather than rejected:
  it needs a mark-file format change, which is its own Story.
- **Adding last-seen to `Mark`.** Rejected: it would either rot behind the
  TUI's directory-stamp refresh or force a beats read on every frame.

## Known limitations

**A same-project orphan never expires.** Renewal is scoped to the project, not
to the session that opened the mark, so an abandoned mark in a repo someone
keeps working is renewed by every new session's turn boundaries — the weekend
incident's shape minus the weekend. Only a session id on the mark distinguishes
the orphan from the live work, and that is deferred. Until then `tt agent list`
is how such a mark gets noticed.
