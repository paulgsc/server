use super::{append, record, state, step, Change, ConversationError, Kept};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
const PREVIOUS_MIGRATION: i64 = 20_261_125_000_100;
const NOW: &str = "2026-11-26T12:00:00.000Z";
const A: &str = "subject-a";
const B: &str = "subject-b";
const C: &str = "why-a-log";

async fn pool() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	MIGRATOR.run(&pool).await.unwrap();
	pool
}

fn green_lit() -> Vec<Value> {
	vec![
		json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "why the record is a log" } }),
		json!({ "event": "version", "by": "agent", "at": "2026-11-26T10:02:00.000Z", "version": {
			"n": 1,
			"speakers": [{ "id": "tin", "character": "tin-owlet", "voice": { "kind": "kokoro" } }],
			"turns": [{ "id": "t1", "speaker": "tin", "voice": "kokoro", "said": "a line" }],
			"answers": []
		} }),
		json!({ "event": "heard", "by": "home", "at": "2026-11-26T10:04:00.000Z", "version": 1, "ms": 60_000 }),
		json!({ "event": "green-light", "by": "owner", "at": "2026-11-26T10:05:00.000Z", "version": 1 }),
		json!({ "event": "offer", "by": "agent", "at": "2026-11-26T10:05:30.000Z", "offer": {
			"preview": "p-close",
			"about": "How the essay closes",
			"recommended": "a",
			"variants": [
				{ "variant": "a", "title": "Calm", "callouts": ["the close is slower"], "rule": "calm-close" },
				{ "variant": "b", "title": "Lively", "callouts": ["the close is quicker"], "rule": "lively-close" }
			]
		} }),
	]
}

fn approve(variant: &str) -> Value {
	json!({
		"event": "approve",
		"by": "owner",
		"at": "2026-11-26T10:06:00.000Z",
		"preview": "p-close",
		"chose": { "kind": "variant", "variant": variant }
	})
}

async fn put(pool: &SqlitePool, subject: &str, value: Value) -> Result<(Change, Kept), ConversationError> {
	let event = step::parse(value).unwrap();
	let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
	let appended = append(&mut tx, subject, C, &event, NOW).await;
	tx.commit().await.unwrap();
	appended
}

async fn rows(pool: &SqlitePool) -> Vec<(String, i64, String)> {
	sqlx::query_as("SELECT subject_id, seq, body FROM conversation_event ORDER BY subject_id, seq")
		.fetch_all(pool)
		.await
		.unwrap()
}

#[tokio::test]
async fn each_event_is_kept_at_the_logs_end_and_the_record_reads_back_as_sent() {
	let pool = pool().await;
	for (at, value) in green_lit().into_iter().enumerate() {
		let (change, kept) = put(&pool, A, value).await.unwrap();
		assert_eq!(change, Change::Added);
		assert_eq!(kept, Kept { kept: at, events: at + 1 });
	}
	let record = record(&mut pool.acquire().await.unwrap(), A, C).await.unwrap().unwrap();
	assert_eq!(record, json!({ "schema": "some-ui.conversation/2", "id": C, "events": green_lit() }));
	let lit = state(&mut pool.acquire().await.unwrap(), A, C).await.unwrap();
	assert_eq!(lit.phase(), step::Phase::GreenLit);
}

#[tokio::test]
async fn a_refused_event_changes_nothing() {
	let pool = pool().await;
	for value in green_lit() {
		put(&pool, A, value).await.unwrap();
	}
	let before = rows(&pool).await;
	let refused = put(&pool, A, approve("c")).await.unwrap_err();
	match refused {
		ConversationError::Refused { at, refusal } => {
			assert_eq!(at, 5);
			assert_eq!(
				refusal,
				step::Refusal::PreviewVariant {
					preview: "p-close".to_owned(),
					variant: "c".to_owned()
				}
			);
		}
		other => panic!("refused as {other}"),
	}
	let out_of_phase = json!({ "event": "point", "by": "owner", "at": "2026-11-26T10:07:00.000Z", "take": "late" });
	assert!(matches!(put(&pool, A, out_of_phase).await, Err(ConversationError::Refused { at: 5, .. })));
	assert_eq!(rows(&pool).await, before, "a refused event writes nothing");
	let replayed = state(&mut pool.acquire().await.unwrap(), A, C).await.unwrap();
	assert!(replayed.standing("p-close").is_none(), "the replayed log holds no refused approval");
}

#[tokio::test]
async fn a_retried_event_is_kept_once_and_a_second_approval_is_refused() {
	let pool = pool().await;
	for value in green_lit() {
		put(&pool, A, value).await.unwrap();
	}
	assert_eq!(put(&pool, A, approve("b")).await.unwrap(), (Change::Added, Kept { kept: 5, events: 6 }));
	assert_eq!(put(&pool, A, approve("b")).await.unwrap(), (Change::Unchanged, Kept { kept: 5, events: 6 }));
	assert!(matches!(
		put(&pool, A, approve("a")).await,
		Err(ConversationError::Refused {
			refusal: step::Refusal::PreviewAgain { .. },
			..
		})
	));
	assert_eq!(rows(&pool).await.len(), 6);
}

