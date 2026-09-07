# 0005 — The `agent` namespace in an entry's custom data

## Status

Implemented.

## Context

An entry's `data` field is free-form JSON that nothing in `tt` reads. Agent work
has facts that neither the description, the tags nor the project field can hold —
which model did the work, at what reasoning effort, for how many tokens.

## Decision

Every agent fact lives under one top-level `agent` object. The schema is:

```json
{"agent": {"label": "code", "model": "opus", "effort": "high", "tokens": {"input": 1200, "output": 300}}}
```

`tt` writes exactly one of those keys: `tt agent end --agent <label>` merges
`agent.label` into whatever `--data` carried. The caller writes the rest through
`--data` and omits what it does not know. A `label` the caller wrote itself wins,
and an `agent` key that is not an object is a usage error before the entry is
recorded.

The merge is key-wise, never a replacement: every other key under `agent`, and
every other top-level key, survives. Nothing validates or requires a key, and
unknown keys are kept. A tool that wants fields of its own takes its own
top-level namespace.

Nothing in `tt` reads any of it yet. `docs/usage.md`'s "Well-known keys" is the
one place the schema is written down; `AGENTS.md` and
`skills/tt-time-logging/SKILL.md` tell an orchestrator what to pass.

## Consequences

- A reader can attribute a fan-out's rows to their subagents without a tag axis
  or a label in the summary prose.
- `--agent` is no longer mark-only: a labelled close now writes to the store
  even when the caller passed no `--data`.
- A caller that already hangs its own `agent` key on an entry it closes with
  `--agent` has to move it or make it an object.
