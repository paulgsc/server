//! Handlers for `routes::db::leetype_operator`. See that module's doc comment
//! for the surface; this is the validate-and-write half.
//!
//! Every handler takes an [`Operator`]: a signed-in subject listed in
//! `OPERATOR_SUBJECTS` (`auth::operator`), and the pool alone rather than
//! `AppState`, so the gate is tested through the real router there.

use crate::auth::operator::Operator;
use crate::handlers::db::leetype::ManifestRound;
use crate::FileHostError;
use axum::{
	extract::{Path, State},
	Json,
};
use chrono::Utc;
use leetype_round_repo::{validate_round, Change, RoundEntry, RoundRepository, WriteError, MANIFEST_CEILING, OPERATOR_LISTING_CEILING};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::instrument;

/// What the operator's tool sends to write one round.
///
/// `body` is the round's JSON as text. The server stores it byte for byte and
/// hashes those bytes, so the client decides the exact serialisation once and
/// every later read returns it unchanged. Nothing else rides along: a round
/// has no metadata outside itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundWrite {
	pub body: String,
}

/// One round as the operator sees it: its manifest entry, plus whether it is
/// retired. Never the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorRound {
	#[serde(flatten)]
	pub round: ManifestRound,
	/// `null` while the round is in the manifest.
	pub retired_at: Option<String>,
}

impl From<RoundEntry> for OperatorRound {
	fn from(mut entry: RoundEntry) -> Self {
		let retired_at = entry.retired_at.take();
		Self { round: entry.into(), retired_at }
	}
}

/// `GET /leetype/operator/rounds`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperatorListing {
	pub rounds: Vec<OperatorRound>,
}

/// What a write did, and the round as it now stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoundWritten {
	/// `inserted`, `contentChanged` or `unchanged` — `leetype_round_repo::Change`.
	pub change: &'static str,
	pub round: OperatorRound,
}

const fn change_name(change: Change) -> &'static str {
	match change {
		Change::Inserted => "inserted",
		Change::ContentChanged => "contentChanged",
		Change::Unchanged => "unchanged",
	}
}

fn refused(err: WriteError) -> FileHostError {
	match err {
		WriteError::Invalid(problems) => FileHostError::unprocessable_entity(problems),
		WriteError::Storage(err) => FileHostError::Sqlite(err),
	}
}

