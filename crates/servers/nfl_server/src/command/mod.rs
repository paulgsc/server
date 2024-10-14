use anyhow::Result;
use sqlx::SqlitePool;
use nest::config::Config;
use crate::models::game_clock::{CreateGameClock, GameClock};
use crate::common::CrudOperations;

pub async fn populate_game_clocks(config: &Config) -> Result<()> {
    let pool = SqlitePool::connect(&config.database_urls).await?;

    let mut game_clocks = Vec::new();
    for minutes in 0..=15 {
        for seconds in 0..=59 {
            game_clocks.push(CreateGameClock { minutes, seconds });
        }
    }

    match GameClock::batch_create(&pool, &game_clocks).await {
        Ok(created) => println!("Successfully inserted {} GameClock records", created.len()),
        Err(err) => eprintln!("Failed to insert GameClock records: {:?}", err),
    }

    Ok(())
}
