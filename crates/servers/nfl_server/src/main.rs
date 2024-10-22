pub mod commands;
pub mod common;
pub mod handlers;
pub mod models;
pub mod routes;

use crate::commands::populate_game_clocks;
use crate::handlers::{GameClockHandlers, GameClockMigrationHandler, PlayTypeHandlers, PlayTypeMigrationHandler};
use anyhow::Result;
use clap::Parser;
use nest::config::{Config, ProgramMode};
use nest::{init_tracing, ApiBuilder, MultiDbHandler, Run};

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	init_tracing(&config);

	match config.program_mode {
		ProgramMode::PopulateGameClocks => {
			populate_game_clocks(&config).await?;
		}
		ProgramMode::Run => {
			let handlers: Vec<Box<dyn MultiDbHandler + Send>> = vec![
				Box::new(GameClockHandlers) as Box<dyn MultiDbHandler + Send>,
				Box::new(PlayTypeHandlers) as Box<dyn MultiDbHandler + Send>,
			];
			ApiBuilder::run(config, handlers, GameClockMigrationHandler).await?;
		}
	}

	Ok(())
}