/// Every round, listed and retired, by id. Bounded like the manifest: one
/// read of one past [`OPERATOR_LISTING_CEILING`], refused over it.
pub(crate) async fn listing(db: &SqlitePool) -> Result<OperatorListing, FileHostError> {
	let rows = RoundRepository::new(db.clone()).all_entries(OPERATOR_LISTING_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most OPERATOR_LISTING_CEILING + 1
	if rows.len() as i64 > OPERATOR_LISTING_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	Ok(OperatorListing {
		rounds: rows.into_iter().map(OperatorRound::from).collect(),
	})
}

/// Write one round, as `now`.
///
/// The same `RoundRepository::upsert` the importer uses, so content hash,
/// version and `published_at` mean one thing whichever way a round arrived. A
/// key or body the server would store wrong is a `422` naming each field
/// (`leetype_round_repo::validate_round`), checked before the transaction so a
/// refusal takes no write lock. Adding a round past [`MANIFEST_CEILING`]
/// listed is refused, so the manifest never starts refusing because of a
/// write. A retired round stays retired.
pub(crate) async fn write(db: &SqlitePool, key: &str, request: &RoundWrite, now: &str) -> Result<RoundWritten, FileHostError> {
	let body = request.body.as_bytes();
	validate_round(key, body).map_err(FileHostError::unprocessable_entity)?;
	// `BEGIN IMMEDIATE`, so the ceiling check and the insert it allows see one
	// snapshot: two concurrent writes cannot both take the last place.
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	if RoundRepository::entry(&mut tx, key).await?.is_none() && RoundRepository::listed_count(&mut tx).await? >= MANIFEST_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	let change = RoundRepository::upsert(&mut tx, key, body, now, false).await.map_err(refused)?;
	let round = RoundRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	tx.commit().await?;
	Ok(RoundWritten {
		change: change_name(change),
		round: round.into(),
	})
}

/// Take a round out of the manifest (`listed = false`) or put it back.
///
/// Idempotent both ways, and neither moves a version. Putting a round back is
/// refused at [`MANIFEST_CEILING`], like adding one.
pub(crate) async fn set_listed(db: &SqlitePool, key: &str, listed: bool, now: &str) -> Result<OperatorRound, FileHostError> {
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let current = RoundRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	if listed {
		if current.retired_at.is_some() && RoundRepository::listed_count(&mut tx).await? >= MANIFEST_CEILING {
			return Err(FileHostError::MaxRecordLimitExceeded);
		}
		RoundRepository::restore(&mut tx, key).await?;
	} else {
		RoundRepository::retire(&mut tx, key, now).await?;
	}
	let round = RoundRepository::entry(&mut tx, key).await?.ok_or(FileHostError::NotFound)?;
	tx.commit().await?;
	Ok(round.into())
}

/// `GET /leetype/operator/rounds`
///
/// # Errors
/// 401 without a session, 403 for a subject that is not an operator, 400 for
/// a table over the listing ceiling, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_operator_listing", skip_all, fields(otel.kind = "server"))]
pub async fn get_rounds(_operator: Operator, State(db): State<SqlitePool>) -> Result<Json<OperatorListing>, FileHostError> {
	listing(&db).await.map(Json)
}

/// `PUT /leetype/operator/rounds/:id`
///
/// # Errors
/// 401/403 as for every operator route, 422 for a key or body that fails
/// validation, 400 for a new round past the manifest ceiling, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_operator_write", skip_all, fields(otel.kind = "server"))]
pub async fn put_round(
	_operator: Operator,
	State(db): State<SqlitePool>,
	Path(id): Path<String>,
	Json(request): Json<RoundWrite>,
) -> Result<Json<RoundWritten>, FileHostError> {
	write(&db, &id, &request, &Utc::now().to_rfc3339()).await.map(Json)
}

/// `POST /leetype/operator/rounds/:id/retire`
///
/// # Errors
/// 401/403 as for every operator route, 404 for an unknown id, and 500 for a
/// storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_operator_retire", skip_all, fields(otel.kind = "server"))]
pub async fn retire_round(_operator: Operator, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<OperatorRound>, FileHostError> {
	set_listed(&db, &id, false, &Utc::now().to_rfc3339()).await.map(Json)
}

/// `POST /leetype/operator/rounds/:id/restore`
///
/// # Errors
/// 401/403 as for every operator route, 404 for an unknown id, 400 past the
/// manifest ceiling, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "leetype_operator_restore", skip_all, fields(otel.kind = "server"))]
pub async fn restore_round(_operator: Operator, State(db): State<SqlitePool>, Path(id): Path<String>) -> Result<Json<OperatorRound>, FileHostError> {
	set_listed(&db, &id, true, &Utc::now().to_rfc3339()).await.map(Json)
}

#[cfg(test)]
mod tests {
	use super::{listing, set_listed, write, RoundWrite};
	use crate::handlers::db::leetype::{manifest, round};
	use crate::FileHostError;
	use leetype_round_repo::{RoundRepository, MANIFEST_CEILING, ROUND_BYTES_CEILING};
	use serde_json::json;
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T0: &str = "2026-09-29T00:00:00+00:00";
	const T1: &str = "2026-09-30T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn body(id: &str, admissible: &str) -> String {
		json!({
			"id": id, "algorithm": { "language": "rust", "source": "fn f() {}" },
			"diffOptions": [
				{ "member": { "propositionId": admissible, "admissible": true } },
				{ "member": { "propositionId": "CW-P8", "admissible": false } }
			]
		})
		.to_string()
	}

	fn request(body: String) -> RoundWrite {
		RoundWrite { body }
	}

	fn field_errors(err: &FileHostError) -> Vec<String> {
		let FileHostError::UnprocessableEntity { errors } = err else {
			panic!("expected a 422, got {err:?}");
		};
		let mut fields: Vec<String> = errors.keys().map(ToString::to_string).collect();
		fields.sort();
		fields
	}

	async fn manifest_ids(pool: &SqlitePool) -> Vec<String> {
		manifest(pool).await.unwrap().rounds.into_iter().map(|round| round.id).collect()
	}

	/// A write is the importer's upsert: a new id is version 1, identical
	/// bytes write nothing, and changed bytes are a version bump with a new
	/// `published_at` and new witnesses. The body reads back verbatim.
	#[tokio::test]
	async fn a_write_reports_what_it_changed_and_the_body_reads_back_verbatim() {
		let pool = pool().await;
		let first = write(&pool, "a", &request(body("a", "CW-P6")), T0).await.unwrap();
		assert_eq!(first.change, "inserted");
		assert_eq!(
			(first.round.round.version, first.round.round.published_at.as_str(), first.round.retired_at.as_deref()),
			(1, T0, None)
		);

		let again = write(&pool, "a", &request(body("a", "CW-P6")), T1).await.unwrap();
		assert_eq!((again.change, again.round.round.published_at.as_str()), ("unchanged", T0), "an identical write is not new");

		let edited = write(&pool, "a", &request(body("a", "CW-P5")), T1).await.unwrap();
		assert_eq!(
			(edited.change, edited.round.round.version, edited.round.round.published_at.as_str()),
			("contentChanged", 2, T1)
		);
		assert_eq!(edited.round.round.witnesses[0].proposition_id, "CW-P5");

		let (hash, stored) = round(&pool, "a").await.unwrap();
		assert_eq!(stored, body("a", "CW-P5"), "stored byte for byte");
		assert_eq!(hash, edited.round.round.content_hash);
		assert_eq!(manifest_ids(&pool).await, ["a"]);
	}

	/// Everything the server checks is a 422 naming each field, sorted here,
	/// and a refused write writes nothing.
	#[tokio::test]
	async fn a_write_the_server_would_store_wrong_is_refused_naming_each_field() {
		let pool = pool().await;
		let oversized = {
			let mut text = body("a", "CW-P6");
			text.push_str(&" ".repeat(ROUND_BYTES_CEILING));
			text
		};
		let one_option = json!({ "id": "a", "algorithm": { "language": "rust" }, "diffOptions": [{ "member": { "propositionId": "CW-P6", "admissible": true } }] }).to_string();
		let two_admissible = body("a", "CW-P6").replace("false", "true");
		let bad_members = json!({ "id": "a", "algorithm": { "language": "go" }, "diffOptions": [
			{ "member": { "propositionId": "P6", "admissible": true } },
			{ "member": { "propositionId": "CW-P8", "admissible": 0 } }
		]})
		.to_string();
		let cases: [(&str, String, &[&str]); 9] = [
			("a/b", body("a/b", "CW-P6"), &["key"]),
			("a.json", body("a.json", "CW-P6"), &["key"]),
			("a", body("b", "CW-P6"), &["body.id"]),
			("a", "not json".to_owned(), &["body"]),
			("a", "[]".to_owned(), &["body"]),
			("a", oversized, &["body"]),
			("a", one_option, &["body.diffOptions"]),
			("a", two_admissible, &["body.diffOptions"]),
			(
				"a",
				bad_members,
				&[
					"body.algorithm.language",
					"body.diffOptions[0].member.propositionId",
					"body.diffOptions[1].member.admissible",
				],
			),
		];
		for (key, text, expected) in cases {
			let err = write(&pool, key, &request(text), T0).await.unwrap_err();
			assert_eq!(field_errors(&err), expected, "{key}");
		}
		assert!(listing(&pool).await.unwrap().rounds.is_empty(), "nothing was written");
	}

	/// Retiring takes a round out of the manifest without deleting it; the
	/// operator still sees it, a learner can still fetch it by id, and
	/// restoring puts it back. Both are idempotent, and neither moves a
	/// version.
	#[tokio::test]
	async fn retiring_unlists_without_deleting_and_restoring_lists_again() {
		let pool = pool().await;
		for id in ["a", "b"] {
			write(&pool, id, &request(body(id, "CW-P6")), T0).await.unwrap();
		}

		let retired = set_listed(&pool, "a", false, T1).await.unwrap();
		assert_eq!(retired.retired_at.as_deref(), Some(T1));
		assert_eq!(manifest_ids(&pool).await, ["b"], "the manifest is the listed set");
		assert!(round(&pool, "a").await.is_ok(), "a retired round is still served by id");
		let again = set_listed(&pool, "a", false, "2026-10-01T00:00:00+00:00").await.unwrap();
		assert_eq!(again.retired_at.as_deref(), Some(T1), "retiring twice keeps when it was first retired");

		let rounds = listing(&pool).await.unwrap().rounds;
		assert_eq!(
			rounds.iter().map(|r| (r.round.id.as_str(), r.retired_at.is_some())).collect::<Vec<_>>(),
			[("a", true), ("b", false)]
		);

		let restored = set_listed(&pool, "a", true, T1).await.unwrap();
		assert_eq!((restored.retired_at, restored.round.version, restored.round.published_at.as_str()), (None, 1, T0));
		assert_eq!(manifest_ids(&pool).await, ["a", "b"]);
		assert!(set_listed(&pool, "a", true, T1).await.is_ok(), "restoring a listed round is a no-op");

		assert!(matches!(set_listed(&pool, "missing", false, T1).await, Err(FileHostError::NotFound)));
		assert!(matches!(set_listed(&pool, "missing", true, T1).await, Err(FileHostError::NotFound)));
	}

	/// An edit to a retired round leaves it retired.
	#[tokio::test]
	async fn writing_a_retired_round_leaves_it_retired() {
		let pool = pool().await;
		write(&pool, "a", &request(body("a", "CW-P6")), T0).await.unwrap();
		set_listed(&pool, "a", false, T0).await.unwrap();
		let edited = write(&pool, "a", &request(body("a", "CW-P5")), T1).await.unwrap();
		assert_eq!((edited.change, edited.round.retired_at.as_deref()), ("contentChanged", Some(T0)));
		assert!(manifest_ids(&pool).await.is_empty());
	}

	/// The manifest ceiling is enforced where a round becomes listed — a new
	/// write or a restore. Rewriting a listed round is always allowed.
	#[tokio::test]
	async fn listing_a_round_past_the_manifest_ceiling_is_refused() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for i in 0..MANIFEST_CEILING {
			let mut id = String::from("round-");
			id.push_str(&i.to_string());
			RoundRepository::upsert(&mut conn, &id, body(&id, "CW-P6").as_bytes(), T0, false).await.unwrap();
		}
		drop(conn);

		assert!(matches!(
			write(&pool, "one-more", &request(body("one-more", "CW-P6")), T0).await,
			Err(FileHostError::MaxRecordLimitExceeded)
		));
		assert!(
			write(&pool, "round-0", &request(body("round-0", "CW-P5")), T0).await.is_ok(),
			"an existing round can still be edited"
		);

		set_listed(&pool, "round-0", false, T0).await.unwrap();
		write(&pool, "one-more", &request(body("one-more", "CW-P6")), T0).await.unwrap();
		assert!(matches!(set_listed(&pool, "round-0", true, T0).await, Err(FileHostError::MaxRecordLimitExceeded)));
		assert!(manifest(&pool).await.is_ok(), "the manifest is at the ceiling, not over it");
	}

	/// The wire shapes: the manifest entry's camelCase fields plus
	/// `retiredAt`, present as `null` while listed, and a write's `change`.
	#[tokio::test]
	async fn the_listing_and_a_write_have_their_wire_shapes() {
		let pool = pool().await;
		let written = serde_json::to_value(write(&pool, "a", &request(body("a", "CW-P6")), T0).await.unwrap()).unwrap();
		let hash = leetype_round_repo::content_hash(body("a", "CW-P6").as_bytes());
		let entry = json!({
			"id": "a", "version": 1, "publishedAt": T0, "contentHash": hash,
			"witnesses": [{ "propositionId": "CW-P6", "admissible": true }, { "propositionId": "CW-P8", "admissible": false }],
			"retiredAt": null
		});
		assert_eq!(written, json!({ "change": "inserted", "round": entry }));
		assert_eq!(serde_json::to_value(listing(&pool).await.unwrap()).unwrap(), json!({ "rounds": [entry] }));
		let retired = serde_json::to_value(set_listed(&pool, "a", false, T1).await.unwrap()).unwrap();
		assert_eq!(retired["retiredAt"], T1, "retire answers the round itself: {retired}");
	}

	/// `{ body }` and nothing else.
	#[test]
	fn a_write_request_is_body_alone() {
		assert!(serde_json::from_value::<RoundWrite>(json!({ "body": "{}" })).is_ok());
		assert!(serde_json::from_value::<RoundWrite>(json!({ "body": "{}", "metadata": {} })).is_err());
		assert!(serde_json::from_value::<RoundWrite>(json!({})).is_err());
	}
}
