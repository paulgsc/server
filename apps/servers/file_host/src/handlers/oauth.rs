//! OAuth endpoints.
//!
//! Metadata, client registration, the approval hand-off from the app, the
//! token endpoint, and the subject's list of connected services. The rules are
//! `auth::oauth`'s; docs/identity.md, "AI services acting for a subject", says
//! why each step happens where it does.
//!
//! Errors follow OAuth (RFC 6749 §5.2): `{ "error", "error_description" }`,
//! never this server's usual envelope, since an AI service's client library is
//! what reads them. Nothing here logs a token, a code or a verifier.

use crate::auth::{
	now,
	oauth::{
		decode_id, encode_id, hash_token, mint_id, mint_token, redirect_host, redirect_uri_allowed, verifier_matches, well_formed_challenge, with_query, Code, OAuthFlows,
		Pending, Scopes, ACCESS_TOKEN_TTL_SECONDS, MAX_CLIENT_NAME_LENGTH, MAX_REDIRECT_URIS,
	},
	AuthContext,
};
use crate::{subject::SubjectId, FileHostError};
use auth_repo::{ClientRow, CreateGrant, NewAccessToken, NewGrant, OAuthRepository, Refresh};
use axum::{
	extract::{Path, State},
	http::{header, StatusCode},
	response::{IntoResponse, Response},
	Form, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

/// How long a `state` the client sends may be. It comes back untouched.
const MAX_STATE_LENGTH: usize = 1_024;

/// An OAuth error response, never cached.
fn oauth_error(status: StatusCode, error: &'static str, description: &'static str) -> Response {
	(
		status,
		[(header::CACHE_CONTROL, "no-store")],
		Json(json!({ "error": error, "error_description": description })),
	)
		.into_response()
}

fn repository(auth: &AuthContext) -> OAuthRepository {
	OAuthRepository::new(auth.pool().clone())
}

/// `GET /.well-known/oauth-authorization-server` (RFC 8414). `404` while
/// OAuth is off, which is what a client probing for it expects.
pub async fn metadata(State(auth): State<AuthContext>) -> Response {
	auth
		.oauth()
		.map_or_else(|_| StatusCode::NOT_FOUND.into_response(), |flows| Json(flows.settings.metadata()).into_response())
}

/// What a client sends to register (RFC 7591). Anything else it sends is
/// ignored.
#[derive(Debug, Deserialize)]
pub struct Registration {
	#[serde(default)]
	redirect_uris: Vec<String>,
	client_name: Option<String>,
	token_endpoint_auth_method: Option<String>,
	grant_types: Option<Vec<String>>,
	response_types: Option<Vec<String>>,
}

/// `POST /oauth/register`: anyone may register a public client. What it
/// registers is bounded ([`MAX_REDIRECT_URIS`], `auth_repo::MAX_CLIENTS`) and
/// grants nothing until a subject approves it.
///
/// # Errors
/// `503` while OAuth is off, and a database failure. A refused registration is
/// an OAuth error response, not an `Err`.
pub async fn register(State(auth): State<AuthContext>, Json(request): Json<Registration>) -> Result<Response, FileHostError> {
	auth.oauth()?;
	let uris = request.redirect_uris;
	if uris.is_empty() || uris.len() > MAX_REDIRECT_URIS || !uris.iter().all(|uri| redirect_uri_allowed(uri)) {
		return Ok(oauth_error(
			StatusCode::BAD_REQUEST,
			"invalid_redirect_uri",
			"redirect URIs must be https, or http on a loopback address",
		));
	}
	if request.token_endpoint_auth_method.as_deref().is_some_and(|method| method != "none") {
		return Ok(oauth_error(
			StatusCode::BAD_REQUEST,
			"invalid_client_metadata",
			"only public clients (token_endpoint_auth_method none) are supported",
		));
	}
	let known_grant = |grant: &String| grant == "authorization_code" || grant == "refresh_token";
	if !request.grant_types.as_ref().is_none_or(|grants| grants.iter().all(known_grant))
		|| !request.response_types.as_ref().is_none_or(|types| types.iter().all(|kind| kind == "code"))
	{
		return Ok(oauth_error(
			StatusCode::BAD_REQUEST,
			"invalid_client_metadata",
			"only the authorization code and refresh token grants are supported",
		));
	}
	let name = request
		.client_name
		.map(|name| name.trim().chars().filter(|c| !c.is_control()).take(MAX_CLIENT_NAME_LENGTH).collect::<String>())
		.filter(|name| !name.is_empty())
		.unwrap_or_else(|| uris.first().map(|uri| redirect_host(uri)).unwrap_or_default());

	let client = ClientRow {
		client_id: mint_id("client-"),
		client_name: name,
		redirect_uris: json!(uris).to_string(),
	};
	if !repository(&auth).register_client(&client).await? {
		return Ok(oauth_error(
			StatusCode::SERVICE_UNAVAILABLE,
			"temporarily_unavailable",
			"too many clients are registered; try again later",
		));
	}
	Ok(
		(
			StatusCode::CREATED,
			[(header::CACHE_CONTROL, "no-store")],
			Json(json!({
				"client_id": client.client_id,
				"client_name": client.client_name,
				"redirect_uris": uris,
				"token_endpoint_auth_method": "none",
				"grant_types": ["authorization_code", "refresh_token"],
				"response_types": ["code"],
			})),
		)
			.into_response(),
	)
}

/// An authorization request as the client put it in the approval page's URL;
/// the page sends the parameters on unchanged.
#[derive(Debug, Deserialize)]
pub struct AuthorizationRequest {
	response_type: Option<String>,
	client_id: Option<String>,
	redirect_uri: Option<String>,
	code_challenge: Option<String>,
	code_challenge_method: Option<String>,
	scope: Option<String>,
	state: Option<String>,
	resource: Option<String>,
}

/// The answer that sends the browser back to the client with an error.
///
/// Only once the redirect URI is known to be the client's own (RFC 6749
/// §4.1.2.1).
fn refuse(flows: &OAuthFlows, redirect_uri: &str, state: Option<&str>, error: &'static str) -> Response {
	let mut params = vec![("error", error)];
	if let Some(state) = state {
		params.push(("state", state));
	}
	params.push(("iss", flows.settings.issuer.as_str()));
	(StatusCode::BAD_REQUEST, Json(json!({ "error": error, "redirectTo": with_query(redirect_uri, &params) }))).into_response()
}

/// `POST /oauth/authorize/requests`: the app's approval page hands in a request.
///
/// It is checked here, held for the subject's answer, and described back so
/// the page can show who is asking, where the answer goes and what it allows.
///
/// # Errors
/// `503` while OAuth is off, and a database failure. A refused request is an
/// OAuth error response, not an `Err`.
pub async fn start_authorization(State(auth): State<AuthContext>, Json(request): Json<AuthorizationRequest>) -> Result<Response, FileHostError> {
	let flows = auth.oauth()?;
	let Some(client) = repository(&auth).client(request.client_id.as_deref().unwrap_or_default()).await? else {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_client", "this client is not registered"));
	};
	let registered: Vec<String> = serde_json::from_str(&client.redirect_uris)?;
	let Some(redirect_uri) = request.redirect_uri.filter(|uri| registered.contains(uri)) else {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "redirect_uri is not one this client registered"));
	};
	let state = request.state.filter(|state| !state.is_empty());
	if state.as_ref().is_some_and(|state| state.len() > MAX_STATE_LENGTH) {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "state is too long"));
	}
	let refused = |error| Ok(refuse(flows, &redirect_uri, state.as_deref(), error));

	if request.response_type.as_deref() != Some("code") {
		return refused("unsupported_response_type");
	}
	let Some(code_challenge) = request
		.code_challenge
		.filter(|challenge| request.code_challenge_method.as_deref() == Some("S256") && well_formed_challenge(challenge))
	else {
		return refused("invalid_request");
	};
	let scopes = match request.scope.as_deref().map(str::trim).filter(|scope| !scope.is_empty()) {
		None => Scopes::all(),
		Some(scope) => match Scopes::parse(scope) {
			Some(scopes) => scopes,
			None => return refused("invalid_scope"),
		},
	};
	let resource = request.resource.unwrap_or_else(|| flows.settings.resource.clone());
	if resource != flows.settings.resource {
		return refused("invalid_target");
	}

	let words = scopes.words();
	let pending = Pending {
		client_id: client.client_id,
		redirect_uri: redirect_uri.clone(),
		state,
		code_challenge,
		scopes,
		resource,
	};
	let Some(id) = flows.pending.open(pending, Instant::now()) else {
		return Ok(oauth_error(
			StatusCode::SERVICE_UNAVAILABLE,
			"temporarily_unavailable",
			"too many approvals are open; try again shortly",
		));
	};
	Ok(
		Json(json!({
			"request": encode_id(&id),
			"clientName": client.client_name,
			"redirectHost": redirect_host(&redirect_uri),
			"scopes": words,
		}))
		.into_response(),
	)
}

