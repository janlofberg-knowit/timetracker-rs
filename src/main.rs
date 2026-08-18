use anyhow::Result;

mod agent;
mod cli;
mod duration;
mod icons;
mod marks;
mod report;
mod storage;
mod tracker;
mod tui;

use cli::{Cli, Commands};
use clap::Parser;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Two commands are dispatched here, *ahead* of the store preamble below, and
    // return. Writing the handlers so they merely never call `with_data` is not
    // enough: the preamble runs before the dispatch table, not inside it.
    //
    // The mark-only `tt agent` commands, because an agent calls `tt agent touch`
    // constantly — letting a heartbeat fall through would take the store's
    // exclusive lock and rewrite `data.json` once per beat, which is exactly what
    // `bin/tt-safe` skips its own lock for the mark verbs to avoid. The ones that
    // *do* log an entry are the complement and must dispatch after the preamble or
    // they would write an unmigrated store — see `AgentCommands::touches_store`.
    //
    // And `tt report`, because it is a pure read that the preamble turned into a
    // writer: a rollup would take the exclusive lock and block a live `tt agent
    // end` while rewriting a file it only wanted to look at. It migrates its own
    // in-memory copy instead, so a pre-project store still reports inferred
    // projects. `today`, `list`, `status` and `active` deliberately stay on the
    // preamble: none of them is long enough to block a close, and every extra fast
    // path is another place migration can be forgotten.
    match &cli.command {
        Commands::Agent { command } if !command.touches_store() => {
            return agent::run(command);
        }
        Commands::Report {
            all,
            week,
            since,
            until,
            project,
            json,
        } => {
            return cli::report(*all, *week, *since, *until, project.clone(), *json);
        }
        _ => {}
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
        } => cli::log(description, time, tags, project, idle, trim, None),
        Commands::Today => cli::today(),
        Commands::Report {
            all,
            week,
            since,
            until,
            project,
            json,
        } => cli::report(all, week, since, until, project, json),
        Commands::List => cli::list(),
        Commands::Tui => tui::run_tui(),
        Commands::Status => cli::status(),
        Commands::Active => cli::active(),
        // The mark-only ones returned above, before the store lock was ever taken;
        // only the entry-logging ones reach here.
        Commands::Agent { command } => agent::run(&command),
    }
}
