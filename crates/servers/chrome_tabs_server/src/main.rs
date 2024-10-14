pub mod routes;
pub mod handlers;
pub mod schema;


use clap::Parser;
use anyhow::Result;
use nest::config::Config;
use nest::{ApiBuilder, MultiDbHandler, Run, init_tracing};
use crate::routes::BrowserTabsHandler;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
	let config = Config::parse();
    init_tracing(&config);

    let handlers: Vec<Box<dyn MultiDbHandler + Send>> = vec![
        Box::new(BrowserTabsHandler) as Box<dyn MultiDbHandler + Send>,
    ];

    ApiBuilder::run(config, handlers).await?;

    Ok(())

}


