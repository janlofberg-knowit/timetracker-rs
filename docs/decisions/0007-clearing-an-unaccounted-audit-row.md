# 0007 — Clearing an unaccounted audit row

## Status

Implemented.

## Context

The unaccounted-activity list is only useful if its default state is empty: an
operator who reads it has to be able to act on every row until none is left.

Nothing could clear a row. `tt agent audit --auto-log` writes one fixed-phase
`#auto` entry past a threshold most operators never set — see
`0002-auto-logging-unaccounted-activity.md` — and every other write stamps its
entry ending *now*, so it cannot cover a past window and the same rows are named
again on the next audit. A row could also not be *named*: with reconciliation
per session (`0006-bounding-an-abandoned-activity-session.md`) two sessions
working one project in the same minute print rows whose text is identical.

## Decision

**1. A row is addressed by its session and its start epoch.** `activity::Session`
carries the sanitised key its file is named with, `audit::Unaccounted` carries it
through, and `tt agent audit --json` prints both with exact epochs. The
human-readable row carries the id's leading characters as a hint, for the eye
only; every command takes the full id. `--project <name>` narrows the list to one
repo's rows so an orchestrator sweeps only its own.

**2. A row that was real work is covered by an entry spanning it exactly.**
`tt agent resolve --session <id> <start> <issue|-> <phase> "<summary>"` matches
the row and logs an entry pinned to its end for its own span, with the project
taken from the matched row and the tags `tt agent item` writes. It never rounds:
`ended_at` is pinned, so a rounded duration would reach back past the row's
start. A pair matching no row exits 64 having written nothing, which is what
keeps time off a window the audit never flagged.

**3. A row that was not work is recorded in a dismissal ledger, not as an
entry.** `uncovered_by_entries` subtracts only `#agent`/`#auto` entries and skips
any shorter than the stretch it would cover, so no entry can clear a row without
billing its minutes to `tt report`. `dismissed/` is therefore a third subtracting
source beside leases and entries — one small file per dismissal, named after the
project key, the session and the span, written with `create_new` so a repeat is
idempotent. `tt agent dismiss --session <id> <project> <start>-<end>
["<reason>"]` writes one, and the audit subtracts it from the fragments of the
session it names, before the idle cut and the floor sum, so a dismissed stretch
counts toward no total. The scope is one session for the same reason the rows
are: project-scoped, one agent's "that was not work" would erase a concurrent
agent's row for the same minutes.

**4. `tt agent end` records its own uncovered remainder as a dismissal**, which
is what makes an empty list the default rather than something an operator
chases. Explicit minutes end the entry now and leave the mark's head uncovered;
`--trim` leaves every gap it cuts. Both are statements that the time was not
work. `--full` writes none — its entry spans its own gaps. Because a dismissal
must name a session and the model is told nothing about the harness, the
per-prompt card injected by `tt-contract-hook.mjs` now names this session's id.
Absent `--session`, `end` writes nothing, guesses no session, and names the
remainder on stderr; a failed dismissal warns and changes no exit code.

Two causes of unclearable rows are removed with the verbs. A fragment shorter
than one whole minute is no longer a row at all: it printed `0h 0m` and no entry
could ever cover it. And an open mark held across a phase covers its span by
existing, so the contract now has the orchestrator hold one `--agent
orchestrator` mark per phase rather than open and close one per dispatch.

## Alternatives considered

- **Dropping a row with no trace.** Rows are recomputed from the ledger on every
  audit, so a row with nothing recorded against it comes back.
- **An entry tagged `#dismissed`.** It would bill the time.
- **A project-scoped dismissal from `end`.** One agent's close would erase a
  concurrent agent's row for the same minutes.
- **Leaving `end` alone and clearing every tail through the sweep.** It works,
  but pays a command per close for a fact the harness can state once.
- **A minute count in `--json`.** Left out: it is the difference of two fields
  in the same object, and materialising it needs either a second type or a
  stored duplicate of derived state.

## Consequences

`tt report`'s overlap counter is no longer a pure drift signal. Two agents
working one project in the same minute are meant to sum, so entries that overlap
by design are expected, and a climbing counter no longer implies that logging
drifted away from the moments it belongs to.

`0002`'s principle is untouched: nothing here guesses a phase. `resolve` takes
the phase from the caller and is not a way to reclassify a `#auto` entry.

## Known limitations

**Two concurrent same-project rows cannot both be resolved.** An entry carries
no session, and the audit subtracts every same-project `#agent`/`#auto` entry
from every session's fragments. So the first `resolve` covers the second
session's overlapping stretch as well, and the second row is gone before it can
be addressed — `resolve` exits 64 on it. Summing two agents' concurrent work
therefore still goes through `tt agent item` for the second agent. Making entry
coverage session-aware is a store change and is not decided here.