/// `POST /oauth/authorize/requests/:request/approve`: the subject says yes.
///
/// Answers with where to send the browser: the client's redirect URI with a
/// one-time code, the request's `state`, and `iss` (RFC 9207).
///
/// # Errors
/// `401` without a session, `404` for a request that is unknown, expired or
/// already answered, and `503` while OAuth is off or too many codes are open.
pub async fn approve(State(auth): State<AuthContext>, subject: SubjectId, Path(request): Path<String>) -> Result<Json<Value>, FileHostError> {
	let flows = auth.oauth()?;
	let now = Instant::now();
	let pending = decode_id(&request).and_then(|id| flows.pending.take(&id, now)).ok_or(FileHostError::NotFound)?;
	let code = Code {
		subject: subject.as_str().to_owned(),
		client_id: pending.client_id,
		redirect_uri: pending.redirect_uri.clone(),
		challenge: pending.code_challenge,
		scopes: pending.scopes,
		resource: pending.resource,
	};
	let id = flows.codes.open(code, now).ok_or(FileHostError::ServiceOverloaded)?;
	let code = encode_id(&id);
	let mut params = vec![("code", code.as_str())];
	if let Some(state) = pending.state.as_deref() {
		params.push(("state", state));
	}
	params.push(("iss", flows.settings.issuer.as_str()));
	Ok(Json(json!({ "redirectTo": with_query(&pending.redirect_uri, &params) })))
}

