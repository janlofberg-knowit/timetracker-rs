use anyhow::Result;
use chrono::{Local, TimeZone};
use clap::{Parser, Subcommand};

use crate::duration;
use crate::icons;
use crate::storage::{load_data, with_data};
use crate::tracker::{IdleInterval, parse_tags};

#[derive(Parser)]
#[command(name = "tt", about = "Simple time tracking CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start tracking a new task
    Start {
        /// Description of the task
        #[arg(required = true)]
        description: Vec<String>,
        /// Project this entry belongs to
        #[arg(long)]
        project: Option<String>,
    },
    /// Stop the current active task
    Stop,
    /// Log a completed task with a specific duration
    Log {
        /// Description of the task
        #[arg(short = 'd', long)]
        description: String,
        /// Duration in format like "1h30m", "45m", "2h"
        #[arg(short = 't', long)]
        time: String,
        /// Comma-separated tags (e.g. tagA,tagB,tagC)
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Project this entry belongs to
        #[arg(long)]
        project: Option<String>,
        /// A silent stretch inside the logged span, as epoch seconds
        /// `<start>-<end>`. Repeatable; records the interval without changing the
        /// logged duration.
        #[arg(long, value_parser = parse_idle)]
        idle: Vec<IdleInterval>,
        /// Trim the recorded idle stretches out of the entry, splitting it into the
        /// pieces between them. Destructive and unconfirmed; requires `--idle`, so
        /// asking for a trim with nothing to trim is a usage error rather than a
        /// silent no-op.
        #[arg(long, requires = "idle")]
        trim: bool,
    },
    /// Show all entries for today
    Today,
    /// Show all entries
    List,
    /// Open interactive TUI
    Tui,
    /// Show current status
    Status,
    /// true/false if something is active
    Active,
    /// Phase marks for the agent layer
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
}

/// The agent layer's commands, ported from `bin/tt-safe`.
///
/// The first nested subcommand in this CLI, and nested rather than flat on
/// purpose: the positionals are the wrapper's verbatim, so every call site in the
/// agent instructions is a textual `tt-safe ` → `tt agent ` swap, and the verbs
/// stay visibly one namespace instead of a handful of top-level commands that
/// only an agent ever types.
///
/// Most of them touch mark files and nothing else; the ones that log an entry say
/// so through [`AgentCommands::touches_store`], which decides where in `main` they
/// dispatch.
#[derive(Subcommand)]
pub enum AgentCommands {
    /// Open a mark for a phase, keeping the start of one already open
    Begin {
        project: String,
        /// Issue number, or `-` for a phase with no issue
        issue: String,
        phase: String,
    },
    /// Record one heartbeat for an open phase
    Touch {
        project: String,
        /// Issue number, or `-` for a phase with no issue
        issue: String,
        phase: String,
    },
    /// Drop a phase's mark and heartbeats without logging anything
    Cancel {
        project: String,
        /// Issue number, or `-` for a phase with no issue
        issue: String,
        phase: String,
    },
    /// List every open mark
    List,
    /// Log one finished piece of work, with no mark involved
    Item {
        project: String,
        /// Issue number, or `-` for a phase with no issue
        issue: String,
        phase: String,
        /// 3-6 words of plain prose, with no issue number in them
        summary: Option<String>,
        /// Whole minutes, rounded up to the nearest quarter hour
        minutes: Option<String>,
    },
}

impl AgentCommands {
    /// Whether this subcommand creates or reads a `tt` entry.
    ///
    /// `main` returns the ones that do **not** ahead of its
    /// `storage::with_data(migrate)` preamble, so a heartbeat never takes the
    /// store's exclusive lock — the same thing `bin/tt-safe` skips its own lock
    /// for the mark verbs to achieve. The ones that do have to dispatch *after*
    /// it, or they would write an unmigrated store.
    ///
    /// An exhaustive `match` rather than a `matches!`, deliberately: a new
    /// subcommand cannot be added without deciding which side of the preamble it
    /// belongs on.
    pub fn touches_store(&self) -> bool {
        match self {
            AgentCommands::Begin { .. }
            | AgentCommands::Touch { .. }
            | AgentCommands::Cancel { .. }
            | AgentCommands::List => false,
            AgentCommands::Item { .. } => true,
        }
    }
}

