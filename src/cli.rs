use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};

use crate::duration;
use crate::icons;
use crate::storage::{load_data, with_data};
use crate::tracker::parse_tags;

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
    },
    /// Stop the current active task
    Stop,
    /// Log a completed task with a specific duration
    Log {
        /// Description of the task
        description: String,
        /// Duration in format like "1h30m", "45m", "2h"
        time: String,
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
}

pub fn start(description: Vec<String>) -> Result<()> {
    let raw_desc = description.join(" ");
    let (desc, tags) = parse_tags(&raw_desc);
    let start_time = Local::now();

    // The active check and the insert share one lock, so two concurrent starts
    // cannot both find nothing active
    let already_tracking = with_data(|data| {
        if let Some(active) = data.active_entry() {
            return Ok(Some((active.description.clone(), active.start_time)));
        }
        data.add_entry(desc.clone(), tags.clone(), start_time, None);
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
        "{}  Started: \"{}\"{} at {}",
        icons::ACTIVE,
        desc,
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

pub fn log(description: String, time_str: String) -> Result<()> {
    let dur = duration::parse(&time_str);
    let end_time = Local::now();
    let start_time = end_time - dur;

    let (desc, tags) = parse_tags(&description);
    with_data(|data| {
        data.add_entry(desc.clone(), tags.clone(), start_time, Some(end_time));
        Ok(())
    })?;

    let tags_display = if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "))
    };

    println!(
        "{} Logged: \"{}\"{} - Duration: {}",
        icons::LOGGED,
        desc,
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
            "{}{} - {}{} ({})",
            status,
            entry.start_time.format("%H:%M"),
            entry.description,
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
            "{}{} {} - {}{} ({})",
            status,
            entry.start_time.format("%Y-%m-%d"),
            entry.start_time.format("%H:%M"),
            entry.description,
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