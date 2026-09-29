//! Who may change what everyone is served.
//!
//! The operator routes rewrite, retire and restore corpus-wide content: the
//! lessons (`routes::db::curriculum_operator`) and the `leetype` rounds
//! (`routes::db::leetype_operator`). Before passkey auth they had no gate but
//! the CORS allowlist. A passkey session alone is not a gate either: anyone
//! who can reach the server can make an account (docs/identity.md, "Who can
//! make an account"). So an operator is a signed-in subject **listed in
//! `OPERATOR_SUBJECTS`**, and nothing else makes one.
//!
//! [`Operator`]'s extractor takes a [`SubjectId`] first, so a request without
//! a live session is `401`, a state-changing one from an untrusted origin is
//! `403` (`auth::csrf`), and the request holds off an account deletion, all
//! exactly as on every subject-scoped route. Then a subject not on the list is
//! `403`. With the list unset or empty, nobody is an operator: every operator
//! route answers `403`, and startup says so once (`AuthContext::from_config`).
//!
//! **Finding your subject id.** A refused subject is logged at `info`, by id
//! (`"a subject not in OPERATOR_SUBJECTS called an operator route"`): sign in
//! on the device you operate from, call any operator route (open the CRM),
//! and copy the `subject` from the server's log into `OPERATOR_SUBJECTS`. The
//! id is not returned to the browser: `GET /auth/session` answers only when
//! the session ends, and giving page script a stable, cross-device id it has
//! never needed would be a new exposure for the sake of a one-time setup step
//! the operator, who reads the log anyway, can do without it. The log already
//! carries subject ids (the nudge waker's decisions), which are random and
//! name nobody. An operator who claimed the pre-auth data with
//! `AUTH_LEGACY_CLAIM_TOKEN` is `subject-local`.

use super::AuthContext;
use crate::{subject::SubjectId, FileHostError};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;

/// A request from a subject listed in `OPERATOR_SUBJECTS`. Every operator
/// handler takes one; see the module docs.
#[derive(Debug, Clone)]
pub struct Operator {
	/// Held, not read: it carries the request's hold against the account
	/// being deleted, like any `SubjectId`.
	_subject: SubjectId,
}

/// `OPERATOR_SUBJECTS` as configured: each entry trimmed, empty ones dropped.
pub(crate) fn parse_subjects(raw: &[String]) -> Vec<String> {
	raw.iter().map(|subject| subject.trim()).filter(|subject| !subject.is_empty()).map(str::to_owned).collect()
}

// `axum-core` 0.4 still defines this trait through `async_trait`, so the impl
// has to be written the same way.
#[axum::async_trait]
impl<S> FromRequestParts<S> for Operator
where
	AuthContext: FromRef<S>,
	S: Send + Sync,
{
	type Rejection = FileHostError;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let subject = SubjectId::from_request_parts(parts, state).await?;
		if AuthContext::from_ref(state).is_operator(subject.as_str()) {
			Ok(Self { _subject: subject })
		} else {
			// How an operator finds the id to configure; see the module docs.
			tracing::info!(subject = %subject.as_str(), "a subject not in OPERATOR_SUBJECTS called an operator route");
			Err(FileHostError::Forbidden)
		}
	}
}

#[cfg(test)]
mod tests {
	//! Every route of every operator module, through its real router, with
	//! real sessions: no cookie is `401`, a signed-in subject that is not an
	//! operator is `403`, and an operator gets through. A handler that forgot
	//! to take [`Operator`] fails here.

	use super::parse_subjects;
	use crate::auth::{cookie::SessionToken, now, AuthContext};
	use crate::routes::table::Module;
	use axum::{
		body::Body,
		extract::FromRef,
		http::{header::COOKIE, Method, Request, StatusCode},
		Router,
	};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");
	const OPERATOR: &str = "subject-operator";
	const LEARNER: &str = "subject-learner";

	/// The two things an operator module's handlers need, and nothing that
	/// connects to NATS.
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

	fn operator_modules() -> Vec<Module<TestState>> {
		vec![crate::routes::db::curriculum_operator()]
	}

	async fn state(operators: &[&str]) -> TestState {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		TestState {
			auth: AuthContext::for_tests_with_operators(pool.clone(), operators),
			pool,
		}
	}