/// `POST /oauth/authorize/requests/:request/deny`: the subject says no.
///
/// Needs no session: declining grants nothing.
///
/// # Errors
/// `404` for a request that is unknown, expired or already answered, and
/// `503` while OAuth is off.
pub async fn deny(State(auth): State<AuthContext>, Path(request): Path<String>) -> Result<Json<Value>, FileHostError> {
	let flows = auth.oauth()?;
	let pending = decode_id(&request).and_then(|id| flows.pending.take(&id, Instant::now())).ok_or(FileHostError::NotFound)?;
	let mut params = vec![("error", "access_denied")];
	if let Some(state) = pending.state.as_deref() {
		params.push(("state", state));
	}
	params.push(("iss", flows.settings.issuer.as_str()));
	Ok(Json(json!({ "redirectTo": with_query(&pending.redirect_uri, &params) })))
}

/// A token request (RFC 6749 §4.1.3 and §6), form-encoded.
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
	grant_type: Option<String>,
	code: Option<String>,
	redirect_uri: Option<String>,
	client_id: Option<String>,
	code_verifier: Option<String>,
	refresh_token: Option<String>,
	resource: Option<String>,
}

fn tokens(access: &str, refresh: &str, scope: &str) -> Response {
	(
		[(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
		Json(json!({
			"access_token": access,
			"token_type": "Bearer",
			"expires_in": ACCESS_TOKEN_TTL_SECONDS,
			"refresh_token": refresh,
			"scope": scope,
		})),
	)
		.into_response()
}

/// `POST /oauth/token`: a code, or a refresh token, for a new pair of tokens.
///
/// # Errors
/// `503` while OAuth is off, and a database failure. A refused grant is an
/// OAuth error response, not an `Err`.
///
/// Both grants write subject-scoped rows with no session behind them, so both
/// hold the account-deletion lock across their writes (docs/identity.md
/// invariant 9); the repository checks the account, or the grant, in the same
/// transaction.
pub async fn token(State(auth): State<AuthContext>, Form(request): Form<TokenRequest>) -> Result<Response, FileHostError> {
	let flows = auth.oauth()?;
	match request.grant_type.as_deref() {
		Some("authorization_code") => exchange_code(&auth, flows, request).await,
		Some("refresh_token") => refresh(&auth, flows, request).await,
		_ => Ok(oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "use authorization_code or refresh_token")),
	}
}

async fn exchange_code(auth: &AuthContext, flows: &OAuthFlows, request: TokenRequest) -> Result<Response, FileHostError> {
	let invalid = || {
		Ok(oauth_error(
			StatusCode::BAD_REQUEST,
			"invalid_grant",
			"the code is unknown, used, expired, or was issued for another request",
		))
	};
	// Taken first, so a wrong guess at the verifier also uses the code up.
	let Some(code) = request.code.as_deref().and_then(decode_id).and_then(|id| flows.codes.take(&id, Instant::now())) else {
		return invalid();
	};
	if request.client_id.as_deref() != Some(code.client_id.as_str())
		|| request.redirect_uri.as_deref() != Some(code.redirect_uri.as_str())
		|| !request.code_verifier.as_deref().is_some_and(|verifier| verifier_matches(verifier, &code.challenge))
	{
		return invalid();
	}
	if request.resource.as_deref().is_some_and(|resource| resource != code.resource) {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_target", "the resource differs from the one approved"));
	}

	let _hold = auth.hold_against_deletion().await;
	let now = now();
	let (access, access_hash) = mint_token("fha_");
	let (refresh, refresh_hash) = mint_token("fhr_");
	let scope = code.scopes.to_text();
	let grant = NewGrant {
		grant_id: &mint_id("grant-"),
		subject_id: &code.subject,
		client_id: &code.client_id,
		scope: &scope,
		resource: &code.resource,
		refresh_hash: &refresh_hash,
		refresh_expires_at: now + auth.session_ttl_seconds(),
	};
	let access_row = NewAccessToken {
		token_hash: &access_hash,
		expires_at: now + ACCESS_TOKEN_TTL_SECONDS,
	};
	match repository(auth).create_grant(&grant, &access_row, now).await? {
		CreateGrant::Created => Ok(tokens(access.expose(), refresh.expose(), &scope)),
		CreateGrant::NoAccount => invalid(),
	}
}

async fn refresh(auth: &AuthContext, flows: &OAuthFlows, request: TokenRequest) -> Result<Response, FileHostError> {
	let (Some(presented), Some(client_id)) = (request.refresh_token.as_deref(), request.client_id.as_deref()) else {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "refresh_token and client_id are required"));
	};
	if request.resource.as_deref().is_some_and(|resource| resource != flows.settings.resource) {
		return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_target", "tokens here are only for the MCP endpoint"));
	}

	let _hold = auth.hold_against_deletion().await;
	let now = now();
	let (access, access_hash) = mint_token("fha_");
	let (next, next_hash) = mint_token("fhr_");
	let access_row = NewAccessToken {
		token_hash: &access_hash,
		expires_at: now + ACCESS_TOKEN_TTL_SECONDS,
	};
	let outcome = repository(auth)
		.refresh(&hash_token(presented), client_id, &next_hash, now + auth.session_ttl_seconds(), &access_row, now)
		.await?;
	match outcome {
		Refresh::Rotated { scope } => Ok(tokens(access.expose(), next.expose(), &scope)),
		Refresh::Reused | Refresh::Unknown => Ok(oauth_error(
			StatusCode::BAD_REQUEST,
			"invalid_grant",
			"the refresh token is unknown, expired or already used",
		)),
	}
}

