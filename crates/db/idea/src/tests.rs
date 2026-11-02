use super::{idea, ideas, lists, mark_entry, mark_furnishing, put_idea, set_aside, Change, IdeaError, List, SetAside, IDEAS_LISTED};
use crate::model::{MAX_LIST_ENTRIES, SEED_KEYWORDS, SEED_TOPICS};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
const PREVIOUS_MIGRATION: i64 = 20_261_101_000_900;
const NOW: &str = "2026-11-01T23:00:00.000Z";
const A: &str = "subject-a";
const B: &str = "subject-b";
const TABLES: [&str; 5] = ["idea_revision", "idea_answer", "idea_furnishing", "idea_set_aside", "idea_list_mark"];

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
		 VALUES (?1, ?2, ?3, 'idea:i1:what', 'then', 'audio/webm', 1, 's', 'now')",
	)
	.bind(subject)
	.bind(note)
	.bind(kind)
	.execute(pool)
	.await
	.unwrap();
}

fn send(id: &str, sent_at: &str) -> Value {
	json!({
		"id": id,
		"createdAt": "2026-11-01T09:00:00.000Z",
		"sentAt": sent_at,
		"title": "Why the mic clips",
		"topic": "Studio",
		"format": "short",
		"keywords": ["voice"],
		"questions": 1,
		"answers": [{ "question": "what", "note": "n1" }, { "question": "takeaway", "typed": "Keep it a hand away." }]
	})
}

async fn put(pool: &SqlitePool, subject: &str, value: &Value) -> Result<(Change, super::Idea), IdeaError> {
	let mut tx = pool.begin().await.unwrap();
	let answer = put_idea(&mut tx, subject, value["id"].as_str().unwrap(), value.to_string().as_bytes(), NOW).await;
	tx.commit().await.unwrap();
	answer
}

async fn count(pool: &SqlitePool, table: &str) -> i64 {
	sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table)).fetch_one(pool).await.unwrap()
}

async fn total(pool: &SqlitePool) -> i64 {
	let mut sum = 0;
	for table in TABLES {
		sum += count(pool, table).await;
	}
	sum
}

async fn entry(pool: &SqlitePool, list: List, name: &str, listed: bool) -> Result<(Change, super::Lists), IdeaError> {
	mark_entry(&mut pool.acquire().await.unwrap(), A, list, name, listed, NOW).await
}

#[tokio::test]
async fn the_migration_round_trips_and_refuses_to_drop_a_record() {
	let pool = pool().await;
	let tables = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'idea\\_%' ESCAPE '\\'");
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 5);
	MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.unwrap();
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 0, "down drops them while they are empty");
	MIGRATOR.run(&pool).await.unwrap();
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 5, "up recreates them");

	entry(&pool, List::Keyword, "lighting", true).await.unwrap();
	assert!(MIGRATOR.undo(&pool, PREVIOUS_MIGRATION).await.is_err(), "a held mark refuses the rollback");
	assert_eq!(tables().fetch_one(&pool).await.unwrap(), 5, "and leaves them as they were");
	assert_eq!(count(&pool, "idea_list_mark").await, 1);
}

/// The columns the migration describes and no others: a new one is a
/// decision about what the server learns from the record.
#[tokio::test]
async fn the_tables_hold_exactly_their_described_columns() {
	let pool = pool().await;
	let columns = |table: &str| sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info(?1) ORDER BY cid").bind(table.to_owned());
	for (table, expected) in [
		(
			"idea_revision",
			&[
				"subject_id",
				"idea_id",
				"revision",
				"created_at",
				"sent_at",
				"title",
				"topic",
				"format",
				"keywords",
				"questions",
				"stored_at",
			][..],
		),
		("idea_answer", &["subject_id", "idea_id", "revision", "question", "note_id", "typed"][..]),
		("idea_furnishing", &["subject_id", "idea_id", "seq", "furnishing", "stored_at"][..]),
		("idea_set_aside", &["subject_id", "idea_id", "stored_at"][..]),
		("idea_list_mark", &["subject_id", "list", "entry", "seq", "listed", "stored_at"][..]),
	] {
		assert_eq!(columns(table).fetch_all(&pool).await.unwrap(), expected, "{table}");
	}
}

