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
//
// `end` is the whole `Stop` event in one process: it closes the window, beats
// this project's marks, then warns about open marks and an unaccounted window.
// Two entries on one event cannot be ordered, and the beat must precede the
// check.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

function readStdin() {
  try {
    return readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

// `TT_PROJECT`, else the session's own directory. Never the hook process's own
// cwd: it may sit in a worktree or an unrelated repo, and a wrong-but-resolvable
// answer would beat the wrong project's marks. No project means no beat at all.
function projectName(cwd) {
  if (process.env.TT_PROJECT) return process.env.TT_PROJECT;
  if (!cwd) {
    // One line, so the silence is visible without failing the event.
    process.stderr.write(
      "tt: no cwd in the hook payload and TT_PROJECT unset; no mark beaten\n",
    );
    return null;
  }
  try {
    const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
      encoding: "utf8",
      cwd,
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (root) return root.split(/[\\/]/).pop();
  } catch {
    // not a repo: no project, no beat
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

if (event !== "end") {
  process.stdout.write("{}");
  process.exit(0);
}

// Warnings, never a block. Both run after the call above, so the beat this
// project's marks got is already on disk when the mark list is read.
const messages = [];

// `tt` does the filtering: it owns the sanitise rule that decides which marks
// a project name owns, and this script must not parse its output.
if (project) {
  let list = "";
  try {
    list = execFileSync("tt", ["agent", "list", project], { encoding: "utf8" });
  } catch (e) {
    list = e.stdout?.toString() ?? "";
  }

  const open = list.trim();
  if (open && open !== "No open marks.") {
    messages.push(
      `tt-time-logging: open mark(s) for '${project}' are still unclosed. ` +
        `Close with 'tt agent end <project> <issue> <phase> "<summary>"' ` +
        `(or 'tt agent cancel' if it shouldn't be logged) before stopping.\n` +
        open,
    );
  }
}

let checked = "";
try {
  checked = execFileSync(
    "tt",
    ["agent", "activity", "check", "--auto-log", sessionId],
    { encoding: "utf8" },
  );
} catch {
  checked = "";
}
const trimmed = checked.trim();
if (trimmed) {
  // `tt` only marks a line `(auto-logged)` when `agent.auto_log_on_stop`
  // is configured and it actually wrote an entry for that line — see
  // docs/decisions/0003-auto-log-on-stop.md.
  const autoLogged = trimmed
    .split("\n")
    .some((line) => line.includes("(auto-logged)"));
  messages.push(
    autoLogged
      ? `tt-time-logging: this session's activity was unaccounted for — ` +
          `auto-logged under #auto (agent.auto_log_on_stop):\n${trimmed}`
      : `tt-time-logging: this session's activity is unaccounted for — no open mark ` +
          `or logged #agent entry covers it:\n${trimmed}`,
  );
}

process.stdout.write(
  messages.length > 0 ? JSON.stringify({ systemMessage: messages.join("\n\n") }) : "{}",
);