/// Parse one `--idle` value: `<start>-<end>` in epoch seconds.
///
/// Epoch seconds rather than clock times because the only producer is `tt-safe`,
/// whose heartbeats are unix timestamps already (#12) — no timezone or format to
/// agree on. A malformed value is a parse error, never a silently dropped
/// interval: an interval that vanishes would make a trim quietly do less than the
/// caller asked.
fn parse_idle(value: &str) -> Result<IdleInterval, String> {
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| format!("expected <start>-<end> epoch seconds, got `{}`", value))?;
    let stamp = |field: &str, raw: &str| {
        raw.trim()
            .parse::<i64>()
            .map_err(|_| format!("{} is not epoch seconds: `{}`", field, raw))
            .and_then(|secs| {
                Local
                    .timestamp_opt(secs, 0)
                    .single()
                    .ok_or_else(|| format!("{} is not a valid timestamp: `{}`", field, raw))
            })
    };
    let start = stamp("start", start)?;
    let end = stamp("end", end)?;
    if end < start {
        return Err(format!(
            "idle interval ends before it starts: `{}`",
            value
        ));
    }
    Ok(IdleInterval::new(start, end))
}

/// Render a project for a single-line listing: ` (demo)`, or nothing when unset.
///
/// `--project` sets the field only — it never adds or rewrites a tag — so the
/// project is displayed separately from the tag list everywhere.
fn project_display(project: Option<&String>) -> String {
    match project {
        Some(p) => format!(" ({})", p),
        None => String::new(),
    }
}

pub fn start(description: Vec<String>, project: Option<String>) -> Result<()> {
    let raw_desc = description.join(" ");
    let (desc, tags) = parse_tags(&raw_desc);
    let start_time = Local::now();

    // The active check and the insert share one lock, so two concurrent starts
    // cannot both find nothing active
    let already_tracking = with_data(|data| {
        if let Some(active) = data.active_entry() {
            return Ok(Some((active.description.clone(), active.start_time)));
        }
        data.add_entry(
            desc.clone(),
            project.clone(),
            tags.clone(),
            start_time,
            None,
        );
        Ok(None)
    })?;

    if let Some((active_desc, active_start)) = already_tracking {
        println!(
            "{}  Already tracking: \"{}\" (started at {})",
            icons::WARNING,
            active_desc,
            active_start.format("%H:%M")
        );
        println!("Stop it first with: tt stop");
        return Ok(());
    }

    let tags_display = if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "))
    };

    println!(
        "{}  Started: \"{}\"{}{} at {}",
        icons::ACTIVE,
        desc,
        project_display(project.as_ref()),
        tags_display,
        start_time.format("%H:%M:%S")
    );
    Ok(())
}

pub fn stop() -> Result<()> {
    let stopped = with_data(|data| {
        // Get info before stopping
        let info = data.active_entry().map(|e| {
            (e.description.clone(), e.format_duration())
        });

        if data.stop_active() {
            Ok(info)
        } else {
            Ok(None)
        }
    })?;

    if let Some((desc, dur)) = stopped {
        println!("{}  Stopped: \"{}\" - Duration: {}", icons::STOPPED, desc, dur);
    } else {
        println!("No active task to stop.");
    }
    Ok(())
}

pub fn log(
    description: String,
    time_str: String,
    extra_tags: Vec<String>,
    project: Option<String>,
    idle: Vec<IdleInterval>,
    trim: bool,
) -> Result<()> {
    let dur = duration::parse(&time_str);
    let end_time = Local::now();
    let start_time = end_time - dur;

    let (desc, mut tags) = parse_tags(&description);
    for tag in extra_tags {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    with_data(|data| {
        // The id is kept rather than discarded: the intervals go on the entry that
        // was just created, inside the same lock, so nothing outside this process
        // ever has to name it.
        let entry_id = data
            .add_entry(
                desc.clone(),
                project.clone(),
                tags.clone(),
                start_time,
                Some(end_time),
            )
            .id;
        if let Some(entry) = data.entries.iter_mut().find(|e| e.id == entry_id) {
            entry.idle = idle;
        }
        // Deliberately inside the same closure as the insert: `with_data` is one
        // lock, one load, one save, so creating the entry and splitting it are a
        // single atomic store write and the id never leaves the process. Do not
        // lift this into a second store call or a second command — see #35's
        // rejected alternatives (no id-printing output for a wrapper to parse, no
        // `--last`-style target).
        if trim {
            data.split_at_idle(entry_id);
        }
        Ok(())
    })?;

    let tags_display = if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "))
    };

    println!(
        "{} Logged: \"{}\"{}{} - Duration: {}",
        icons::LOGGED,
        desc,
        project_display(project.as_ref()),
        tags_display,
        duration::format(dur)
    );
    Ok(())
}

