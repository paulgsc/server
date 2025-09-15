use crate::core::model::{CreateMoodEvent, MoodEvent};
use sqlx::{Error, Sqlite, SqlitePool, Transaction};

pub async fn get_next_index(tx: &mut Transaction<'_, Sqlite>) -> Result<i64, Error> {
	let next_index = sqlx::query!("SELECT COALESCE(MAX(index_pos), -1) + 1 as next_index FROM mood_events")
		.fetch_one(&mut **tx)
		.await?
		.next_index;
	Ok(next_index)
}

pub async fn get_previous_mood(tx: &mut Transaction<'_, Sqlite>, index: i64) -> Result<i64, Error> {
	if index == 0 {
		return Ok(100); // base mood
	}
	let prev_mood = sqlx::query!("SELECT mood FROM mood_events WHERE index_pos = ? ORDER BY index_pos DESC LIMIT 1", index - 1)
		.fetch_one(&mut **tx)
		.await?
		.mood;
	Ok(prev_mood)
}

pub async fn insert_mood_event(tx: &mut Transaction<'_, Sqlite>, index_pos: i64, event: CreateMoodEvent, mood: i64) -> Result<i64, Error> {
	let id = sqlx::query!(
		r#"
        INSERT INTO mood_events (index_pos, week, label, description, team, category, delta, mood)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
		index_pos,
		event.week,
		event.label,
		event.description,
		event.team,
		event.category,
		event.delta,
		mood
	)
	.execute(&mut **tx)
	.await?
	.last_insert_rowid();

	Ok(id)
}

pub async fn fetch_mood_event_by_id(pool: &SqlitePool, id: i64) -> Result<Option<MoodEvent>, Error> {
	let row = sqlx::query!(
		r#"
        SELECT id, index_pos as "index_pos: i64", week, label, description, team, category, delta, mood
        FROM mood_events 
        WHERE id = ?
        "#,
		id
	)
	.fetch_optional(pool)
	.await?;

	Ok(row.map(|r| MoodEvent {
		id: r.id,
		index: r.index_pos,
		week: r.week,
		label: r.label,
		description: r.description,
		team: r.team,
		category: r.category,
		delta: r.delta,
		mood: r.mood,
	}))
}

pub async fn fetch_all_mood_events(pool: &SqlitePool) -> Result<Vec<MoodEvent>, Error> {
	let rows = sqlx::query!(
		r#"
        SELECT id, index_pos as "index_pos: i64", week, label, description, team, category, delta, mood
        FROM mood_events 
        ORDER BY index_pos ASC
        "#
	)
	.fetch_all(pool)
	.await?;

	Ok(
		rows
			.into_iter()
			.map(|r| MoodEvent {
				id: r.id,
				index: r.index_pos,
				week: r.week,
				label: r.label,
				description: r.description,
				team: r.team,
				category: r.category,
				delta: r.delta,
				mood: r.mood,
			})
			.collect(),
	)
}

pub async fn fetch_by_week(pool: &SqlitePool, week: i64) -> Result<Vec<MoodEvent>, Error> {
	let rows = sqlx::query!(
		r#"
        SELECT id, index_pos as "index_pos: i64", week, label, description, team, category, delta, mood
        FROM mood_events 
        WHERE week = ?
        ORDER BY index_pos ASC
        "#,
		week
	)
	.fetch_all(pool)
	.await?;

	Ok(
		rows
			.into_iter()
			.map(|r| MoodEvent {
				id: r.id,
				index: r.index_pos,
				week: r.week,
				label: r.label,
				description: r.description,
				team: r.team,
				category: r.category,
				delta: r.delta,
				mood: r.mood,
			})
			.collect(),
	)
}

pub async fn fetch_by_team(pool: &SqlitePool, team: &str) -> Result<Vec<MoodEvent>, Error> {
	let rows = sqlx::query!(
		r#"
        SELECT id, index_pos as "index_pos: i64", week, label, description, team, category, delta, mood
        FROM mood_events 
        WHERE team = ?
        ORDER BY index_pos ASC
        "#,
		team
	)
	.fetch_all(pool)
	.await?;

	Ok(
		rows
			.into_iter()
			.map(|r| MoodEvent {
				id: r.id,
				index: r.index_pos,
				week: r.week,
				label: r.label,
				description: r.description,
				team: r.team,
				category: r.category,
				delta: r.delta,
				mood: r.mood,
			})
			.collect(),
	)
}

pub async fn recalculate_moods_from_index(tx: &mut Transaction<'_, Sqlite>, start_index: i64) -> Result<(), Error> {
	let base_mood = 100i64;
	let prev_mood = if start_index == 0 {
		base_mood
	} else {
		sqlx::query!("SELECT mood FROM mood_events WHERE index_pos = ? ORDER BY index_pos DESC LIMIT 1", start_index - 1)
			.fetch_one(&mut **tx)
			.await?
			.mood
	};

	let events = sqlx::query!("SELECT id, delta FROM mood_events WHERE index_pos >= ? ORDER BY index_pos ASC", start_index)
		.fetch_all(&mut **tx)
		.await?;

	let mut current_mood = prev_mood;
	for event in events {
		current_mood += event.delta;
		sqlx::query!("UPDATE mood_events SET mood = ? WHERE id = ?", current_mood, event.id)
			.execute(&mut **tx)
			.await?;
	}

	Ok(())
}
