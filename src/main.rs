use anyhow::Result;

mod cli;
mod config;
mod duration;
mod icons;
mod storage;
mod tracker;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { description } => cli::start(description),
        Commands::Stop => cli::stop(),
        Commands::Log { description, time, tags } => cli::log(description, time, tags),
        Commands::Today => cli::today(),
        Commands::List { limit } => cli::list(limit),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
        Commands::Active => cli::active(),
    }
}
