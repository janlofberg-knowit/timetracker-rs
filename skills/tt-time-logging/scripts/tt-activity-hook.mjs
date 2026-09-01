#!/usr/bin/env node
// SessionStart/UserPromptSubmit/Stop/SubagentStop hook: writes to `tt`'s hook-only activity
// ledger (`tt agent activity …`) — see
// docs/decisions/0001-agent-activity-tracking.md. Installed by
// install-hooks.mjs.
//
// Usage: node tt-activity-hook.mjs <begin|end|subagent|prompt>
//
// `session_id` (read from the hook's JSON payload on stdin) is the key every
// entry is filed under. Missing or unparseable payload: silently skipped —
// a hook must never fail the harness event it's attached to. `prompt` files
// nothing and only beats this project's open marks, so it needs no session id.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

function readStdin() {
  try {
    return readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

// An unresolved project means no beat at all, so both the payload's own `cwd`
// and the process's are tried. The payload comes **first**: the session's own
// directory is the project being worked in, while the hook process may sit in a
// worktree or an unrelated repo, and a wrong-but-resolvable answer would beat
// the wrong project's marks.
function projectName(cwd) {
  if (process.env.TT_PROJECT) return process.env.TT_PROJECT;
  const candidates = cwd ? [cwd, undefined] : [undefined];
  for (const dir of candidates) {
    try {
      const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
        encoding: "utf8",
        cwd: dir,
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
      if (root) return root.split(/[\\/]/).pop();
    } catch {
      // try the next candidate
    }
  }
  return null;
}

const event = process.argv[2];
if (!["begin", "end", "subagent", "prompt"].includes(event)) {
  process.stdout.write("{}");
  process.exit(0);
}

let payload = {};
try {
  payload = JSON.parse(readStdin()) ?? {};
} catch {
  payload = {};
}

const sessionId = payload.session_id;
// Every event but `prompt` is filed under the session id and needs one.
if (!sessionId && event !== "prompt") {
  process.stdout.write("{}");
  process.exit(0);
}

// Passed on every event: `begin` files it in the ledger, and the rest beat that
// project's open marks. An unresolved project means no beat at all.
const args = ["agent", "activity", event];
if (event !== "prompt") args.push(sessionId);
const project = projectName(payload.cwd);
if (project) args.push(project);

try {
  execFileSync("tt", args, { stdio: "ignore" });
} catch {
  // never fail the harness event over `tt` being missing or erroring
}

process.stdout.write("{}");
