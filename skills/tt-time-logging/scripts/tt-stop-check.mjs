#!/usr/bin/env node
// Stop hook: warns (never blocks) if this project has tt agent marks still
// open when the agent stops, so a forgotten `tt agent end` doesn't silently
// lose the phase. Installed by install-hooks.mjs.

import { execSync } from "node:child_process";

function projectName() {
  try {
    const root = execSync("git rev-parse --show-toplevel", { encoding: "utf8" }).trim();
    return root.split(/[\\/]/).pop();
  } catch {
    return null;
  }
}

const proj = projectName();
if (!proj) {
  process.stdout.write("{}");
  process.exit(0);
}

let list = "";
try {
  list = execSync("tt agent list", { encoding: "utf8" });
} catch (e) {
  list = e.stdout?.toString() ?? "";
}

const openLines = list
  .split("\n")
  .filter((line) => line.includes(proj));

if (openLines.length > 0) {
  const msg =
    `tt-time-logging: open mark(s) for '${proj}' are still unclosed. ` +
    `Close with 'tt agent end <project> <issue> <phase> "<summary>"' ` +
    `(or 'tt agent cancel' if it shouldn't be logged) before stopping.\n` +
    openLines.join("\n");
  process.stdout.write(JSON.stringify({ systemMessage: msg }));
} else {
  process.stdout.write("{}");
}
