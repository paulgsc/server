use anyhow::Context;
use clap::Parser;

use nest::config::Config;
use nest::http;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let config = Config::parse();
	// Initialize SQLite connection pool
	let db = SqlitePoolOptions::new()
		.max_connections(5)
		.connect(&config.database_url)
		.await
		.context("could not connect to database_url")?;

	// Initialize the database
	sqlx::migrate!().run(&db).await?;

	// Finally, we spin up our API.
	http::serve(config, db).await?;

	Ok(())
}
