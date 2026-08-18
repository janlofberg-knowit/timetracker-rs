use anyhow::Result;

mod agent;
mod cli;
mod duration;
mod icons;
mod marks;
mod storage;
mod tracker;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Dispatched here, *ahead* of the store preamble below, and returning. The
    // `tt agent` commands only ever touch mark files, and an agent calls
    // `tt agent touch` constantly — so letting them fall through would take the
    // store's exclusive lock and rewrite `data.json` once per heartbeat, which is
    // exactly what `bin/tt-safe` skips its own lock for the mark verbs to avoid.
    // Writing the handlers so they merely never call `with_data` is not enough:
    // the preamble below runs before the dispatch table, not inside it.
    if let Commands::Agent { command } = &cli.command {
        return agent::run(command);
    }

    // Migrate under the store lock, once, before any command reads the data.
    // Deliberately not in `load_data`: that runs on read-only paths and on the
    // TUI's reload, and a write from a loader would surprise every later reader.
    storage::with_data(|data| {
        tracker::migrate(data);
        Ok(())
    })?;

    match cli.command {
        Commands::Start {
            description,
            project,
        } => cli::start(description, project),
        Commands::Stop => cli::stop(),
        Commands::Log {
            description,
            time,
            tags,
            project,
            idle,
            trim,
        } => cli::log(description, time, tags, project, idle, trim),
        Commands::Today => cli::today(),
        Commands::List => cli::list(),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
        Commands::Active => cli::active(),
        // Returned above, before the store lock was ever taken.
        Commands::Agent { .. } => unreachable!("the agent commands dispatch ahead of the preamble"),
    }
}
