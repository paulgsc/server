use super::{queue, videos, Change, DnaError, Queued, VIDEOS_LISTED};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
const PREVIOUS_MIGRATION: i64 = 20_261_103_001_700;
const NOW: &str = "2026-11-05T17:03:00.000Z";
const A: &str = "subject-a";
const B: &str = "subject-b";

async fn pool() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	for subject in [A, B] {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		for (note, kind) in [("n1", "studio"), ("n2", "studio"), ("r1", "reflection")] {
			note_held(&pool, subject, note, kind).await;
		}
	}
	pool
}

async fn note_held(pool: &SqlitePool, subject: &str, note: &str, kind: &str) {
	sqlx::query(
		"INSERT INTO voice_note (subject_id, note_id, of_kind, of_ref, recorded_at, content_type, size, sha256, stored_at)
		 VALUES (?1, ?2, ?3, 'p1:division:t1', 'then', 'audio/webm', 1, 's', 'now')",
	)
	.bind(subject)
	.bind(note)
	.bind(kind)
	.execute(pool)
	.await
	.unwrap();
}

fn dna(video: &str, rev: i64, queued_at: &str) -> Value {
	json!({
		"schema": "some-ui.video-dna/1",
		"video": video,
		"rev": rev,
		"title": "Playoff Delta · 2026 week 8",
		"queuedAt": queued_at,
		"template": { "kind": "show-format", "id": "playoff-delta", "version": 2 },
		"takes": [{ "scene": "division", "note": "n1" }, { "scene": "outro", "note": "n2" }],
		"shape": { "scenes": [{ "id": "division" }, { "id": "outro" }] },
		"data": { "pin": { "period": "2026-w8" } }
	})
}

async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<(Change, Queued), DnaError> {
	put_bytes(pool, subject, value, value.to_string().as_bytes()).await
}

async fn put_bytes(pool: &SqlitePool, subject: &str, value: &Value, body: &[u8]) -> Result<(Change, Queued), DnaError> {
	let mut tx = pool.begin().await.unwrap();
	let rev = value["rev"].to_string();
	let answer = queue(&mut tx, subject, value["video"].as_str().unwrap(), &rev, body, NOW).await;
	tx.commit().await.unwrap();
	answer
}

async fn count(pool: &SqlitePool) -> i64 {
	sqlx::query_scalar("SELECT COUNT(*) FROM studio_video_queued").fetch_one(pool).await.unwrap()
}

async fn listed(pool: &SqlitePool, subject: &str) -> Vec<(String, i64)> {
	videos(&mut pool.acquire().await.unwrap(), subject)
		.await
		.unwrap()
		.videos
		.into_iter()
		.map(|video| (video.video, video.rev))
		.collect()
}

#[tokio::test]
async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
	let pool = pool().await;
	let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'studio_video_queued'");
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
	MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops it while it is empty");
	MIGRATOR.run(&pool).await.unwrap();
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "up recreates it");

	put(&pool, A, &dna("v1", 1, "2026-11-05T17:02:11.123Z")).await.unwrap();
	assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "a queued video refuses the rollback");
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1, "and leaves it as it was");
	assert_eq!(count(&pool).await, 1);
}

/// The columns the migration describes and no others: a new one is a
/// decision about what the server learns from the record.
#[tokio::test]
async fn the_table_holds_exactly_its_described_columns() {
	let pool = pool().await;
	let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('studio_video_queued') ORDER BY cid")
		.fetch_all(&pool)
		.await
		.unwrap();
	assert_eq!(
		columns,
		[
			"subject_id",
			"video_id",
			"rev",
			"title",
			"queued_at",
			"template_kind",
			"template_id",
			"template_version",
			"dna",
			"dna_sha256",
			"stored_at"
		]
	);
}

