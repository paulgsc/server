//! Handlers for `routes::db::leetype` (#327, LTY-SRV3; #381, LTY-EXEC). See
//! that module's doc comment for the surface; this is the query-and-respond
//! half.

use crate::handlers::db::activities::{etag_value, if_none_match_hits, not_modified};
use crate::FileHostError;
use axum::{
	body::Body,
	extract::{Path, State},
	http::{
		header::{CONTENT_TYPE, ETAG},
		HeaderMap, HeaderValue,
	},
	response::{IntoResponse, Response},
	Json,
};
use leetype_round_repo::{content_hash, RoundEntry, RoundRepository, RoundRuns, Witness, MANIFEST_CEILING};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// One listed round as the manifest describes it: enough to pick, cache and
/// sample rounds without fetching a body. `witnesses` is the round's `μ`, one
/// per member of its option set, in option order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRound {
	pub id: String,
	pub version: i64,
	pub published_at: String,
	pub content_hash: String,
	pub witnesses: Vec<Witness>,
}

impl From<RoundEntry> for ManifestRound {
	fn from(entry: RoundEntry) -> Self {
		Self {
			id: entry.id,
			version: entry.version,
			published_at: entry.published_at,
			content_hash: entry.content_hash,
			witnesses: entry.witnesses,
		}
	}
}

/// `GET /leetype/rounds`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoundManifest {
	/// [`content_hash`] over the serialised `rounds` array, and the `ETag`.
	/// Opaque to the client; it changes exactly when anything listed does.
	pub version: String,
	pub rounds: Vec<ManifestRound>,
}

/// The listed rounds, by id, and the tag that names them.
///
/// The same shape of answer as the lesson manifest: one read of one past
/// [`MANIFEST_CEILING`], refused rather than truncated over it, and an empty
/// corpus is a valid, empty manifest.
pub(crate) async fn manifest(db: &SqlitePool) -> Result<RoundManifest, FileHostError> {
	let rows = RoundRepository::new(db.clone()).entries(MANIFEST_CEILING + 1).await?;
	#[allow(clippy::cast_possible_wrap)] // at most MANIFEST_CEILING + 1
	if rows.len() as i64 > MANIFEST_CEILING {
		return Err(FileHostError::MaxRecordLimitExceeded);
	}
	let rounds: Vec<ManifestRound> = rows.into_iter().map(ManifestRound::from).collect();
	// Disallowed for tracing; this is the hashed listing itself.
	#[allow(clippy::disallowed_methods)]
	let listing = serde_json::to_vec(&rounds)?;
	Ok(RoundManifest {
		version: content_hash(&listing),
		rounds,
	})
}

/// One round's stored bytes, verbatim, and their hash — or `NotFound`.
/// Retired rounds included: a learner may be part-way through one.
pub(crate) async fn round(db: &SqlitePool, id: &str) -> Result<(String, String), FileHostError> {
	RoundRepository::new(db.clone()).body(id).await?.ok_or(FileHostError::NotFound)
}

