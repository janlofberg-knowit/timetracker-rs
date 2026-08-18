//! The `tt agent` commands: the agent layer's phase marks.
//!
//! Presentation only. Every fact about the mark files themselves lives in
//! [`crate::marks`], which owns the format for both the reader and the writer;
//! this module turns the four subcommands into calls on it and prints what the
//! shell wrapper printed.
//!
//! **The mark commands touch no store.** A mark is not an entry, so `begin`,
//! `touch`, `cancel` and `list` never read `data.json`, take the store lock or
//! call `storage::with_data`, and `main` dispatches them *ahead* of its migration
//! preamble to keep that true. `item` and `end` are the complement: they create
//! an entry through [`crate::cli::log`], so they dispatch *after* the preamble.
//! See `AgentCommands::touches_store`.
//!
//! The messages are `bin/tt-safe`'s, verbatim apart from the `tt-safe: ` prefix
//! becoming `tt: `, because the caller of these commands is an agent following
//! prose instructions and a changed wording is a changed contract. The one
//! deliberate divergence in the whole port is `list`'s row format, which follows
//! `tt`'s house style rather than the wrapper's — see [`list`].

use anyhow::{Context, Result};

use crate::tracker::IdleInterval;

use crate::cli::{self, AgentCommands};
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
        AgentCommands::Item {
            project,
            issue,
            phase,
            summary,
            minutes,
        } => item(
            project,
            issue,
            phase,
            summary.as_deref(),
            minutes.as_deref(),
        ),
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

/// `tt agent item <project> <issue|-> <phase> <summary> <minutes>`: log one
/// finished piece of work in one call, with no mark involved.
///
/// The whole of `bin/tt-safe`'s `item`: the convention's three tags, the
/// 15-minute rounding, and nothing else. No mark file is read, written or
/// cleared — a phase short enough to report in one call never needed one.
fn item(
    project: &str,
    issue: &str,
    phase: &str,
    summary: Option<&str>,
    minutes: Option<&str>,
) -> Result<()> {
    let (Some(summary), Some(minutes)) = (summary, minutes) else {
        eprintln!("tt: usage: tt agent item <project> <issue|-> <phase> <summary> <minutes>");
        std::process::exit(64);
    };
    let minutes = whole_minutes(minutes);
    log_entry(project, issue, phase, summary, minutes, Vec::new(), false)
}

/// A minutes argument as the wrapper accepted it, or exit 64.
///
/// Hand-parsed rather than typed as an integer in the clap surface: clap answers
/// a non-numeric positional with exit **2** and its own usage block, where the
/// wrapper says `minutes must be a whole number, got '<x>'` and exits 64
/// (`bin/tt-safe:144-147`) — and the caller here is an agent following prose
/// instructions, for which a changed message is a changed contract.
///
/// Bash's `case "$m" in ''|*[!0-9]*)` is stricter than `parse`: no sign, no
/// whitespace, no `+`.
fn whole_minutes(raw: &str) -> i64 {
    let digits = !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit());
    match digits.then(|| raw.parse().ok()).flatten() {
        Some(minutes) => minutes,
        None => {
            eprintln!("tt: minutes must be a whole number, got '{raw}'");
            std::process::exit(64);
        }
    }
}

/// Log the entry both `item` and `end` end at: the convention's description, the
/// rounded duration, the project as a real field, and whatever silence was found.
///
/// `extra_tags` is deliberately empty — every tag is already in the description,
/// so `parse_tags` produces exactly the set the wrapper's `--description=` did.
fn log_entry(
    project: &str,
    issue: &str,
    phase: &str,
    summary: &str,
    minutes: i64,
    idle: Vec<IdleInterval>,
    trim: bool,
) -> Result<()> {
    cli::log(
        description(project, issue, phase, summary),
        format!("{}m", round_quarter(minutes)),
        Vec::new(),
        Some(project.to_string()),
        idle,
        trim,
    )
}

// --- the shared convention -------------------------------------------------
//
// `item` and `end` both log an entry, and they must log it the same way: the same
// rounding, the same stray-`#` stripping and the same three tags. The convention
// therefore lives here once, in the module that owns the agent layer's
// presentation, rather than being spelled out in each handler.

/// Round minutes up to the nearest quarter hour, never below 15.
///
/// `((m + 7) / 15) * 15` on integers, `bin/tt-safe:110`'s `round_quarter`
/// verbatim: the `+ 7` makes it round to the *nearest* quarter with the halfway
/// point going up, and the floor makes a two-minute errand cost a quarter of an
/// hour, because a quarter hour is the smallest unit anybody bills.
fn round_quarter(minutes: i64) -> i64 {
    (((minutes + 7) / 15) * 15).max(15)
}

