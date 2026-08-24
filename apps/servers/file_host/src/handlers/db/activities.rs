//! Handlers for `routes::db::activities`. See that module's doc comment for
//! the surface; this is the query-and-respond half.

use crate::{AppState, FileHostError};
use activity_repo::{ActivityRecord, ActivityRepoError, ActivityRepository};
use axum::{
	extract::{Path, State},
	http::{
		header::{ETAG, IF_NONE_MATCH},
		HeaderMap, HeaderValue, StatusCode,
	},
	response::{IntoResponse, Response},
	Json,
};
use std::fmt::Write as _;
use tracing::instrument;

fn repo(state: &AppState) -> ActivityRepository {
	ActivityRepository::new(state.core.shared_db.clone())
}

/// [`ActivityRepoError::CatalogTooLarge`] is the one variant a caller needs
/// to react to differently — it maps onto the record-limit refusal
/// `FileHostError` already carries for exactly this shape of problem.
/// Everything else is an operational failure with no more specific meaning
/// a client could act on.
fn to_http(err: &ActivityRepoError) -> FileHostError {
	match err {
		ActivityRepoError::CatalogTooLarge { .. } => FileHostError::MaxRecordLimitExceeded,
		other => FileHostError::OperationError(other.to_string()),
	}
}

/// Wraps a fingerprint in the quoted form `ETag` requires (RFC 9110 §8.8.3).
/// Always ASCII hex plus an `@`/quote, so this cannot fail on the values this
/// module ever passes it — `expect` documents that invariant rather than
/// threading a fallible conversion through two call sites for a string this
/// crate built itself.
fn etag_value(tag: &str) -> HeaderValue {
	// `write!` rather than `format!`: `clippy.toml` disallows `format!`
	// (eager allocation ahead of tracing) — same substitution
	// `ts_emitter::render_ts` already makes.
	let mut quoted = String::with_capacity(tag.len() + 2);
	let _ = write!(quoted, "\"{tag}\"");
	HeaderValue::from_str(&quoted).expect("an activities ETag is always valid header content")
}

/// The opaque-tag component of an entity-tag, with any leading weak
/// indicator (`W/`) stripped.
///
/// `If-None-Match` is defined (RFC 9110 §8.8.3.2, §13.1.2) to always use
/// *weak* comparison, which considers two entity-tags equal whenever their
/// opaque-tags match regardless of either side's strength — so `W/"abc123"`
/// must be treated as a hit against the strong tag `"abc123"` this module
/// always emits. Stripping unconditionally on both sides implements that
/// directly rather than special-casing which side might carry the prefix.
fn opaque_tag(raw: &[u8]) -> &[u8] {
	raw.strip_prefix(b"W/").unwrap_or(raw)
}

/// Whether `If-None-Match` names the `ETag` this response would carry.
///
/// `If-None-Match` may list several comma-separated tags, or `*` to match
/// anything — both handled here, matching the header's actual grammar
/// (RFC 9110 §13.1.2) rather than a single-value shortcut.
fn if_none_match_hits(headers: &HeaderMap, etag: &HeaderValue) -> bool {
	let Some(raw) = headers.get(IF_NONE_MATCH).and_then(|value| value.to_str().ok()) else {
		return false;
	};
	let etag = opaque_tag(etag.as_bytes());
	raw.split(',').map(str::trim).any(|candidate| candidate == "*" || opaque_tag(candidate.as_bytes()) == etag)
}

fn not_modified(etag: HeaderValue) -> Response {
	let mut response = StatusCode::NOT_MODIFIED.into_response();
	response.headers_mut().insert(ETAG, etag);
	response
}

fn ok_with_etag<T: serde::Serialize>(body: &T, etag: HeaderValue) -> Response {
	let mut response = Json(body).into_response();
	response.headers_mut().insert(ETAG, etag);
	response
}

/// `GET /activities`
///
/// The `ETag` is [`ActivityRepository::fingerprint`] — the one "did the
/// catalogue change" value this route and #273's `CurriculumUpdated`
/// producer both derive from the same place. Computed before deciding
/// whether to answer `304`, since a miss still needs it to hand back the
/// current tag.
#[axum::debug_handler]
#[instrument(name = "list_activities", skip_all, fields(otel.kind = "server"))]
pub async fn list_activities(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, FileHostError> {
	let repo = repo(&state);
	let fingerprint = repo.fingerprint().await.map_err(|err| to_http(&err))?;
	let etag = etag_value(&fingerprint);

	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}

	let catalogue = repo.list().await.map_err(|err| to_http(&err))?;
	Ok(ok_with_etag(&catalogue, etag))
}

