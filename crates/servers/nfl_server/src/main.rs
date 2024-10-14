pub mod common;
pub mod handlers;
pub mod models;
pub mod routes;

use anyhow::Result;
use clap::Parser;
use handlers::GameClockHandlers;
use nest::config::Config;
use nest::{init_tracing, ApiBuilder, MultiDbHandler, Run};

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	init_tracing(&config);

	let handlers: Vec<Box<dyn MultiDbHandler + Send>> = vec![Box::new(GameClockHandlers) as Box<dyn MultiDbHandler + Send>];

	ApiBuilder::run(config, handlers).await?;

	Ok(())
}