/// A revision is queued once, kept byte for byte with its hash; the same
/// bytes again are unchanged, other bytes a refusal, and the next revision a
/// row beside it.
#[tokio::test]
async fn a_revision_is_queued_once_as_sent() {
	let pool = pool().await;
	let first = dna("v1", 1, "2026-11-05T17:02:11.123Z");
	// Spacing the phone chose: home keeps it as sent.
	let sent = serde_json::to_string_pretty(&first).unwrap();
	let (change, held) = put_bytes(&pool, A, &first, sent.as_bytes()).await.unwrap();
	assert_eq!(change, Change::Added);
	assert_eq!(
		serde_json::to_value(&held).unwrap(),
		json!({
			"video": "v1", "rev": 1, "title": "Playoff Delta · 2026 week 8",
			"queuedAt": "2026-11-05T17:02:11.123Z", "storedAt": NOW, "stage": "queued"
		})
	);
	let (kept, sha256, kind, id, version): (String, String, String, String, i64) =
		sqlx::query_as("SELECT dna, dna_sha256, template_kind, template_id, template_version FROM studio_video_queued")
			.fetch_one(&pool)
			.await
			.unwrap();
	assert_eq!(kept, sent, "byte for byte");
	assert_eq!(sha256.len(), 64);
	assert!(sha256.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
	assert_eq!((kind.as_str(), id.as_str(), version), ("show-format", "playoff-delta", 2));

	assert_eq!(
		put_bytes(&pool, A, &first, sent.as_bytes()).await.unwrap(),
		(Change::Unchanged, held.clone()),
		"again, unchanged"
	);
	assert!(
		matches!(put(&pool, A, &first).await, Err(DnaError::Rewritten)),
		"the same DNA spaced otherwise is other bytes"
	);
	let mut retitled = first.clone();
	retitled["title"] = json!("Another title");
	assert!(matches!(put_bytes(&pool, A, &retitled, retitled.to_string().as_bytes()).await, Err(DnaError::Rewritten)));
	assert_eq!(count(&pool).await, 1, "nothing refused was written");

	assert_eq!(put(&pool, A, &dna("v1", 2, "2026-11-05T18:00:00.000Z")).await.unwrap().0, Change::Added);
	assert_eq!(listed(&pool, A).await, [("v1".to_owned(), 2), ("v1".to_owned(), 1)], "each revision, newest queued first");
}

#[tokio::test]
async fn no_row_is_ever_updated() {
	let pool = pool().await;
	put(&pool, A, &dna("v1", 1, "2026-11-05T17:02:11.123Z")).await.unwrap();
	let update = sqlx::query("UPDATE studio_video_queued SET title = 'x'").execute(&pool).await;
	assert!(update.unwrap_err().to_string().contains("append-only"));
}

/// Each take must be the subject's own `studio` note on home, not set
/// aside; a refusal names each note once, in the order the takes first
/// name them, and writes nothing.
#[tokio::test]
async fn every_take_must_be_the_subjects_own_studio_note_on_home() {
	let pool = pool().await;
	note_held(&pool, B, "b-only", "studio").await;
	note_held(&pool, A, "let-go", "studio").await;
	sqlx::query("INSERT INTO voice_note_set_aside (subject_id, note_id, set_aside_at, stored_at) VALUES (?1, 'let-go', 'then', 'now')")
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
	let mut sent = dna("v1", 1, "2026-11-05T17:02:11.123Z");
	sent["takes"] = json!([
		{ "scene": "division", "note": "never-sent" },
		{ "scene": "recap", "note": "r1" },
		{ "scene": "outro", "note": "b-only" },
		{ "scene": "intro", "note": "let-go" },
		{ "scene": "cold-open", "note": "never-sent" },
		{ "scene": "close", "note": "n2" }
	]);
	match put(&pool, A, &sent).await {
		Err(DnaError::TakesNotHeld(notes)) => assert_eq!(notes, ["never-sent", "r1", "b-only", "let-go"]),
		other => panic!("{other:?}"),
	}
	assert_eq!(count(&pool).await, 0, "nothing refused is written");
	note_held(&pool, A, "never-sent", "studio").await;
	note_held(&pool, A, "b-only", "studio").await;
	sent["takes"] = json!([{ "scene": "division", "note": "never-sent" }, { "scene": "outro", "note": "b-only" }]);
	assert_eq!(put(&pool, A, &sent).await.unwrap().0, Change::Added, "once home holds them");
}

#[tokio::test]
async fn a_dna_breaking_a_rule_is_refused_by_field() {
	let pool = pool().await;
	let mut bad = dna("v1", 1, "soon");
	bad["shape"] = json!(null);
	match put(&pool, A, &bad).await {
		Err(DnaError::Invalid(problems)) => {
			let fields: Vec<&str> = problems.iter().map(|(field, _)| *field).collect();
			assert_eq!(fields, ["queuedAt", "shape"]);
		}
		other => panic!("{other:?}"),
	}
	assert_eq!(count(&pool).await, 0);
}

/// Newest queued first, at most `VIDEOS_LISTED`, and only the subject's own.
#[tokio::test]
async fn the_list_is_the_subjects_newest_queued() {
	let pool = pool().await;
	put(&pool, B, &dna("b-video", 1, "2026-11-05T20:00:00.000Z")).await.unwrap();
	let listed_cap = usize::try_from(VIDEOS_LISTED).unwrap();
	for n in 0..=listed_cap {
		let two = |part: usize| if part < 10 { String::from("0") + &part.to_string() } else { part.to_string() };
		let at = String::from("2026-11-05T") + &two(n / 60) + ":" + &two(n % 60) + ":00.000Z";
		put(&pool, A, &dna(&(String::from("v") + &n.to_string()), 1, &at)).await.unwrap();
	}
	let mine = listed(&pool, A).await;
	assert_eq!(mine.len(), listed_cap);
	assert_eq!(mine[0], (String::from("v") + &listed_cap.to_string(), 1), "newest queued first");
	assert!(!mine.iter().any(|(video, _)| video == "v0"), "the oldest queued is the one left out");
	assert_eq!(listed(&pool, B).await, [("b-video".to_owned(), 1)], "B lists only B's");
}

#[tokio::test]
async fn the_record_leaves_with_the_account() {
	let pool = pool().await;
	for subject in [A, B] {
		put(&pool, subject, &dna("v1", 1, "2026-11-05T17:02:11.123Z")).await.unwrap();
	}
	sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
	let left: Vec<String> = sqlx::query_scalar("SELECT DISTINCT subject_id FROM studio_video_queued").fetch_all(&pool).await.unwrap();
	assert_eq!(left, [B], "A's record went with A's account, B's stayed");
}