pub fn today() -> Result<()> {
    let data = load_data()?;
    let today_entries = data.today_entries();

    if today_entries.is_empty() {
        println!("No entries for today.");
        return Ok(());
    }

    println!("{} Today's entries:\n", icons::CALENDAR);
    for entry in &today_entries {
        let status = if entry.is_active() { entry.status_icon() } else { "  " };
        let tags_display = if entry.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", entry.format_tags())
        };
        println!(
            "{}{} - {}{}{} ({})",
            status,
            entry.start_time.format("%H:%M"),
            entry.description,
            project_display(entry.project.as_ref()),
            tags_display,
            entry.format_duration()
        );
    }
    println!("\nTotal: {}", duration::format(data.today_total()));
    Ok(())
}

pub fn list() -> Result<()> {
    let data = load_data()?;

    if data.entries.is_empty() {
        println!("No entries yet.");
        return Ok(());
    }

    println!("{} All entries:\n", icons::LIST);
    for entry in data.entries.iter().rev().take(20) {
        let status = if entry.is_active() { entry.status_icon() } else { "  " };
        let tags_display = if entry.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", entry.format_tags())
        };
        println!(
            "{}{} {} - {}{}{} ({})",
            status,
            entry.start_time.format("%Y-%m-%d"),
            entry.start_time.format("%H:%M"),
            entry.description,
            project_display(entry.project.as_ref()),
            tags_display,
            entry.format_duration()
        );
    }
    Ok(())
}

pub fn status() -> Result<()> {
    let data = load_data()?;

    if let Some(active) = data.active_entry() {
        println!("{}  Currently tracking: \"{}\"", icons::ACTIVE, active.description);
        if let Some(project) = &active.project {
            println!("   Project: {}", project);
        }
        if !active.tags.is_empty() {
            println!("   Tags: {}", active.format_tags());
        }
        println!("   Started at: {}", active.start_time.format("%H:%M:%S"));
        println!("   Duration: {}", active.format_duration());
    } else {
        println!("No active task. Start one with: tt start <description>");
    }
    Ok(())
}

