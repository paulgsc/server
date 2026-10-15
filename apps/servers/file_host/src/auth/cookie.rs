//! The session cookie.
//!
//! A session is 32 random bytes, base64url in a cookie the browser attaches on
//! its own and JavaScript cannot read. The server stores only the SHA-256 of
//! those bytes, so a copy of the database opens no session.
//!
//! The attributes, each for a reason:
//!
//! - `__Host-` prefix: the browser refuses the cookie unless it is `Secure`,
//!   `Path=/` and has no `Domain`, so no sibling subdomain can set or shadow
//!   it.
//! - `HttpOnly`: script on the page, including anything injected, cannot read
//!   the token.
//! - `SameSite=Strict`: a request another site starts carries no session, which
//!   is the CSRF defence. The app reaches `file_host` through its own origin's
//!   `/api/file-host` proxy, so its own requests are same-site.
//!
//! A paired phone holds the same kind of token, handed to it in a response
//! body (`super::pairing`), and sends it as `Authorization: Bearer`. A browser
//! never attaches that header on its own, so it is no ambient credential.
//!
//! The two kinds are stored under different digests ([`SessionToken::hash`]):
//! a phone's token presented as a cookie, or a browser's as a bearer, finds
//! no session. So how a token arrived is what kind of session it is, which
//! is what lets `subject::BrowserSubject` judge by the carrier.

use crate::redacted::Redacted;
use axum::http::{
	header::{AUTHORIZATION, COOKIE},
	HeaderMap, HeaderValue,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const SESSION_COOKIE: &str = "__Host-session";

/// Prefixed to a phone's token before hashing ([`SessionToken::hash`]).
const PHONE_DOMAIN: &[u8] = b"some-ui paired phone session\0";

/// A session token as the client holds it. Never logged, never stored.
pub(crate) struct SessionToken(Redacted<[u8; 32]>);

impl SessionToken {
	pub(crate) fn mint() -> Self {
		let mut token = [0_u8; 32];
		rand::rng().fill_bytes(&mut token);
		Self(Redacted::new(token))
	}

	/// The token a request carries, and how: a well-formed
	/// `Authorization: Bearer` (a paired phone) first, else the session
	/// cookie. HTTP/2 may split cookies across several headers, so every one
	/// is read.
	pub(crate) fn from_headers(headers: &HeaderMap) -> Option<(Self, Carrier)> {
		bearer(headers).map(|token| (token, Carrier::Bearer)).or_else(|| {
			headers
				.get_all(COOKIE)
				.iter()
				.filter_map(|value| value.to_str().ok())
				.flat_map(|value| value.split(';'))
				.filter_map(|pair| pair.trim().split_once('='))
				.find(|(name, _)| *name == SESSION_COOKIE)
				.and_then(|(_, value)| decode(value))
				.map(|token| (token, Carrier::Cookie))
		})
	}

	/// The token as a paired phone holds it: base64url, sent back as
	/// `Authorization: Bearer`.
	pub(crate) fn encoded(&self) -> String {
		URL_SAFE_NO_PAD.encode(self.0.expose())
	}

	/// What the server stores and looks sessions up by, for a token carried
	/// as `carrier`: a browser's is the token's SHA-256 (every session made
	/// before pairing existed is one), a phone's is domain-separated, so
	/// neither opens the other's kind of session.
	pub(crate) fn hash(&self, carrier: Carrier) -> [u8; 32] {
		match carrier {
			Carrier::Cookie => Sha256::digest(self.0.expose()).into(),
			Carrier::Bearer => Sha256::new().chain_update(PHONE_DOMAIN).chain_update(self.0.expose()).finalize().into(),
		}
	}

	/// The `Set-Cookie` that hands this token to the browser for
	/// `max_age_seconds`.
	pub(crate) fn set_cookie(&self, max_age_seconds: i64) -> HeaderValue {
		cookie_header(&URL_SAFE_NO_PAD.encode(self.0.expose()), max_age_seconds)
	}
}

/// How a request carried its session token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Carrier {
	/// The browser's `__Host-session` cookie.
	Cookie,
	/// `Authorization: Bearer`, from a paired phone. Nothing is ever set in
	/// a cookie for it.
	Bearer,
}

fn bearer(headers: &HeaderMap) -> Option<SessionToken> {
	let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
	let (scheme, token) = value.trim().split_once(' ')?;
	if scheme.eq_ignore_ascii_case("bearer") {
		decode(token.trim())
	} else {
		None
	}
}

