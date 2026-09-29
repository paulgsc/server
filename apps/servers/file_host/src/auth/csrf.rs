//! Which origins may change state with the session cookie.
//!
//! `SameSite=Strict` stops a cross-*site* request from carrying the session.
//! It does not stop a same-site sibling: a page on another subdomain of the
//! same registrable domain is same-site, so the browser attaches the cookie to
//! its form posts. A bodyless `POST /auth/sign-out-everywhere` needs no CORS
//! preflight, so CORS does not stop it either. This does.
//!
//! The rule, for every method that can change state:
//!
//! - `Sec-Fetch-Site: same-origin` passes. The browser sets it, and a page
//!   cannot forge it; it is what the app's own requests through the
//!   `/api/file-host` proxy carry.
//! - Otherwise an `Origin` header must name a trusted origin
//!   (`ALLOWED_ORIGINS` or `WEBAUTHN_ORIGINS`), or the request is refused.
//! - A request with neither header did not come from a browser page, so it
//!   carries no ambient cookie a page could have borrowed, and passes.
//!
//! `GET`, `HEAD` and `OPTIONS` are not checked: they change nothing.

use axum::http::{HeaderMap, Method};

/// Whether a request may use the session cookie to change state.
pub(crate) fn allowed(method: &Method, headers: &HeaderMap, trusted: &[String]) -> bool {
	if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
		return true;
	}
	if header(headers, "sec-fetch-site") == Some("same-origin") {
		return true;
	}
	header(headers, "origin").map_or_else(
		|| header(headers, "sec-fetch-site").is_none(),
		|origin| trusted.iter().any(|trusted| trusted.trim_end_matches('/') == origin.trim_end_matches('/')),
	)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
	headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
	use super::allowed;
	use axum::http::{HeaderMap, HeaderValue, Method};

	fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
		let mut headers = HeaderMap::new();
		for (name, value) in pairs {
			headers.insert(*name, HeaderValue::from_static(value));
		}
		headers
	}

	#[test]
	fn a_sibling_origin_cannot_change_state_with_the_cookie() {
		let trusted = [String::from("https://app.test")];
		let sibling = headers(&[("origin", "https://evil.app.test"), ("sec-fetch-site", "same-site")]);

		assert!(!allowed(&Method::POST, &sibling, &trusted));
		assert!(!allowed(&Method::DELETE, &sibling, &trusted));
		assert!(allowed(&Method::GET, &sibling, &trusted), "reads change nothing");
	}

	#[test]
	fn the_app_itself_passes_through_the_proxy_or_from_a_trusted_origin() {
		let trusted = [String::from("https://app.test/")];
		assert!(allowed(
			&Method::POST,
			&headers(&[("sec-fetch-site", "same-origin"), ("origin", "https://app.test")]),
			&trusted
		));
		assert!(allowed(
			&Method::POST,
			&headers(&[("origin", "https://app.test"), ("sec-fetch-site", "same-site")]),
			&trusted
		));
		assert!(allowed(&Method::POST, &HeaderMap::new(), &trusted), "not a browser page: no borrowed cookie");
	}

	#[test]
	fn a_browser_request_without_an_origin_is_not_waved_through() {
		let trusted = [String::from("https://app.test")];
		assert!(!allowed(&Method::POST, &headers(&[("sec-fetch-site", "cross-site")]), &trusted));
	}
}