pub fn active() -> Result<()> {
    let data = load_data()?;

    if data.active_entry().is_some() {
        println!("true");
    } else {
        println!("false");
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;
    use crate::storage::env_guard;
    use clap::Parser;
    use std::path::PathBuf;

    /// Point `HOME` at a fresh scratch dir so the store resolves inside it. The
    /// real store is written continuously by live agent sessions and must never be
    /// touched by a test.
    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tt-cli-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("HOME", &dir) };
        let path = storage::get_data_path().unwrap();
        assert!(
            path.starts_with(&dir),
            "sandbox HOME not in effect: {path:?}"
        );
        dir
    }

    /// The `Log` variant as clap parses it from a real argument vector — so the
    /// flags are exercised through the same path the binary uses.
    fn parse_log(args: &[&str]) -> Commands {
        let mut argv = vec!["tt", "log"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv).expect("log arguments").command
    }

    fn run_log(command: Commands) {
        match command {
            Commands::Log {
                description,
                time,
                tags,
                project,
                idle,
                trim,
            } => log(description, time, tags, project, idle, trim).unwrap(),
            _ => panic!("parse_log produced something other than a Log command"),
        }
    }

    #[test]
    fn idle_intervals_are_recorded_without_changing_the_logged_duration() {
        let _guard = env_guard();
        sandbox("log-idle");
        let start = Local::now().timestamp() - 3600;
        let end = start + 900;

        run_log(parse_log(&[
            "-d",
            "wrote the thing",
            "-t",
            "90m",
            &format!("--idle={}-{}", start, end),
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(
            data.entries.len(),
            1,
            "--idle alone must not split anything"
        );
        let entry = &data.entries[0];
        assert_eq!(
            entry.duration(),
            duration::parse("90m"),
            "--idle changed the logged duration"
        );
        assert_eq!(entry.idle.len(), 1);
        assert_eq!(entry.idle[0].start.timestamp(), start);
        assert_eq!(entry.idle[0].end.timestamp(), end);
    }

    #[test]
    fn two_idle_arguments_are_both_recorded_in_order() {
        let _guard = env_guard();
        sandbox("log-idle-twice");
        let base = Local::now().timestamp() - 7200;

        run_log(parse_log(&[
            "-d",
            "long session",
            "-t",
            "2h",
            &format!("--idle={}-{}", base + 600, base + 900),
            &format!("--idle={}-{}", base + 3000, base + 3600),
        ]));

        let data = storage::load_data().unwrap();
        let stamps: Vec<(i64, i64)> = data.entries[0]
            .idle
            .iter()
            .map(|gap| (gap.start.timestamp(), gap.end.timestamp()))
            .collect();
        assert_eq!(
            stamps,
            vec![(base + 600, base + 900), (base + 3000, base + 3600)]
        );
    }

    #[test]
    fn a_malformed_idle_value_is_a_parse_error_not_a_dropped_interval() {
        for bad in ["notanumber", "100", "100-", "abc-200", "100-abc", "300-200"] {
            let parsed = Cli::try_parse_from([
                "tt",
                "log",
                "-d",
                "x",
                "-t",
                "5m",
                &format!("--idle={}", bad),
            ]);
            assert!(parsed.is_err(), "`{bad}` parsed instead of failing");
        }
    }

    #[test]
    fn trim_splits_the_logged_entry_in_the_same_command() {
        let _guard = env_guard();
        let dir = sandbox("log-trim");
        // Two holes inside a two-hour span, so a correct trim leaves three pieces.
        let end = Local::now().timestamp();
        let start = end - 7200;
        let gaps = [(start + 600, start + 1500), (start + 4000, start + 5800)];

        run_log(parse_log(&[
            "-d",
            "long session",
            "-t",
            "2h",
            &format!("--idle={}-{}", gaps[0].0, gaps[0].1),
            &format!("--idle={}-{}", gaps[1].0, gaps[1].1),
            "--trim",
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(
            data.entries.len(),
            gaps.len() + 1,
            "one piece per span left"
        );
        let idle_total: i64 = gaps.iter().map(|(from, to)| to - from).sum();
        // Summed as durations, not as truncated seconds: the span's endpoints carry
        // sub-second parts that per-piece truncation would lose.
        let logged = data
            .entries
            .iter()
            .fold(chrono::Duration::zero(), |acc, e| acc + e.duration());
        assert_eq!(
            logged,
            duration::parse("2h") - chrono::Duration::seconds(idle_total),
            "the pieces do not sum to the span minus the idle stretches"
        );
        // The earliest piece kept the original id and the later ones took fresh
        // ones in sequence, which is what an insert and a split sharing one load
        // produce — the split never re-read the store.
        let ids: Vec<u64> = data.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        assert_eq!(data.next_id, 3, "no id was spent twice");
        assert!(
            data.entries.iter().all(|e| e.idle.is_empty()),
            "a piece kept an interval it excludes"
        );
        // One `with_data`, so one temp-file write and rename: nothing half-written
        // is left beside the store.
        let store = storage::get_data_path().unwrap();
        assert!(
            store.starts_with(&dir) && store.exists(),
            "the sandbox store"
        );
        assert!(
            !store.with_extension("json.tmp").exists(),
            "a temp store file survived the command"
        );
    }

    #[test]
    fn idle_without_trim_leaves_a_single_entry() {
        let _guard = env_guard();
        sandbox("log-no-trim");
        let end = Local::now().timestamp();
        let start = end - 3600;

        run_log(parse_log(&[
            "-d",
            "left alone",
            "-t",
            "60m",
            &format!("--idle={}-{}", start + 600, start + 1200),
        ]));

        let data = storage::load_data().unwrap();
        assert_eq!(data.entries.len(), 1, "recording is not trimming");
        assert_eq!(data.entries[0].duration(), duration::parse("60m"));
        assert_eq!(data.entries[0].idle.len(), 1, "the interval is still there");
    }

    #[test]
    fn trim_without_idle_is_a_usage_error() {
        let parsed = Cli::try_parse_from(["tt", "log", "-d", "x", "-t", "5m", "--trim"]);
        assert!(
            parsed.is_err(),
            "--trim alone parsed, so it would be a silent no-op"
        );
    }

    #[test]
    fn a_well_formed_idle_value_parses_to_the_epoch_seconds_given() {
        let interval = parse_idle("1700000000-1700000600").unwrap();
        assert_eq!(interval.start.timestamp(), 1_700_000_000);
        assert_eq!(interval.end.timestamp(), 1_700_000_600);
        assert_eq!(interval.duration(), chrono::Duration::minutes(10));
    }
}