/// `GET /leetype/rounds`
///
/// # Errors
/// 400 for a corpus over the ceiling; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "leetype_rounds", skip_all, fields(otel.kind = "server"))]
pub async fn get_rounds(State(db): State<SqlitePool>, headers: HeaderMap) -> Result<Response, FileHostError> {
	let manifest = manifest(&db).await?;
	let etag = etag_value(&manifest.version)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Json(&manifest).into_response();
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// `GET /leetype/rounds/:id`
///
/// The round exactly as written — this server reads four fields of it and
/// never re-serialises it — with its content hash as the `ETag`. An unknown
/// id is a real `404` with a JSON error body.
///
/// # Errors
/// 404 for an unknown id; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "leetype_round", skip_all, fields(otel.kind = "server"))]
pub async fn get_round(State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let (hash, body) = round(&db, &id).await?;
	let etag = etag_value(&hash)?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

/// One round's recorded runs for its current version — or `NotFound`.
/// Retired rounds included, as [`round`] does.
pub(crate) async fn round_runs(db: &SqlitePool, id: &str) -> Result<RoundRuns, FileHostError> {
	RoundRepository::new(db.clone()).runs(id).await?.ok_or(FileHostError::NotFound)
}

/// `GET /leetype/rounds/:id/runs`
///
/// The round's execution transcript (#381): for `A` and each `A + d`, at the
/// bounds of `constraintDiff.before` and of `constraintDiff.after`, the
/// client's `RunResult`, as `leetype_runner` recorded it offline for the
/// round's **current** content hash. A run recorded for an older body is
/// never served; a round with nothing recorded for this version answers
/// `runs: []`. The `ETag` is the hash of the serialised body.
///
/// **Its only input is the path's round id** (#381's never #1, by
/// construction): the extractors are `State`, the headers (read for
/// `If-None-Match` alone) and `Path<String>` — no `Json`, no `Form`, no
/// `Query`, no body — and the route is registered for `GET` only, so any other
/// method on the path is a `405`. There is no field a caller could put source
/// in. Nothing is compiled or executed here; the handler reads rows.
///
/// # Errors
/// 404 for an unknown id; 500 for a storage failure.
#[axum::debug_handler]
#[instrument(name = "leetype_round_runs", skip_all, fields(otel.kind = "server"))]
pub async fn get_round_runs(State(db): State<SqlitePool>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let runs = round_runs(&db, &id).await?;
	// Disallowed for tracing; this is the response body itself, hashed for
	// its tag.
	#[allow(clippy::disallowed_methods)]
	let body = serde_json::to_vec(&runs)?;
	let etag = etag_value(&content_hash(&body))?;
	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}
	let mut response = Response::new(Body::from(body));
	response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	response.headers_mut().insert(ETAG, etag);
	Ok(response)
}

#[cfg(test)]
mod tests {
	//! The logic functions over a migrated pool, and the real routes through
	//! their router for what only a response shows: headers, `304`, `404`.

	use super::{manifest, round};
	use crate::FileHostError;
	use axum::{
		body::Body,
		http::{
			header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH},
			Method, Request, StatusCode,
		},
		Router,
	};
	use leetype_round_repo::{Bounds, Elapsed, ErrorClass, ExecutionError, Observation, RecordedRun, RoundRuns, RunResult, Variant};
	use leetype_round_repo::{Change, RoundRepository, MANIFEST_CEILING};
	use serde_json::{json, Value};
	use sqlx::sqlite::SqlitePoolOptions;
	use sqlx::SqlitePool;
	use std::collections::{BTreeMap, BTreeSet};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const T0: &str = "2026-09-29T00:00:00+00:00";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// A round body with deliberate, non-canonical whitespace, so "verbatim"
	/// is tested rather than assumed.
	fn body(id: &str, admissible: &str) -> String {
		let mut body = String::from("{ \"id\":  \"");
		body.push_str(id);
		body.push_str("\", \"algorithm\": {\"language\":\"rust\"}, \"graph\": {\"kind\": \"W\"},\n \"diffOptions\": [{\"member\": {\"propositionId\": \"");
		body.push_str(admissible);
		body.push_str("\", \"admissible\": true}}, {\"member\": {\"propositionId\": \"CW-P8\", \"admissible\": false}}] }\n");
		body
	}

	async fn put(pool: &SqlitePool, id: &str, admissible: &str, now: &str) -> Change {
		RoundRepository::upsert(&mut pool.acquire().await.unwrap(), id, body(id, admissible).as_bytes(), now, false)
			.await
			.unwrap()
	}

	fn app(pool: SqlitePool) -> Router {
		crate::routes::db::leetype::<SqlitePool>().into_table_router().with_state(pool)
	}

	async fn get(app: &Router, path: &str, if_none_match: Option<&str>) -> (StatusCode, Option<String>, Option<String>, String) {
		let mut request = Request::builder().uri(path);
		if let Some(tag) = if_none_match {
			request = request.header(IF_NONE_MATCH, tag);
		}
		let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
		let status = response.status();
		let etag = response.headers().get(ETAG).map(|value| value.to_str().unwrap().to_owned());
		let content_type = response.headers().get(CONTENT_TYPE).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, etag, content_type, String::from_utf8(bytes.to_vec()).unwrap())
	}

	/// An empty corpus is a valid, empty manifest at 200.
	#[tokio::test]
	async fn an_empty_corpus_is_a_valid_empty_manifest() {
		let json = serde_json::to_value(manifest(&pool().await).await.unwrap()).unwrap();
		assert!(json["version"].is_string() && json["rounds"] == json!([]), "{json}");
	}

	/// The manifest's wire shape: listed rounds by id, camelCase, each with
	/// its witnesses in option order, and a version that moves exactly when a
	/// listing does.
	#[tokio::test]
	async fn the_manifest_lists_rounds_with_their_witnesses() {
		let pool = pool().await;
		put(&pool, "b", "CW-P7", T0).await;
		put(&pool, "a", "CW-P6", T0).await;
		let listed = manifest(&pool).await.unwrap();
		let hash = |id: &str| leetype_round_repo::content_hash(body(id, if id == "a" { "CW-P6" } else { "CW-P7" }).as_bytes());
		assert_eq!(
			serde_json::to_value(&listed).unwrap(),
			json!({ "version": listed.version, "rounds": [
				{ "id": "a", "version": 1, "publishedAt": T0, "contentHash": hash("a"),
					"witnesses": [{ "propositionId": "CW-P6", "admissible": true }, { "propositionId": "CW-P8", "admissible": false }] },
				{ "id": "b", "version": 1, "publishedAt": T0, "contentHash": hash("b"),
					"witnesses": [{ "propositionId": "CW-P7", "admissible": true }, { "propositionId": "CW-P8", "admissible": false }] }
			]})
		);
		#[allow(clippy::disallowed_methods)] // the hashed listing, not a tracing argument
		let serialized = serde_json::to_vec(&listed.rounds).unwrap();
		assert_eq!(listed.version, leetype_round_repo::content_hash(&serialized), "the version is the hash of the listing");
		assert_eq!(manifest(&pool).await.unwrap().version, listed.version, "stable while nothing changes");

		put(&pool, "a", "CW-P5", "2026-09-30T00:00:00+00:00").await;
		assert_ne!(manifest(&pool).await.unwrap().version, listed.version, "an edit changes it");
	}

	/// A retired round leaves the manifest and is still served by id.
	#[tokio::test]
	async fn a_retired_round_is_unlisted_but_still_served() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6", T0).await;
		RoundRepository::retire(&mut pool.acquire().await.unwrap(), "a", T0).await.unwrap();
		assert!(manifest(&pool).await.unwrap().rounds.is_empty());
		assert_eq!(round(&pool, "a").await.unwrap().1, body("a", "CW-P6"));
	}

	/// Over the ceiling is a refusal, not a short manifest.
	#[tokio::test]
	async fn a_corpus_over_the_ceiling_is_refused_not_truncated() {
		let pool = pool().await;
		let mut conn = pool.acquire().await.unwrap();
		for i in 0..=MANIFEST_CEILING {
			let mut id = String::from("round-");
			id.push_str(&i.to_string());
			RoundRepository::upsert(&mut conn, &id, body(&id, "CW-P6").as_bytes(), T0, false).await.unwrap();
		}
		drop(conn);
		assert!(matches!(manifest(&pool).await, Err(FileHostError::MaxRecordLimitExceeded)));
	}

	/// Through the router: a round is its stored bytes as `application/json`
	/// with its content hash as the `ETag`; the tag answers `304`; the
	/// manifest carries its version as its tag; an unknown id is a JSON 404.
	#[tokio::test]
	async fn the_routes_serve_verbatim_bodies_with_etags_304s_and_json_404s() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6", T0).await;
		let app = app(pool.clone());

		let (status, etag, content_type, text) = get(&app, "/leetype/rounds/a", None).await;
		assert_eq!((status, content_type.as_deref()), (StatusCode::OK, Some("application/json")));
		assert_eq!(text, body("a", "CW-P6"), "verbatim, never re-serialised");
		let etag = etag.unwrap();
		assert_eq!(etag, String::from("\"") + &leetype_round_repo::content_hash(text.as_bytes()) + "\"");
		let (status, again, _, text) = get(&app, "/leetype/rounds/a", Some(&etag)).await;
		assert_eq!((status, again.as_deref(), text.as_str()), (StatusCode::NOT_MODIFIED, Some(etag.as_str()), ""));

		let (status, manifest_etag, _, text) = get(&app, "/leetype/rounds", None).await;
		assert_eq!(status, StatusCode::OK);
		let listed: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(manifest_etag.as_deref(), Some((String::from("\"") + listed["version"].as_str().unwrap() + "\"").as_str()));
		assert_eq!(get(&app, "/leetype/rounds", manifest_etag.as_deref()).await.0, StatusCode::NOT_MODIFIED);

		let (status, _, _, text) = get(&app, "/leetype/rounds/no-such-round", None).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		let error: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(error["error"]["code"], "not_found");
		assert!(!text.contains("<html"), "an error shape, not a page");
	}

	/// A transcript with one run of each kind: `A` finished at `C`, and the
	/// three ways a run fails.
	fn transcript() -> Vec<RecordedRun> {
		let error = |variant, bounds, error_class| RecordedRun {
			variant,
			bounds,
			sizes: BTreeMap::from([(String::from("n"), 100_000), (String::from("q"), 100_000)]),
			result: RunResult::Error {
				input_size: 100_000,
				error: ExecutionError {
					error_class,
					message: String::from("detail"),
				},
			},
		};
		vec![
			RecordedRun {
				variant: Variant::Algorithm,
				bounds: Bounds::Before,
				sizes: BTreeMap::from([(String::from("n"), 1_000), (String::from("q"), 1_000)]),
				result: RunResult::Ok {
					input_size: 1_000,
					observation: Observation {
						output: String::from("false"),
						logs: vec![String::from("a note")],
						elapsed: Elapsed { milliseconds: 4 },
					},
				},
			},
			error(Variant::Algorithm, Bounds::After, ErrorClass::BudgetExceeded),
			error(Variant::Diff(0), Bounds::Before, ErrorClass::Compile),
			error(Variant::Diff(1), Bounds::After, ErrorClass::Runtime),
		]
	}

	async fn record(pool: &SqlitePool, id: &str, runs: &[RecordedRun]) {
		let repository = RoundRepository::new(pool.clone());
		let (hash, _) = repository.body(id).await.unwrap().unwrap();
		assert!(repository.replace_runs(id, &hash, runs, T0).await.unwrap());
	}

	async fn send(app: &Router, method: Method, path: &str) -> StatusCode {
		let request = Request::builder()
			.method(method)
			.uri(path)
			.header(CONTENT_TYPE, "application/json")
			.body(Body::from(r#"{"source":"fn main() {}"}"#))
			.unwrap();
		app.clone().oneshot(request).await.unwrap().status()
	}

	/// Through the router: an unknown round is a JSON 404; a known one with
	/// nothing recorded is `runs: []`; a recorded one is its transcript, in
	/// order, as JSON with the body's hash as its `ETag`, answering the tag
	/// with `304`; and after the round's body changes, the old transcript is
	/// not served.
	#[tokio::test]
	async fn the_runs_route_serves_the_current_versions_transcript() {
		let pool = pool().await;
		let app = app(pool.clone());

		let (status, _, _, text) = get(&app, "/leetype/rounds/no-such-round/runs", None).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["error"]["code"], "not_found");

		put(&pool, "a", "CW-P6", T0).await;
		let hash = leetype_round_repo::content_hash(body("a", "CW-P6").as_bytes());
		let (status, etag, content_type, text) = get(&app, "/leetype/rounds/a/runs", None).await;
		assert_eq!((status, content_type.as_deref()), (StatusCode::OK, Some("application/json")));
		assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({ "roundId": "a", "contentHash": hash, "runs": [] }));
		let empty_etag = etag.unwrap();

		let mut shuffled = transcript();
		shuffled.reverse();
		record(&pool, "a", &shuffled).await;
		let (status, etag, _, text) = get(&app, "/leetype/rounds/a/runs", None).await;
		assert_eq!(status, StatusCode::OK);
		let served: RoundRuns = serde_json::from_str(&text).unwrap();
		assert_eq!(
			served,
			RoundRuns {
				round_id: String::from("a"),
				content_hash: hash,
				runs: transcript()
			},
			"A first, then d0, d1; before then after"
		);
		let etag = etag.unwrap();
		assert_ne!(etag, empty_etag, "the tag moves with the transcript");
		assert_eq!(etag, String::from("\"") + &leetype_round_repo::content_hash(text.as_bytes()) + "\"");
		let (status, again, _, text) = get(&app, "/leetype/rounds/a/runs", Some(&etag)).await;
		assert_eq!((status, again.as_deref(), text.as_str()), (StatusCode::NOT_MODIFIED, Some(etag.as_str()), ""));

		put(&pool, "a", "CW-P5", "2026-09-30T00:00:00+00:00").await;
		let (status, _, _, text) = get(&app, "/leetype/rounds/a/runs", Some(&etag)).await;
		assert_eq!(status, StatusCode::OK, "a new version is a new body");
		let served: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(served["runs"], json!([]), "runs recorded for the old body are not served for the new one");
		assert_eq!(served["contentHash"], leetype_round_repo::content_hash(body("a", "CW-P5").as_bytes()));
	}

	/// Never #1, by construction: the route answers `GET` alone, so there is
	/// no request a caller could put source in — any other method with a
	/// body carrying `source` is a `405` — and a `GET` with a query or a body
	/// answers exactly what the bare path does.
	#[tokio::test]
	async fn the_runs_route_takes_no_input_but_the_round_id() {
		let pool = pool().await;
		put(&pool, "a", "CW-P6", T0).await;
		record(&pool, "a", &transcript()).await;
		let app = app(pool.clone());
		for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
			assert_eq!(send(&app, method.clone(), "/leetype/rounds/a/runs").await, StatusCode::METHOD_NOT_ALLOWED, "{method}");
		}
		let bare = get(&app, "/leetype/rounds/a/runs", None).await;
		let with_query = get(&app, "/leetype/rounds/a/runs?source=fn%20main()%20%7B%7D&variant=d0", None).await;
		assert_eq!(bare, with_query);
		let with_body = app
			.clone()
			.oneshot(Request::builder().uri("/leetype/rounds/a/runs").body(Body::from(r#"{"source":"fn main() {}"}"#)).unwrap())
			.await
			.unwrap();
		let bytes = axum::body::to_bytes(with_body.into_body(), usize::MAX).await.unwrap();
		assert_eq!(String::from_utf8(bytes.to_vec()).unwrap(), bare.3);
	}

	/// Never #3: every key a runs response can carry, walked through a
	/// sample with each branch of `RunResult`, is on this list — none of them
	/// a class, a Θ, an "admissible", or any other claim. A new field fails
	/// here and has to be argued for. (`sizes`' keys are dimension names from
	/// the round, not fields, and are not walked.)
	#[test]
	fn a_runs_response_carries_no_complexity_claim() {
		const ALLOWED: [&str; 17] = [
			"roundId",
			"contentHash",
			"runs",
			"variant",
			"bounds",
			"sizes",
			"result",
			"kind",
			"inputSize",
			"observation",
			"output",
			"logs",
			"elapsed",
			"milliseconds",
			"error",
			"errorClass",
			"message",
		];
		fn keys(value: &Value, found: &mut BTreeSet<String>) {
			match value {
				Value::Object(map) => {
					for (key, value) in map {
						found.insert(key.clone());
						if key != "sizes" {
							keys(value, found);
						}
					}
				}
				Value::Array(items) => items.iter().for_each(|item| keys(item, found)),
				Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
			}
		}
		let sample = RoundRuns {
			round_id: String::from("a"),
			content_hash: String::from("h"),
			runs: transcript(),
		};
		let mut found = BTreeSet::new();
		keys(&serde_json::to_value(&sample).unwrap(), &mut found);
		let allowed: BTreeSet<String> = ALLOWED.iter().map(|key| (*key).to_owned()).collect();
		assert_eq!(found, allowed, "every allowed key is exercised, and nothing else appears");
		let kinds: BTreeSet<&str> = sample
			.runs
			.iter()
			.map(|run| if matches!(run.result, RunResult::Ok { .. }) { "ok" } else { "error" })
			.collect();
		assert_eq!(kinds.len(), 2, "both branches walked");
	}

	/// Every file under `src/` that starts a process, and why it may. EX1
	/// (`docs/leetype-execution.md`) is the claim; this list is its
	/// falsifier, so adding to it amends EX1 and says so there.
	const SPAWNS: [(&str, &str); 1] = [(
		"transcribe/engine.rs",
		"ffmpeg, a fixed decoder of a stored voice note's audio, on the transcriber's blocking thread; never a program, never in a request",
	)];

	/// The code lines of `source` (comments dropped), and of the handler's
	/// own file only what precedes its tests, so this test's list is not
	/// found in itself.
	fn code(source: &str) -> String {
		let source = source.split(concat!("#[cfg(", "test)]")).next().unwrap_or_default();
		source.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
	}

	/// Never #5, and EX1 ("nothing `file_host` serves compiles or runs a
	/// program in a request, and `file_host` spawns no compiler"), as checks
	/// over the source:
	///
	/// - the runs surface — this handler, its route module, and both
	///   commands — names nothing that produces or carries a `StudySignal`
	///   (`study_domain`, `handlers::signals`, the nudge, outcomes, sessions,
	///   engagement). `leetype_runner`'s own closure is checked by its
	///   `tests/nevers.rs`;
	/// - no file of the server but `record-leetype-runs` names
	///   `leetype_runner`, and none names `rust_runner`, the crate that
	///   compiles and runs (the `rust-runner` container does that, over its
	///   socket);
	/// - no file but those in [`SPAWNS`] starts a process at all, and each
	///   of those says why it may; nor does `rust_rounds_wire`, the runner's
	///   client half `file_host` links.
	#[test]
	fn the_runs_surface_reaches_neither_a_study_signal_nor_the_runner() {
		let surface = [
			("handlers/db/leetype.rs", include_str!("leetype.rs")),
			("routes/db/leetype.rs", include_str!("../../routes/db/leetype.rs")),
			("bin/record_leetype_runs.rs", include_str!("../../bin/record_leetype_runs.rs")),
			("bin/dump_leetype_snapshot.rs", include_str!("../../bin/dump_leetype_snapshot.rs")),
		];
		let forbidden = [
			["Study", "Signal"].concat(),
			["study", "_domain"].concat(),
			["handlers::", "signals"].concat(),
			["crate::", "nudge"].concat(),
			["file_host::", "nudge"].concat(),
			["outcome", "_repo"].concat(),
			["session", "_repo"].concat(),
			["engagement", "_repo"].concat(),
			["intervention", "::"].concat(),
		];
		for (file, source) in surface {
			let code = code(source);
			for word in &forbidden {
				assert!(!code.contains(word.as_str()), "{file} names {word}");
			}
		}

		let runner = ["leetype", "_runner"].concat();
		let spawner = ["rust", "_runner"].concat();
		let spawn_markers = [["Command", "::new"].concat(), ["process::", "Command"].concat(), ["tokio::", "process"].concat()];
		let spawns_in_a_list = regex::Regex::new(&["process::\\{[^}]*\\b", "Command", "\\b"].concat()).unwrap();
		let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
		let mut stack = vec![root.clone()];
		let mut scanned = 0;
		while let Some(dir) = stack.pop() {
			for entry in std::fs::read_dir(&dir).unwrap() {
				let path = entry.unwrap().path();
				if path.is_dir() {
					stack.push(path);
				} else if path.extension().is_some_and(|extension| extension == "rs") {
					scanned += 1;
					let relative = path.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
					let code = code(&std::fs::read_to_string(&path).unwrap());
					assert_eq!(code.contains(runner.as_str()), relative == "bin/record_leetype_runs.rs", "{relative}");
					assert!(!code.contains(spawner.as_str()), "{relative} names the crate that compiles and runs programs");
					let spawns = spawn_markers.iter().any(|marker| code.contains(marker.as_str())) || spawns_in_a_list.is_match(&code);
					assert_eq!(
						spawns,
						SPAWNS.iter().any(|(file, _)| *file == relative),
						"{relative} starts a process: EX1 lists every file that may"
					);
				}
			}
		}
		assert!(scanned > 50, "the tree was walked: {scanned} files");

		// The runner's client half, which file_host links to ask it (RP1):
		// it starts no process either.
		let wire = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../crates/rust_rounds_wire/src");
		let mut wire_files = 0;
		for entry in std::fs::read_dir(&wire).unwrap() {
			let path = entry.unwrap().path();
			if path.extension().is_some_and(|extension| extension == "rs") {
				wire_files += 1;
				let code = code(&std::fs::read_to_string(&path).unwrap());
				let spawns = spawn_markers.iter().any(|marker| code.contains(marker.as_str())) || spawns_in_a_list.is_match(&code);
				assert!(!spawns, "{} starts a process", path.display());
			}
		}
		assert!(wire_files >= 3, "the client half was read: {wire_files} files");

		// EX1 names every file that may start a process, so adding one to
		// `SPAWNS` without amending the claim fails here.
		let doc = include_str!("../../../docs/leetype-execution.md");
		for (file, _) in SPAWNS {
			assert!(doc.contains(&(String::from("`") + file + "`")), "EX1 does not name {file}");
		}
	}

	/// EX1's other half, from `cargo metadata`'s resolved graph (normal and
	/// build edges; dev edges are tests'): `rust_runner`, the crate that
	/// compiles and runs, reaches `file_host` only through `leetype_runner`.
	/// The source scan above then holds `leetype_runner` to
	/// `record-leetype-runs`: Cargo's graph is per package, not per binary,
	/// so that half is the scan's.
	#[test]
	fn the_runner_reaches_file_host_only_through_leetype_runner() {
		// Set by cargo for the test it runs: the toolchain's own `cargo`.
		#[allow(clippy::disallowed_methods)]
		let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
		let metadata = std::process::Command::new(cargo)
			.args(["metadata", "--format-version", "1", "--offline", "--manifest-path"])
			.arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
			.output()
			.unwrap();
		assert!(metadata.status.success(), "{}", String::from_utf8_lossy(&metadata.stderr));
		let metadata: Value = serde_json::from_slice(&metadata.stdout).unwrap();
		let names: std::collections::BTreeMap<&str, &str> = metadata["packages"]
			.as_array()
			.unwrap()
			.iter()
			.map(|package| (package["id"].as_str().unwrap(), package["name"].as_str().unwrap()))
			.collect();
		let nodes: std::collections::BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
			.as_array()
			.unwrap()
			.iter()
			.map(|node| (node["id"].as_str().unwrap(), node))
			.collect();
		let id = |name: &str| names.iter().find(|(_, package)| **package == name).map(|(id, _)| *id).unwrap();
		// What file_host reaches by normal and build edges, never passing
		// through `skip`.
		let reached = |skip: Option<&str>| {
			let mut closure = std::collections::BTreeSet::new();
			let mut stack = vec![id("file_host")];
			while let Some(at) = stack.pop() {
				if Some(at) == skip || !closure.insert(at) {
					continue;
				}
				for dep in nodes[at]["deps"].as_array().unwrap() {
					if dep["dep_kinds"].as_array().unwrap().iter().any(|kind| kind["kind"].as_str() != Some("dev")) {
						stack.push(dep["pkg"].as_str().unwrap());
					}
				}
			}
			closure.into_iter().map(|at| names[at]).collect::<std::collections::BTreeSet<_>>()
		};
		let runner = ["rust", "_runner"].concat();
		assert!(
			reached(None).contains(runner.as_str()),
			"the graph was walked: file_host reaches it through leetype_runner today"
		);
		assert!(
			!reached(Some(id("leetype_runner"))).contains(runner.as_str()),
			"file_host reaches {runner} other than through leetype_runner"
		);
	}
}
