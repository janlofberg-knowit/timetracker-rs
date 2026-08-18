//! The `tt agent` commands: the agent layer's phase marks.
//!
//! Presentation only. Every fact about the mark files themselves lives in
//! [`crate::marks`], which owns the format for both the reader and the writer;
//! this module turns the four subcommands into calls on it and prints what the
//! shell wrapper printed.
//!
//! **Nothing here touches the store.** A mark is not an entry — the work becomes
//! a `tt` entry only at `end` — so no handler reads `data.json`, takes the store
//! lock or calls `storage::with_data`. `main` dispatches these commands *ahead*
//! of its migration preamble to keep that true; see the comment there.
//!
//! The messages are `bin/tt-safe`'s, verbatim apart from the `tt-safe: ` prefix
//! becoming `tt: `, because the caller of these commands is an agent following
//! prose instructions and a changed wording is a changed contract. The one
//! deliberate divergence in the whole port is `list`'s row format, which follows
//! `tt`'s house style rather than the wrapper's — see [`list`].

use anyhow::{Context, Result};

use crate::cli::AgentCommands;
use crate::icons;
use crate::marks::{self, Begin, Touch};

/// Run one `tt agent` subcommand.
pub fn run(command: &AgentCommands) -> Result<()> {
    match command {
        AgentCommands::Begin {
            project,
            issue,
            phase,
        } => begin(project, issue, phase),
        AgentCommands::Touch {
            project,
            issue,
            phase,
        } => touch(project, issue, phase),
        AgentCommands::Cancel {
            project,
            issue,
            phase,
        } => cancel(project, issue, phase),
        AgentCommands::List => list(),
    }
}

/// The mark directory, or a real error rather than a silent no-op: a `begin`
/// that quietly recorded nothing would lose the phase's start.
fn mark_dir() -> Result<std::path::PathBuf> {
    marks::mark_dir().context("could not determine a cache directory for the marks")
}

/// The phase as the wrapper's messages name it: `<project>/<issue> <phase>`,
/// with the `-` sentinel left in place. Only `list` collapses it, because only
/// `list` is reading names back rather than echoing the ones it was given.
fn phase_name(project: &str, issue: &str, phase: &str) -> String {
    format!("{}/{} {}", project, issue, phase)
}

/// `tt agent begin <project> <issue|-> <phase>`: open a mark, or keep the one
/// already open.
///
/// Idempotent by design and not by accident: an agent that lost its context
/// re-begins the phase it is already inside, and the original start is the thing
/// worth keeping. Saying so on stderr with exit 0 means a re-begin is not an
/// error the caller has to handle.
fn begin(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    match marks::begin_in(&dir, project, issue, phase)? {
        Begin::Created(start) => println!(
            "marked {} at {}",
            phase_name(project, issue, phase),
            start.format("%H:%M")
        ),
        Begin::AlreadyOpen(start) => {
            // `??:??` for a mark whose contents are not a timestamp, exactly as
            // the wrapper's `fmt_time` falls back.
            let since = start.map_or_else(
                || "??:??".to_string(),
                |start| start.format("%H:%M").to_string(),
            );
            eprintln!(
                "tt: already marked {} (since {}) — using the original start",
                phase_name(project, issue, phase),
                since
            );
        }
    }
    Ok(())
}

/// `tt agent touch <project> <issue|-> <phase>`: record one heartbeat.
///
/// Exits 64 on an unmarked phase, which is `bin/tt-safe`'s code for a usage
/// error and the one exit code the oracle asserts here. `main` returns
/// `Result`, and anyhow exits 1, so the code has to be set explicitly.
fn touch(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    match marks::touch_in(&dir, project, issue, phase)? {
        Touch::Recorded => Ok(()),
        Touch::NoMark => {
            eprintln!(
                "tt: no mark for {} — nothing to touch",
                phase_name(project, issue, phase)
            );
            std::process::exit(64);
        }
    }
}

/// `tt agent cancel <project> <issue|-> <phase>`: drop a mark without logging.
///
/// Succeeds whether or not there was anything to drop: the caller wants the
/// phase gone, and it already is.
fn cancel(project: &str, issue: &str, phase: &str) -> Result<()> {
    let dir = mark_dir()?;
    marks::cancel_in(&dir, project, issue, phase)?;
    println!("dropped mark for {}", phase_name(project, issue, phase));
    Ok(())
}

/// `tt agent list`: every open mark, newest first.
///
/// The house style, not `tt-safe marks`' terse row: a `{} Open marks:` header in
/// the shape of `cli::list`'s `{} All entries:`, a blank line, and rows at the
/// two-column indent the entry lists reserve for their status glyph. Owner
/// ruling, 2026-08-18 — the port's one deliberate divergence from the shell
/// oracle, because a `tt` subcommand should look like `tt`.
///
/// The empty case is a bare `No open marks.` with no emoji, matching
/// `No entries yet.` — and, as it happens, the wrapper's own line exactly.
fn list() -> Result<()> {
    let marks = marks::open_marks();
    if marks.is_empty() {
        println!("No open marks.");
        return Ok(());
    }

    println!("{} Open marks:\n", icons::MARKS);
    for row in marks::rows(&marks) {
        println!("  {}", row);
    }
    Ok(())
}
