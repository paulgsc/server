use sqlx::{Error, SqlitePool};

pub async fn init_schema(pool: &SqlitePool) -> Result<(), Error> {
	sqlx::query!(
		r#"
        CREATE TABLE IF NOT EXISTS mood_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            index_pos INTEGER NOT NULL,
            week INTEGER NOT NULL,
            label TEXT NOT NULL,
            description TEXT NOT NULL,
            team TEXT NOT NULL,
            category TEXT NOT NULL,
            delta INTEGER NOT NULL,
            mood INTEGER NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(index_pos)
        )
        "#
	)
	.execute(pool)
	.await?;

	sqlx::query!(
		r#"
        CREATE INDEX IF NOT EXISTS idx_mood_events_week ON mood_events(week);
        CREATE INDEX IF NOT EXISTS idx_mood_events_team ON mood_events(team);
        CREATE INDEX IF NOT EXISTS idx_mood_events_category ON mood_events(category);
        CREATE INDEX IF NOT EXISTS idx_mood_events_index_pos ON mood_events(index_pos);
        "#
	)
	.execute(pool)
	.await?;

	Ok(())
}
