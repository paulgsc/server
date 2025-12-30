use crate::common::CrudOperations;
use crate::models::game_clock::GameClock;
use anyhow::Result;
use nest::config::Config;
use sqlx::SqlitePool;

pub async fn populate_game_clocks(config: &Config) -> Result<()> {
	let pool = SqlitePool::connect(&config.database_urls).await?;

	let mut game_clocks = Vec::new();
	for minutes in 0..=15 {
		for seconds in 0..=59 {
			// We set a dummyid to confirm to struct, but won't be inserted into db
			// id field in db auto increments
			game_clocks.push(GameClock { id: 0, minutes, seconds });
		}
	}

	match GameClock::batch_create(&pool, &game_clocks).await {
		Ok(_) => println!("Successfully inserted {} GameClock records", game_clocks.len()),
		Err(err) => eprintln!("Failed to insert GameClock records: {:?}", err),
	}

	Ok(())
}
