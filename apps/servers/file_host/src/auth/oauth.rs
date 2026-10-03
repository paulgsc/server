//! OAuth 2.1 for AI services acting for a subject.
//!
//! `file_host` is its own authorization server, and the MCP endpoint is the
//! resource it issues tokens for (docs/identity.md, "AI services acting for a
//! subject").
//!
//! In one line each:
//!
//! - **Off unless configured.** `OAUTH_ISSUER`, `OAUTH_AUTHORIZE_URL` and
//!   `OAUTH_RESOURCE` together turn it on; any one without the others is a
//!   startup error, as is any of them with passkey auth off.
//! - **The subject approves in the app.** The authorize step is the app's page
//!   (passkeys are bound to the app's origin); it hands the request here and
//!   approves it with the subject's own session.
//! - **PKCE `S256` always**, and redirect URIs match exactly what the client
//!   registered.
//! - **Tokens are random and stored hashed**, like sessions. A code and a
//!   pending request live only in memory ([`OneTimeStore`]).
//! - **A refresh token is replaced on every use**, and presenting a replaced
//!   one ends the grant (`auth_repo::Refresh::Reused`).
//! - **A token does only what was approved** ([`Scopes`]), only for
//!   [`OAuthSettings::resource`], and only through `subject::Delegated`. No
//!   route that takes `SubjectId` accepts one.

use super::ceremony::{OneTimeId, OneTimeStore};
use crate::redacted::Redacted;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use webauthn_rs::prelude::Url;

/// How long an access token lasts. Clients refresh before or on expiry.
pub const ACCESS_TOKEN_TTL_SECONDS: i64 = 3_600;

/// How long the app's approval page has between handing a request in and the
/// subject approving it: time to sign in with a passkey and read the screen.
pub const PENDING_TTL: Duration = Duration::from_secs(600);

/// How long an authorization code may wait to be exchanged. The client's
/// servers exchange it the moment the browser lands on their callback.
pub const CODE_TTL: Duration = Duration::from_secs(60);

/// How many pending approvals, and separately codes, may be open at once.
pub const MAX_OPEN: usize = 10_000;

/// How many redirect URIs a client may register, and how long each may be.
pub const MAX_REDIRECT_URIS: usize = 8;
pub const MAX_REDIRECT_URI_LENGTH: usize = 512;

/// How long a client's self-chosen name may be.
pub const MAX_CLIENT_NAME_LENGTH: usize = 100;

/// One thing a token may do. Each maps to tools on the MCP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
	/// Read the lessons everyone is served.
	LessonsRead,
	/// Read the subject's own study outcomes.
	ProgressRead,
	/// Read and keep items on the subject's own shelf.
	Shelf,
}

impl Scope {
	pub const ALL: [Self; 3] = [Self::LessonsRead, Self::ProgressRead, Self::Shelf];

	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::LessonsRead => "lessons:read",
			Self::ProgressRead => "progress:read",
			Self::Shelf => "shelf",
		}
	}

	fn parse(word: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|scope| scope.as_str() == word)
	}
}

/// A set of [`Scope`]s, written as OAuth writes them: space-separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scopes(Vec<Scope>);

impl Scopes {
	/// Every scope: what a request that names none is given.
	#[must_use]
	pub fn all() -> Self {
		Self(Scope::ALL.to_vec())
	}

	/// Parse a space-separated list. An unknown word, or an empty list, is
	/// `None`: a client gets exactly what it asked for or nothing.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		let mut scopes = text.split_whitespace().map(Scope::parse).collect::<Option<Vec<_>>>()?;
		scopes.sort_unstable();
		scopes.dedup();
		(!scopes.is_empty()).then_some(Self(scopes))
	}

	#[must_use]
	pub fn contains(&self, scope: Scope) -> bool {
		self.0.contains(&scope)
	}

	#[must_use]
	pub fn words(&self) -> Vec<&'static str> {
		self.0.iter().map(|scope| scope.as_str()).collect()
	}

	#[must_use]
	pub fn to_text(&self) -> String {
		self.words().join(" ")
	}
}