/// `GET /oauth/grants`: the services this subject has connected.
///
/// # Errors
/// `401` without a session, and a database failure.
pub async fn grants(State(auth): State<AuthContext>, subject: SubjectId) -> Result<Json<Value>, FileHostError> {
	let grants = repository(&auth).grants(subject.as_str(), now()).await?;
	let grants: Vec<Value> = grants
		.into_iter()
		.map(|grant| json!({ "id": grant.grant_id, "clientName": grant.client_name, "scopes": grant.scope.split_whitespace().collect::<Vec<_>>() }))
		.collect();
	Ok(Json(json!({ "grants": grants })))
}

/// `DELETE /oauth/grants/:grant`: disconnect a service, ending its tokens.
///
/// `204` whether or not this subject held that grant, so another subject's
/// grant ids are not observable.
///
/// # Errors
/// `401` without a session, and a database failure.
pub async fn delete_grant(State(auth): State<AuthContext>, subject: SubjectId, Path(grant): Path<String>) -> Result<StatusCode, FileHostError> {
	repository(&auth).delete_grant(subject.as_str(), &grant).await?;
	Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
	//! The whole flow over the real routes: a client registers, the app hands
	//! in its request, a signed-in subject approves, the client trades the
	//! code with its PKCE verifier, and the token it gets works where
	//! `Delegated` is asked for and nowhere else.

	use crate::auth::{cookie::SessionToken, oauth::Scope, AuthContext};
	use crate::subject::{Delegated, SubjectId};
	use auth_repo::AuthRepository;
	use axum::{
		body::Body,
		http::{header, Method, Request, StatusCode},
		routing::get,
		Router,
	};
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
	use serde_json::{json, Value};
	use sha2::{Digest, Sha256};
	use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
	use tower::ServiceExt;
	use webauthn_rs::prelude::Url;

	static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../../migrations");

	const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
	const RESOURCE: &str = "https://lessons.test/api/v1/mcp";

	async fn pool() -> SqlitePool {
		let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
		MIGRATOR.run(&pool).await.unwrap();
		pool
	}

	/// The OAuth routes, plus two probes: one that takes a `Delegated` and
	/// one that takes a `SubjectId`.
	fn app(auth: AuthContext) -> Router {
		// Throwaway probes, not served routes, so they are not on a RouteTable.
		#[allow(clippy::disallowed_methods)]
		let probes = Router::new()
			.route(
				"/delegated",
				get(|delegated: Delegated| async move { delegated.subject().as_str().to_owned() + " shelf=" + &delegated.allows(Scope::Shelf).to_string() }),
			)
			.route("/whoami", get(|subject: SubjectId| async move { subject.as_str().to_owned() }));
		crate::routes::oauth::oauth_metadata::<AuthContext>()
			.into_table_router()
			.merge(crate::routes::oauth::oauth::<AuthContext>().into_table_router())
			.merge(crate::routes::oauth::oauth_approval::<AuthContext>().into_table_router())
			.merge(probes)
			.with_state(auth)
	}

	/// An account, signed in: the `Cookie` header its browser sends.
	async fn signed_in(pool: &SqlitePool, subject: &str) -> String {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(subject)
			.execute(pool)
			.await
			.unwrap();
		let token = SessionToken::mint();
		AuthRepository::new(pool.clone()).create_session(&token.hash(), subject, i64::MAX, 0).await.unwrap();
		let set_cookie = token.set_cookie(3_600);
		set_cookie.to_str().unwrap().split(';').next().unwrap().to_owned()
	}

	async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
		let response = app.clone().oneshot(request).await.unwrap();
		let status = response.status();
		let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
		(
			status,
			serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
		)
	}

	fn post_json(uri: &str, body: &Value, cookie: Option<&str>) -> Request<Body> {
		let mut request = Request::builder()
			.method(Method::POST)
			.uri(uri)
			.header(header::CONTENT_TYPE, "application/json")
			.header("sec-fetch-site", "same-origin");
		if let Some(cookie) = cookie {
			request = request.header(header::COOKIE, cookie);
		}
		request.body(Body::from(body.to_string())).unwrap()
	}

	fn post_form(uri: &str, fields: &[(&str, &str)]) -> Request<Body> {
		let body = fields
			.iter()
			.map(|(name, value)| String::from(*name) + "=" + &url_encode(value))
			.collect::<Vec<_>>()
			.join("&");
		Request::builder()
			.method(Method::POST)
			.uri(uri)
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
			.body(Body::from(body))
			.unwrap()
	}

	fn url_encode(value: &str) -> String {
		let mut url = Url::parse("https://x.test/").unwrap();
		url.query_pairs_mut().append_pair("v", value);
		url.query().unwrap().trim_start_matches("v=").to_owned()
	}

	fn get_with(uri: &str, header_name: header::HeaderName, value: &str) -> Request<Body> {
		Request::builder().uri(uri).header(header_name, value).body(Body::empty()).unwrap()
	}

	fn query_param(url: &str, name: &str) -> Option<String> {
		Url::parse(url).unwrap().query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned())
	}

	const VERIFIER: &str = "a-verifier-that-is-at-least-forty-three-characters-long";

	fn challenge() -> String {
		URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
	}

	async fn register(app: &Router) -> String {
		let (status, body) = send(app, post_json("/oauth/register", &json!({ "client_name": "Claude", "redirect_uris": [CALLBACK] }), None)).await;
		assert_eq!(status, StatusCode::CREATED, "{body}");
		body["client_id"].as_str().unwrap().to_owned()
	}

	fn authorization(client_id: &str) -> Value {
		json!({
			"response_type": "code",
			"client_id": client_id,
			"redirect_uri": CALLBACK,
			"code_challenge": challenge(),
			"code_challenge_method": "S256",
			"scope": "lessons:read shelf",
			"state": "xyz",
			"resource": RESOURCE,
		})
	}

	/// Register, hand in, approve: the code the client's callback receives.
	async fn approved_code(app: &Router, cookie: &str) -> (String, String) {
		let client_id = register(app).await;
		let (status, request) = send(app, post_json("/oauth/authorize/requests", &authorization(&client_id), None)).await;
		assert_eq!(status, StatusCode::OK, "{request}");
		assert_eq!(request["clientName"], "Claude");
		assert_eq!(request["redirectHost"], "claude.ai");
		assert_eq!(request["scopes"], json!(["lessons:read", "shelf"]));

		let uri = String::from("/oauth/authorize/requests/") + request["request"].as_str().unwrap() + "/approve";
		let (status, answer) = send(app, post_json(&uri, &json!({}), Some(cookie))).await;
		assert_eq!(status, StatusCode::OK, "{answer}");
		let redirect = answer["redirectTo"].as_str().unwrap();
		assert!(redirect.starts_with(CALLBACK));
		assert_eq!(query_param(redirect, "state").as_deref(), Some("xyz"));
		assert_eq!(query_param(redirect, "iss").as_deref(), Some("https://lessons.test"));
		(client_id, query_param(redirect, "code").unwrap())
	}

	fn exchange<'a>(client_id: &'a str, code: &'a str, verifier: &'a str) -> Vec<(&'a str, &'a str)> {
		vec![
			("grant_type", "authorization_code"),
			("code", code),
			("redirect_uri", CALLBACK),
			("client_id", client_id),
			("code_verifier", verifier),
			("resource", RESOURCE),
		]
	}

	#[tokio::test]
	async fn an_approved_service_gets_a_token_that_works_only_where_delegation_is_asked_for() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let cookie = signed_in(&pool, "subject-alice").await;
		let (client_id, code) = approved_code(&app, &cookie).await;

		let (status, tokens) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, VERIFIER))).await;
		assert_eq!(status, StatusCode::OK, "{tokens}");
		assert_eq!(tokens["token_type"], "Bearer");
		assert_eq!(tokens["scope"], "lessons:read shelf");
		let access = tokens["access_token"].as_str().unwrap();
		let bearer = String::from("Bearer ") + access;

		let (status, body) = send(&app, get_with("/delegated", header::AUTHORIZATION, &bearer)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body, "subject-alice shelf=true");
		let (status, _) = send(&app, get_with("/delegated", header::AUTHORIZATION, &(String::from("bearer ") + access))).await;
		assert_eq!(status, StatusCode::OK, "the scheme is case-insensitive");

		// A token is not a session: every route that takes a `SubjectId`
		// refuses it, so it can never reach sign-out or account deletion.
		let (status, _) = send(&app, get_with("/whoami", header::AUTHORIZATION, &bearer)).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		let (status, _) = send(&app, get_with("/oauth/grants", header::AUTHORIZATION, &bearer)).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);

		// The code was single use.
		let (status, body) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, VERIFIER))).await;
		assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
	}

	#[tokio::test]
	async fn a_refresh_token_rotates_and_a_replayed_one_disconnects_the_service() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let cookie = signed_in(&pool, "subject-alice").await;
		let (client_id, code) = approved_code(&app, &cookie).await;
		let (_, first) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, VERIFIER))).await;
		let first_refresh = first["refresh_token"].as_str().unwrap();

		let refresh = |token: &str| post_form("/oauth/token", &[("grant_type", "refresh_token"), ("refresh_token", token), ("client_id", &client_id)]);
		let (status, second) = send(&app, refresh(first_refresh)).await;
		assert_eq!(status, StatusCode::OK, "{second}");
		assert_ne!(second["refresh_token"], first["refresh_token"]);
		let (status, _) = send(
			&app,
			get_with("/delegated", header::AUTHORIZATION, &(String::from("Bearer ") + first["access_token"].as_str().unwrap())),
		)
		.await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "the old access token ended");

		let (status, connected) = send(&app, get_with("/oauth/grants", header::COOKIE, &cookie)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(connected["grants"].as_array().unwrap().len(), 1);
		assert_eq!(connected["grants"][0]["clientName"], "Claude");

		// Only a copy could still hold the replaced token.
		let (status, body) = send(&app, refresh(first_refresh)).await;
		assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
		let (status, body) = send(&app, refresh(second["refresh_token"].as_str().unwrap())).await;
		assert_eq!(
			(status, body["error"].as_str()),
			(StatusCode::BAD_REQUEST, Some("invalid_grant")),
			"the replay ended the grant"
		);
		let (_, connected) = send(&app, get_with("/oauth/grants", header::COOKIE, &cookie)).await;
		assert!(connected["grants"].as_array().unwrap().is_empty());
	}

	#[tokio::test]
	async fn a_wrong_verifier_gets_nothing_and_uses_the_code_up() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let cookie = signed_in(&pool, "subject-alice").await;
		let (client_id, code) = approved_code(&app, &cookie).await;

		let wrong = "another-verifier-that-is-at-least-forty-three-characters";
		let (status, body) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, wrong))).await;
		assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
		let (status, _) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, VERIFIER))).await;
		assert_eq!(status, StatusCode::BAD_REQUEST, "a guess used the code up");
	}

	#[tokio::test]
	async fn a_code_exchanged_after_its_account_is_deleted_writes_nothing() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let cookie = signed_in(&pool, "subject-alice").await;
		let (client_id, code) = approved_code(&app, &cookie).await;
		sqlx::query("DELETE FROM account WHERE subject_id = 'subject-alice'").execute(&pool).await.unwrap();

		let (status, body) = send(&app, post_form("/oauth/token", &exchange(&client_id, &code, VERIFIER))).await;
		assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
		let rows: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM oauth_grant) + (SELECT COUNT(*) FROM oauth_access_token)")
			.fetch_one(&pool)
			.await
			.unwrap();
		assert_eq!(rows, 0);
	}

	#[tokio::test]
	async fn a_bad_request_goes_back_to_the_client_only_once_its_redirect_is_known_to_be_its_own() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let client_id = register(&app).await;

		let mut stranger = authorization(&client_id);
		stranger["redirect_uri"] = json!("https://evil.test/cb");
		let (status, body) = send(&app, post_json("/oauth/authorize/requests", &stranger, None)).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(body["error"], "invalid_request");
		assert!(body.get("redirectTo").is_none(), "never send anyone to an unregistered address");

		let mut greedy = authorization(&client_id);
		greedy["scope"] = json!("shelf admin");
		let (status, body) = send(&app, post_json("/oauth/authorize/requests", &greedy, None)).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		let redirect = body["redirectTo"].as_str().unwrap();
		assert_eq!(query_param(redirect, "error").as_deref(), Some("invalid_scope"));
		assert_eq!(query_param(redirect, "state").as_deref(), Some("xyz"));

		let mut elsewhere = authorization(&client_id);
		elsewhere["resource"] = json!("https://other.test/mcp");
		let (_, body) = send(&app, post_json("/oauth/authorize/requests", &elsewhere, None)).await;
		assert_eq!(query_param(body["redirectTo"].as_str().unwrap(), "error").as_deref(), Some("invalid_target"));

		let mut plain = authorization(&client_id);
		plain["code_challenge_method"] = json!("plain");
		let (_, body) = send(&app, post_json("/oauth/authorize/requests", &plain, None)).await;
		assert_eq!(query_param(body["redirectTo"].as_str().unwrap(), "error").as_deref(), Some("invalid_request"));
	}

	#[tokio::test]
	async fn approving_needs_a_signed_in_subject_and_declining_does_not() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool.clone()));
		let client_id = register(&app).await;
		let (_, request) = send(&app, post_json("/oauth/authorize/requests", &authorization(&client_id), None)).await;
		let id = request["request"].as_str().unwrap();

		let (status, _) = send(&app, post_json(&(String::from("/oauth/authorize/requests/") + id + "/approve"), &json!({}), None)).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);

		let (status, answer) = send(&app, post_json(&(String::from("/oauth/authorize/requests/") + id + "/deny"), &json!({}), None)).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(query_param(answer["redirectTo"].as_str().unwrap(), "error").as_deref(), Some("access_denied"));
		let (status, _) = send(&app, post_json(&(String::from("/oauth/authorize/requests/") + id + "/deny"), &json!({}), None)).await;
		assert_eq!(status, StatusCode::NOT_FOUND, "a request is answered once");
	}

	#[tokio::test]
	async fn registration_takes_only_safe_redirects_and_public_clients() {
		let pool = pool().await;
		let app = app(AuthContext::for_tests_with_oauth(pool));
		for body in [
			json!({ "redirect_uris": [] }),
			json!({ "redirect_uris": ["http://evil.test/cb"] }),
			json!({ "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "client_secret_basic" }),
			json!({ "redirect_uris": [CALLBACK], "grant_types": ["client_credentials"] }),
		] {
			let (status, answer) = send(&app, post_json("/oauth/register", &body, None)).await;
			assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {answer}");
		}
		let (status, answer) = send(&app, post_json("/oauth/register", &json!({ "redirect_uris": ["http://127.0.0.1:33418/callback"] }), None)).await;
		assert_eq!(status, StatusCode::CREATED);
		assert_eq!(answer["client_name"], "127.0.0.1", "a nameless client is shown by where it redirects");
	}

	#[tokio::test]
	async fn metadata_says_how_to_connect_and_is_absent_while_oauth_is_off() {
		let (status, metadata) = send(
			&app(AuthContext::for_tests_with_oauth(pool().await)),
			Request::builder().uri("/.well-known/oauth-authorization-server").body(Body::empty()).unwrap(),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(metadata["issuer"], "https://lessons.test");
		assert_eq!(metadata["authorization_endpoint"], "https://app.test/connect");
		assert_eq!(metadata["token_endpoint"], "https://lessons.test/api/v1/oauth/token");
		assert_eq!(metadata["code_challenge_methods_supported"], json!(["S256"]));
		assert_eq!(metadata["authorization_response_iss_parameter_supported"], true);

		let (status, _) = send(
			&app(AuthContext::for_tests(pool().await, 100)),
			Request::builder().uri("/.well-known/oauth-authorization-server").body(Body::empty()).unwrap(),
		)
		.await;
		assert_eq!(status, StatusCode::NOT_FOUND);
	}

	#[tokio::test]
	async fn a_token_issued_for_another_resource_is_refused() {
		let pool = pool().await;
		let auth = AuthContext::for_tests_with_oauth(pool.clone());
		let app = app(auth);
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('subject-alice', 'h')")
			.execute(&pool)
			.await
			.unwrap();
		let token = "fha_elsewhere";
		let hash = crate::auth::oauth::hash_token(token).to_vec();
		sqlx::query("INSERT INTO oauth_grant (grant_id, subject_id, client_id, scope, resource, refresh_hash, expires_at) VALUES ('g', 'subject-alice', 'c', 'shelf', 'https://other.test/mcp', x'00', 9999999999)")
			.execute(&pool)
			.await
			.unwrap();
		sqlx::query("INSERT INTO oauth_access_token (token_hash, grant_id, subject_id, scope, expires_at) VALUES (?1, 'g', 'subject-alice', 'shelf', 9999999999)")
			.bind(hash)
			.execute(&pool)
			.await
			.unwrap();
		let (status, _) = send(&app, get_with("/delegated", header::AUTHORIZATION, &(String::from("Bearer ") + token))).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
	}
}
