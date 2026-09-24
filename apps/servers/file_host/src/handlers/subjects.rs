//! `/api/v1/subjects/me/stats` — the numbers the recommender's second axis
//! refers to (#289, TEL4).
//!
//! `me` rather than an id in the path, so the day auth lands this URL does not
//! change: whose stats these are is the `SubjectId` extractor's question, the
//! same reasoning #261 gave for the session routes.
//!
//! The payload maps onto the client's `RankingSignals`
//! (`packages/activity-catalog/src/lib/rank.ts`, `paulgsc/some-ui`):
//!
//! - `history` is `ReadonlyArray<ActivityPlay>` exactly — `{ activityId, at }`,
//!   `at` in ms since the epoch — with **one entry per activity, at its most
//!   recent play**. That is what the client's recency axis reads; the full play
//!   list is not sent, because it grows with every block played while this
//!   response is bounded by the catalogue. The client's frequency axis gets its
//!   count from `activities[].plays` instead of by counting entries.
//! - `activities` carries what `ActivityPlay` has no room for: completion and
//!   abandonment rates, and the mean over *assessed* scores (a `null` score is
//!   excluded, never averaged in as zero).
//!
//! Sharing the input vocabulary is the point; the two rankers still disagree
//! about weights on purpose (see `activity_repo::recommender`'s module docs).

use crate::subject::SubjectId;
use crate::{AppState, FileHostError};
use axum::{extract::State, Json};
use outcome_repo::{ActivityStats, OutcomeRepository, StatsError};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectStats {
	pub activities: Vec<ActivityStatsView>,
	pub history: Vec<ActivityPlay>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStatsView {
	#[serde(flatten)]
	pub stats: ActivityStats,
	pub completion_rate: Option<f64>,
	pub abandonment_rate: Option<f64>,
}

/// The client's `ActivityPlay`, field for field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPlay {
	pub activity_id: String,
	/// ms since the epoch.
	pub at: i64,
}

/// `GET /subjects/me/stats`
///
/// # Errors
/// 400 when the subject's outcomes span more activities than
/// `outcome_repo::STATS_CEILING` (refused, never truncated); 500 for a
/// storage failure.
#[axum::debug_handler]
#[instrument(name = "subject_stats", skip_all, fields(otel.kind = "server"))]
pub async fn stats(State(state): State<AppState>, subject: SubjectId) -> Result<Json<SubjectStats>, FileHostError> {
	subject_stats(&state.core.shared_db, subject.as_str()).await.map(Json)
}

/// The handler's body, over a pool rather than `AppState`, so it is testable
/// without the NATS connection `AppState::build` needs.
pub(crate) async fn subject_stats(db: &SqlitePool, subject_id: &str) -> Result<SubjectStats, FileHostError> {
	let stats = OutcomeRepository::new(db.clone()).stats(subject_id).await.map_err(|err| match err {
		StatsError::OverCeiling => FileHostError::MaxRecordLimitExceeded,
		StatsError::Storage(err) => FileHostError::Sqlite(err),
	})?;

	let history = stats
		.iter()
		.filter_map(|activity| {
			let at = crate::nudge::clock::parse_timestamp(activity.last_played_at.as_deref()?)?;
			Some(ActivityPlay {
				activity_id: activity.activity_id.clone(),
				at: at.timestamp_millis(),
			})
		})
		.collect();
	let activities = stats
		.into_iter()
		.map(|stats| ActivityStatsView {
			completion_rate: stats.completion_rate(),
			abandonment_rate: stats.abandonment_rate(),
			stats,
		})
		.collect();

	Ok(SubjectStats { activities, history })
}

#[cfg(test)]
mod tests {
	use super::{subject_stats, ActivityPlay};
	use sqlx::sqlite::SqlitePoolOptions;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	#[tokio::test]
	async fn stats_map_onto_ranking_signals_and_exclude_unassessed_scores_from_the_mean() {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();

		assert_eq!(
			serde_json::to_value(subject_stats(&pool, "subject-local").await.unwrap()).unwrap(),
			serde_json::json!({ "activities": [], "history": [] })
		);

		for (session, block, outcome, score, ended_at) in [
			("s1", 0, "completed", Some(0.9), "2026-09-20T10:00:00+00:00"),
			("s1", 1, "completed", None, "2026-09-20T10:10:00+00:00"),
			("s2", 0, "abandoned", None, "2026-09-22T10:00:00+00:00"),
			("s3", 0, "skipped", None, "2026-09-23T10:00:00+00:00"),
		] {
			sqlx::query!(
				"INSERT INTO activity_outcome (subject_id, session_id, activity_id, block_index, started_at, ended_at, planned_ms, elapsed_ms, outcome, score) VALUES ('subject-local', ?, 'honeycomb', ?, ?, ?, 60000, 60000, ?, ?)",
				session,
				block,
				ended_at,
				ended_at,
				outcome,
				score
			)
			.execute(&pool)
			.await
			.unwrap();
		}

		let stats = subject_stats(&pool, "subject-local").await.unwrap();
		let json = serde_json::to_value(&stats).unwrap();
		let honeycomb = &json["activities"][0];
		assert_eq!(honeycomb["activityId"], "honeycomb");
		assert_eq!(honeycomb["plays"], 3, "the skipped block was never played");
		assert_eq!(honeycomb["meanScore"], 0.9, "the NULL score is excluded, not averaged in as zero");
		assert_eq!(honeycomb["abandonmentRate"], serde_json::json!(1.0 / 3.0));
		assert_eq!(
			stats.history,
			vec![ActivityPlay {
				activity_id: "honeycomb".to_owned(),
				// 2026-09-22T10:00:00Z — the latest *played* block; the later
				// skip does not count as a play.
				at: 1_790_071_200_000,
			}]
		);
	}
}