/// What `OAUTH_ISSUER`, `OAUTH_AUTHORIZE_URL` and `OAUTH_RESOURCE` configure.
#[derive(Debug, Clone)]
pub struct OAuthSettings {
	/// The public origin the AI services reach, e.g. `https://lessons.example.com`.
	pub issuer: String,
	/// The app's approval page, on the origin passkeys belong to.
	pub authorize_url: String,
	/// The MCP endpoint's URL: the one audience tokens are issued for.
	pub resource: String,
}

/// Why OAuth settings were refused at startup.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OAuthConfigError {
	#[error("set all of OAUTH_ISSUER, OAUTH_AUTHORIZE_URL and OAUTH_RESOURCE, or none of them")]
	Partial,
	#[error("OAuth needs passkey sign-in: AUTH_ENABLED=false leaves nobody to approve a service")]
	AuthOff,
	#[error("{0} must be an https URL")]
	NotHttps(&'static str),
	#[error("OAUTH_ISSUER must be an origin, with no path, query or fragment")]
	IssuerHasPath,
}

impl OAuthSettings {
	/// Read the three settings. `Ok(None)` when none is set.
	///
	/// # Errors
	/// When only some are set, when passkey auth is off, when any is not an
	/// https URL, or when the issuer carries a path.
	pub fn from_parts(auth_enabled: bool, issuer: Option<&str>, authorize_url: Option<&str>, resource: Option<&str>) -> Result<Option<Self>, OAuthConfigError> {
		match (set(issuer), set(authorize_url), set(resource)) {
			(None, None, None) => Ok(None),
			(Some(issuer), Some(authorize_url), Some(resource)) => {
				if !auth_enabled {
					return Err(OAuthConfigError::AuthOff);
				}
				let issuer = issuer.trim_end_matches('/');
				let parsed = https(issuer, "OAUTH_ISSUER")?;
				if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
					return Err(OAuthConfigError::IssuerHasPath);
				}
				https(authorize_url, "OAUTH_AUTHORIZE_URL")?;
				https(resource, "OAUTH_RESOURCE")?;
				Ok(Some(Self {
					issuer: issuer.to_owned(),
					authorize_url: authorize_url.to_owned(),
					resource: resource.to_owned(),
				}))
			}
			_ => Err(OAuthConfigError::Partial),
		}
	}

	#[must_use]
	pub fn token_endpoint(&self) -> String {
		self.issuer.clone() + crate::API_V1_BASE_PATH + "/oauth/token"
	}

	#[must_use]
	pub fn registration_endpoint(&self) -> String {
		self.issuer.clone() + crate::API_V1_BASE_PATH + "/oauth/register"
	}

	/// RFC 8414 metadata, as the MCP authorization spec asks for it.
	///
	/// No `client_id_metadata_document_supported` yet: clients fall back to
	/// registering (RFC 7591), which `claude.ai` and `ChatGPT` both do.
	#[must_use]
	pub fn metadata(&self) -> Value {
		json!({
			"issuer": self.issuer,
			"authorization_endpoint": self.authorize_url,
			"token_endpoint": self.token_endpoint(),
			"registration_endpoint": self.registration_endpoint(),
			"response_types_supported": ["code"],
			"grant_types_supported": ["authorization_code", "refresh_token"],
			"code_challenge_methods_supported": ["S256"],
			"token_endpoint_auth_methods_supported": ["none"],
			"scopes_supported": Scopes::all().words(),
			"authorization_response_iss_parameter_supported": true,
		})
	}
}

/// A setting, unless it is unset or blank.
fn set(value: Option<&str>) -> Option<&str> {
	value.map(str::trim).filter(|value| !value.is_empty())
}

fn https(value: &str, name: &'static str) -> Result<Url, OAuthConfigError> {
	Url::parse(value)
		.ok()
		.filter(|url| url.scheme() == "https" && url.host_str().is_some())
		.ok_or(OAuthConfigError::NotHttps(name))
}

/// An authorization request the app's page handed in, waiting for the
/// subject's answer.
#[derive(Debug, Clone)]
pub(crate) struct Pending {
	pub client_id: String,
	pub redirect_uri: String,
	pub state: Option<String>,
	pub code_challenge: String,
	pub scopes: Scopes,
	pub resource: String,
}