	/// An account for `subject` and a live session on it, as the cookie a
	/// browser would send. The ceremonies that normally get here are
	/// `handlers::auth`'s to test.
	async fn signed_in(state: &TestState, subject: &str) -> String {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?2)")
			.bind(subject)
			.bind(subject.as_bytes())
			.execute(&state.pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		let now = now();
		assert!(state.auth.repository().create_session(&token.hash(), subject, now + 3_600, now).await.unwrap());
		token.set_cookie(3_600).to_str().unwrap().split(';').next().unwrap().to_owned()
	}

	fn app(state: TestState) -> Router {
		operator_modules()
			.into_iter()
			.fold(Router::new(), |app, module| app.merge(module.into_table_router()))
			.with_state(state)
	}

	/// Every registered operator route, with its captures filled in.
	fn requests() -> Vec<(Method, String)> {
		operator_modules()
			.iter()
			.flat_map(Module::descriptors)
			.map(|route| {
				let path: Vec<&str> = route.path.split('/').map(|segment| if segment.starts_with(':') { "x" } else { segment }).collect();
				(Method::from_bytes(route.method.as_bytes()).unwrap(), path.join("/"))
			})
			.collect()
	}

	async fn status(app: &Router, method: Method, path: &str, cookie: Option<&str>, origin: Option<&str>) -> StatusCode {
		let mut request = Request::builder().method(method).uri(path);
		if let Some(cookie) = cookie {
			request = request.header(COOKIE, cookie);
		}
		if let Some(origin) = origin {
			request = request.header("origin", origin).header("sec-fetch-site", "same-site");
		}
		app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
	}

	#[tokio::test]
	async fn only_a_listed_subject_gets_past_any_operator_route() {
		let state = state(&[OPERATOR]).await;
		let operator = signed_in(&state, OPERATOR).await;
		let learner = signed_in(&state, LEARNER).await;
		let app = app(state);
		let requests = requests();
		assert!(requests.len() >= 4, "{requests:?}");

		for (method, path) in requests {
			assert_eq!(
				status(&app, method.clone(), &path, None, None).await,
				StatusCode::UNAUTHORIZED,
				"{method} {path} without a session"
			);
			assert_eq!(
				status(&app, method.clone(), &path, Some(&learner), None).await,
				StatusCode::FORBIDDEN,
				"{method} {path} for a signed-in subject that is not an operator"
			);
			let passed = status(&app, method.clone(), &path, Some(&operator), None).await;
			assert!(
				passed != StatusCode::UNAUTHORIZED && passed != StatusCode::FORBIDDEN,
				"{method} {path} for the operator answered {passed}"
			);
		}
	}

	/// The operator's session is still held to the origin check: a sibling
	/// page cannot use it to rewrite what everyone is served.
	#[tokio::test]
	async fn an_operators_session_from_an_untrusted_origin_is_refused() {
		let state = state(&[OPERATOR]).await;
		let operator = signed_in(&state, OPERATOR).await;
		let app = app(state);
		let path = "/curriculum/operator/lessons/x/retire";
		assert_eq!(
			status(&app, Method::POST, path, Some(&operator), Some("https://evil.app.test")).await,
			StatusCode::FORBIDDEN
		);
		assert_eq!(status(&app, Method::POST, path, Some(&operator), Some("https://app.test")).await, StatusCode::NOT_FOUND);
		assert_eq!(status(&app, Method::GET, "/curriculum/operator/lessons", Some(&operator), None).await, StatusCode::OK);
	}

	/// Unset, nobody is an operator — not even a subject that is signed in.
	#[tokio::test]
	async fn with_no_operators_configured_every_operator_route_is_refused() {
		let state = state(&[]).await;
		let signed_in = signed_in(&state, OPERATOR).await;
		let app = app(state);
		for (method, path) in requests() {
			assert_eq!(status(&app, method.clone(), &path, Some(&signed_in), None).await, StatusCode::FORBIDDEN, "{method} {path}");
		}
	}

	#[test]
	fn the_list_is_trimmed_and_empty_entries_name_nobody() {
		let raw: Vec<String> = [" subject-a", "subject-b ", "", "  "].iter().map(|&entry| entry.to_owned()).collect();
		assert_eq!(parse_subjects(&raw), ["subject-a", "subject-b"]);
		assert!(parse_subjects(&[String::new()]).is_empty(), "OPERATOR_SUBJECTS=\"\" is unset");
	}
}
