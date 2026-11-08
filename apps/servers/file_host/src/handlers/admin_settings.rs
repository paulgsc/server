//! Handlers for `routes::admin_settings`. See that module's doc comment for
//! the surface; this is the read-check-append half.
//!
//! Every handler takes an [`Operator`] (`auth::operator`) and the pool alone,
//! so the gate is tested through the real router (`auth::operator`'s tests
//! walk every route here).

use crate::auth::operator::Operator;
use crate::settings::{self, Entry};
use crate::FileHostError;
use admin_setting_repo::{Change, Revision};
use axum::{
	body::Bytes,
	extract::{Path, State},
	Json,
};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use tracing::instrument;

/// A setting as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
	pub namespace: &'static str,
	/// What home follows now: the value set, or the default while none is.
	pub value: Value,
	/// `true` while the operator has never set it: `value` is `default`.
	pub is_default: bool,
	pub default: Value,
	/// The standing revision, `null` while it is the default.
	pub revision: Option<i64>,
	/// `owner` or `operator`, `null` while it is the default.
	pub set_by: Option<&'static str>,
	/// ISO-8601 UTC, `null` while it is the default.
	pub set_at: Option<String>,
}

/// What a `PUT` did, and the setting as it now stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
	/// `added`: a new revision. `unchanged`: the value was the one standing.
	pub change: &'static str,
	#[serde(flatten)]
	pub standing: Standing,
}

/// One revision, in a history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRevision {
	pub revision: i64,
	pub value: Value,
	pub set_by: &'static str,
	pub set_at: String,
}

/// `GET /admin/settings/:namespace/history`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
	pub namespace: &'static str,
	pub default: Value,
	/// Newest first, at most `admin_setting_repo::HISTORY_CEILING`; empty
	/// while the setting has never been set.
	pub revisions: Vec<HistoryRevision>,
}

fn unreadable() -> FileHostError {
	FileHostError::OperationError(String::from("a stored admin setting does not read under its namespace"))
}

/// A stored value, held to its namespace again on the way out, so a value a
/// later schema no longer accepts is an error, never served as if it held.
fn value_of(entry: &Entry, revision: &Revision) -> Result<Value, FileHostError> {
	(entry.accept)(revision.body.as_bytes()).map_err(|_| unreadable())?;
	serde_json::from_str(&revision.body).map_err(|_| unreadable())
}

fn standing(entry: &'static Entry, revision: Option<&Revision>) -> Result<Standing, FileHostError> {
	let default = (entry.default)();
	Ok(match revision {
		None => Standing {
			namespace: entry.name,
			value: default.clone(),
			is_default: true,
			default,
			revision: None,
			set_by: None,
			set_at: None,
		},
		Some(revision) => Standing {
			namespace: entry.name,
			value: value_of(entry, revision)?,
			is_default: false,
			default,
			revision: Some(revision.revision),
			set_by: Some(revision.set_by.as_str()),
			set_at: Some(revision.set_at.clone()),
		},
	})
}

fn known(name: &str) -> Result<&'static Entry, FileHostError> {
	settings::namespace(name).ok_or(FileHostError::NotFound)
}

/// `GET /admin/settings/:namespace`
///
/// # Errors
/// 401/403 as for every operator route, 404 for a namespace home does not
/// have, and 500 for a storage failure or a stored value that no longer reads.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "admin_setting_read", skip_all, fields(otel.kind = "server"))]
pub async fn get_setting(_operator: Operator, State(db): State<SqlitePool>, Path(namespace): Path<String>) -> Result<Json<Standing>, FileHostError> {
	let entry = known(&namespace)?;
	let current = admin_setting_repo::current(&mut *db.acquire().await?, entry.name).await?;
	standing(entry, current.as_ref()).map(Json)
}

/// `GET /admin/settings/:namespace/history`
///
/// # Errors
/// As [`get_setting`].
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "admin_setting_history", skip_all, fields(otel.kind = "server"))]
pub async fn get_history(_operator: Operator, State(db): State<SqlitePool>, Path(namespace): Path<String>) -> Result<Json<History>, FileHostError> {
	let entry = known(&namespace)?;
	let revisions = admin_setting_repo::history(&mut *db.acquire().await?, entry.name).await?;
	let revisions = revisions
		.iter()
		.map(|revision| {
			Ok(HistoryRevision {
				revision: revision.revision,
				value: value_of(entry, revision)?,
				set_by: revision.set_by.as_str(),
				set_at: revision.set_at.clone(),
			})
		})
		.collect::<Result<_, FileHostError>>()?;
	Ok(Json(History {
		namespace: entry.name,
		default: (entry.default)(),
		revisions,
	}))
}

/// Sets `namespace` to `body` as `operator`, as of `now`.
pub(crate) async fn write(db: &SqlitePool, operator: &Operator, namespace: &str, body: &[u8], now: &str) -> Result<Written, FileHostError> {
	let entry = known(namespace)?;
	let value = (entry.accept)(body).map_err(|why| FileHostError::unprocessable_entity([("value", why)]))?;
	let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
	let (change, revision) = admin_setting_repo::append(&mut tx, entry.name, &value, operator.role(), now).await?;
	tx.commit().await?;
	Ok(Written {
		change: match change {
			Change::Added => "added",
			Change::Unchanged => "unchanged",
		},
		standing: standing(entry, Some(&revision))?,
	})
}

