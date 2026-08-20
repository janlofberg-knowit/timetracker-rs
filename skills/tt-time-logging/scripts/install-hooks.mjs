#!/usr/bin/env node
// Claude-Code-specific enforcement for the tt-time-logging skill.
//
// `npx skills add` only copies SKILL.md (and this directory) into place — it
// is tool-agnostic and knows nothing about Claude Code's hooks. Prose in a
// skill is model-invoked and gets skipped under context pressure, so this
// script wires two hooks into the *consumer's own* .claude/settings.json:
//
//   SessionStart - injects this skill's contract into every session's
//                  context directly, unconditionally (no model judgment).
//   Stop         - runs `tt agent list` and warns (non-blocking) if this
//                  project has marks still open when the agent stops.
//
// Safe to re-run: both entries are added only if not already present.
//
// Usage (from the consumer repo root, after `npx skills add ...`):
//   node .claude/skills/tt-time-logging/scripts/install-hooks.mjs

import { existsSync, mkdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skillDir = dirname(scriptDir); // .../.claude/skills/tt-time-logging
const cwd = process.cwd();

const settingsPath = join(cwd, ".claude", "settings.json");
const hooksDir = join(cwd, ".claude", "hooks");
const stopCheckDest = join(hooksDir, "tt-stop-check.mjs");
const stopCheckSrc = join(scriptDir, "tt-stop-check.mjs");
const skillMdRelFromCwd = relative(cwd, join(skillDir, "SKILL.md")).replace(/\\/g, "/");

mkdirSync(hooksDir, { recursive: true });
copyFileSync(stopCheckSrc, stopCheckDest);

let settings = {};
if (existsSync(settingsPath)) {
  settings = JSON.parse(readFileSync(settingsPath, "utf8"));
} else {
  mkdirSync(dirname(settingsPath), { recursive: true });
}

settings.hooks ??= {};
settings.hooks.SessionStart ??= [];
settings.hooks.Stop ??= [];

const sessionStartCmd = `node -e "const fs=require('fs');process.stdout.write(JSON.stringify({hookSpecificOutput:{hookEventName:'SessionStart',additionalContext:fs.readFileSync('${skillMdRelFromCwd}','utf8')}}))"`;
const stopCmd = "node .claude/hooks/tt-stop-check.mjs";

const hasCommand = (event, command) =>
  settings.hooks[event].some((entry) => entry.hooks?.some((h) => h.command === command));

if (!hasCommand("SessionStart", sessionStartCmd)) {
  settings.hooks.SessionStart.push({
    hooks: [{ type: "command", command: sessionStartCmd, statusMessage: "Loading tt-time-logging contract" }],
  });
}

if (!hasCommand("Stop", stopCmd)) {
  settings.hooks.Stop.push({
    hooks: [{ type: "command", command: stopCmd, statusMessage: "Checking for unclosed tt marks" }],
  });
}

writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + "\n");

console.log(`tt-time-logging hooks installed into ${relative(cwd, settingsPath)}.`);
console.log("Restart Claude Code or open /hooks once so the new settings file is picked up.");
