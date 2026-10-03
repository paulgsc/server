//! The MCP endpoint: what an AI service the subject approved can do for them
//! (docs/identity.md, "AI services acting for a subject", "The MCP
//! endpoint").
//!
//! Streamable HTTP, stateless: every message is one `POST`, answered with one
//! JSON body (never an event stream), and there is no `Mcp-Session-Id`, since
//! nothing here outlives a request. A `GET` or `DELETE` is `405`. One
//! JSON-RPC message per request: batches are refused, as MCP since 2025-06-18
//! does.
//!
//! Every request carries an OAuth access token (`subject::Delegated`). Without
//! a live one the answer is `401` naming this endpoint's RFC 9728 metadata, so
//! a client can find the authorization server. A tool the subject did not
//! approve is not listed, and calling it is `403 insufficient_scope`.
//!
//! The tools, and the permission each needs:
//!
//! ```text
//! get_lesson_prompt   any        how to write a lesson the app can play (MCP_LESSON_PROMPT_FILE)
//! list_lessons        lessons:read  the corpus's manifest
//! get_lesson          lessons:read  one corpus lesson, verbatim
//! get_progress        progress:read the subject's per-activity outcome stats
//! list_my_lessons     shelf      the subject's kept TOPIK lessons, keys and hashes
//! get_my_lesson       shelf      one kept lesson, verbatim
//! keep_lesson         shelf      keep a lesson on the subject's shelf, never replacing one
//! ```
//!
//! Each tool is a thin call into the handler the app's own route uses
//! (`db::curriculum`, `subjects`, `shelf`), so the endpoint reads and writes
//! exactly what the app does and nothing else. Nothing here reaches the
//! corpus's writes, another subject's rows, a survey (surveys stay on the
//! phone) or any account route. Nothing here logs a token, a tool's
//! arguments or what it returned.

use crate::auth::oauth::{OAuthSettings, Scope};
use crate::auth::AuthContext;
use crate::handlers::{db::curriculum, shelf, subjects};
use crate::subject::Delegated;
use crate::FileHostError;
use axum::{
	body::Bytes,
	extract::State,
	http::{header, HeaderMap, HeaderValue, Method, StatusCode},
	response::{IntoResponse, Response},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Map, Value};
use tracing::instrument;

/// Protocol revisions this endpoint speaks, newest first. It answers an
/// `initialize` with the client's revision when it is one of these, and with
/// the newest otherwise (the client then decides whether to continue). The
/// surface used here (tools, text content, `structuredContent`) is the same in
/// all three.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// The shelf activity AI-written lessons are kept under.
const SHELF_ACTIVITY: &str = "topik";

/// How `@some-ui/topik` names a lesson it plays from the shelf
/// (`LOCAL_LESSON_PREFIX`); a kept document's `meta.key` carries it.
const LOCAL_LESSON_PREFIX: &str = "local:";

// JSON-RPC 2.0's own error codes.
const PARSE_ERROR: i64 = -32_700;
const INVALID_REQUEST: i64 = -32_600;
const METHOD_NOT_FOUND: i64 = -32_601;
const INVALID_PARAMS: i64 = -32_602;
const INTERNAL_ERROR: i64 = -32_603;

const INSTRUCTIONS: &str = "Lessons and study progress for one learner, who approved this connection. \
	Nothing here says who the learner is. \
	To write a lesson for them, call get_lesson_prompt and follow it, then keep the result with keep_lesson: \
	it appears on their shelf in the app, which checks it before playing it. \
	A kept lesson is never replaced: choose a new key for a new lesson.";

/// One tool, and the permission it needs (`None`: any approved service).
struct Tool {
	name: &'static str,
	title: &'static str,
	description: &'static str,
	scope: Option<Scope>,
	read_only: bool,
	input: Value,
}

