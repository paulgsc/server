//! `POST /api/v1/phrases/lookup`: a hunch's words in, the phrases that do it
//! or `kept` out (paulgsc/server#1261, KPLAY's KP4; `crate::phrases`).
//!
//! The shop's port to home (some-ui's `packages/ui/topik/src/lib/topik/shop`,
//! `ShopPort.lookUp(words, hunch)`). Home decides: `crate::phrases` holds the
//! threshold, and on a miss this request keeps the hunch
//! (`korean_repo::hunch`) before it answers, so a `kept` the phone reads is
//! a hunch home holds, and one sent again under its name is kept once.
//!
//! Nothing of a hunch reaches a log line: the handler records only
//! `otel.kind` (docs/identity.md invariant 41).

use crate::meaning::MeaningContext;
use crate::phrases::Lookup;
use crate::subject::Paired;
use crate::FileHostError;
use axum::{extract::State, Json};
use chrono::{SecondsFormat, Utc};
use korean_repo::hunch::{self, is_hunch, is_words};
use serde::Deserialize;
use sqlx::SqlitePool;
use tracing::instrument;

/// The body: the hunch's words, and the phone's name for the hunch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asked {
	words: String,
	hunch: String,
}

/// `POST /phrases/lookup`, body `{ words, hunch }`
///
/// `{ kind: "hit", phrases: [id, …] }`, closest first, or `{ kind: "kept" }`
/// once the hunch is kept. A hit keeps nothing.
///
/// # Errors
/// 401 without a session and 403 but from a paired phone; 422 naming `words`
/// (empty once trimmed, or over 200 characters), `hunch` (not 1 to 64 of
/// `A-Z a-z 0-9 _ . : -`) or `body` (not that JSON); 503
/// `feature_not_configured` while the meaning index is off, or the phrase
/// shelf did not index at start (`meaning::phrase_index`), since home can
/// then tell no hunch's phrases and keeps none; 500 for a storage failure,
/// and then nothing was kept.
#[axum::debug_handler(state = crate::AppState)]
#[instrument(name = "phrases_lookup", skip_all, fields(otel.kind = "server"))]
pub async fn post_lookup(
	paired: Paired,
	State(db): State<SqlitePool>,
	State(meaning): State<Option<MeaningContext>>,
	body: axum::body::Bytes,
) -> Result<Json<Lookup>, FileHostError> {
	let asked: Asked = serde_json::from_slice(&body).map_err(|_unreadable| FileHostError::unprocessable_entity([("body", "is not { words, hunch }")]))?;
	let mut problems = Vec::new();
	if !is_words(&asked.words) {
		problems.push(("words", "must be 1 to 200 characters"));
	}
	if !is_hunch(&asked.hunch) {
		problems.push(("hunch", "must be 1 to 64 of A-Z a-z 0-9 _ . : -"));
	}
	if !problems.is_empty() {
		return Err(FileHostError::unprocessable_entity(problems));
	}
	let meaning = meaning.ok_or(FileHostError::FeatureNotConfigured("the meaning index (MEANING_ENABLED)"))?;
	let shelf = meaning.phrases.as_ref().ok_or(FileHostError::FeatureNotConfigured("the phrase shelf's index"))?;
	let words = asked.words.trim();
	let phrases: Vec<String> = shelf.look_up(&meaning.model.embed(words)).into_iter().map(str::to_owned).collect();
	if !phrases.is_empty() {
		return Ok(Json(Lookup::Hit { phrases }));
	}
	let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
	hunch::keep(&mut *db.acquire().await?, paired.subject.as_str(), &asked.hunch, words, &now).await?;
	Ok(Json(Lookup::Kept))
}

#[cfg(test)]
mod tests;
