use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `TopikMetadataSchema.difficulty` (`paulgsc/some-ui`,
/// `packages/ui/topik/src/lib/topik/entity/topik-metadata.ts`): closed, and
/// refused rather than defaulted when unrecognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
	Beginner,
	Intermediate,
	Advanced,
}

impl Level {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Beginner => "beginner",
			Self::Intermediate => "intermediate",
			Self::Advanced => "advanced",
		}
	}

	/// `None` on anything outside the three-value vocabulary.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"beginner" => Some(Self::Beginner),
			"intermediate" => Some(Self::Intermediate),
			"advanced" => Some(Self::Advanced),
			_ => None,
		}
	}
}

/// One lesson's manifest-facing fields plus what this server tracks about it
/// — every column except the `body` blob.
///
/// Serialises (camelCase) to exactly a `TopikMetadata` manifest entry for the
/// fields that type has, via [`CurriculumEntry::manifest_entry`]; the server's
/// own bookkeeping (`activity_id`, `published_at`, `version`, `content_hash`,
/// `retired_at`) is not part of that shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurriculumEntry {
	pub key: String,
	pub activity_id: String,
	pub level: Option<Level>,
	pub display_name: String,
	pub description: String,
	pub batch_count: i64,
	pub total_questions: i64,
	pub total_messages: i64,
	pub tags: Option<Vec<String>>,
	pub published_at: String,
	pub version: i64,
	pub content_hash: String,
	/// When the operator retired it (ISO-8601 UTC), or `None` while it is
	/// listed — see `20260927000100_add_curriculum_retired_at.up.sql`.
	pub retired_at: Option<String>,
}

/// A `TopikMetadata` manifest entry, field for field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
	pub key: String,
	pub display_name: String,
	pub description: String,
	pub batch_count: i64,
	pub total_questions: i64,
	pub total_messages: i64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub difficulty: Option<Level>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tags: Option<Vec<String>>,
}

impl CurriculumEntry {
	/// The `TopikMetadata` this row answers a manifest request with.
	#[must_use]
	pub fn manifest_entry(&self) -> ManifestEntry {
		ManifestEntry {
			key: self.key.clone(),
			display_name: self.display_name.clone(),
			description: self.description.clone(),
			batch_count: self.batch_count,
			total_questions: self.total_questions,
			total_messages: self.total_messages,
			difficulty: self.level,
			tags: self.tags.clone(),
		}
	}
}

/// Whether `key` names a lesson rather than a path or URL — the one rule the
/// importer (#275) and the operator's write route share.
///
/// The client passes a path or URL key through to fetch from elsewhere; the
/// importer reads `<key>.json` from disk, and neither an offline command nor
/// a write route has any business following one.
///
/// A key is also a URL path segment: learners fetch
/// `/curriculum/<key>.json`, and a `?`, `#`, `%` or space in it would change
/// the request rather than name the lesson - listed in the manifest, then a
/// 404 (a real `chatgpt-codex-connector` finding on #393). So its characters
/// are RFC 3986's unreserved set (`A-Z a-z 0-9 - . _ ~`), the ones a path
/// segment carries without encoding.
#[must_use]
pub fn is_plain_key(key: &str) -> bool {
	let unreserved = key.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'));
	unreserved && !key.is_empty() && !key.starts_with("http") && !key.starts_with('.')
}

/// **The** definition of a lesson's content hash: lowercase hex SHA-256 over
/// the lesson file's exact bytes, as read from disk — no parsing, no
/// normalisation, no manifest metadata.
///
/// Exact bytes, so that "did this change" never depends on a JSON
/// re-serialisation choice this server would otherwise have to keep stable.
/// Over the file only, so that editing a manifest entry's `displayName` is not
/// new material. The importer (#275), the `ETag` (#276), and
/// `CurriculumUpdated` (#277) all derive from this one function.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
	use super::{content_hash, is_plain_key, Level};

	#[test]
	fn level_round_trips_and_an_unknown_one_is_refused() {
		for level in [Level::Beginner, Level::Intermediate, Level::Advanced] {
			assert_eq!(Level::parse(level.as_str()), Some(level));
		}
		assert_eq!(Level::parse("expert"), None);
	}

	#[test]
	fn a_plain_key_is_an_identifier_not_a_path_or_url() {
		for key in ["beginner", "week-39.a", "lesson.json"] {
			assert!(is_plain_key(key), "{key}");
		}
		for key in ["", "a/b", "a\\b", "https://example.test/x", "../x", ".hidden"] {
			assert!(!is_plain_key(key), "{key}");
		}
		// A key is a URL path segment: nothing that would change the request.
		for key in ["a?b", "a#b", "a%2Fb", "a b", "caf\u{e9}", "a:b", "a+b"] {
			assert!(!is_plain_key(key), "{key}");
		}
		assert!(is_plain_key("week_40~a"), "the rest of the unreserved set");
	}

	#[test]
	fn content_hash_is_sha256_of_the_exact_bytes() {
		assert_eq!(content_hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
		assert_ne!(content_hash(b"{\"a\":1}"), content_hash(b"{ \"a\": 1 }"), "whitespace is content: no normalisation");
	}
}
