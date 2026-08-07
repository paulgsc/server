//! The contract with `sw.js`.
//!
//! The receiving end is already written, deployed, and claimed:
//! `apps/www/public/sw.js` in `paulgsc/some-ui` parses `{ title, body, url, tag }`
//! and nothing else. So this is a contract to *match*, not to design — extra
//! fields are dropped by the worker, and a missing one falls back rather than
//! failing, because a `push` event that resolves without showing a notification
//! is reported as a broken worker and Chrome eventually revokes the
//! subscription over it.
//!
//! Three details of the worker that constrain what is built here:
//!
//! - It uses `tag` so a second nudge **replaces** the first rather than
//!   stacking under it.
//! - It renders two actions, `start` and `later`.
//! - It resolves `url` against `self.registration.scope`, so an absolute URL
//!   and an app-relative path both work.

use push_repo::Topic;
use serde::Serialize;
use study_domain::StudyAction;

/// Shared so a second nudge replaces the first in the notification centre.
///
/// This is the **third** hand-maintained copy of the constant: `public/sw.js`
/// and `src/lib/study-nudge/service-worker.ts` in `paulgsc/some-ui` hold the
/// other two, and neither can import from the other — one is a plain `public/`
/// asset. Changing it means changing three files in two repositories, and a
/// mismatch shows up as notifications stacking rather than replacing. Noted in
/// `docs/study-nudge.md` as a known cost rather than fixed here, because fixing
/// it is a `some-ui` build change.
pub const NUDGE_TAG: &str = "some-ui.study-nudge";

/// The practical payload ceiling.
///
/// Push services cap payloads below this, and `413 Payload Too Large` is the
/// failure. A nudge is four short strings; anything approaching the limit is a
/// bug rather than a big notification.
pub const MAX_PAYLOAD_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NudgePayload {
	pub title: String,
	pub body: String,
	pub url: String,
	pub tag: String,
}

impl NudgePayload {
	/// Render a decided [`StudyAction`] into the four keys `sw.js` reads.
	///
	/// The words live here rather than in `study_domain` because they are a
	/// property of *this channel*: the same action rendered into an email or an
	/// in-app banner would say something different, and the domain should not
	/// have to know which one it is going to.
	#[must_use]
	pub fn for_action(base_url: &str, action: &StudyAction) -> Self {
		let (title, body) = match action {
			StudyAction::LessonReady { .. } => ("Today's session is ready", "Pick up where you left off."),
			StudyAction::ResumeAbandoned { .. } => ("Pick up where you left off", "You started something and didn't finish."),
			StudyAction::SuggestReview { .. } => ("A quick review might help", "Some of the last set didn't stick."),
			StudyAction::NewMaterial { .. } => ("There's something new", "New material is waiting for you."),
		};

		Self::for_session(base_url, action.session_id(), title.to_owned(), body.to_owned())
	}

	/// Build the payload for a session deep link.
	///
	/// `base_url` is the study app's base, and the link is
	/// `{base}sessions/{id}` — the same shape the client builds, because a
	/// different one lands the click on the dashboard instead of the session.
	#[must_use]
	pub fn for_session(base_url: &str, session_id: &str, title: String, body: String) -> Self {
		let base = if base_url.ends_with('/') { base_url.to_owned() } else { format!("{base_url}/") };

		Self {
			title,
			body,
			url: format!("{base}sessions/{session_id}"),
			tag: NUDGE_TAG.to_owned(),
		}
	}

	/// Serialize, refusing anything the push service would reject on size.
	///
	/// # Errors
	/// Fails if serialization fails, or if the result exceeds
	/// [`MAX_PAYLOAD_BYTES`].
	pub fn to_bytes(&self) -> Result<Vec<u8>, PayloadError> {
		// `serde_json::to_vec` is on clippy.toml's disallowed list to keep eager
		// serialization out of tracing calls. This is the wire format itself,
		// which is the one place the bytes are the point.
		#[allow(clippy::disallowed_methods)]
		let encoded = serde_json::to_vec(self).map_err(PayloadError::Serialize)?;
		if encoded.len() > MAX_PAYLOAD_BYTES {
			return Err(PayloadError::TooLarge(encoded.len()));
		}
		Ok(encoded)
	}
}

#[derive(Debug)]
pub enum PayloadError {
	Serialize(serde_json::Error),
	TooLarge(usize),
}

impl std::fmt::Display for PayloadError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Serialize(err) => write!(f, "could not serialize nudge payload: {err}"),
			Self::TooLarge(size) => write!(f, "nudge payload is {size} bytes, over the {MAX_PAYLOAD_BYTES} byte ceiling"),
		}
	}
}

impl std::error::Error for PayloadError {}

/// Which consent grant an action needs. Lives beside the payload because both
/// answer "what is this notification, from the recipient's point of view".
#[must_use]
pub const fn topic_for(action: &StudyAction) -> Topic {
	match action {
		StudyAction::LessonReady { .. } => Topic::LessonReady,
		StudyAction::ResumeAbandoned { .. } | StudyAction::SuggestReview { .. } => Topic::Coaching,
		StudyAction::NewMaterial { .. } => Topic::NewMaterial,
	}
}

#[cfg(test)]
mod tests {
	use super::{NudgePayload, NUDGE_TAG};

	#[test]
	fn the_deep_link_matches_the_shape_the_client_builds() {
		let payload = NudgePayload::for_session("https://nixos.local:5173/", "session-abc", "t".to_owned(), "b".to_owned());
		assert_eq!(payload.url, "https://nixos.local:5173/sessions/session-abc");
		assert_eq!(payload.tag, NUDGE_TAG);
	}

	#[test]
	fn a_base_url_without_its_trailing_slash_still_produces_one_slash() {
		let payload = NudgePayload::for_session("https://nixos.local:5173", "session-abc", "t".to_owned(), "b".to_owned());
		assert_eq!(payload.url, "https://nixos.local:5173/sessions/session-abc");
	}

	#[test]
	fn a_subpath_base_survives_intact_for_the_pages_deployment() {
		let payload = NudgePayload::for_session("https://paulgsc.github.io/some-ui/", "s1", "t".to_owned(), "b".to_owned());
		assert_eq!(payload.url, "https://paulgsc.github.io/some-ui/sessions/s1");
	}

	#[test]
	fn the_serialized_shape_is_exactly_the_four_keys_the_worker_reads() {
		let payload = NudgePayload::for_session("https://x/", "s1", "Title".to_owned(), "Body".to_owned());
		let json: serde_json::Value = serde_json::from_slice(&payload.to_bytes().unwrap()).unwrap();
		// Sorted, because serde_json's default map is a BTreeMap and the worker
		// reads by key anyway. What matters is the set: an extra field is
		// silently dropped by `sw.js`, so a typo here would be invisible in
		// production.
		let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
		assert_eq!(keys, vec!["body", "tag", "title", "url"]);
	}

	#[test]
	fn an_oversized_payload_is_refused_here_rather_than_by_a_413() {
		let payload = NudgePayload::for_session("https://x/", "s1", "t".to_owned(), "b".repeat(5000));
		assert!(payload.to_bytes().is_err());
	}
}
