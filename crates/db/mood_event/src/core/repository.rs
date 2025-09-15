use super::model::{CreateMoodEvent, MoodEvent, UpdateMoodEvent};
use super::queries;
use super::schema;
use sqlx::{Error, Sqlite, SqlitePool, Transaction};
use std::collections::HashMap;

pub struct MoodEventRepository {
	pub pool: SqlitePool,
}

impl MoodEventRepository {
	pub fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	pub async fn init_schema(&self) -> Result<(), Error> {
		schema::init_schema(&self.pool).await
	}

	pub async fn create(&self, event: CreateMoodEvent) -> Result<MoodEvent, Error> {
		let mut tx = self.pool.begin().await?;
		let result = self.create_with_transaction(&mut tx, event).await?;
		tx.commit().await?;
		Ok(result)
	}

	pub async fn create_with_transaction(&self, tx: &mut Transaction<'_, Sqlite>, event: CreateMoodEvent) -> Result<MoodEvent, Error> {
		let next_index = queries::get_next_index(tx).await?;
		let prev_mood = queries::get_previous_mood(tx, next_index).await?;
		let current_mood = prev_mood + event.delta;

		let id = queries::insert_mood_event(tx, next_index, event.clone(), current_mood).await?;
		queries::recalculate_moods_from_index(tx, next_index + 1).await?;

		Ok(MoodEvent {
			id,
			index: next_index,
			week: event.week,
			label: event.label,
			description: event.description,
			team: event.team,
			category: event.category,
			delta: event.delta,
			mood: current_mood,
		})
	}

	pub async fn batch_create(&self, events: Vec<CreateMoodEvent>) -> Result<Vec<MoodEvent>, Error> {
		if events.is_empty() {
			return Ok(vec![]);
		}

		let mut tx = self.pool.begin().await?;
		let mut results = Vec::with_capacity(events.len());

		for event in events {
			let result = self.create_with_transaction(&mut tx, event).await?;
			results.push(result);
		}

		tx.commit().await?;
		Ok(results)
	}

	pub async fn get_by_id(&self, id: i64) -> Result<Option<MoodEvent>, Error> {
		queries::fetch_mood_event_by_id(&self.pool, id).await
	}

	pub async fn get_all(&self) -> Result<Vec<MoodEvent>, Error> {
		queries::fetch_all_mood_events(&self.pool).await
	}

	pub async fn get_by_week(&self, week: i64) -> Result<Vec<MoodEvent>, Error> {
		queries::fetch_by_week(&self.pool, week).await
	}

	pub async fn get_by_team(&self, team: &str) -> Result<Vec<MoodEvent>, Error> {
		queries::fetch_by_team(&self.pool, team).await
	}

	pub async fn update(&self, id: i64, update: UpdateMoodEvent) -> Result<Option<MoodEvent>, Error> {
		let mut tx = self.pool.begin().await?;

		let current = sqlx::query!("SELECT index_pos FROM mood_events WHERE id = ?", id).fetch_optional(&mut tx).await?;

		let index_pos = match current {
			Some(row) => row.index_pos,
			None => return Ok(None),
		};

		let delta_changed = update.delta.is_some();

		// explicit branches with literal queries
		if let Some(week_val) = update.week {
			sqlx::query!("UPDATE mood_events SET week = ? WHERE id = ?", week_val, id).execute(&mut tx).await?;
		}
		if let Some(label_val) = update.label.as_ref() {
			sqlx::query!("UPDATE mood_events SET label = ? WHERE id = ?", label_val, id).execute(&mut tx).await?;
		}
		if let Some(desc_val) = update.description.as_ref() {
			sqlx::query!("UPDATE mood_events SET description = ? WHERE id = ?", desc_val, id).execute(&mut tx).await?;
		}
		if let Some(team_val) = update.team.as_ref() {
			sqlx::query!("UPDATE mood_events SET team = ? WHERE id = ?", team_val, id).execute(&mut tx).await?;
		}
		if let Some(cat_val) = update.category.as_ref() {
			sqlx::query!("UPDATE mood_events SET category = ? WHERE id = ?", cat_val, id).execute(&mut tx).await?;
		}
		if let Some(delta_val) = update.delta {
			sqlx::query!("UPDATE mood_events SET delta = ? WHERE id = ?", delta_val, id).execute(&mut tx).await?;
		}

		sqlx::query!("UPDATE mood_events SET updated_at = CURRENT_TIMESTAMP WHERE id = ?", id)
			.execute(&mut tx)
			.await?;

		if delta_changed {
			queries::recalculate_moods_from_index(&mut tx, index_pos).await?;
		}

		let updated = queries::fetch_mood_event_by_id(&self.pool, id).await?;
		tx.commit().await?;
		Ok(updated)
	}

	pub async fn batch_update(&self, updates: HashMap<i64, UpdateMoodEvent>) -> Result<Vec<MoodEvent>, Error> {
		if updates.is_empty() {
			return Ok(vec![]);
		}

		let mut tx = self.pool.begin().await?;
		let mut results = Vec::new();
		let mut recalc_needed = false;

		for (id, update) in updates {
			if update.delta.is_some() {
				recalc_needed = true;
			}
			if let Some(event) = self.update_with_transaction(&mut tx, id, update).await? {
				results.push(event);
			}
		}

		if recalc_needed {
			queries::recalculate_moods_from_index(&mut tx, 0).await?;
		}

		tx.commit().await?;
		Ok(results)
	}