#[tokio::test]
async fn each_subject_keeps_its_own_log() {
	let pool = pool().await;
	for value in green_lit() {
		put(&pool, A, value).await.unwrap();
	}
	assert!(record(&mut pool.acquire().await.unwrap(), B, C).await.unwrap().is_none());
	assert!(matches!(put(&pool, B, approve("a")).await, Err(ConversationError::Refused { at: 0, .. })));
	let (_, kept) = put(&pool, B, green_lit().remove(0)).await.unwrap();
	assert_eq!(kept, Kept { kept: 0, events: 1 });
}

#[tokio::test]
async fn the_table_is_append_only_gapless_and_leaves_with_the_account() {
	let pool = pool().await;
	for value in green_lit() {
		put(&pool, A, value).await.unwrap();
	}
	assert!(sqlx::query("UPDATE conversation_event SET at = 'x'").execute(&pool).await.is_err(), "an UPDATE is refused");
	let gap = "INSERT INTO conversation_event (subject_id, conversation_id, seq, event, at, body, stored_at) VALUES (?1, ?2, 9, 'notes', 'now', '{\"event\":\"notes\",\"at\":\"now\"}', 'now')";
	assert!(sqlx::query(gap).bind(A).bind(C).execute(&pool).await.is_err(), "a gap in the log is refused");
	let mismatch = "INSERT INTO conversation_event (subject_id, conversation_id, seq, event, at, body, stored_at) VALUES (?1, ?2, 6, 'notes', 'now', '{\"event\":\"approve\",\"at\":\"now\"}', 'now')";
	assert!(sqlx::query(mismatch).bind(A).bind(C).execute(&pool).await.is_err(), "the body's kind is the row's");
	sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, x'01')")
		.bind(A)
		.execute(&pool)
		.await
		.unwrap();
	sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
	assert!(rows(&pool).await.is_empty());
}

/// Down refuses while the table holds a row, and drops it once empty.
#[tokio::test]
async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
	let pool = pool().await;
	let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'conversation_event'");
	MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops an empty table");
	MIGRATOR.run(&pool).await.unwrap();
	put(&pool, A, green_lit().remove(0)).await.unwrap();
	assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "down refuses a table holding an event");
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn a_subjects_conversations_are_listed_newest_first_by_the_logs_order() {
	let pool = pool().await;
	let put_in = |conversation: &'static str, value: Value| {
		let pool = pool.clone();
		async move {
			let event = step::parse(value).unwrap();
			let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
			append(&mut tx, A, conversation, &event, NOW).await.unwrap();
			tx.commit().await.unwrap();
		}
	};
	let topic = |said: &str| json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": said } });
	put_in("first", topic("The first conversation")).await;
	put_in("second", topic(&"a long topic ".repeat(10))).await;
	for value in green_lit().into_iter().skip(1) {
		put_in("first", value).await;
	}
	put_in("first", approve("b")).await;
	put(&pool, B, topic("someone else's")).await.unwrap();
	let listing = super::list(&mut pool.acquire().await.unwrap(), A).await.unwrap();
	assert!(listing.unreadable.is_empty());
	let listed = listing.conversations;
	let ids: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
	assert_eq!(ids, ["first", "second"], "first was kept to last, so it is newest; B's are none of A's");
	assert_eq!(listed[0].topic, "The first conversation");
	assert_eq!(listed[0].phase, step::Phase::GreenLit);
	assert_eq!((listed[0].offers, listed[0].standing), (1, 1));
	assert_eq!(listed[1].topic.chars().count(), super::LIST_TOPIC_MOST);
	assert!(listed[1].topic.ends_with('…'));
	assert_eq!(listed[1].phase, step::Phase::Drafting);
}

#[tokio::test]
async fn the_list_is_bounded() {
	let pool = pool().await;
	for n in 0..(super::LIST_MOST + 3) {
		let id = String::from("c") + &n.to_string();
		let event = step::parse(json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "a topic" } })).unwrap();
		let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
		append(&mut tx, A, &id, &event, NOW).await.unwrap();
		tx.commit().await.unwrap();
	}
	let listed = super::list(&mut pool.acquire().await.unwrap(), A).await.unwrap().conversations;
	assert_eq!(listed.len(), usize::try_from(super::LIST_MOST).unwrap());
	assert_eq!(listed[0].id, String::from("c") + &(super::LIST_MOST + 2).to_string(), "the newest first");
}

#[tokio::test]
async fn a_log_that_does_not_replay_is_named_and_hides_no_other() {
	let pool = pool().await;
	let topic = json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "a topic" } });
	for id in ["fine", "broken"] {
		let event = step::parse(topic.clone()).unwrap();
		let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
		append(&mut tx, A, id, &event, NOW).await.unwrap();
		tx.commit().await.unwrap();
	}
	// A second topic, which the step refuses once drafting: only a write
	// around the repo could keep it.
	sqlx::query("INSERT INTO conversation_event (subject_id, conversation_id, seq, event, at, body, stored_at) VALUES (?1, 'broken', 2, 'topic', ?2, ?3, ?4)")
		.bind(A)
		.bind("2026-11-26T10:00:00.000Z")
		.bind(topic.to_string())
		.bind(NOW)
		.execute(&pool)
		.await
		.unwrap();
	let listing = super::list(&mut pool.acquire().await.unwrap(), A).await.unwrap();
	assert_eq!(listing.unreadable, ["broken"]);
	let ids: Vec<&str> = listing.conversations.iter().map(|c| c.id.as_str()).collect();
	assert_eq!(ids, ["fine"]);
}
