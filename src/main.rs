use anyhow::Result;

mod cli;
mod duration;
mod icons;
mod storage;
mod tracker;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Migrate under the store lock, once, before any command reads the data.
    // Deliberately not in `load_data`: that runs on read-only paths and on the
    // TUI's reload, and a write from a loader would surprise every later reader.
    storage::with_data(|data| {
        tracker::migrate(data);
        Ok(())
    })?;

    match cli.command {
        Commands::Start { description, project } => cli::start(description, project),
        Commands::Stop => cli::stop(),
        Commands::Log { description, time, tags, project } => {
            cli::log(description, time, tags, project)
        }
        Commands::Today => cli::today(),
        Commands::List => cli::list(),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
        Commands::Active => cli::active(),
    }
}