	pub async fn delete(&self, id: i64) -> Result<bool, Error> {
		let mut tx = self.pool.begin().await?;

		let index_row = sqlx::query!("SELECT index_pos FROM mood_events WHERE id = ?", id).fetch_optional(&mut tx).await?;

		let index_pos = match index_row {
			Some(r) => r.index_pos,
			None => return Ok(false),
		};

		let rows_affected = sqlx::query!("DELETE FROM mood_events WHERE id = ?", id).execute(&mut tx).await?.rows_affected();

		if rows_affected > 0 {
			sqlx::query!("UPDATE mood_events SET index_pos = index_pos - 1 WHERE index_pos > ?", index_pos)
				.execute(&mut tx)
				.await?;

			queries::recalculate_moods_from_index(&mut tx, index_pos).await?;
		}

		tx.commit().await?;
		Ok(rows_affected > 0)
	}

	pub async fn batch_delete(&self, ids: Vec<i64>) -> Result<u64, Error> {
		if ids.is_empty() {
			return Ok(0);
		}

		let mut tx = self.pool.begin().await?;
		let mut total_deleted = 0;

		let mut id_index_pairs = Vec::new();
		for id in &ids {
			if let Some(row) = sqlx::query!("SELECT index_pos FROM mood_events WHERE id = ?", id).fetch_optional(&mut tx).await? {
				id_index_pairs.push((*id, row.index_pos));
			}
		}

		id_index_pairs.sort_by(|a, b| b.1.cmp(&a.1));

		for (id, _) in id_index_pairs {
			if self.delete_with_transaction(&mut tx, id).await? {
				total_deleted += 1;
			}
		}

		tx.commit().await?;
		Ok(total_deleted)
	}

	pub async fn get_mood_stats(&self) -> Result<super::model::MoodStats, Error> {
		let stats = sqlx::query!(
			r#"
            SELECT 
                COUNT(*) as "total_events!",
                MIN(mood) as "min_mood?",
                MAX(mood) as "max_mood?",
                AVG(mood) as "avg_mood?",
                SUM(CASE WHEN delta > 0 THEN 1 ELSE 0 END) as "positive_events?",
                SUM(CASE WHEN delta < 0 THEN 1 ELSE 0 END) as "negative_events?",
                SUM(CASE WHEN delta = 0 THEN 1 ELSE 0 END) as "neutral_events?"
            FROM mood_events
            "#
		)
		.fetch_one(&self.pool)
		.await?;

		Ok(super::model::MoodStats {
			total_events: stats.total_events,
			min_mood: stats.min_mood,
			max_mood: stats.max_mood,
			avg_mood: stats.avg_mood,
			positive_events: stats.positive_events,
			negative_events: stats.negative_events,
			neutral_events: stats.neutral_events,
		})
	}

	async fn update_with_transaction(&self, tx: &mut Transaction<'_, Sqlite>, id: i64, update: UpdateMoodEvent) -> Result<Option<MoodEvent>, Error> {
		let current = sqlx::query!("SELECT index_pos FROM mood_events WHERE id = ?", id).fetch_optional(&mut *tx).await?;

		let _index_pos = match current {
			Some(row) => row.index_pos,
			None => return Ok(None),
		};

		if let Some(week_val) = update.week {
			sqlx::query!("UPDATE mood_events SET week = ? WHERE id = ?", week_val, id).execute(&mut *tx).await?;
		}
		if let Some(label_val) = update.label.as_ref() {
			sqlx::query!("UPDATE mood_events SET label = ? WHERE id = ?", label_val, id).execute(&mut *tx).await?;
		}
		if let Some(desc_val) = update.description.as_ref() {
			sqlx::query!("UPDATE mood_events SET description = ? WHERE id = ?", desc_val, id).execute(&mut *tx).await?;
		}
		if let Some(team_val) = update.team.as_ref() {
			sqlx::query!("UPDATE mood_events SET team = ? WHERE id = ?", team_val, id).execute(&mut *tx).await?;
		}
		if let Some(cat_val) = update.category.as_ref() {
			sqlx::query!("UPDATE mood_events SET category = ? WHERE id = ?", cat_val, id).execute(&mut *tx).await?;
		}
		if let Some(delta_val) = update.delta {
			sqlx::query!("UPDATE mood_events SET delta = ? WHERE id = ?", delta_val, id).execute(&mut *tx).await?;
		}

		sqlx::query!("UPDATE mood_events SET updated_at = CURRENT_TIMESTAMP WHERE id = ?", id)
			.execute(&mut *tx)
			.await?;

		queries::fetch_mood_event_by_id(&self.pool, id).await
	}

	async fn delete_with_transaction(&self, tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<bool, Error> {
		let rows_affected = sqlx::query!("DELETE FROM mood_events WHERE id = ?", id).execute(&mut *tx).await?.rows_affected();
		Ok(rows_affected > 0)
	}
}
