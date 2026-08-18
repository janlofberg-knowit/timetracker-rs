# timetracker-rs

A personal time tracking CLI built in Rust. Track your working hours directly from the terminal.

## Installation

```sh
cargo install --git https://github.com/linus-skold/timetracker-rs
```

## Commands

### `tt start <description>`

Start tracking a new task. Only one task can be active at a time.  
Tags can be embedded inline using `#tag` syntax.

```sh
tt start Working on login page
tt start Fixing auth bug #backend #bugfix
```

---

### `tt stop`

Stop the currently active task and record its duration.

```sh
tt stop
```

---

### `tt log -d <description> -t <duration> [--tags <tags>]`

Log a completed task with an explicit duration. Useful for recording work after the fact.

- `-d` / `--description` — description of the task
- `-t` / `--time` — duration (see [Duration Format](#duration-format))
- `--tags` — comma-separated list of tags *(optional)*

Tags can also be embedded inline in the description using `#tag` syntax. Both styles can be combined.

```sh
tt log -d "Code review" -t 45m
tt log -d "Deploy to staging" -t 1h30m --tags devops,deployment
tt log -d "Fix #frontend bug" -t 2h --tags bugfix
```

---

### `tt today`

Show all entries logged today along with a total.

```sh
tt today
```

---

### `tt report [--week|--all|--since <date>] [--until <date>] [--project <name>] [--json]`

Roll up logged time by project and item, with a per-phase breakdown.

- `--week` — this week, from Monday
- `--all` — every entry, unbounded
- `--since <YYYY-MM-DD>` — from that date onwards
- `--until <YYYY-MM-DD>` — up to and including that date; narrows one of the three
  scopes above and is a usage error without one
- `--project <name>` — only entries whose project is `<name>`
- `--json` — machine-readable output

With no scope it reports today. Project totals come from each entry's **project
field** (`--project` on `tt start` / `tt log`), never from its tags.

A trailing `*` marks an item with a running entry. When spans overlap, the report
says so: `tt log` back-dates from now, so entries logged in a batch claim
overlapping slots — the totals stay right, the timeline does not.

```sh
tt report
tt report --week
tt report --since 2026-08-01 --until 2026-08-05
tt report --project timetracker-rs --json
```

---

### `tt list`

Show the 20 most recent entries across all days.

```sh
tt list
```

---

### `tt status`

Show the currently active task, when it started, and how long it has been running.

```sh
tt status
```

---

### `tt active`

Prints `true` if a task is currently being tracked, `false` otherwise. Useful for scripts and prompt integrations.

```sh
tt active
```

---

### `tt tui`

Open the interactive terminal UI for browsing and managing your entries.

```sh
tt tui
```

---

## Duration Format

Durations are used with `tt log -t`. Supported formats:

| Input   | Meaning         |
|---------|-----------------|
| `2h`    | 2 hours         |
| `45m`   | 45 minutes      |
| `1h30m` | 1 hour 30 min   |
| `90`    | 90 minutes      |

---

## Tags

Tags can be added to any entry in two ways:

1. **Inline** in the description using `#` prefix: `tt start Working on #frontend`
2. **Explicit flag** with `tt log --tags tagA,tagB,tagC`

Both methods can be combined and duplicates are automatically removed.