/// An approved request, waiting to be exchanged at the token endpoint.
#[derive(Debug, Clone)]
pub(crate) struct Code {
	pub subject: String,
	pub client_id: String,
	pub redirect_uri: String,
	pub challenge: String,
	pub scopes: Scopes,
	pub resource: String,
}

/// The in-memory half of OAuth: pending approvals and unexchanged codes.
pub(crate) struct OAuthFlows {
	pub settings: OAuthSettings,
	pub pending: OneTimeStore<Pending>,
	pub codes: OneTimeStore<Code>,
}

impl OAuthFlows {
	pub(crate) fn new(settings: OAuthSettings) -> Self {
		Self {
			settings,
			pending: OneTimeStore::new(PENDING_TTL, MAX_OPEN),
			codes: OneTimeStore::new(CODE_TTL, MAX_OPEN),
		}
	}
}

/// A [`OneTimeId`] as it travels: base64url, no padding.
pub(crate) fn encode_id(id: &OneTimeId) -> String {
	URL_SAFE_NO_PAD.encode(id)
}

pub(crate) fn decode_id(text: &str) -> Option<OneTimeId> {
	URL_SAFE_NO_PAD.decode(text).ok().and_then(|bytes| OneTimeId::try_from(bytes).ok())
}

/// A new random token, and the hash it is stored and looked up by. The
/// prefix (`fha_`, `fhr_`) says which kind a leaked one is.
pub(crate) fn mint_token(prefix: &str) -> (Redacted<String>, [u8; 32]) {
	let mut bytes = [0_u8; 32];
	rand::rng().fill_bytes(&mut bytes);
	let token = String::from(prefix) + &URL_SAFE_NO_PAD.encode(bytes);
	let hash = hash_token(&token);
	(Redacted::new(token), hash)
}

pub(crate) fn hash_token(token: &str) -> [u8; 32] {
	Sha256::digest(token.as_bytes()).into()
}

/// A random id for a client or a grant: 16 bytes, hex.
pub(crate) fn mint_id(prefix: &str) -> String {
	let mut bytes = [0_u8; 16];
	rand::rng().fill_bytes(&mut bytes);
	String::from(prefix) + &hex::encode(bytes)
}

/// Whether `challenge` is a well-formed `S256` code challenge: base64url of
/// a SHA-256, so 43 characters.
pub(crate) fn well_formed_challenge(challenge: &str) -> bool {
	challenge.len() == 43 && URL_SAFE_NO_PAD.decode(challenge).is_ok_and(|bytes| bytes.len() == 32)
}

/// RFC 7636: the verifier is 43 to 128 unreserved characters, and
/// `BASE64URL(SHA256(verifier))` is the challenge.
pub(crate) fn verifier_matches(verifier: &str, challenge: &str) -> bool {
	let unreserved = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
	(43..=128).contains(&verifier.len()) && verifier.bytes().all(unreserved) && URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge
}

/// Whether a client may register `uri` as a redirect: https anywhere, or
/// http on a loopback address (a native client such as Claude Code, RFC
/// 8252), with no fragment, at most [`MAX_REDIRECT_URI_LENGTH`] long.
pub(crate) fn redirect_uri_allowed(uri: &str) -> bool {
	if uri.len() > MAX_REDIRECT_URI_LENGTH {
		return false;
	}
	let Ok(url) = Url::parse(uri) else { return false };
	if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
		return false;
	}
	match (url.scheme(), url.host_str()) {
		("https", Some(_)) => true,
		("http", Some(host)) => matches!(host, "localhost" | "127.0.0.1" | "[::1]"),
		_ => false,
	}
}

/// The host a redirect goes to, for the approval page to show: the subject
/// should see where the code is about to be sent.
pub(crate) fn redirect_host(uri: &str) -> String {
	Url::parse(uri).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default()
}

/// `redirect_uri` with `params` appended to its query.
pub(crate) fn with_query(redirect_uri: &str, params: &[(&str, &str)]) -> String {
	let Ok(mut url) = Url::parse(redirect_uri) else {
		return redirect_uri.to_owned();
	};
	{
		let mut query = url.query_pairs_mut();
		for (name, value) in params {
			query.append_pair(name, value);
		}
	}
	url.into()
}