/// `PUT /admin/settings/:namespace`
///
/// # Errors
/// 401/403 as for every operator route, 404 for a namespace home does not
/// have, 422 with the reason for a body its schema refuses, 413 past the
/// route's body limit, and 500 for a storage failure.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "admin_setting_write", skip_all, fields(otel.kind = "server"))]
pub async fn put_setting(operator: Operator, State(db): State<SqlitePool>, Path(namespace): Path<String>, body: Bytes) -> Result<Json<Written>, FileHostError> {
	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
	write(&db, &operator, &namespace, &body, &now).await.map(Json)
}

#[cfg(test)]
mod tests {
	//! The routes through the real router, with the owner's session: the
	//! default reads as the default, a write adds a revision that reads back
	//! with who set it, a refused body changes nothing. The gate itself is
	//! `auth::operator`'s tests.

	use crate::auth::{cookie::SessionToken, now, AuthContext};
	use axum::{
		body::Body,
		extract::FromRef,
		http::{header::COOKIE, Method, Request, StatusCode},
		Router,
	};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const OWNER: &str = "subject-owner";
	const PATH: &str = "/admin/settings/rust-rounds";

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

	/// The owner's box: one account, `OPERATOR_SUBJECTS` unset, and the
	/// owner's session cookie.
	async fn owners_box() -> (Router, SqlitePool, String) {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		let auth = AuthContext::for_tests_with_operators(pool.clone(), &[]);
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(OWNER)
			.bind(OWNER.as_bytes())
			.execute(&pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(auth
			.repository()
			.create_session(&token.hash(crate::auth::cookie::Carrier::Cookie), OWNER, now + 3_600, now)
			.await
			.unwrap());
		let cookie = token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned();
		let app = crate::routes::admin_settings::admin_settings()
			.into_table_router()
			.with_state(TestState { pool: pool.clone(), auth });
		(app, pool, cookie)
	}

	async fn call(app: &Router, method: Method, path: &str, cookie: &str, body: Option<&str>) -> (StatusCode, Value) {
		let request = Request::builder().method(method).uri(path).header(COOKIE, cookie);
		let request = match body {
			Some(body) => request.header("content-type", "application/json").body(Body::from(body.to_owned())),
			None => request.body(Body::empty()),
		};
		let response = app.clone().oneshot(request.unwrap()).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
	}

	#[tokio::test]
	async fn an_unset_setting_reads_as_its_default_and_says_so() {
		let (app, _, cookie) = owners_box().await;
		let (status, body) = call(&app, Method::GET, PATH, &cookie, None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			body,
			json!({
				"namespace": "rust-rounds",
				"value": { "recording": true },
				"isDefault": true,
				"default": { "recording": true },
				"revision": null,
				"setBy": null,
				"setAt": null,
			})
		);
		let (_, history) = call(&app, Method::GET, &(String::from(PATH) + "/history"), &cookie, None).await;
		assert_eq!(history, json!({ "namespace": "rust-rounds", "default": { "recording": true }, "revisions": [] }));
	}

	#[tokio::test]
	async fn a_write_adds_a_revision_that_reads_back_with_who_set_it() {
		let (app, _, cookie) = owners_box().await;
		let (status, written) = call(&app, Method::PUT, PATH, &cookie, Some(r#"{"recording":false}"#)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(
			(&written["change"], &written["value"], &written["isDefault"], &written["revision"], &written["setBy"]),
			(&json!("added"), &json!({ "recording": false }), &json!(false), &json!(1), &json!("owner"))
		);
		let (_, again) = call(&app, Method::PUT, PATH, &cookie, Some(r#"{"recording":false}"#)).await;
		assert_eq!((&again["change"], &again["revision"]), (&json!("unchanged"), &json!(1)));
		call(&app, Method::PUT, PATH, &cookie, Some(r#"{"recording":true}"#)).await;

		let (_, standing) = call(&app, Method::GET, PATH, &cookie, None).await;
		assert_eq!(
			(&standing["value"], &standing["isDefault"], &standing["revision"]),
			(&json!({ "recording": true }), &json!(false), &json!(2))
		);
		let (_, history) = call(&app, Method::GET, &(String::from(PATH) + "/history"), &cookie, None).await;
		let revisions: Vec<(Value, Value, Value)> = history["revisions"]
			.as_array()
			.unwrap()
			.iter()
			.map(|revision| (revision["revision"].clone(), revision["value"].clone(), revision["setBy"].clone()))
			.collect();
		assert_eq!(
			revisions,
			[
				(json!(2), json!({ "recording": true }), json!("owner")),
				(json!(1), json!({ "recording": false }), json!("owner")),
			]
		);
	}

	#[tokio::test]
	async fn a_body_its_schema_refuses_is_422_and_changes_nothing() {
		let (app, pool, cookie) = owners_box().await;
		for refused in [r#"{"recording":"off"}"#, r#"{"recording":false,"why":"x"}"#, "{}", "paused"] {
			let (status, body) = call(&app, Method::PUT, PATH, &cookie, Some(refused)).await;
			assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
			assert!(body.to_string().contains("value"), "{refused}: {body}");
		}
		let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_setting").fetch_one(&pool).await.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn a_namespace_home_does_not_have_is_404() {
		let (app, _, cookie) = owners_box().await;
		assert_eq!(call(&app, Method::GET, "/admin/settings/nudge", &cookie, None).await.0, StatusCode::NOT_FOUND);
		assert_eq!(call(&app, Method::PUT, "/admin/settings/nudge", &cookie, Some("{}")).await.0, StatusCode::NOT_FOUND);
	}
}
