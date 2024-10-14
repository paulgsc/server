pub mod handlers;
pub mod routes;
pub mod schema;

use crate::routes::BrowserTabsHandler;
use anyhow::Result;
use clap::Parser;
use nest::config::Config;
use nest::{init_tracing, ApiBuilder, MultiDbHandler, Run};

#[tokio::main]
async fn main() -> Result<()> {
	dotenv::dotenv().ok();
	let config = Config::parse();
	init_tracing(&config);

	let handlers: Vec<Box<dyn MultiDbHandler + Send>> = vec![Box::new(BrowserTabsHandler) as Box<dyn MultiDbHandler + Send>];

	ApiBuilder::run(config, handlers).await?;

	Ok(())
}