#[tokio::test]
async fn a_send_is_a_revision_and_the_same_bundle_again_is_unchanged() {
	let pool = pool().await;
	let first = send("i1", "2026-11-01T09:10:00.000Z");
	let (change, held) = put(&pool, A, &first).await.unwrap();
	assert_eq!((change, held.revision), (Change::Added, 1));
	assert_eq!(
		serde_json::to_value(&held).unwrap(),
		json!({
			"id": "i1", "createdAt": "2026-11-01T09:00:00.000Z", "revision": 1, "sentAt": "2026-11-01T09:10:00.000Z",
			"title": "Why the mic clips", "topic": "Studio", "format": "short", "keywords": ["voice"], "questions": 1,
			"answers": [
				{ "question": "what", "note": "n1", "typed": null, "heard": null, "setAside": false },
				{ "question": "takeaway", "note": null, "typed": "Keep it a hand away.", "heard": null, "setAside": false }
			],
			"furnishing": false, "setAside": false
		})
	);
	assert_eq!(put(&pool, A, &first).await.unwrap().0, Change::Unchanged, "a retried send writes nothing");
	let mut later = first.clone();
	later["sentAt"] = json!("2026-11-01T10:00:00.000Z");
	assert_eq!(put(&pool, A, &later).await.unwrap().0, Change::Unchanged, "nor does the same bundle sent again");
	let mut reordered = later.clone();
	reordered["answers"] = json!([later["answers"][1], later["answers"][0]]);
	reordered["title"] = json!("  Why the mic clips ");
	assert_eq!(
		put(&pool, A, &reordered).await.unwrap().0,
		Change::Unchanged,
		"however its answers are ordered or its title padded"
	);
	let mut keywords = later.clone();
	keywords["keywords"] = json!(["voice", "home"]);
	assert_eq!(put(&pool, A, &keywords).await.unwrap().1.keywords, ["home", "voice"], "kept sorted");
	keywords["keywords"] = json!(["home", "voice"]);
	keywords["sentAt"] = json!("2026-11-01T10:30:00.000Z");
	assert_eq!(put(&pool, A, &keywords).await.unwrap().0, Change::Unchanged, "so keywords reordered are the same bundle");

	let mut changed = keywords.clone();
	changed["sentAt"] = json!("2026-11-01T10:40:00.000Z");
	changed["title"] = json!("The mic clips");
	changed["answers"][1] = json!({ "question": "spark", "note": "n2" });
	let (change, held) = put(&pool, A, &changed).await.unwrap();
	assert_eq!(
		(change, held.revision, held.title.as_str()),
		(Change::Added, 3, "The mic clips"),
		"a changed bundle is a new revision"
	);
	assert_eq!(held.answers.iter().map(|answer| answer.question.as_str()).collect::<Vec<_>>(), ["what", "spark"]);
	assert_eq!(put(&pool, A, &first).await.unwrap().1.revision, 3, "the first send retried late changes nothing");

	let mut rewritten = first.clone();
	rewritten["format"] = json!("video");
	assert!(matches!(put(&pool, A, &rewritten).await, Err(IdeaError::Rewritten)), "a held send carries one bundle");
	let mut moved = send("i1", "2026-11-01T11:00:00.000Z");
	moved["createdAt"] = json!("2026-11-01T08:00:00.000Z");
	assert!(matches!(put(&pool, A, &moved).await, Err(IdeaError::Invalid(problems)) if problems == [("createdAt", "must be the idea's, as its first send carried it")]));

	assert_eq!(count(&pool, "idea_revision").await, 3, "every revision stays");
	assert_eq!(count(&pool, "idea_answer").await, 6);
	let kept: Vec<(i64, String, String)> = sqlx::query_as("SELECT revision, title, keywords FROM idea_revision ORDER BY revision")
		.fetch_all(&pool)
		.await
		.unwrap();
	assert_eq!(
		kept,
		[
			(1, "Why the mic clips".into(), r#"["voice"]"#.into()),
			(2, "Why the mic clips".into(), r#"["home","voice"]"#.into()),
			(3, "The mic clips".into(), r#"["home","voice"]"#.into())
		]
	);
	assert_eq!(put(&pool, B, &rewritten).await.unwrap().0, Change::Added, "B's ideas are B's");
}

#[tokio::test]
async fn no_row_is_ever_updated() {
	let pool = pool().await;
	put(&pool, A, &send("i1", "2026-11-01T09:10:00.000Z")).await.unwrap();
	mark_furnishing(&mut pool.acquire().await.unwrap(), A, "i1", true, NOW).await.unwrap();
	set_aside(&mut pool.acquire().await.unwrap(), A, "i1", NOW).await.unwrap();
	entry(&pool, List::Topic, "Cooking", true).await.unwrap();
	for table in TABLES {
		assert!(count(&pool, table).await > 0, "{table} holds a row");
		let update = sqlx::query(&(String::from("UPDATE ") + table + " SET subject_id = 'x'")).execute(&pool).await;
		assert!(update.unwrap_err().to_string().contains("append-only"), "{table}");
	}
}

#[tokio::test]
async fn a_named_note_must_be_the_subjects_own_studio_note_on_home() {
	let pool = pool().await;
	note_held(&pool, B, "b-only", "studio").await;
	let mut sent = send("i1", "2026-11-01T09:10:00.000Z");
	sent["answers"] = json!([
		{ "question": "what", "note": "never-sent" },
		{ "question": "takeaway", "note": "r1" },
		{ "question": "spark", "note": "b-only" },
		{ "question": "picture", "note": "n2" }
	]);
	match put(&pool, A, &sent).await {
		Err(IdeaError::NotesNotHeld(notes)) => assert_eq!(notes, ["never-sent", "r1", "b-only"], "each named, in the answers' order"),
		other => panic!("{other:?}"),
	}
	assert_eq!(total(&pool).await, 0, "nothing refused is written");
	note_held(&pool, A, "never-sent", "studio").await;
	note_held(&pool, A, "b-only", "studio").await;
	sent["answers"][1]["note"] = json!("n1");
	assert_eq!(put(&pool, A, &sent).await.unwrap().0, Change::Added, "once home holds them");
}

/// What a send adds to an idea must be on the lists as they stand; what its
/// latest revision already carries stays valid once taken off the lists
/// (#645; the prototype the owner approved on paulgsc/some-ui#2453).
#[tokio::test]
async fn what_a_send_adds_must_be_on_the_lists_as_they_stand() {
	const TOPIC_OFF: (&str, &str) = ("topic", "must be on the topics list as it stands, or the one the idea carries");
	const KEYWORD_OFF: (&str, &str) = ("keywords", "each must be on the keywords list as it stands, or one the idea carries");
	let pool = pool().await;
	let mut sent = send("i1", "2026-11-01T09:10:00.000Z");
	sent["topic"] = json!("Cooking");
	sent["keywords"] = json!(["voice", "lighting"]);
	assert!(matches!(put(&pool, A, &sent).await, Err(IdeaError::Invalid(problems)) if problems == [TOPIC_OFF, KEYWORD_OFF]));
	entry(&pool, List::Topic, "Cooking", true).await.unwrap();
	entry(&pool, List::Keyword, "lighting", true).await.unwrap();
	assert_eq!(put(&pool, A, &sent).await.unwrap().0, Change::Added);
	assert!(put(&pool, B, &sent).await.is_err(), "A's lists are not B's");

	entry(&pool, List::Topic, "Cooking", false).await.unwrap();
	let (_, standing) = entry(&pool, List::Keyword, "lighting", false).await.unwrap();
	assert!(!standing.keywords.contains(&"lighting".to_owned()));
	let held = idea(&mut pool.acquire().await.unwrap(), A, "i1").await.unwrap().unwrap();
	assert_eq!(
		(held.revision, held.topic.as_str(), held.keywords),
		(1, "Cooking", vec!["lighting".to_owned(), "voice".to_owned()]),
		"taking an entry off leaves the ideas that use it as they are"
	);
	assert_eq!(put(&pool, A, &sent).await.unwrap().0, Change::Unchanged, "and a held send still reads as held");
	sent["sentAt"] = json!("2026-11-01T10:00:00.000Z");
	sent["title"] = json!("Another title");
	assert_eq!(
		put(&pool, A, &sent).await.unwrap().1.revision,
		2,
		"a new revision may keep the entries the idea carries, though they are off the lists"
	);
	sent["sentAt"] = json!("2026-11-01T11:00:00.000Z");
	sent["keywords"] = json!(["voice", "lighting", "Cooking"]);
	assert!(
		matches!(put(&pool, A, &sent).await, Err(IdeaError::Invalid(problems)) if problems == [KEYWORD_OFF]),
		"but what it adds must be on them"
	);
	sent["keywords"] = json!(["voice"]);
	assert_eq!(put(&pool, A, &sent).await.unwrap().1.revision, 3, "one dropped is gone from the idea");
	sent["sentAt"] = json!("2026-11-01T12:00:00.000Z");
	sent["keywords"] = json!(["voice", "lighting"]);
	assert!(
		matches!(put(&pool, A, &sent).await, Err(IdeaError::Invalid(problems)) if problems == [KEYWORD_OFF]),
		"so putting it back adds it, and it must be on the list again"
	);
	let mut moved = send("i2", "2026-11-01T09:10:00.000Z");
	moved["topic"] = json!("Cooking");
	assert!(
		matches!(put(&pool, A, &moved).await, Err(IdeaError::Invalid(problems)) if problems == [TOPIC_OFF]),
		"another idea cannot take an entry off the list"
	);
}

#[tokio::test]
async fn the_lists_start_with_their_seeds_and_each_mark_is_kept() {
	let pool = pool().await;
	let standing = lists(&mut pool.acquire().await.unwrap(), A).await.unwrap();
	assert_eq!(standing.topics, SEED_TOPICS);
	assert_eq!(standing.keywords, SEED_KEYWORDS);

	assert_eq!(entry(&pool, List::Topic, "Studio", true).await.unwrap().0, Change::Unchanged, "a seed is on already");
	assert_eq!(
		entry(&pool, List::Topic, "Cooking", false).await.unwrap().0,
		Change::Unchanged,
		"an entry not on is off already"
	);
	assert_eq!(total(&pool).await, 0);
	let (change, standing) = entry(&pool, List::Topic, "Cooking", true).await.unwrap();
	assert_eq!(change, Change::Added);
	assert_eq!(standing.topics.last().map(String::as_str), Some("Cooking"), "added after the seeds");
	let (_, standing) = entry(&pool, List::Topic, "Korean", false).await.unwrap();
	assert_eq!(standing.topics, ["Studio", "LeetType", "Playoff Delta", "Rust Rounds", "Cooking"], "a seed taken off");
	let (_, standing) = entry(&pool, List::Topic, "Korean", true).await.unwrap();
	assert_eq!(standing.topics[2], "Korean", "and put back in its place");
	let marks: Vec<(String, i64, i64)> = sqlx::query_as("SELECT entry, seq, listed FROM idea_list_mark ORDER BY entry, seq")
		.fetch_all(&pool)
		.await
		.unwrap();
	assert_eq!(marks, [("Cooking".into(), 1, 1), ("Korean".into(), 1, 0), ("Korean".into(), 2, 1)], "every mark is kept");
	assert_eq!(lists(&mut pool.acquire().await.unwrap(), B).await.unwrap().topics, SEED_TOPICS, "B's lists are B's");

	for bad in ["", " padded", "a/b", &"x".repeat(41)] {
		assert!(matches!(entry(&pool, List::Keyword, bad, true).await, Err(IdeaError::Invalid(_))), "{bad:?}");
	}
	let room = MAX_LIST_ENTRIES - SEED_KEYWORDS.len();
	for n in 0..room {
		entry(&pool, List::Keyword, &(String::from("k") + &n.to_string()), true).await.unwrap();
	}
	assert!(
		matches!(entry(&pool, List::Keyword, "one-more", true).await, Err(IdeaError::Invalid(_))),
		"a list holds at most 100"
	);
	assert_eq!(entry(&pool, List::Keyword, "k0", false).await.unwrap().0, Change::Added, "taking one off makes room");
	assert_eq!(entry(&pool, List::Keyword, "one-more", true).await.unwrap().0, Change::Added);
}

#[tokio::test]
async fn the_latest_furnishing_mark_stands_and_every_mark_is_kept() {
	let pool = pool().await;
	let mut conn = pool.acquire().await.unwrap();
	assert_eq!(mark_furnishing(&mut conn, A, "i1", true, NOW).await.unwrap(), None, "an idea home does not hold");
	drop(conn);
	put(&pool, A, &send("i1", "2026-11-01T09:10:00.000Z")).await.unwrap();
	let mut conn = pool.acquire().await.unwrap();
	assert_eq!(
		mark_furnishing(&mut conn, A, "i1", false, NOW).await.unwrap(),
		Some(Change::Unchanged),
		"not furnishing until marked"
	);
	assert_eq!(mark_furnishing(&mut conn, A, "i1", true, NOW).await.unwrap(), Some(Change::Added));
	assert_eq!(mark_furnishing(&mut conn, A, "i1", true, NOW).await.unwrap(), Some(Change::Unchanged));
	assert!(idea(&mut conn, A, "i1").await.unwrap().unwrap().furnishing);
	assert_eq!(mark_furnishing(&mut conn, A, "i1", false, NOW).await.unwrap(), Some(Change::Added));
	assert!(!idea(&mut conn, A, "i1").await.unwrap().unwrap().furnishing, "the latest stands");
	assert_eq!(mark_furnishing(&mut conn, B, "i1", true, NOW).await.unwrap(), None, "B holds no such idea");
	drop(conn);
	let marks: Vec<(i64, i64)> = sqlx::query_as("SELECT seq, furnishing FROM idea_furnishing ORDER BY seq").fetch_all(&pool).await.unwrap();
	assert_eq!(marks, [(1, 1), (2, 0)], "every mark is kept");
}

#[tokio::test]
async fn an_idea_set_aside_leaves_the_list_for_good() {
	let pool = pool().await;
	for (id, created) in [("i1", "2026-11-01T09:00:00.000Z"), ("i2", "2026-11-01T10:00:00.000Z"), ("i3", "2026-11-01T08:00:00.000Z")] {
		let mut sent = send(id, "2026-11-01T11:00:00.000Z");
		sent["createdAt"] = json!(created);
		put(&pool, A, &sent).await.unwrap();
	}
	put(&pool, B, &send("b1", "2026-11-01T11:00:00.000Z")).await.unwrap();
	let mut conn = pool.acquire().await.unwrap();
	let listed = |held: super::Ideas| held.ideas.into_iter().map(|idea| idea.id).collect::<Vec<_>>();
	assert_eq!(listed(ideas(&mut conn, A).await.unwrap()), ["i2", "i1", "i3"], "newest begun first, and only A's");

	assert_eq!(set_aside(&mut conn, A, "i1", NOW).await.unwrap(), SetAside::Marked);
	assert_eq!(set_aside(&mut conn, A, "i1", "2026-11-02T00:00:00.000Z").await.unwrap(), SetAside::AlreadyMarked);
	assert_eq!(set_aside(&mut conn, A, "nope", NOW).await.unwrap(), SetAside::NotHeld);
	assert_eq!(set_aside(&mut conn, B, "i2", NOW).await.unwrap(), SetAside::NotHeld, "another subject's idea");
	assert_eq!(listed(ideas(&mut conn, A).await.unwrap()), ["i2", "i3"]);
	let held = idea(&mut conn, A, "i1").await.unwrap().unwrap();
	assert!(held.set_aside, "it is still read by its id");
	drop(conn);
	let stored: Vec<String> = sqlx::query_scalar("SELECT stored_at FROM idea_set_aside").fetch_all(&pool).await.unwrap();
	assert_eq!(stored, [NOW], "the first mark stands");

	let mut changed = send("i1", "2026-11-01T12:00:00.000Z");
	changed["title"] = json!("Back again");
	assert!(matches!(put(&pool, A, &changed).await, Err(IdeaError::SetAside)), "it takes no new revision");
	assert_eq!(
		put(&pool, A, &send("i1", "2026-11-01T11:00:00.000Z")).await.unwrap().0,
		Change::Unchanged,
		"a held send retried is still held"
	);
}

#[tokio::test]
async fn a_read_carries_what_home_heard_in_each_note() {
	let pool = pool().await;
	put(&pool, A, &send("i1", "2026-11-01T09:10:00.000Z")).await.unwrap();
	sqlx::query(
		"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
		 VALUES (?1, 'n1', 'ggml-small', 'whisper.cpp', 1, 'heard', 'en', 0.9, 1000, 'the mic clips', 10, '2026-11-01T09:20:00.000Z')",
	)
	.bind(A)
	.execute(&pool)
	.await
	.unwrap();
	let held = ideas(&mut pool.acquire().await.unwrap(), A).await.unwrap();
	assert_eq!(
		serde_json::to_value(&held.ideas[0].answers).unwrap(),
		json!([
			{ "question": "what", "note": "n1", "typed": null, "heard": { "text": "the mic clips", "language": "en", "model": "ggml-small", "heardAt": "2026-11-01T09:20:00.000Z" }, "setAside": false },
			{ "question": "takeaway", "note": null, "typed": "Keep it a hand away.", "heard": null, "setAside": false }
		])
	);
	assert!(idea(&mut pool.acquire().await.unwrap(), B, "i1").await.unwrap().is_none(), "B reads none of A's");
}

/// A note the owner set aside after the idea was sent (on the phone, Delete:
/// MIC D3) is not held for a send, and the idea shows its answer set aside,
/// without its words.
#[tokio::test]
async fn a_note_set_aside_is_not_held_and_its_words_leave_the_idea() {
	let pool = pool().await;
	let first = send("i1", "2026-11-01T09:10:00.000Z");
	put(&pool, A, &first).await.unwrap();
	sqlx::query(
		"INSERT INTO voice_transcript (subject_id, note_id, model, engine, attempt, outcome, language, language_p, duration_ms, text, took_ms, heard_at)
		 VALUES (?1, 'n1', 'ggml-small', 'whisper.cpp', 1, 'heard', 'en', 0.9, 1000, 'the mic clips', 10, '2026-11-01T09:20:00.000Z')",
	)
	.bind(A)
	.execute(&pool)
	.await
	.unwrap();
	let mut conn = pool.acquire().await.unwrap();
	voice_repo::set_aside::set_aside(&mut conn, A, "n1", "2026-11-01T09:30:00.000Z", NOW).await.unwrap();
	for held in [idea(&mut conn, A, "i1").await.unwrap().unwrap(), ideas(&mut conn, A).await.unwrap().ideas.remove(0)] {
		assert_eq!(
			serde_json::to_value(&held.answers[0]).unwrap(),
			json!({ "question": "what", "note": "n1", "typed": null, "heard": null, "setAside": true }),
			"set aside, and its words left out"
		);
		assert!(!held.answers[1].set_aside, "a typed answer is never set aside");
	}
	drop(conn);
	assert_eq!(put(&pool, A, &first).await.unwrap().0, Change::Unchanged, "a held send retried is still held");
	let mut again = first.clone();
	again["sentAt"] = json!("2026-11-01T10:00:00.000Z");
	again["title"] = json!("Said again");
	match put(&pool, A, &again).await {
		Err(IdeaError::NotesNotHeld(notes)) => assert_eq!(notes, ["n1"], "a new send may not name it"),
		other => panic!("{other:?}"),
	}
	again["answers"][0]["note"] = json!("n2");
	assert_eq!(put(&pool, A, &again).await.unwrap().0, Change::Added, "the answer said again");
	let mut conn = pool.acquire().await.unwrap();
	assert_eq!(voice_repo::set_aside::set_aside_at(&mut conn, B, "n1").await.unwrap(), None, "B's n1 is not A's");
}

/// The list holds at most [`IDEAS_LISTED`] ideas, and says when there are
/// more.
#[tokio::test]
async fn the_list_says_when_it_holds_back_ideas() {
	let pool = pool().await;
	let fill = |n: i64| {
		sqlx::query(
			"WITH RECURSIVE k(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM k WHERE n < ?2)
			 INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
			 SELECT ?1, 'i' || n, 1, printf('2026-11-01T09:00:00.%03dZ', n % 1000), 'then', 't', 'Studio', 'short', '[]', 1, 'now' FROM k",
		)
		.bind(A)
		.bind(n)
	};
	fill(IDEAS_LISTED).execute(&pool).await.unwrap();
	let mut conn = pool.acquire().await.unwrap();
	let listed = ideas(&mut conn, A).await.unwrap();
	assert_eq!((listed.ideas.len(), listed.more), (1_000, false), "all of them");
	drop(conn);
	sqlx::query(
		"INSERT INTO idea_revision (subject_id, idea_id, revision, created_at, sent_at, title, topic, format, keywords, questions, stored_at)
		 VALUES (?1, 'one-more', 1, '2026-11-01T08:00:00.000Z', 'then', 't', 'Studio', 'short', '[]', 1, 'now')",
	)
	.bind(A)
	.execute(&pool)
	.await
	.unwrap();
	let listed = ideas(&mut pool.acquire().await.unwrap(), A).await.unwrap();
	assert_eq!((listed.ideas.len(), listed.more), (1_000, true), "the newest 1,000, and more held back");
	assert!(!listed.ideas.iter().any(|idea| idea.id == "one-more"), "the oldest begun is the one left out");
}

#[tokio::test]
async fn the_record_leaves_with_the_account() {
	let pool = pool().await;
	for subject in [A, B] {
		put(&pool, subject, &send("i1", "2026-11-01T09:10:00.000Z")).await.unwrap();
		let mut conn = pool.acquire().await.unwrap();
		mark_furnishing(&mut conn, subject, "i1", true, NOW).await.unwrap();
		set_aside(&mut conn, subject, "i1", NOW).await.unwrap();
		mark_entry(&mut conn, subject, List::Topic, "Cooking", true, NOW).await.unwrap();
	}
	sqlx::query("DELETE FROM account WHERE subject_id = ?1").bind(A).execute(&pool).await.unwrap();
	for table in TABLES {
		let left: Vec<String> = sqlx::query_scalar(&(String::from("SELECT DISTINCT subject_id FROM ") + table))
			.fetch_all(&pool)
			.await
			.unwrap();
		assert_eq!(left, [B], "{table}: A's record went with A's account, B's stayed");
	}
}
