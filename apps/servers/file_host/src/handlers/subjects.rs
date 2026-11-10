//! `/api/v1/subjects/me/stats` — the numbers the recommender's second axis
//! refers to (#289, TEL4).
//!
//! `me` rather than an id in the path, so passkey auth landing did not change
//! this URL: whose stats these are is the `SubjectId` extractor's question, the
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
//!
//! `/api/v1/subjects/me/kept` is what home keeps for good of the caller's
//! record, counted for the phone's My data page (paulgsc/some-ui#2519,
//! MY-DATA MD2b): per kind, how many and since when (`storage_repo::kept`).

use crate::subject::{Paired, SubjectId};
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

/// `GET /subjects/me/kept`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
	/// What home keeps for good of each kind, in `storage_repo::kept::Record`'s
	/// order; a kind it holds none of is not listed.
	pub kinds: Vec<KindKept>,
}

/// One kind's count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KindKept {
	/// The kind, as `storage_repo::kept::Record::as_str` names it.
	pub kind: &'static str,
	/// How many home keeps.
	pub count: i64,
	/// The earliest time its records name, ISO-8601 UTC.
	pub since: String,
}

/// `GET /subjects/me/kept`
///
/// What home keeps for good of the caller's record, for the phone's My data
/// page (paulgsc/some-ui#2519, MY-DATA MD2b): for each kind the owner never
/// deletes, how many and since when. Counts and times only: no body, text,
/// title or id.
///
/// # Errors
/// 401 without a session, 403 for a browser's (only a paired phone reads
/// it), and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "subject_kept", skip_all, fields(otel.kind = "server"))]
pub async fn kept(paired: Paired, State(db): State<SqlitePool>) -> Result<Json<Kept>, FileHostError> {
	// A week whose deadline has passed is final whether or not its record
	// was read; its final is what is kept for good (RR22).
	crate::handlers::rust_week_record::finalize_due(&db, paired.subject.as_str()).await?;
	let mut conn = db.acquire().await?;
	let kinds = storage_repo::kept::kept(&mut conn, paired.subject.as_str())
		.await?
		.into_iter()
		.map(|held| KindKept {
			kind: held.record.as_str(),
			count: held.count,
			since: held.since,
		})
		.collect();
	Ok(Json(Kept { kinds }))
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

	/// The kept read answers the caller's own counts, to a paired phone only.
	/// Counting each kind is `storage_repo::kept`'s tests.
	mod kept {
		use crate::auth::{
			cookie::{Carrier, SessionToken},
			now, AuthContext,
		};
		use crate::routes::table::{Module, RouteTable};
		use axum::{
			body::Body,
			extract::FromRef,
			http::{
				header::{AUTHORIZATION, COOKIE},
				HeaderName, Method, Request, StatusCode,
			},
			Router,
		};
		use serde_json::{json, Value};
		use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
		use tower::ServiceExt;

		const A: &str = "subject-a";
		const B: &str = "subject-b";

		#[derive(Clone)]
		struct TestState {
			pool: SqlitePool,
			auth: AuthContext,
		}

		impl FromRef<TestState> for SqlitePool {
			fn from_ref(state: &TestState) -> Self {
				state.pool.clone()
			}
		}

		impl FromRef<TestState> for AuthContext {
			fn from_ref(state: &TestState) -> Self {
				state.auth.clone()
			}
		}

		async fn state() -> TestState {
			let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
			super::MIGRATOR.run(&pool).await.unwrap();
			TestState {
				auth: AuthContext::for_tests(pool.clone(), 100),
				pool,
			}
		}

		/// An account for `subject` and a live session on it, in the header
		/// `carrier` carries it in.
		async fn session(state: &TestState, subject: &str, carrier: Carrier) -> (HeaderName, String) {
			sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?2)")
				.bind(subject)
				.bind(subject.as_bytes())
				.execute(&state.pool)
				.await
				.unwrap();
			let token = SessionToken::mint();
			let now = now();
			assert!(state.auth.repository().create_session(&token.hash(carrier), subject, now + 3_600, now).await.unwrap());
			match carrier {
				Carrier::Bearer => (AUTHORIZATION, String::from("Bearer ") + &token.encoded()),
				Carrier::Cookie => (COOKIE, token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned()),
			}
		}

		/// The route as `routes::subjects` registers it, over a state without
		/// the NATS connection `AppState` needs.
		fn app(state: TestState) -> Router {
			Module::versioned("subjects", RouteTable::new().get("/subjects/me/kept", super::super::kept))
				.into_table_router()
				.with_state(state)
		}

		async fn get(app: &Router, auth: Option<&(HeaderName, String)>) -> (StatusCode, Value) {
			let mut request = Request::builder().method(Method::GET).uri("/subjects/me/kept");
			if let Some((name, value)) = auth {
				request = request.header(name, value);
			}
			let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
			let status = response.status();
			let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
			(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
		}

		#[tokio::test]
		async fn a_paired_phone_reads_how_much_home_keeps_for_good_of_its_own_record() {
			let state = state().await;
			let phone = session(&state, A, Carrier::Bearer).await;
			let browser = session(&state, A, Carrier::Cookie).await;
			let other = session(&state, B, Carrier::Bearer).await;
			sqlx::query(
				"INSERT INTO written_reflection (subject_id, reflection_id, session_id, at, text, written_at)
				 VALUES (?1, 'r1', 's1', '2026-09-03T10:00:00.000Z', 'my own words', '2026-09-03T10:00:01.000Z')",
			)
			.bind(A)
			.execute(&state.pool)
			.await
			.unwrap();
			let app = app(state);

			let (status, body) = get(&app, Some(&phone)).await;
			assert_eq!(status, StatusCode::OK, "{body}");
			assert_eq!(
				body,
				json!({ "kinds": [{ "kind": "reflections", "count": 1, "since": "2026-09-03T10:00:00.000Z" }] }),
				"counts and times only, never the words"
			);
			let (status, body) = get(&app, Some(&other)).await;
			assert_eq!(status, StatusCode::OK, "{body}");
			assert_eq!(body, json!({ "kinds": [] }), "another subject's record is not counted");
			assert_eq!(get(&app, None).await.0, StatusCode::UNAUTHORIZED);
			assert_eq!(get(&app, Some(&browser)).await.0, StatusCode::FORBIDDEN, "a paired phone's only");
		}
	}
}