#[cfg(test)]
mod tests {
	use super::{redirect_uri_allowed, verifier_matches, well_formed_challenge, with_query, OAuthConfigError, OAuthSettings, Scope, Scopes};
	use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
	use sha2::{Digest, Sha256};

	#[test]
	fn scopes_are_exactly_what_was_asked_for_or_nothing() {
		let scopes = Scopes::parse("shelf lessons:read shelf").unwrap();
		assert_eq!(scopes.to_text(), "lessons:read shelf");
		assert!(scopes.contains(Scope::Shelf));
		assert!(!scopes.contains(Scope::ProgressRead));
		assert!(Scopes::parse("shelf admin").is_none(), "an unknown word refuses the whole request");
		assert!(Scopes::parse("  ").is_none());
		assert_eq!(Scopes::all().to_text(), "lessons:read progress:read shelf");
	}

	#[test]
	fn pkce_takes_the_verifier_whose_hash_is_the_challenge_and_nothing_else() {
		let verifier = "a".repeat(43);
		let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
		assert!(well_formed_challenge(&challenge));
		assert!(verifier_matches(&verifier, &challenge));
		assert!(!verifier_matches(&"b".repeat(43), &challenge));
		assert!(!verifier_matches(&challenge, &challenge), "the challenge is not its own verifier");
		let short = "a".repeat(42);
		assert!(!verifier_matches(&short, &URL_SAFE_NO_PAD.encode(Sha256::digest(short.as_bytes()))), "too short");
		assert!(!well_formed_challenge("plain-text-challenge"));
	}

	#[test]
	fn redirects_are_https_or_loopback_and_carry_no_fragment() {
		assert!(redirect_uri_allowed("https://claude.ai/api/mcp/auth_callback"));
		assert!(redirect_uri_allowed("http://127.0.0.1:43123/callback"));
		assert!(redirect_uri_allowed("http://localhost:8080/cb"));
		for refused in [
			"http://evil.test/cb",
			"https://claude.ai/cb#frag",
			"javascript:alert(1)",
			"https://user:pw@host.test/",
			"not a url",
			"custom-scheme://cb",
		] {
			assert!(!redirect_uri_allowed(refused), "{refused}");
		}
		assert!(!redirect_uri_allowed(&(String::from("https://x.test/") + &"a".repeat(600))));
	}

	#[test]
	fn a_redirect_keeps_its_own_query_and_gains_the_answer() {
		assert_eq!(
			with_query("https://cb.test/back?keep=1", &[("code", "abc"), ("state", "x y")]),
			"https://cb.test/back?keep=1&code=abc&state=x+y"
		);
	}

	#[test]
	fn settings_are_all_or_nothing_https_and_need_passkeys() {
		let issuer = Some("https://lessons.test/");
		let page = Some("https://nixos.local:5173/connect");
		let resource = Some("https://lessons.test/api/v1/mcp");
		assert!(OAuthSettings::from_parts(true, None, None, None).unwrap().is_none());
		let settings = OAuthSettings::from_parts(true, issuer, page, resource).unwrap().unwrap();
		assert_eq!(settings.issuer, "https://lessons.test", "a trailing slash is dropped so it matches what clients compare");
		assert_eq!(settings.token_endpoint(), "https://lessons.test/api/v1/oauth/token");
		assert_eq!(settings.metadata()["code_challenge_methods_supported"][0], "S256");

		assert_eq!(OAuthSettings::from_parts(true, issuer, None, resource).unwrap_err(), OAuthConfigError::Partial);
		assert_eq!(OAuthSettings::from_parts(false, issuer, page, resource).unwrap_err(), OAuthConfigError::AuthOff);
		assert_eq!(
			OAuthSettings::from_parts(true, Some("http://lessons.test"), page, resource).unwrap_err(),
			OAuthConfigError::NotHttps("OAUTH_ISSUER")
		);
		assert_eq!(
			OAuthSettings::from_parts(true, Some("https://lessons.test/api"), page, resource).unwrap_err(),
			OAuthConfigError::IssuerHasPath
		);
	}
}