/// `GET /activities/:id`
///
/// An unknown id is a `404` through [`FileHostError::NotFound`], not an
/// empty `200` — #271's own acceptance criterion.
///
/// The `ETag` here is scoped to this one row (`id@version`), not
/// [`ActivityRepository::fingerprint`]: tying a single activity's cache
/// validity to every other activity's edits would invalidate a client's
/// cached copy on changes this response never reflects.
#[axum::debug_handler]
#[instrument(name = "get_activity", skip_all, fields(otel.kind = "server"))]
pub async fn get_activity(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Response, FileHostError> {
	let repo = repo(&state);
	let record: ActivityRecord = repo.get(&id).await.map_err(|err| to_http(&err))?.ok_or(FileHostError::NotFound)?;
	let mut tag = String::new();
	let _ = write!(tag, "{}@{}", record.id, record.version);
	let etag = etag_value(&tag);

	if if_none_match_hits(&headers, &etag) {
		return Ok(not_modified(etag));
	}

	Ok(ok_with_etag(&record, etag))
}

/// #271's own acceptance criteria, made concrete against the pure logic this
/// module can test without an `AppState` — building one needs a real NATS
/// connection ([`AppState::build`]'s `NatsTransport::connect_pooled`), so
/// (matching `nudge::waker`'s test module, which exercises `Engine`
/// directly rather than a live `AppState` for the same reason) these test
/// the conditional-request decision and the error mapping, not the axum
/// wiring around them — the wiring is thin enough that a successful build
/// under `#[axum::debug_handler]` already checks its shape.
#[cfg(test)]
mod tests {
	use super::{etag_value, if_none_match_hits, to_http};
	use activity_repo::ActivityRepoError;
	use axum::http::{header::IF_NONE_MATCH, HeaderMap, HeaderValue};

	fn headers_with_if_none_match(raw: &str) -> HeaderMap {
		let mut headers = HeaderMap::new();
		headers.insert(IF_NONE_MATCH, HeaderValue::from_str(raw).expect("a valid test header value"));
		headers
	}

	#[test]
	fn etag_value_is_wrapped_in_the_quoted_form_the_header_requires() {
		let etag = etag_value("abc123");
		assert_eq!(etag.to_str().unwrap(), "\"abc123\"");
	}

	#[test]
	fn if_none_match_misses_when_the_header_is_absent() {
		let etag = etag_value("abc123");
		assert!(!if_none_match_hits(&HeaderMap::new(), &etag), "no If-None-Match header at all must never read as a hit");
	}

	#[test]
	fn if_none_match_hits_on_an_exact_match() {
		let etag = etag_value("abc123");
		let headers = headers_with_if_none_match("\"abc123\"");
		assert!(if_none_match_hits(&headers, &etag));
	}

	#[test]
	fn if_none_match_misses_on_a_different_tag() {
		let etag = etag_value("abc123");
		let headers = headers_with_if_none_match("\"someone-elses-tag\"");
		assert!(!if_none_match_hits(&headers, &etag));
	}

	/// RFC 9110 §8.8.3.2 requires weak comparison for `If-None-Match`: a
	/// weak-tagged candidate (`W/"..."`) must still hit against the strong
	/// tag this module always emits, since weak comparison only requires the
	/// opaque-tags to match, not the strength.
	#[test]
	fn if_none_match_hits_a_weak_validator_against_a_strong_tag() {
		let etag = etag_value("abc123");
		let headers = headers_with_if_none_match("W/\"abc123\"");
		assert!(if_none_match_hits(&headers, &etag), "a weak validator naming the same opaque tag must still be a hit");
	}

	#[test]
	fn if_none_match_hits_on_the_wildcard() {
		let etag = etag_value("abc123");
		let headers = headers_with_if_none_match("*");
		assert!(if_none_match_hits(&headers, &etag), "`*` must match any current ETag, per RFC 9110 §13.1.2");
	}

	#[test]
	fn if_none_match_hits_when_the_tag_is_one_of_several_comma_separated() {
		let etag = etag_value("abc123");
		let headers = headers_with_if_none_match("\"other-tag\", \"abc123\", \"a-third-tag\"");
		assert!(
			if_none_match_hits(&headers, &etag),
			"a comma-separated list must be checked entry by entry, not compared whole"
		);
	}

	#[test]
	fn an_over_ceiling_catalogue_maps_to_the_max_record_limit_error() {
		let err = ActivityRepoError::CatalogTooLarge { rows: 600, ceiling: 500 };
		assert!(
			matches!(to_http(&err), crate::FileHostError::MaxRecordLimitExceeded),
			"the over-ceiling refusal must surface as FileHostError::MaxRecordLimitExceeded, not a generic operation error"
		);
	}
}
