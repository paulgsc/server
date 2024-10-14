pub mod common;
pub mod handlers;
pub mod models;
pub mod routes;
pub mod commands;

use anyhow::Result;
use clap::Parser;
use nest::config::Config;
use nest::cli::{Cli, Commands};
use nest::{init_tracing, ApiBuilder, MultiDbHandler, Run};
use crate::handlers::GameClockHandlers;
use crate::commands::populate_game_clocks;

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	init_tracing(&config);

    let cli = Cli::parse();
    match cli.command {
        Some(Commands::PopulateGameClocks) => {
            populate_game_clocks(&config).await?;
        },
        None => {
            let handlers: Vec<Box<dyn MultiDbHandler + Send>> = vec![Box::new(GameClockHandlers) as Box<dyn MultiDbHandler + Send>];
            ApiBuilder::run(config, handlers).await?;
        }
    }

	Ok(())
}
