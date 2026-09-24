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
/// own bookkeeping (`activity_id`, `published_at`, `version`, `content_hash`)
/// is not part of that shape.
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
	use super::{content_hash, Level};

	#[test]
	fn level_round_trips_and_an_unknown_one_is_refused() {
		for level in [Level::Beginner, Level::Intermediate, Level::Advanced] {
			assert_eq!(Level::parse(level.as_str()), Some(level));
		}
		assert_eq!(Level::parse("expert"), None);
	}

	#[test]
	fn content_hash_is_sha256_of_the_exact_bytes() {
		assert_eq!(content_hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
		assert_ne!(content_hash(b"{\"a\":1}"), content_hash(b"{ \"a\": 1 }"), "whitespace is content: no normalisation");
	}
}
