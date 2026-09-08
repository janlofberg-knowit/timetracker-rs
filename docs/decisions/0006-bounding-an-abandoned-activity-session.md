# 0006 — Reconciling per session, and bounding an abandoned one

## Status

Implemented.

## Context

An activity session is one file per Claude Code session, written only by hooks.
`Stop` appends its `end=`; a session that crashes, or whose hooks were never
installed, leaves the file open forever.

`audit::unaccounted` measured such a session to `now`. One open file therefore
reported a row that grew with every audit — 316 hours by the time it was
noticed — and it overlapped every later session of the same project, which
printed the same stretch twice.

The answer released in 0.10.1 merged each project's sessions into one
non-overlapping window before the mark and entry subtraction. That removed the
duplicate rows and, with them, the second agent: two orchestrators working one
project in the same minute became one window and one row.

## Decision

**Reconciliation is per session.** Two orchestrators dispatching in the same
minute are two agents whose work must sum, so each session is reconciled
against the marks and entries on its own: the project is the session's, the
dispatch evidence is the session's `subagent=` lines, and the floor applies to
that session's own active total. This supersedes the per-project merge released
in 0.10.1; the duplicate rows that motivated it are removed by the bound below
instead.

**An open session's evidence of life is its `start` and its own `subagent=`
lines.** Nothing else is read. The file's mtime is redundant — every writer
appends a timestamped line, so for an open session the mtime equals the last
parsed line. Mark beats are redundant too: the stretch a same-project lease
vouches for is already subtracted, and a beat that proves a session was alive
also refreshes that lease.

**The bound is the lease's own grace**, through the one function
`marks::grace_minutes` that `Lease::expires_at` also calls — see
`0004-mark-expiry.md` for the rule and for the principle it rests on, that no
evidence is treated as no evidence. An open session ends at its last evidence
plus that grace, clamped to `now`; a session with an `end=` keeps it.

**The bounded trailing row reads `[abandoned]`**, in the position the `[stale]`
marker takes on a mark row. Only the row that ends at the bound carries it.

## Alternatives considered

- **A second copy of the expiry arithmetic in `audit.rs`.** Rejected: two
  copies of one boundary is the drift 0004 closed between the coverage bound
  and the `[stale]` marker.
- **Bounding at the last evidence with no grace.** The abandoned session would
  then report nothing at all, and a bounded row is what the operator has to act
  on.
- **Keeping the per-project merge and de-duplicating rows some other way.**
  Merging is what loses the second agent's work.

## Known limitations

**Two rows can print identical text.** Two sessions covering the same minutes
of one project produce rows whose text is character-for-character equal. The
sum is the point; addressing such a row on its own is `tt agent resolve`'s job.

**A live session with no dispatches is under-reported until it stops.** The
`UserPromptSubmit` hook beats marks, not the session file, and carries no
session id, so a running session that dispatches nothing leaves no per-turn
evidence. It reports at most the unvouched grace while it runs, and its true
window appears when `Stop` writes `end=`. This is 0004's accepted cost applied
to the ledger.

**A mark opened with the session hides the row entirely.** An unvouched lease
from the same start expires exactly where the bound falls, so the subtraction
leaves nothing and the audit reports nothing; the `[stale]` row in `tt agent
list` is the operator's only nudge.

Healing the cache and pruning old activity files were both left out: no hook
writes an `end=` it did not observe.