fn decode(value: &str) -> Option<SessionToken> {
	URL_SAFE_NO_PAD
		.decode(value)
		.ok()
		.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
		.map(|token| SessionToken(Redacted::new(token)))
}

/// The `Set-Cookie` that removes the session cookie from the browser.
pub(crate) fn clear_cookie() -> HeaderValue {
	cookie_header("", 0)
}

fn cookie_header(value: &str, max_age_seconds: i64) -> HeaderValue {
	let mut header = String::with_capacity(128);
	header.push_str(SESSION_COOKIE);
	header.push('=');
	header.push_str(value);
	header.push_str("; Path=/; Max-Age=");
	header.push_str(&max_age_seconds.max(0).to_string());
	header.push_str("; HttpOnly; Secure; SameSite=Strict");
	// Only base64url, digits and fixed ASCII: always a valid header value.
	HeaderValue::from_str(&header).unwrap_or_else(|_| HeaderValue::from_static("__Host-session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict"))
}

#[cfg(test)]
mod tests {
	use super::{clear_cookie, Carrier, SessionToken, SESSION_COOKIE};
	use axum::http::{
		header::{AUTHORIZATION, COOKIE},
		HeaderMap, HeaderValue,
	};

	fn headers(values: &[&str]) -> HeaderMap {
		let mut headers = HeaderMap::new();
		for value in values {
			headers.append(COOKIE, HeaderValue::from_str(value).unwrap());
		}
		headers
	}

	fn cookie_value(set_cookie: &HeaderValue) -> String {
		let text = set_cookie.to_str().unwrap();
		let pair = text.split(';').next().unwrap();
		pair.split_once('=').unwrap().1.to_owned()
	}

	#[test]
	fn a_minted_token_round_trips_through_its_own_cookie() {
		let token = SessionToken::mint();
		let set_cookie = token.set_cookie(3_600);
		let text = set_cookie.to_str().unwrap();
		assert!(text.starts_with("__Host-session="));
		assert!(text.ends_with("; Path=/; Max-Age=3600; HttpOnly; Secure; SameSite=Strict"));

		let value = cookie_value(&set_cookie);
		let (read, carrier) = SessionToken::from_headers(&headers(&[&(String::from("theme=dark; ") + SESSION_COOKIE + "=" + &value)])).unwrap();
		assert_eq!(read.hash(carrier), token.hash(Carrier::Cookie));
		assert_eq!(carrier, Carrier::Cookie);
	}

	#[test]
	fn a_session_cookie_is_found_in_any_cookie_header() {
		let token = SessionToken::mint();
		let value = cookie_value(&token.set_cookie(60));
		let (read, carrier) = SessionToken::from_headers(&headers(&["theme=dark", &(String::from(SESSION_COOKIE) + "=" + &value)])).unwrap();
		assert_eq!(read.hash(carrier), token.hash(Carrier::Cookie));
	}

	#[test]
	fn a_bearer_token_is_read_before_any_cookie() {
		let phone = SessionToken::mint();
		let browser = SessionToken::mint();
		let mut both = headers(&[&(String::from(SESSION_COOKIE) + "=" + &browser.encoded())]);
		both.insert(AUTHORIZATION, HeaderValue::from_str(&(String::from("Bearer ") + &phone.encoded())).unwrap());

		let (read, carrier) = SessionToken::from_headers(&both).unwrap();
		assert_eq!(read.hash(carrier), phone.hash(Carrier::Bearer));
		assert_eq!(carrier, Carrier::Bearer);
		assert_ne!(phone.hash(Carrier::Bearer), phone.hash(Carrier::Cookie), "a phone's session is no browser's");

		for malformed in ["Bearer", "Bearer not*base64", "Basic AAAA", "Bearer c2hvcnQ"] {
			let mut only = HeaderMap::new();
			only.insert(AUTHORIZATION, HeaderValue::from_static(malformed));
			assert!(SessionToken::from_headers(&only).is_none(), "{malformed}");
		}
	}

	#[test]
	fn anything_but_a_well_formed_token_reads_as_no_session() {
		for cookie in ["", "theme=dark", "__Host-session=", "__Host-session=not*base64", "__Host-session=c2hvcnQ", "session=AAAA"] {
			assert!(SessionToken::from_headers(&headers(&[cookie])).is_none(), "{cookie}");
		}
		assert!(SessionToken::from_headers(&HeaderMap::new()).is_none());
	}

	#[test]
	fn clearing_expires_the_cookie_now() {
		assert_eq!(clear_cookie().to_str().unwrap(), "__Host-session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict");
	}
}
