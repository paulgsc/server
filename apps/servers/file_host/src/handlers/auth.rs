//! `/api/v1/auth/*`: create an account, sign in, add a passkey, keep a session
//! alive, sign out, and leave.
//!
//! Every ceremony is two calls. `…/start` returns `{ ceremony, options }`:
//! `options` is what the browser's `navigator.credentials.create` or `.get`
//! takes (as WebAuthn JSON), and `ceremony` is the handle `…/finish` sends back
//! beside the browser's `credential`. A finish that creates a session answers
//! `{ expiresAt }` (ms since the epoch) and sets the session cookie.
//!
//! No request or response carries anything about the person: not a name, not
//! an email, not the authenticator's model. See `crate::auth`.

use crate::auth::{
	ceremony::{Ceremony, CeremonyId},
	cookie::{clear_cookie, SessionToken},
	now, passkey, AuthContext, PRODUCT_NAME,
};
use crate::subject::{legacy_subject, new_account_subject, SubjectId, SUBJECT_SCOPED_TABLES};
use crate::FileHostError;
use auth_repo::{AddPasskey, MAX_PASSKEYS_PER_SUBJECT};
use axum::{
	extract::State,
	http::{
		header::{HeaderName, SET_COOKIE},
		HeaderMap, HeaderValue, Method,
	},
	response::{AppendHeaders, IntoResponse, Response},
	Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::Instant;
use tracing::instrument;
use webauthn_rs::prelude::{CredentialID, DiscoverableKey, Passkey, PublicKeyCredential, RegisterPublicKeyCredential, Uuid};

/// A ceremony has begun: hand `options` to the browser, keep `ceremony`.
#[derive(Debug, Serialize)]
pub struct CeremonyStarted {
	pub ceremony: String,
	pub options: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishRegistration {
	pub ceremony: String,
	pub credential: RegisterPublicKeyCredential,
	/// The operator's `AUTH_LEGACY_CLAIM_TOKEN`, to take over the pre-auth
	/// data. Absent for every ordinary account.
	#[serde(default)]
	pub legacy_claim: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FinishSignIn {
	pub ceremony: String,
	pub credential: PublicKeyCredential,
}

/// A live session, as the client may see it: only when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
	/// ms since the epoch.
	pub expires_at: i64,
}

type WithCookie<T> = (AppendHeaders<[(HeaderName, HeaderValue); 1]>, Json<T>);

/// `POST /auth/register/start`: begin creating a new account.
///
/// # Errors
/// 503 when passkey auth is not configured, or when today's new-account cap
/// or the open-ceremony cap is reached.
#[instrument(name = "auth_register_start", skip_all, fields(otel.kind = "server"))]
pub async fn start_registration(State(auth): State<AuthContext>) -> Result<Json<CeremonyStarted>, FileHostError> {
	let relying_party = auth.relying_party()?;
	if !auth.signups_have_room(now()) {
		return Err(FileHostError::ServiceOverloaded);
	}
	let user_handle = random_user_handle();
	let (options, state) = relying_party
		.start_passkey_registration(user_handle, PRODUCT_NAME, PRODUCT_NAME, None)
		.map_err(FileHostError::upstream)?;
	open(
		&auth,
		Ceremony::Register {
			state,
			user_handle,
			subject: None,
		},
		passkey::registration_options(&options)?,
	)
}

/// `POST /auth/register/finish`: verify the new passkey, create the account,
/// and sign it in.
///
/// A new account gets a random subject. With `legacyClaim` equal to the
/// operator's `AUTH_LEGACY_CLAIM_TOKEN`, it takes the pre-auth subject and
/// its rows instead, once.
///
/// # Errors
/// 422 for an unknown or expired ceremony, or a passkey that does not
/// verify; 403 for a claim that does not match the configured token; 409 for
/// a passkey already registered, or pre-auth data already claimed; 503 as
/// for `start`.
#[instrument(name = "auth_register_finish", skip_all, fields(otel.kind = "server"))]
pub async fn finish_registration(State(auth): State<AuthContext>, Json(request): Json<FinishRegistration>) -> Result<WithCookie<SessionView>, FileHostError> {
	let relying_party = auth.relying_party()?;
	let Ceremony::Register {
		state,
		user_handle,
		subject: None,
	} = take(&auth, &request.ceremony)?
	else {
		return Err(stale_ceremony());
	};
	let verified = relying_party.finish_passkey_registration(&request.credential, &state).map_err(|_| unverified())?;

	let repository = auth.repository();
	let subject = match request.legacy_claim.as_deref() {
		None => new_account_subject(),
		Some(claim) if auth.legacy_claim_matches(claim) => {
			if repository.has_account(legacy_subject()).await? {
				return Err(already_claimed());
			}
			legacy_subject().to_owned()
		}
		Some(_) => return Err(FileHostError::Forbidden),
	};

	let now = now();
	if !auth.take_signup(now) {
		return Err(FileHostError::ServiceOverloaded);
	}
	repository
		.create_account(&subject, user_handle.as_bytes(), &passkey::for_storage(verified)?)
		.await
		.map_err(|err| if subject == legacy_subject() { claim_conflict(err) } else { already_registered(err) })?;
	start_session(&auth, &subject, now).await
}

/// `POST /auth/sign-in/start`: begin a username-less sign-in.
///
/// # Errors
/// 503 when passkey auth is not configured or the open-ceremony cap is
/// reached.
#[instrument(name = "auth_sign_in_start", skip_all, fields(otel.kind = "server"))]
pub async fn start_sign_in(State(auth): State<AuthContext>) -> Result<Json<CeremonyStarted>, FileHostError> {
	let (options, state) = auth.relying_party()?.start_discoverable_authentication().map_err(FileHostError::upstream)?;
	open(&auth, Ceremony::SignIn { state }, passkey::sign_in_options(options)?)
}

/// `POST /auth/sign-in/finish`: verify the passkey's signature and sign in.
///
/// # Errors
/// 401 when the passkey belongs to no account or does not verify; 422 for an
/// unknown or expired ceremony.
#[instrument(name = "auth_sign_in_finish", skip_all, fields(otel.kind = "server"))]
pub async fn finish_sign_in(State(auth): State<AuthContext>, Json(request): Json<FinishSignIn>) -> Result<WithCookie<SessionView>, FileHostError> {
	let relying_party = auth.relying_party()?;
	let Ceremony::SignIn { state } = take(&auth, &request.ceremony)? else {
		return Err(stale_ceremony());
	};
	let (user_handle, _) = relying_party
		.identify_discoverable_authentication(&request.credential)
		.map_err(|_| FileHostError::Unauthorized)?;

	let repository = auth.repository();
	let subject = repository.subject_for_user_handle(user_handle.as_bytes()).await?.ok_or(FileHostError::Unauthorized)?;
	let mut passkeys = repository
		.passkeys(&subject)
		.await?
		.iter()
		.map(passkey::from_storage)
		.collect::<Result<Vec<Passkey>, _>>()?;
	let keys: Vec<DiscoverableKey> = passkeys.iter().map(DiscoverableKey::from).collect();
	let result = relying_party
		.finish_discoverable_authentication(&request.credential, state, &keys)
		.map_err(|_| FileHostError::Unauthorized)?;

	if result.needs_update() {
		for key in &mut passkeys {
			if key.update_credential(&result) == Some(true) {
				repository.update_passkey(&subject, &passkey::for_storage(key.clone())?).await?;
			}
		}
	}
	start_session(&auth, &subject, now()).await
}

/// `POST /auth/passkeys/start`: begin adding another passkey to the signed-in
/// account, e.g. on a device outside the first passkey's sync ecosystem.
///
/// # Errors
/// 401 without a session; 409 when the account already holds the most
/// passkeys it may; 503 as for sign-in.
#[instrument(name = "auth_add_passkey_start", skip_all, fields(otel.kind = "server"))]
pub async fn start_adding_passkey(State(auth): State<AuthContext>, subject: SubjectId) -> Result<Json<CeremonyStarted>, FileHostError> {
	let relying_party = auth.relying_party()?;
	let repository = auth.repository();
	let user_handle = repository.user_handle(subject.as_str()).await?.ok_or(FileHostError::Unauthorized)?;
	let user_handle = Uuid::from_slice(&user_handle).map_err(FileHostError::upstream)?;
	let held: Vec<CredentialID> = repository
		.passkeys(subject.as_str())
		.await?
		.into_iter()
		.map(|stored| CredentialID::from(stored.credential_id))
		.collect();
	if i64::try_from(held.len())? >= MAX_PASSKEYS_PER_SUBJECT {
		return Err(too_many_passkeys());
	}
	let (options, state) = relying_party
		.start_passkey_registration(user_handle, PRODUCT_NAME, PRODUCT_NAME, Some(held))
		.map_err(FileHostError::upstream)?;
	open(
		&auth,
		Ceremony::Register {
			state,
			user_handle,
			subject: Some(subject.as_str().to_owned()),
		},
		passkey::registration_options(&options)?,
	)
}

/// `POST /auth/passkeys/finish`: verify and store the added passkey.
///
/// # Errors
/// 401 without a session; 422 for an unknown, expired or someone else's
/// ceremony, or a passkey that does not verify; 409 for a passkey already
/// registered or an account at its passkey cap.
#[instrument(name = "auth_add_passkey_finish", skip_all, fields(otel.kind = "server"))]
pub async fn finish_adding_passkey(State(auth): State<AuthContext>, subject: SubjectId, Json(request): Json<FinishRegistration>) -> Result<Json<Value>, FileHostError> {
	let relying_party = auth.relying_party()?;
	let Ceremony::Register { state, subject: Some(owner), .. } = take(&auth, &request.ceremony)? else {
		return Err(stale_ceremony());
	};
	if owner != subject.as_str() {
		return Err(stale_ceremony());
	}
	let verified = relying_party.finish_passkey_registration(&request.credential, &state).map_err(|_| unverified())?;
	match auth
		.repository()
		.add_passkey(subject.as_str(), &passkey::for_storage(verified)?)
		.await
		.map_err(already_registered)?
	{
		AddPasskey::Added => Ok(Json(Value::Object(Map::new()))),
		AddPasskey::AtCapacity => Err(too_many_passkeys()),
		AddPasskey::NoAccount => Err(FileHostError::Unauthorized),
	}
}

/// `GET /auth/session`: is this browser signed in, and until when?
///
/// Also where a session slides: past the half of its lifetime, it is
/// extended to a full one and the cookie is re-issued, when the request
/// comes from a trusted origin (`AuthContext::may_renew`). The app calls this
/// when it opens, so a session lasts as long as the app keeps being used.
///
/// # Errors
/// 401 (clearing the cookie) without a live session.
#[instrument(name = "auth_session", skip_all, fields(otel.kind = "server"))]
pub async fn session(State(auth): State<AuthContext>, headers: HeaderMap) -> Result<Response, FileHostError> {
	let Some(token) = SessionToken::from_headers(&headers) else {
		return Ok(signed_out(FileHostError::Unauthorized));
	};
	let repository = auth.repository();
	let now = now();
	let Some(live) = repository.live_session(&token.hash(), now).await? else {
		return Ok(signed_out(FileHostError::Unauthorized));
	};

	let ttl = auth.session_ttl_seconds();
	// Reporting is open to any GET; extending is not (`AuthContext::may_renew`).
	if live.expires_at - now < ttl / 2 && auth.may_renew(&headers) {
		let expires_at = now + ttl;
		repository.extend_session(&token.hash(), expires_at).await?;
		return Ok(with_cookie(token.set_cookie(ttl), expires_at).into_response());
	}
	Ok(
		Json(SessionView {
			expires_at: live.expires_at * 1_000,
		})
		.into_response(),
	)
}

/// `POST /auth/sign-out`: end this browser's session. Succeeds without one,
/// so a client can always call it.
///
/// # Errors
/// 403 from an untrusted origin (`auth::csrf`); 500 for a storage failure.
#[instrument(name = "auth_sign_out", skip_all, fields(otel.kind = "server"))]
pub async fn sign_out(State(auth): State<AuthContext>, method: Method, headers: HeaderMap) -> Result<Response, FileHostError> {
	// No `SubjectId` here, so the origin check its extractor makes is made
	// directly: a sibling page must not be able to sign this browser out.
	auth.check_origin(&method, &headers)?;
	if let Some(token) = SessionToken::from_headers(&headers) {
		auth.repository().end_session(&token.hash()).await?;
	}
	Ok(signed_out(Json(Value::Object(Map::new()))))
}

/// `POST /auth/sign-out-everywhere`: end every session this account holds.
///
/// # Errors
/// 401 without a session.
#[instrument(name = "auth_sign_out_everywhere", skip_all, fields(otel.kind = "server"))]
pub async fn sign_out_everywhere(State(auth): State<AuthContext>, subject: SubjectId) -> Result<Response, FileHostError> {
	auth.repository().end_all_sessions(subject.as_str()).await?;
	Ok(signed_out(Json(Value::Object(Map::new()))))
}

/// `DELETE /auth/account`: delete the account and everything stored for it.
///
/// One transaction over every table in [`SUBJECT_SCOPED_TABLES`], the same
/// list the privacy schema test holds every per-person table to, so nothing
/// the account owned outlives it. The passkeys go with it: they no longer
/// open anything.
///
/// # Errors
/// 401 without a session.
#[instrument(name = "auth_delete_account", skip_all, fields(otel.kind = "server"))]
pub async fn delete_account(State(auth): State<AuthContext>, subject: SubjectId) -> Result<Response, FileHostError> {
	// This request's own hold goes first, or the exclusive hold below would
	// wait for it forever.
	let subject = subject.release();
	delete_subject(&auth, &subject).await?;
	Ok(signed_out(Json(Value::Object(Map::new()))))
}

/// Delete every row `subject` owns, once no request acting for a subject is
/// still running (`AuthContext::exclude_requests`). A write such a request
/// makes either lands first and is deleted here, or comes after a session
/// lookup that finds nothing.
pub(crate) async fn delete_subject(auth: &AuthContext, subject: &str) -> Result<(), FileHostError> {
	let _exclusive = auth.exclude_requests().await;
	let mut tx = auth.pool().begin().await?;
	for (table, _) in SUBJECT_SCOPED_TABLES {
		// Table names come from a constant list, never from a request.
		let statement = String::from("DELETE FROM ") + table + " WHERE subject_id = ?1";
		sqlx::query(&statement).bind(subject).execute(&mut *tx).await?;
	}
	tx.commit().await?;
	Ok(())
}

async fn start_session(auth: &AuthContext, subject: &str, now: i64) -> Result<WithCookie<SessionView>, FileHostError> {
	let token = SessionToken::mint();
	let ttl = auth.session_ttl_seconds();
	// False when the account was deleted while this sign-in was in flight.
	if !auth.repository().create_session(&token.hash(), subject, now + ttl, now).await? {
		return Err(FileHostError::Unauthorized);
	}
	Ok(with_cookie(token.set_cookie(ttl), now + ttl))
}

const fn with_cookie(set_cookie: HeaderValue, expires_at: i64) -> WithCookie<SessionView> {
	(AppendHeaders([(SET_COOKIE, set_cookie)]), Json(SessionView { expires_at: expires_at * 1_000 }))
}

fn signed_out(body: impl IntoResponse) -> Response {
	(AppendHeaders([(SET_COOKIE, clear_cookie())]), body).into_response()
}

fn open(auth: &AuthContext, ceremony: Ceremony, options: Value) -> Result<Json<CeremonyStarted>, FileHostError> {
	let id = auth.ceremonies().open(ceremony, Instant::now()).ok_or(FileHostError::ServiceOverloaded)?;
	Ok(Json(CeremonyStarted {
		ceremony: URL_SAFE_NO_PAD.encode(id),
		options,
	}))
}

fn take(auth: &AuthContext, ceremony: &str) -> Result<Ceremony, FileHostError> {
	URL_SAFE_NO_PAD
		.decode(ceremony)
		.ok()
		.and_then(|bytes| CeremonyId::try_from(bytes).ok())
		.and_then(|id| auth.ceremonies().take(&id, Instant::now()))
		.ok_or_else(stale_ceremony)
}

/// The WebAuthn `user.id` for a new account: 16 random bytes. The library
/// types it as a UUID; nothing reads it as one.
fn random_user_handle() -> Uuid {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	Uuid::from_bytes(bytes)
}

fn stale_ceremony() -> FileHostError {
	FileHostError::unprocessable_entity([("ceremony", "unknown or expired; start again")])
}

fn unverified() -> FileHostError {
	FileHostError::unprocessable_entity([("credential", "the passkey could not be verified")])
}

const fn too_many_passkeys() -> FileHostError {
	FileHostError::Conflict("this account already holds the most passkeys it may")
}

const fn already_claimed() -> FileHostError {
	FileHostError::Conflict("the data saved before auth has already been claimed")
}

fn claim_conflict(err: sqlx::Error) -> FileHostError {
	if err.as_database_error().is_some_and(sqlx::error::DatabaseError::is_unique_violation) {
		already_claimed()
	} else {
		FileHostError::Sqlite(err)
	}
}

fn already_registered(err: sqlx::Error) -> FileHostError {
	if err.as_database_error().is_some_and(sqlx::error::DatabaseError::is_unique_violation) {
		FileHostError::Conflict("this passkey is already registered")
	} else {
		FileHostError::Sqlite(err)
	}
}

#[cfg(test)]
mod tests {
	//! Real passkey ceremonies against the real routes: a software
	//! authenticator signs what the server challenges, over the router the
	//! server serves. Nothing here fakes a verification.

	use crate::auth::{cookie::SESSION_COOKIE, AuthContext};
	use crate::subject::{SubjectId, SUBJECT_SCOPED_TABLES};
	use axum::{
		body::Body,
		http::{header::COOKIE, header::SET_COOKIE, Method, Request, StatusCode},
		routing::get,
		Router,
	};
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
	use serde_json::{json, Value};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;
	use webauthn_authenticator_rs::{softpasskey::SoftPasskey, WebauthnAuthenticator};
	use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	type Authenticator = WebauthnAuthenticator<SoftPasskey>;

	/// A signed-in browser: its session cookie, and the passkey it holds.
	struct Browser {
		cookie: String,
		user_handle: Vec<u8>,
		credential_id: String,
	}

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// The auth routes, plus one subject-scoped probe that answers with the
	/// subject the extractor resolved.
	fn app(auth: AuthContext) -> Router {
		// A throwaway probe, not a served route, so it is not on a RouteTable.
		#[allow(clippy::disallowed_methods)]
		let probe = Router::new().route("/whoami", get(|subject: SubjectId| async move { subject.as_str().to_owned() }));
		crate::routes::auth::auth::<AuthContext>().into_table_router().merge(probe).with_state(auth)
	}

	fn authenticator() -> Authenticator {
		WebauthnAuthenticator::new(SoftPasskey::new(true))
	}

	fn origin() -> Url {
		Url::parse("https://app.test").unwrap()
	}

	async fn call(app: &Router, method: Method, path: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Option<String>, Value) {
		call_with(app, method, path, cookie, body, &[]).await
	}

	async fn call_with(app: &Router, method: Method, path: &str, cookie: Option<&str>, body: Option<Value>, headers: &[(&str, &str)]) -> (StatusCode, Option<String>, Value) {
		let mut request = Request::builder().method(method).uri(path);
		for (name, value) in headers {
			request = request.header(*name, *value);
		}
		if let Some(cookie) = cookie {
			request = request.header(COOKIE, cookie);
		}
		let request = match body {
			Some(body) => request.header("content-type", "application/json").body(Body::from(body.to_string())),
			None => request.body(Body::empty()),
		}
		.unwrap();
		let response = app.clone().oneshot(request).await.unwrap();
		let status = response.status();
		let set_cookie = response.headers().get(SET_COOKIE).map(|value| value.to_str().unwrap().to_owned());
		let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
		let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
		(status, set_cookie, body)
	}

	/// `name=value` from a `Set-Cookie`, ready to send back.
	fn cookie_pair(set_cookie: &str) -> String {
		set_cookie.split(';').next().unwrap().to_owned()
	}

	/// The software authenticator cannot store a discoverable credential, so
	/// it is asked for an ordinary one. What the server verifies is the same.
	fn for_soft_authenticator(mut options: Value) -> CreationChallengeResponse {
		let selection = options.pointer_mut("/publicKey/authenticatorSelection").unwrap().as_object_mut().unwrap();
		selection.insert(String::from("requireResidentKey"), json!(false));
		selection.insert(String::from("residentKey"), json!("discouraged"));
		serde_json::from_value(options).unwrap()
	}

	async fn create_passkey(app: &Router, authenticator: &mut Authenticator, path: &str, cookie: Option<&str>) -> (StatusCode, Option<String>, Value, Vec<u8>, String) {
		let (status, _, started) = call(app, Method::POST, &(String::from(path) + "/start"), cookie, None).await;
		assert_eq!(status, StatusCode::OK, "{started}");
		let user_handle = URL_SAFE_NO_PAD.decode(started["options"]["publicKey"]["user"]["id"].as_str().unwrap()).unwrap();
		let credential = authenticator.do_registration(origin(), for_soft_authenticator(started["options"].clone())).unwrap();
		let credential = serde_json::to_value(&credential).unwrap();
		let credential_id = credential["id"].as_str().unwrap().to_owned();
		let body = json!({ "ceremony": started["ceremony"], "credential": credential });
		let (status, set_cookie, finished) = call(app, Method::POST, &(String::from(path) + "/finish"), cookie, Some(body)).await;
		(status, set_cookie, finished, user_handle, credential_id)
	}

	async fn register(app: &Router, authenticator: &mut Authenticator) -> Browser {
		let (status, set_cookie, body, user_handle, credential_id) = create_passkey(app, authenticator, "/auth/register", None).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert!(body["expiresAt"].as_i64().unwrap() > 0);
		Browser {
			cookie: cookie_pair(&set_cookie.unwrap()),
			user_handle,
			credential_id,
		}
	}

	/// A username-less sign-in. A real authenticator finds its own
	/// credential and returns its user handle; the software one needs the
	/// credential named, and the test supplies the user handle, which is
	/// not part of what the authenticator signs.
	async fn sign_in(app: &Router, authenticator: &mut Authenticator, credential_id: &str, user_handle: &[u8]) -> (StatusCode, Option<String>, Value) {
		let (status, _, started) = call(app, Method::POST, "/auth/sign-in/start", None, None).await;
		assert_eq!(status, StatusCode::OK);
		assert!(
			started["options"]["publicKey"]["allowCredentials"].as_array().unwrap().is_empty(),
			"sign-in names no account"
		);
		let mut options = started["options"].clone();
		options["publicKey"]["allowCredentials"] = json!([{ "type": "public-key", "id": credential_id }]);
		let options: RequestChallengeResponse = serde_json::from_value(options).unwrap();
		let credential = authenticator.do_authentication(origin(), options).unwrap();
		let mut credential = serde_json::to_value(&credential).unwrap();
		credential["response"]["userHandle"] = json!(URL_SAFE_NO_PAD.encode(user_handle));
		call(
			app,
			Method::POST,
			"/auth/sign-in/finish",
			None,
			Some(json!({ "ceremony": started["ceremony"], "credential": credential })),
		)
		.await
	}

	async fn whoami(app: &Router, cookie: &str) -> (StatusCode, Value) {
		let (status, _, body) = call(app, Method::GET, "/whoami", Some(cookie), None).await;
		(status, body)
	}

	#[tokio::test]
	async fn registration_options_ask_for_nothing_identifying() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let (_, _, first) = call(&app, Method::POST, "/auth/register/start", None, None).await;
		let (_, _, second) = call(&app, Method::POST, "/auth/register/start", None, None).await;
		let options = &first["options"]["publicKey"];

		assert_eq!(options["attestation"], "none", "invariant 7: no authenticator model");
		assert_eq!(options["authenticatorSelection"]["residentKey"], "required", "sign-in names no account");
		assert_eq!(options["authenticatorSelection"]["requireResidentKey"], true);
		assert_eq!(options["authenticatorSelection"]["userVerification"], "required");
		assert_eq!(options["user"]["name"], "Some UI");
		assert_eq!(options["user"]["displayName"], "Some UI");
		let user_handle = URL_SAFE_NO_PAD.decode(options["user"]["id"].as_str().unwrap()).unwrap();
		assert_eq!(user_handle.len(), 16, "invariant 7: user.id is random bytes");
		assert_ne!(options["user"]["id"], second["options"]["publicKey"]["user"]["id"]);
		assert_eq!(options["rp"]["id"], "app.test");
	}

	/// Registers with `legacyClaim` in the finish body.
	async fn register_claiming(app: &Router, claim: &str) -> (StatusCode, Option<String>, Value) {
		let (_, _, started) = call(app, Method::POST, "/auth/register/start", None, None).await;
		let credential = authenticator().do_registration(origin(), for_soft_authenticator(started["options"].clone())).unwrap();
		let body = json!({ "ceremony": started["ceremony"], "credential": serde_json::to_value(&credential).unwrap(), "legacyClaim": claim });
		call(app, Method::POST, "/auth/register/finish", None, Some(body)).await
	}

	#[tokio::test]
	async fn being_first_does_not_inherit_the_pre_auth_data() {
		let app = app(AuthContext::for_tests_with_claim(pool().await, 100, Some("operator-secret")));

		let first = register(&app, &mut authenticator()).await;
		let (status, subject) = whoami(&app, &first.cookie).await;
		assert_eq!(status, StatusCode::OK);
		assert!(subject.as_str().unwrap().starts_with("subject-") && subject != "subject-local", "{subject}");
	}

	#[tokio::test]
	async fn only_the_operators_claim_token_inherits_the_pre_auth_data_and_only_once() {
		let app = app(AuthContext::for_tests_with_claim(pool().await, 100, Some("operator-secret")));

		let (status, _, _) = register_claiming(&app, "a-guess").await;
		assert_eq!(status, StatusCode::FORBIDDEN, "a wrong token claims nothing");

		let (status, set_cookie, body) = register_claiming(&app, "operator-secret").await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert_eq!(whoami(&app, &cookie_pair(&set_cookie.unwrap())).await, (StatusCode::OK, json!("subject-local")));

		let (status, set_cookie, _) = register_claiming(&app, "operator-secret").await;
		assert_eq!(status, StatusCode::CONFLICT, "the token works once");
		assert!(set_cookie.is_none());
	}

	#[tokio::test]
	async fn with_no_claim_token_configured_nobody_can_claim() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let (status, _, _) = register_claiming(&app, "").await;
		assert_eq!(status, StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn a_sibling_origin_cannot_sign_a_browser_out() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let browser = register(&app, &mut authenticator()).await;
		let sibling = [("origin", "https://evil.app.test"), ("sec-fetch-site", "same-site")];

		for path in ["/auth/sign-out-everywhere", "/auth/sign-out"] {
			let (status, set_cookie, _) = call_with(&app, Method::POST, path, Some(&browser.cookie), None, &sibling).await;
			assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
			assert!(set_cookie.is_none(), "{path}");
		}
		assert_eq!(whoami(&app, &browser.cookie).await.0, StatusCode::OK, "still signed in");

		let own = [("origin", "https://app.test"), ("sec-fetch-site", "same-site")];
		let (status, _, _) = call_with(&app, Method::POST, "/auth/sign-out-everywhere", Some(&browser.cookie), None, &own).await;
		assert_eq!(status, StatusCode::OK, "the app's own origin may");
	}

	#[tokio::test]
	async fn a_stored_passkey_keeps_no_attestation_and_no_transports() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests(pool.clone(), 100));
		register(&app, &mut authenticator()).await;

		let stored: String = sqlx::query_scalar("SELECT passkey FROM passkey").fetch_one(&pool).await.unwrap();
		let stored: Value = serde_json::from_str(&stored).unwrap();
		assert_eq!(stored["cred"]["attestation_format"], "none");
		assert_eq!(stored["cred"]["attestation"], json!({ "data": "None", "metadata": "None" }));
		assert_eq!(stored["cred"]["transports"], Value::Null);
	}

	#[tokio::test]
	async fn signing_out_ends_the_session_and_the_passkey_signs_back_in_to_the_same_account() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let mut authenticator = authenticator();
		let browser = register(&app, &mut authenticator).await;
		let (_, subject) = whoami(&app, &browser.cookie).await;

		let (status, set_cookie, _) = call(&app, Method::POST, "/auth/sign-out", Some(&browser.cookie), None).await;
		assert_eq!(status, StatusCode::OK);
		assert!(set_cookie.unwrap().contains("Max-Age=0"), "the browser is told to drop the cookie");
		assert_eq!(whoami(&app, &browser.cookie).await.0, StatusCode::UNAUTHORIZED, "the old cookie opens nothing");

		let (status, set_cookie, body) = sign_in(&app, &mut authenticator, &browser.credential_id, &browser.user_handle).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		let cookie = cookie_pair(&set_cookie.unwrap());
		assert_ne!(cookie, browser.cookie, "a new session, not the old one back");
		assert_eq!(whoami(&app, &cookie).await, (StatusCode::OK, subject));
	}

	#[tokio::test]
	async fn a_passkey_cannot_sign_in_to_an_account_it_does_not_belong_to() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let mut mine = authenticator();
		let me = register(&app, &mut mine).await;
		let other = register(&app, &mut authenticator()).await;

		// My passkey's signature, presented under the other account's handle.
		let (status, set_cookie, _) = sign_in(&app, &mut mine, &me.credential_id, &other.user_handle).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		assert!(set_cookie.is_none());

		let (status, _, _) = sign_in(&app, &mut mine, &me.credential_id, &[9; 16]).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "an unknown user handle opens nothing");
	}

	#[tokio::test]
	async fn a_ceremony_is_finished_once() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let (_, _, started) = call(&app, Method::POST, "/auth/register/start", None, None).await;
		let credential = authenticator().do_registration(origin(), for_soft_authenticator(started["options"].clone())).unwrap();
		let body = json!({ "ceremony": started["ceremony"], "credential": serde_json::to_value(&credential).unwrap() });

		assert_eq!(call(&app, Method::POST, "/auth/register/finish", None, Some(body.clone())).await.0, StatusCode::OK);
		let (status, set_cookie, _) = call(&app, Method::POST, "/auth/register/finish", None, Some(body)).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a replay finds no ceremony");
		assert!(set_cookie.is_none());
	}

	#[tokio::test]
	async fn without_a_live_session_every_subject_scoped_route_answers_401() {
		let app = app(AuthContext::for_tests(pool().await, 100));

		let (status, _, _) = call(&app, Method::GET, "/whoami", None, None).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "no fallback subject");
		let forged = String::from(SESSION_COOKIE) + "=" + &URL_SAFE_NO_PAD.encode([0_u8; 32]);
		assert_eq!(whoami(&app, &forged).await.0, StatusCode::UNAUTHORIZED);

		let (status, set_cookie, _) = call(&app, Method::GET, "/auth/session", Some(&forged), None).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		assert!(set_cookie.unwrap().contains("Max-Age=0"), "a dead cookie is cleared");
		for path in ["/auth/passkeys/start", "/auth/sign-out-everywhere"] {
			assert_eq!(call(&app, Method::POST, path, None, None).await.0, StatusCode::UNAUTHORIZED, "{path}");
		}
		assert_eq!(call(&app, Method::DELETE, "/auth/account", None, None).await.0, StatusCode::UNAUTHORIZED);
	}

	#[tokio::test]
	async fn a_session_reports_when_it_ends_and_slides_past_its_halfway_point() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests(pool.clone(), 100));
		let browser = register(&app, &mut authenticator()).await;

		let (status, set_cookie, body) = call(&app, Method::GET, "/auth/session", Some(&browser.cookie), None).await;
		assert_eq!(status, StatusCode::OK);
		assert!(set_cookie.is_none(), "a fresh session is not re-issued");
		let expires_at = body["expiresAt"].as_i64().unwrap();

		// Age the session to a week left of thirty days.
		let week_left = crate::auth::now() + 7 * 86_400;
		sqlx::query("UPDATE auth_session SET expires_at = ?1").bind(week_left).execute(&pool).await.unwrap();
		let (status, set_cookie, body) = call(&app, Method::GET, "/auth/session", Some(&browser.cookie), None).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(cookie_pair(&set_cookie.unwrap()), browser.cookie, "the same session, re-issued for a full term");
		assert!(body["expiresAt"].as_i64().unwrap() >= expires_at);
	}

	#[tokio::test]
	async fn an_added_passkey_opens_the_same_account() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let browser = register(&app, &mut authenticator()).await;
		let (_, subject) = whoami(&app, &browser.cookie).await;
		let mut laptop = authenticator();

		let (status, set_cookie, body, user_handle, credential_id) = create_passkey(&app, &mut laptop, "/auth/passkeys", Some(&browser.cookie)).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		assert!(set_cookie.is_none(), "adding a passkey does not start a session");
		assert_eq!(user_handle, browser.user_handle, "the new passkey carries the account's user handle");

		let (status, set_cookie, _) = sign_in(&app, &mut laptop, &credential_id, &user_handle).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(whoami(&app, &cookie_pair(&set_cookie.unwrap())).await, (StatusCode::OK, subject));
	}

	#[tokio::test]
	async fn adding_a_passkey_needs_the_session_that_started_it() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let me = register(&app, &mut authenticator()).await;
		let other = register(&app, &mut authenticator()).await;

		let (_, _, started) = call(&app, Method::POST, "/auth/passkeys/start", Some(&me.cookie), None).await;
		let credential = authenticator().do_registration(origin(), for_soft_authenticator(started["options"].clone())).unwrap();
		let body = json!({ "ceremony": started["ceremony"], "credential": serde_json::to_value(&credential).unwrap() });
		let (status, _, _) = call(&app, Method::POST, "/auth/passkeys/finish", Some(&other.cookie), Some(body)).await;
		assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "another account cannot finish my ceremony");
	}

	#[tokio::test]
	async fn signing_out_everywhere_ends_every_session_of_that_account_only() {
		let app = app(AuthContext::for_tests(pool().await, 100));
		let mut phone = authenticator();
		let first = register(&app, &mut phone).await;
		let (_, set_cookie, _) = sign_in(&app, &mut phone, &first.credential_id, &first.user_handle).await;
		let second = cookie_pair(&set_cookie.unwrap());
		let stranger = register(&app, &mut authenticator()).await;

		assert_eq!(call(&app, Method::POST, "/auth/sign-out-everywhere", Some(&first.cookie), None).await.0, StatusCode::OK);
		assert_eq!(whoami(&app, &first.cookie).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(whoami(&app, &second).await.0, StatusCode::UNAUTHORIZED);
		assert_eq!(whoami(&app, &stranger.cookie).await.0, StatusCode::OK);
	}

	#[tokio::test]
	async fn a_sibling_page_cannot_keep_a_session_alive() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests(pool.clone(), 100));
		let browser = register(&app, &mut authenticator()).await;
		let week_left = crate::auth::now() + 7 * 86_400;
		sqlx::query("UPDATE auth_session SET expires_at = ?1").bind(week_left).execute(&pool).await.unwrap();

		// What an `<img src=…/auth/session>` on a sibling origin sends: the
		// cookie, `Sec-Fetch-Site: same-site`, and no `Origin`.
		let image = [("sec-fetch-site", "same-site")];
		let (status, set_cookie, _) = call_with(&app, Method::GET, "/auth/session", Some(&browser.cookie), None, &image).await;
		assert_eq!(status, StatusCode::OK, "it may still read that the session is live");
		assert!(set_cookie.is_none(), "but not re-issue it");
		let stored: i64 = sqlx::query_scalar("SELECT expires_at FROM auth_session").fetch_one(&pool).await.unwrap();
		assert_eq!(stored, week_left, "or extend it");

		let own = [("sec-fetch-site", "same-origin")];
		let (_, set_cookie, _) = call_with(&app, Method::GET, "/auth/session", Some(&browser.cookie), None, &own).await;
		assert!(set_cookie.is_some(), "the app itself still slides it");
	}

	#[tokio::test]
	async fn a_write_already_in_flight_does_not_outlive_the_account_it_was_for() {
		use std::sync::Arc;
		use tokio::sync::Notify;

		let pool = pool().await;
		let resolved = Arc::new(Notify::new());
		let go = Arc::new(Notify::new());
		// A subject-scoped write that stalls between resolving its subject
		// and writing, which is where a deletion could otherwise slip in.
		let write = {
			let (pool, resolved, go) = (pool.clone(), Arc::clone(&resolved), Arc::clone(&go));
			move |subject: SubjectId| async move {
				resolved.notify_one();
				go.notified().await;
				sqlx::query("INSERT INTO presence_leases (subject_id, context_key, observed_at) VALUES (?1, 'k', 'now')")
					.bind(subject.as_str())
					.execute(&pool)
					.await
					.unwrap();
			}
		};
		// A throwaway probe, not a served route, so it is not on a RouteTable.
		#[allow(clippy::disallowed_methods)]
		let probe = Router::new().route("/write", axum::routing::post(write));
		let app = crate::routes::auth::auth::<AuthContext>()
			.into_table_router()
			.merge(probe)
			.with_state(AuthContext::for_tests(pool.clone(), 100));
		let browser = register(&app, &mut authenticator()).await;

		let in_flight = tokio::spawn({
			let (app, cookie) = (app.clone(), browser.cookie.clone());
			async move { call(&app, Method::POST, "/write", Some(&cookie), None).await }
		});
		resolved.notified().await;
		let deletion = tokio::spawn({
			let (app, cookie) = (app.clone(), browser.cookie.clone());
			async move { call(&app, Method::DELETE, "/auth/account", Some(&cookie), None).await }
		});
		tokio::time::sleep(std::time::Duration::from_millis(50)).await;
		assert!(!deletion.is_finished(), "the deletion waits for the write it would otherwise race");

		go.notify_one();
		assert_eq!(in_flight.await.unwrap().0, StatusCode::OK);
		assert_eq!(deletion.await.unwrap().0, StatusCode::OK);
		let leftover: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM presence_leases").fetch_one(&pool).await.unwrap();
		assert_eq!(leftover, 0, "the write landed first, so the deletion took it too");
		let (status, _, _) = call(&app, Method::POST, "/write", Some(&browser.cookie), None).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "and a write after it finds no session");
	}

	#[tokio::test]
	async fn deleting_an_account_removes_its_rows_from_every_subject_scoped_table_and_nobody_elses() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests(pool.clone(), 100));
		let mut phone = authenticator();
		let me = register(&app, &mut phone).await;
		let (_, my_subject) = whoami(&app, &me.cookie).await;
		let stranger = register(&app, &mut authenticator()).await;
		let (_, stranger_subject) = whoami(&app, &stranger.cookie).await;

		for subject in [my_subject.as_str().unwrap(), stranger_subject.as_str().unwrap()] {
			sqlx::query(
				"INSERT INTO sessions (id, subject_id, name, status, origin, layout_mode, total_duration_ms, created_at, updated_at, activities, scenes)
				 VALUES (?1, ?2, 'n', 'draft', 'authored', 'basic', 0, 'now', 'now', '[]', '[]')",
			)
			.bind(String::from("s-") + subject)
			.bind(subject)
			.execute(&pool)
			.await
			.unwrap();
			sqlx::query("INSERT INTO presence_leases (subject_id, context_key, observed_at) VALUES (?1, 'k', 'now')")
				.bind(subject)
				.execute(&pool)
				.await
				.unwrap();
		}

		let (status, set_cookie, _) = call(&app, Method::DELETE, "/auth/account", Some(&me.cookie), None).await;
		assert_eq!(status, StatusCode::OK);
		assert!(set_cookie.unwrap().contains("Max-Age=0"));

		for (table, _) in SUBJECT_SCOPED_TABLES {
			let mine: i64 = sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table + " WHERE subject_id = ?1"))
				.bind(my_subject.as_str().unwrap())
				.fetch_one(&pool)
				.await
				.unwrap();
			assert_eq!(mine, 0, "{table} kept a deleted account's row");
		}
		for table in ["account", "passkey", "auth_session", "sessions", "presence_leases"] {
			let theirs: i64 = sqlx::query_scalar(&(String::from("SELECT COUNT(*) FROM ") + table + " WHERE subject_id = ?1"))
				.bind(stranger_subject.as_str().unwrap())
				.fetch_one(&pool)
				.await
				.unwrap();
			assert_eq!(theirs, 1, "{table} lost another account's row");
		}
		assert_eq!(whoami(&app, &me.cookie).await.0, StatusCode::UNAUTHORIZED);
		let (status, _, _) = sign_in(&app, &mut phone, &me.credential_id, &me.user_handle).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "a deleted account's passkey opens nothing");
	}

	/// docs/identity.md, "Secrets are wrapped when they exist": a whole
	/// sign-up, sign-in and refused sign-in, captured field by field at every
	/// level the production subscriber lets through, names no session token,
	/// credential id or user handle.
	#[tokio::test]
	async fn no_secret_reaches_a_log_line() {
		let (captured, _guard) = crate::privacy::capture_where(|metadata| crate::auth::loggable(metadata) && !metadata.target().starts_with("webauthn_authenticator_rs"));
		let app = app(AuthContext::for_tests(pool().await, 100));
		let mut phone = authenticator();

		let browser = register(&app, &mut phone).await;
		let (_, set_cookie, _) = sign_in(&app, &mut phone, &browser.credential_id, &browser.user_handle).await;
		let (status, _, _) = sign_in(&app, &mut phone, &browser.credential_id, &[9; 16]).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "sanity check: the refusal path ran");
		assert_eq!(whoami(&app, "__Host-session=garbage").await.0, StatusCode::UNAUTHORIZED);

		let lines = captured.lines();
		assert!(
			lines.iter().any(|line| line.contains("request rejected")),
			"nothing on the refusal path was captured: {lines:#?}"
		);
		let secrets = [
			browser.cookie.split_once('=').unwrap().1.to_owned(),
			cookie_pair(&set_cookie.unwrap()).split_once('=').unwrap().1.to_owned(),
			browser.credential_id.clone(),
			URL_SAFE_NO_PAD.encode(&browser.user_handle),
		];
		for line in &lines {
			for secret in &secrets {
				assert!(!line.contains(secret.as_str()), "a secret reached a log line: {line}");
			}
		}
	}

	/// The filter above is doing real work: without it, the library's own
	/// debug span records the credential.
	#[tokio::test]
	async fn without_the_filter_the_library_would_log_the_credential() {
		let (captured, _guard) = crate::privacy::capture_where(|metadata| !metadata.target().starts_with("webauthn_authenticator_rs"));
		let app = app(AuthContext::for_tests(pool().await, 100));
		let browser = register(&app, &mut authenticator()).await;
		assert!(
			captured.lines().iter().any(|line| line.contains(browser.credential_id.as_str())),
			"if this fails, the library stopped logging credentials and `auth::loggable` may no longer be needed"
		);
	}

	#[tokio::test]
	async fn account_creation_stops_at_the_daily_cap() {
		let app = app(AuthContext::for_tests(pool().await, 1));
		register(&app, &mut authenticator()).await;
		let (status, _, _) = call(&app, Method::POST, "/auth/register/start", None, None).await;
		assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
	}

	#[tokio::test]
	async fn unconfigured_auth_refuses_every_ceremony_with_503() {
		let app = app(AuthContext::unconfigured_for_tests(pool().await));
		for path in ["/auth/register/start", "/auth/sign-in/start"] {
			let (status, _, body) = call(&app, Method::POST, path, None, None).await;
			assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
			assert_eq!(body["error"]["code"], "feature_not_configured");
		}
	}
}
