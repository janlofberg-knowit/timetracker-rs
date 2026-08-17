# timetracker-rs

This is just a personal project to test out rust in different ways while also building a tool that can help me keep track of my working hours.

```cargo install --git https://github.com/linus-skold/timetracker-rs```

## Tags

Any `#word` in a description is pulled out as a tag, on both `start` and `log`:

```
tt start writing the readme #docs #timetracker
tt log "fixed the cursor drift #bugfix" 45m
```

Tags are shown alongside the entry in `list`, `today` and `status`. In the TUI,
`f` filters down to the tags on the selected entry.