/// Strip a `#` run that begins a word, keeping the word and the whitespace it
/// followed.
///
/// `bin/tt-safe:123`'s `sed -E 's/(^|[[:space:]])#+/\1/g'`. [`parse_tags`]
/// harvests every whitespace-delimited word starting with `#`, so a summary that
/// merely mentions "#12" would silently become a tag — and since the store
/// migration infers a project from the tags, a stray numeric one can even be
/// picked as the project (#11). A **mid-word** `#` is deliberately left alone,
/// because `parse_tags` ignores it and `C#`/`F#` are real words.
///
/// The double space this can leave behind is invisible in the stored description:
/// `parse_tags` splits on whitespace and re-joins on single spaces.
///
/// [`parse_tags`]: crate::tracker::parse_tags
fn strip_stray_tags(summary: &str) -> String {
    let mut stripped = String::with_capacity(summary.len());
    let mut at_word_start = true;
    for c in summary.chars() {
        if c == '#' && at_word_start {
            // The whole run goes, and the position stays a word start so the
            // rest of it goes too — `#+` in the sed is greedy for that reason.
            continue;
        }
        at_word_start = c.is_ascii_whitespace();
        stripped.push(c);
    }
    stripped
}

/// The description `cli::log` is given: the summary plus the convention's tags.
///
/// One string rather than a `Vec` of `extra_tags` because `cli::log` runs
/// `parse_tags` on its `description` (`src/cli.rs:237`), so this reproduces the
/// wrapper's `--description=` argv exactly — same tags, same order, same stored
/// prose.
///
/// Three tags, deliberately sparse, one per axis the `project` field cannot
/// express: the item (`<project>/<issue>`, omitted entirely for the `-`
/// sentinel), the phase, and `#agent` marking the entry as written by an agent
/// rather than by hand. There is **no bare `#<project>` tag**: the project is a
/// real field with its own axis, and duplicating it only made the tag list name
/// every project twice (`bin/tt-safe:150-154`).
fn description(project: &str, issue: &str, phase: &str, summary: &str) -> String {
    let mut description = strip_stray_tags(summary);
    if issue != "-" {
        description.push_str(&format!(" #{project}/{issue}"));
    }
    description.push_str(&format!(" #{phase} #agent"));
    description
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_goes_up_to_the_nearest_quarter() {
        // The halfway point rounds up, and each side of it lands where it should.
        assert_eq!(round_quarter(37), 30);
        assert_eq!(round_quarter(38), 45);
        assert_eq!(round_quarter(43), 45);
        assert_eq!(round_quarter(45), 45);
    }

    #[test]
    fn rounding_never_goes_below_a_quarter_hour() {
        // A quarter hour is the smallest unit anybody bills, so a two-minute
        // errand costs one — and zero minutes is still a quarter, not nothing.
        assert_eq!(round_quarter(0), 15);
        assert_eq!(round_quarter(2), 15);
        assert_eq!(round_quarter(7), 15);
        assert_eq!(round_quarter(8), 15);
    }

    #[test]
    fn a_word_starting_with_a_hash_keeps_the_word_and_loses_the_hash() {
        // What #11 was about: `parse_tags` would harvest `#12` as a tag.
        assert_eq!(strip_stray_tags("closed #12 at last"), "closed 12 at last");
        assert_eq!(strip_stray_tags("#12 closed"), "12 closed");
        // A run goes whole, and the whitespace it followed survives.
        assert_eq!(strip_stray_tags("closed ##12"), "closed 12");
        assert_eq!(strip_stray_tags("a\t#12"), "a\t12");
    }

    #[test]
    fn a_mid_word_hash_survives() {
        // `parse_tags` ignores it, and C#/F# are real words.
        assert_eq!(
            strip_stray_tags("ported the C# bridge"),
            "ported the C# bridge"
        );
        assert_eq!(strip_stray_tags("F#"), "F#");
    }

    #[test]
    fn the_description_carries_the_item_phase_and_agent_tags() {
        assert_eq!(
            description("loremind", "77", "impl", "store/links boundary"),
            "store/links boundary #loremind/77 #impl #agent"
        );
    }

    #[test]
    fn the_sentinel_issue_drops_the_item_tag_and_no_bare_project_tag_is_emitted() {
        let built = description("loremind", "-", "plan", "sketched the shape");
        assert_eq!(built, "sketched the shape #plan #agent");
        // The project is a real field with its own axis — never a tag.
        assert!(
            !built.contains("#loremind"),
            "a bare project tag was emitted: {built:?}"
        );
    }
}