fn no_arguments() -> Value {
	json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn key_argument(what: &'static str) -> Value {
	json!({
		"type": "object",
		"properties": { "key": { "type": "string", "description": what } },
		"required": ["key"],
		"additionalProperties": false,
	})
}

fn tools() -> [Tool; 7] {
	[
		Tool {
			name: "get_lesson_prompt",
			title: "How to write a lesson",
			description: "The prompt the app gives a model to write one Korean listening lesson: the setting, the probe rules, \
				the JSON it must return and how it is checked. Follow it to write a lesson, adding the learner's level and scene \
				where its 'This request' section says, then keep the result with keep_lesson.",
			scope: None,
			read_only: true,
			input: no_arguments(),
		},
		Tool {
			name: "list_lessons",
			title: "List the lessons",
			description: "The lessons every learner here is served: each one's key, name, description, difficulty, tags and counts.",
			scope: Some(Scope::LessonsRead),
			read_only: true,
			input: no_arguments(),
		},
		Tool {
			name: "get_lesson",
			title: "Read a lesson",
			description: "One lesson from list_lessons, as the app plays it: its conversations, lines and probes, as JSON.",
			scope: Some(Scope::LessonsRead),
			read_only: true,
			input: key_argument("The lesson's key, from list_lessons."),
		},
		Tool {
			name: "get_progress",
			title: "Read the learner's progress",
			description: "How the learner has done in each activity: plays, completed, abandoned and skipped blocks, completion \
				and abandonment rates, the mean of assessed scores, and when they last played. No survey answers: those stay on \
				the learner's phone.",
			scope: Some(Scope::ProgressRead),
			read_only: true,
			input: no_arguments(),
		},
		Tool {
			name: "list_my_lessons",
			title: "List the learner's kept lessons",
			description: "The TOPIK lessons on the learner's own shelf (only they see it): each one's key, content hash and when \
				it was kept, and how many the shelf holds.",
			scope: Some(Scope::Shelf),
			read_only: true,
			input: no_arguments(),
		},
		Tool {
			name: "get_my_lesson",
			title: "Read a kept lesson",
			description: "One lesson from the learner's shelf, as kept: { version, meta, batches }.",
			scope: Some(Scope::Shelf),
			read_only: true,
			input: key_argument("The kept lesson's key, from list_my_lessons."),
		},
		Tool {
			name: "keep_lesson",
			title: "Keep a lesson on the learner's shelf",
			description: "Keep a lesson written with get_lesson_prompt on the learner's own shelf, where the app lists it to \
				play. Pass the prompt's two outputs: the lesson file (the array of conversations) and its manifest entry, whose \
				`key` (letters, digits, '-', '_', '.', '~') names it on the shelf. A key already holding a different lesson is \
				refused, never replaced; the shelf holds at most 20 lessons. The app checks the lesson when it is played.",
			scope: Some(Scope::Shelf),
			read_only: false,
			input: json!({
				"type": "object",
				"properties": {
					"lesson": { "type": "array", "description": "The lesson file: an array of conversations.", "items": { "type": "object" } },
					"manifest": {
						"type": "object",
						"description": "The lesson's manifest entry, with its key.",
						"properties": { "key": { "type": "string" } },
						"required": ["key"],
					},
				},
				"required": ["lesson", "manifest"],
				"additionalProperties": false,
			}),
		},
	]
}

/// Whether this deployment has `tool` at all: the lesson prompt is configured.
fn available(tool: &Tool, settings: &OAuthSettings) -> bool {
	tool.name != "get_lesson_prompt" || settings.lesson_prompt.is_some()
}

/// Whether `tool` is listed for this service: available, and approved.
fn offered(tool: &Tool, delegated: &Delegated, settings: &OAuthSettings) -> bool {
	available(tool, settings) && tool.scope.is_none_or(|scope| delegated.allows(scope))
}

fn listing(tool: &Tool) -> Value {
	json!({
		"name": tool.name,
		"title": tool.title,
		"description": tool.description,
		"inputSchema": tool.input,
		"annotations": {
			"title": tool.title,
			"readOnlyHint": tool.read_only,
			"destructiveHint": false,
			"idempotentHint": true,
			"openWorldHint": false,
		},
	})
}

/// A JSON body, never cached.
fn answer(status: StatusCode, body: &Value) -> Response {
	(status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn result(id: &Value, result: &Value) -> Response {
	answer(StatusCode::OK, &json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
	answer(StatusCode::OK, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
}

/// A refusal that says how to get what is missing (RFC 6750 §3, RFC 9728
/// §5.1): which metadata to read, and for a `403`, which permission.
fn challenge(status: StatusCode, settings: &OAuthSettings, missing: Option<Scope>) -> Response {
	let mut value = String::from("Bearer ");
	if let Some(scope) = missing {
		value += "error=\"insufficient_scope\", scope=\"";
		value += scope.as_str();
		value += "\", ";
	}
	value += "resource_metadata=\"";
	value += &settings.resource_metadata_url();
	value += "\"";
	let (error, description) = missing.map_or(("invalid_token", "a live access token for this endpoint is required"), |_| {
		("insufficient_scope", "the learner did not allow this; reconnect to ask them for it")
	});
	let mut response = answer(status, &json!({ "error": error, "error_description": description }));
	if let Ok(value) = HeaderValue::from_str(&value) {
		response.headers_mut().insert(header::WWW_AUTHENTICATE, value);
	}
	response
}

/// A tool's answer: its text, and its value as `structuredContent` when that
/// is an object.
fn content(value: &Value) -> Value {
	let mut answer = json!({ "content": [{ "type": "text", "text": value.to_string() }], "isError": false });
	if value.is_object() {
		answer["structuredContent"] = value.clone();
	}
	answer
}

fn text(text: &str) -> Value {
	json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

/// A refusal the model can act on, as a tool result (MCP reports these in
/// the result, not as a protocol error, so the model sees them).
fn refusal(why: &str) -> Value {
	json!({ "content": [{ "type": "text", "text": why }], "isError": true })
}

/// What a tool's failure tells the model, or the error itself when it is the
/// server's and not the request's.
fn refused(err: FileHostError, missing: &str) -> Result<Value, FileHostError> {
	match err {
		FileHostError::NotFound => Ok(refusal(missing)),
		FileHostError::Conflict(why) => Ok(refusal(why)),
		FileHostError::UnprocessableEntity { errors } => {
			let mut problems: Vec<String> = errors
				.into_iter()
				.flat_map(|(field, whys)| whys.into_iter().map(move |why| field.to_string() + " " + &why))
				.collect();
			problems.sort();
			Ok(refusal(&(String::from("refused: ") + &problems.join("; "))))
		}
		FileHostError::MaxRecordLimitExceeded => Ok(refusal("there is more here than this endpoint answers with")),
		other => Err(other),
	}
}

fn key_of(arguments: &Value) -> Option<&str> {
	arguments.get("key").and_then(Value::as_str)
}

fn now() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The shelf document `@some-ui/topik` keeps and plays
/// (`adapter/shelf`'s `keptBodyOf`): the lesson and its manifest entry, with
/// `meta.key` the `local:` form of the shelf key.
fn kept_document(lesson: &[Value], manifest: &Map<String, Value>, key: &str) -> Value {
	let mut meta = manifest.clone();
	meta.insert(String::from("key"), Value::String(String::from(LOCAL_LESSON_PREFIX) + key));
	json!({ "version": 1, "meta": meta, "batches": lesson })
}

/// Runs one tool for the subject. `Err` is the server's failure, never the
/// request's.
async fn run(name: &str, arguments: &Value, auth: &AuthContext, settings: &OAuthSettings, delegated: &Delegated) -> Result<Value, FileHostError> {
	let db = auth.pool();
	let subject = delegated.subject().as_str();
	match name {
		"get_lesson_prompt" => Ok(settings.lesson_prompt.as_deref().map_or_else(|| refusal("no lesson prompt is configured"), text)),
		"list_lessons" => Ok(content(&serde_json::to_value(curriculum::manifest(db).await?)?)),
		"get_lesson" => {
			let Some(key) = key_of(arguments) else { return Ok(refusal("give the lesson's key")) };
			curriculum::lesson(db, key)
				.await
				.map_or_else(|err| refused(err, "no lesson is served under that key"), |(_, body)| Ok(text(&body)))
		}
		"get_progress" => Ok(content(&serde_json::to_value(subjects::subject_stats(db, subject).await?)?)),
		"list_my_lessons" => Ok(content(&serde_json::to_value(shelf::listing(db, subject, SHELF_ACTIVITY).await?)?)),
		"get_my_lesson" => {
			let Some(key) = key_of(arguments) else {
				return Ok(refusal("give the kept lesson's key"));
			};
			shelf::item(db, subject, SHELF_ACTIVITY, key)
				.await
				.map_or_else(|err| refused(err, "nothing is kept under that key"), |body| Ok(text(&body)))
		}
		"keep_lesson" => {
			let lesson = arguments.get("lesson").and_then(Value::as_array);
			let manifest = arguments.get("manifest").and_then(Value::as_object);
			let (Some(lesson), Some(manifest)) = (lesson, manifest) else {
				return Ok(refusal("give `lesson`, the array of conversations, and `manifest`, its manifest entry"));
			};
			let Some(key) = manifest.get("key").and_then(Value::as_str) else {
				return Ok(refusal("the manifest entry needs a `key`"));
			};
			let body = kept_document(lesson, manifest, key).to_string();
			match shelf::keep(db, subject, SHELF_ACTIVITY, key, body.as_bytes(), &now(), shelf::Replace::Refused).await {
				Ok(written) => Ok(content(&serde_json::to_value(written)?)),
				Err(err) => refused(err, "the shelf refused it"),
			}
		}
		_ => Err(FileHostError::NotFound),
	}
}

fn initialize(params: &Value) -> Value {
	let asked = params.get("protocolVersion").and_then(Value::as_str);
	let version = asked.filter(|asked| PROTOCOL_VERSIONS.contains(asked)).unwrap_or(PROTOCOL_VERSIONS[0]);
	json!({
		"protocolVersion": version,
		"capabilities": { "tools": { "listChanged": false } },
		"serverInfo": { "name": "file_host", "title": "Lessons", "version": env!("CARGO_PKG_VERSION") },
		"instructions": INSTRUCTIONS,
	})
}

/// `POST /mcp`
///
/// # Errors
/// `404` while OAuth is off (there is no endpoint to describe), `403` for a
/// browser page on an untrusted origin (MCP's DNS-rebinding guard), and a
/// storage failure.
#[instrument(name = "mcp", skip_all, fields(otel.kind = "server"))]
pub async fn mcp(State(auth): State<AuthContext>, delegated: Result<Delegated, FileHostError>, headers: HeaderMap, body: Bytes) -> Result<Response, FileHostError> {
	let settings = &auth.oauth().map_err(|_| FileHostError::NotFound)?.settings;
	auth.check_origin(&Method::POST, &headers)?;
	let delegated = match delegated {
		Ok(delegated) => delegated,
		Err(FileHostError::Unauthorized) => return Ok(challenge(StatusCode::UNAUTHORIZED, settings, None)),
		Err(other) => return Err(other),
	};
	let version = headers.get("mcp-protocol-version").and_then(|value| value.to_str().ok());
	if version.is_some_and(|version| !PROTOCOL_VERSIONS.contains(&version)) {
		return Ok(answer(
			StatusCode::BAD_REQUEST,
			&json!({ "jsonrpc": "2.0", "id": null, "error": { "code": INVALID_REQUEST, "message": "unsupported MCP-Protocol-Version" } }),
		));
	}

	let Ok(message) = serde_json::from_slice::<Value>(&body) else {
		return Ok(rpc_error(&Value::Null, PARSE_ERROR, "the body is not JSON"));
	};
	let Some(message) = message.as_object() else {
		return Ok(rpc_error(&Value::Null, INVALID_REQUEST, "one JSON-RPC message per request; batches are not supported"));
	};
	// A notification or a response: nothing to answer.
	let (Some(method), Some(id)) = (message.get("method").and_then(Value::as_str), message.get("id")) else {
		return Ok(StatusCode::ACCEPTED.into_response());
	};
	if !(id.is_string() || id.is_number()) {
		return Ok(rpc_error(&Value::Null, INVALID_REQUEST, "the id must be a string or a number"));
	}
	let params = message.get("params").unwrap_or(&Value::Null);

	match method {
		"initialize" => Ok(result(id, &initialize(params))),
		"ping" => Ok(result(id, &json!({}))),
		"tools/list" => {
			let tools: Vec<Value> = tools().iter().filter(|tool| offered(tool, &delegated, settings)).map(listing).collect();
			Ok(result(id, &json!({ "tools": tools })))
		}
		"tools/call" => {
			let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
			let Some(tool) = tools().into_iter().find(|tool| tool.name == name && available(tool, settings)) else {
				return Ok(rpc_error(id, INVALID_PARAMS, "no such tool"));
			};
			if let Some(scope) = tool.scope.filter(|scope| !delegated.allows(*scope)) {
				return Ok(challenge(StatusCode::FORBIDDEN, settings, Some(scope)));
			}
			let arguments = params.get("arguments").unwrap_or(&Value::Null);
			match run(tool.name, arguments, &auth, settings, &delegated).await {
				Ok(answer) => Ok(result(id, &answer)),
				Err(err) => {
					tracing::error!(error = ?err, tool = tool.name, "an MCP tool failed");
					Ok(rpc_error(id, INTERNAL_ERROR, "the server failed; try again later"))
				}
			}
		}
		_ => Ok(rpc_error(id, METHOD_NOT_FOUND, "this endpoint offers tools only")),
	}
}

/// `GET /.well-known/oauth-protected-resource/api/v1/mcp` (RFC 9728): which
/// authorization server issues this endpoint's tokens. `404` while OAuth is
/// off.
pub async fn resource_metadata(State(auth): State<AuthContext>) -> Response {
	auth.oauth().map_or_else(
		|_| StatusCode::NOT_FOUND.into_response(),
		|flows| Json(flows.settings.protected_resource_metadata()).into_response(),
	)
}

#[cfg(test)]
mod tests {
	use crate::auth::{oauth::hash_token, AuthContext};
	use auth_repo::{ClientRow, CreateGrant, NewAccessToken, NewGrant, OAuthRepository};
	use axum::{
		body::Body,
		http::{header, Request, StatusCode},
		Router,
	};
	use curriculum_repo::{CurriculumRepository, ManifestEntry};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	const RESOURCE: &str = "https://lessons.test/api/v1/mcp";
	const PROMPT: &str = "# Topik Lesson Generator\n";
	const ALL: &str = "lessons:read progress:read shelf";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	fn app(auth: AuthContext) -> Router {
		crate::routes::mcp::mcp_metadata::<AuthContext>()
			.into_table_router()
			.merge(crate::routes::mcp::mcp::<AuthContext>().into_table_router())
			.with_state(auth)
	}

	/// An account whose subject approved a service for `scope`: the bearer
	/// token that service holds.
	async fn token(pool: &SqlitePool, subject: &str, scope: &str) -> String {
		sqlx::query("INSERT OR IGNORE INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(subject)
			.execute(pool)
			.await
			.unwrap();
		let oauth = OAuthRepository::new(pool.clone());
		let client = String::from("client-") + subject + "-" + &scope.replace(' ', "-");
		oauth
			.register_client(&ClientRow {
				client_id: client.clone(),
				client_name: String::from("Claude"),
				redirect_uris: String::from("[\"https://claude.ai/cb\"]"),
			})
			.await
			.unwrap();
		let access = String::from("access-") + &client;
		let refresh = hash_token(&(String::from("refresh-") + &client));
		let grant = NewGrant {
			grant_id: &(String::from("grant-") + &client),
			subject_id: subject,
			client_id: &client,
			scope,
			resource: RESOURCE,
			refresh_hash: &refresh,
			refresh_expires_at: i64::MAX,
		};
		let created = oauth
			.create_grant(
				&grant,
				&NewAccessToken {
					token_hash: &hash_token(&access),
					expires_at: i64::MAX,
				},
				crate::auth::now(),
			)
			.await
			.unwrap();
		assert_eq!(created, CreateGrant::Created);
		access
	}

	async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
		let response = app.clone().oneshot(request).await.unwrap();
		let status = response.status();
		let challenge = response.headers().get(header::WWW_AUTHENTICATE).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
		let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
		(status, challenge, body)
	}

	fn post(token: Option<&str>, body: &Value) -> Request<Body> {
		let mut request = Request::post("/mcp").header(header::CONTENT_TYPE, "application/json");
		if let Some(token) = token {
			request = request.header(header::AUTHORIZATION, String::from("Bearer ") + token);
		}
		request.body(Body::from(body.to_string())).unwrap()
	}

	fn call(tool: &str, arguments: &Value) -> Value {
		json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": tool, "arguments": arguments } })
	}

	/// A tool's result: its text, whether it is an error, and its structured value.
	async fn tool(app: &Router, token: &str, name: &str, arguments: &Value) -> (String, bool, Value) {
		let (status, _, body) = send(app, post(Some(token), &call(name, arguments))).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(body["id"], 7);
		let result = &body["result"];
		(
			result["content"][0]["text"].as_str().unwrap().to_owned(),
			result["isError"].as_bool().unwrap(),
			result["structuredContent"].clone(),
		)
	}

	fn listed(body: &Value) -> Vec<&str> {
		body["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect()
	}

	#[tokio::test]
	async fn without_a_token_the_answer_says_where_to_get_one() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let initialize = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });

		let (status, challenge, _) = send(&app, post(None, &initialize)).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		assert_eq!(
			challenge.as_deref(),
			Some("Bearer resource_metadata=\"https://lessons.test/.well-known/oauth-protected-resource/api/v1/mcp\"")
		);
		let (status, challenge, _) = send(&app, post(Some("not-a-token"), &initialize)).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		assert!(challenge.is_some());

		let metadata = Request::get("/.well-known/oauth-protected-resource/api/v1/mcp").body(Body::empty()).unwrap();
		let (status, _, body) = send(&app, metadata).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["resource"], RESOURCE);
		assert_eq!(body["authorization_servers"], json!(["https://lessons.test"]));
		assert_eq!(body["scopes_supported"], json!(["lessons:read", "progress:read", "shelf"]));
	}

	#[tokio::test]
	async fn with_oauth_off_there_is_no_endpoint() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests(pool.clone(), 100));
		let (status, _, _) = send(&app, post(Some("anything"), &json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		let metadata = Request::get("/.well-known/oauth-protected-resource/api/v1/mcp").body(Body::empty()).unwrap();
		assert_eq!(send(&app, metadata).await.0, StatusCode::NOT_FOUND);
	}

	#[tokio::test]
	async fn it_speaks_json_rpc_one_message_at_a_time() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let token = token(&pool, "subject-alice", ALL).await;

		let initialize = json!({ "jsonrpc": "2.0", "id": "a", "method": "initialize", "params": { "protocolVersion": "2025-06-18" } });
		let (status, _, body) = send(&app, post(Some(&token), &initialize)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["id"], "a");
		assert_eq!(body["result"]["protocolVersion"], "2025-06-18", "a revision it speaks is echoed");
		assert_eq!(body["result"]["capabilities"], json!({ "tools": { "listChanged": false } }));
		let newer = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2099-01-01" } });
		assert_eq!(send(&app, post(Some(&token), &newer)).await.2["result"]["protocolVersion"], "2025-11-25");

		let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
		let (status, _, body) = send(&app, post(Some(&token), &initialized)).await;
		assert_eq!((status, body), (StatusCode::ACCEPTED, Value::Null));

		let ping = json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" });
		assert_eq!(send(&app, post(Some(&token), &ping)).await.2["result"], json!({}));
		let batch = json!([ping]);
		assert_eq!(send(&app, post(Some(&token), &batch)).await.2["error"]["code"], -32_600);
		let unknown = json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" });
		assert_eq!(send(&app, post(Some(&token), &unknown)).await.2["error"]["code"], -32_601);
		let missing = call("delete_everything", &json!({}));
		assert_eq!(send(&app, post(Some(&token), &missing)).await.2["error"]["code"], -32_602);

		// No event stream to open and no session to end.
		for method in ["GET", "DELETE"] {
			let request = Request::builder().method(method).uri("/mcp").body(Body::empty()).unwrap();
			assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::METHOD_NOT_ALLOWED);
		}

		let mut stale = post(Some(&token), &ping);
		stale.headers_mut().insert("mcp-protocol-version", "1999-01-01".parse().unwrap());
		assert_eq!(send(&app, stale).await.0, StatusCode::BAD_REQUEST);
	}

	#[tokio::test]
	async fn a_browser_page_on_another_origin_is_refused() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let token = token(&pool, "subject-alice", ALL).await;
		let mut request = post(Some(&token), &json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }));
		request.headers_mut().insert(header::ORIGIN, "https://evil.test".parse().unwrap());
		assert_eq!(send(&app, request).await.0, StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn a_service_sees_and_calls_only_what_the_subject_approved() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_lesson_prompt(pool.clone(), Some(PROMPT)));
		let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });

		let everything = token(&pool, "subject-alice", ALL).await;
		let (_, _, body) = send(&app, post(Some(&everything), &list)).await;
		assert_eq!(
			listed(&body),
			[
				"get_lesson_prompt",
				"list_lessons",
				"get_lesson",
				"get_progress",
				"list_my_lessons",
				"get_my_lesson",
				"keep_lesson"
			]
		);

		let reader = token(&pool, "subject-alice", "lessons:read").await;
		let (_, _, body) = send(&app, post(Some(&reader), &list)).await;
		assert_eq!(listed(&body), ["get_lesson_prompt", "list_lessons", "get_lesson"]);
		let (status, challenge, _) = send(&app, post(Some(&reader), &call("get_progress", &json!({})))).await;
		assert_eq!(status, StatusCode::FORBIDDEN);
		assert_eq!(
			challenge.as_deref(),
			Some(
				"Bearer error=\"insufficient_scope\", scope=\"progress:read\", \
				 resource_metadata=\"https://lessons.test/.well-known/oauth-protected-resource/api/v1/mcp\""
			)
		);
		assert_eq!(tool(&app, &reader, "get_lesson_prompt", &json!({})).await.0, PROMPT);

		// Without a configured prompt, the tool does not exist.
		let unprompted = self::app(AuthContext::for_tests_with_oauth(pool.clone()));
		let (_, _, body) = send(&unprompted, post(Some(&reader), &list)).await;
		assert_eq!(listed(&body), ["list_lessons", "get_lesson"]);
		let (_, _, body) = send(&unprompted, post(Some(&reader), &call("get_lesson_prompt", &json!({})))).await;
		assert_eq!(body["error"]["code"], -32_602);
	}

	#[tokio::test]
	async fn it_reads_the_corpus_and_the_subjects_own_progress() {
		let pool = pool().await;
		let entry = ManifestEntry {
			key: String::from("beginner"),
			display_name: String::from("Beginner"),
			description: String::from("d"),
			batch_count: 1,
			total_questions: 1,
			total_messages: 1,
			difficulty: None,
			tags: None,
		};
		CurriculumRepository::upsert(&mut pool.acquire().await.unwrap(), "topik", &entry, b"[{\"id\":1}]", "2026-10-03T00:00:00Z", false)
			.await
			.unwrap();
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let token = token(&pool, "subject-alice", ALL).await;

		let (_, error, manifest) = tool(&app, &token, "list_lessons", &json!({})).await;
		assert!(!error);
		assert_eq!(manifest["topiks"][0]["key"], "beginner");
		assert_eq!(tool(&app, &token, "get_lesson", &json!({ "key": "beginner" })).await.0, "[{\"id\":1}]", "verbatim");
		let (text, error, _) = tool(&app, &token, "get_lesson", &json!({ "key": "missing" })).await;
		assert!(error, "{text}");

		let (_, error, progress) = tool(&app, &token, "get_progress", &json!({})).await;
		assert!(!error);
		assert_eq!(progress, json!({ "activities": [], "history": [] }));
	}

	#[tokio::test]
	async fn a_kept_lesson_lands_on_the_subjects_shelf_and_never_replaces_one() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let alice = token(&pool, "subject-alice", "shelf").await;
		let bob = token(&pool, "subject-bob", "shelf").await;
		let lesson = json!([{ "id": "c1", "messages": [], "questions": [], "probes": [] }]);
		let manifest = json!({ "key": "first-dinner", "displayName": "The first family dinner" });

		let (text, error, kept) = tool(&app, &alice, "keep_lesson", &json!({ "lesson": lesson, "manifest": manifest })).await;
		assert!(!error, "{text}");
		assert_eq!(kept["change"], "kept");
		assert_eq!(kept["item"]["key"], "first-dinner");
		let again = tool(&app, &alice, "keep_lesson", &json!({ "lesson": lesson, "manifest": manifest })).await;
		assert_eq!(again.2["change"], "unchanged", "the same lesson again writes nothing");

		let (text, error, _) = tool(&app, &alice, "keep_lesson", &json!({ "lesson": [], "manifest": manifest })).await;
		assert!(error);
		assert!(text.contains("choose another key"), "{text}");

		// What the app plays from the shelf (`@some-ui/topik`'s `keptLessonOf`).
		let (body, _, _) = tool(&app, &alice, "get_my_lesson", &json!({ "key": "first-dinner" })).await;
		let document: Value = serde_json::from_str(&body).unwrap();
		assert_eq!(document["version"], 1);
		assert_eq!(document["meta"]["key"], "local:first-dinner");
		assert_eq!(document["meta"]["displayName"], "The first family dinner");
		assert_eq!(document["batches"], lesson);

		let (_, _, listing) = tool(&app, &alice, "list_my_lessons", &json!({})).await;
		assert_eq!(listing["items"].as_array().unwrap().len(), 1);
		assert_eq!(listing["cap"], 20);
		let (_, _, listing) = tool(&app, &bob, "list_my_lessons", &json!({})).await;
		assert_eq!(listing["items"], json!([]), "another subject's shelf is their own");
		let (_, error, _) = tool(&app, &bob, "get_my_lesson", &json!({ "key": "first-dinner" })).await;
		assert!(error);

		let (text, error, _) = tool(&app, &alice, "keep_lesson", &json!({ "lesson": lesson, "manifest": { "key": "http://x" } })).await;
		assert!(error);
		assert!(text.starts_with("refused: key"), "{text}");
		let (_, error, _) = tool(&app, &alice, "keep_lesson", &json!({ "lesson": "not an array", "manifest": manifest })).await;
		assert!(error);
	}
}
