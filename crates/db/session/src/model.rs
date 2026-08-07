use serde::{Deserialize, Deserializer, Serialize};

/// The five statuses `SessionStatus` admits in the client. Not four, not six —
/// the vocabulary is fixed, and the policy's candidate ranking is written
/// against exactly these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
	Draft,
	Scheduled,
	Active,
	Paused,
	Completed,
}

impl SessionStatus {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Draft => "draft",
			Self::Scheduled => "scheduled",
			Self::Active => "active",
			Self::Paused => "paused",
			Self::Completed => "completed",
		}
	}

	/// Parse the stored `TEXT`.
	///
	/// A row whose status is not one of the five is a row written by something
	/// that is not this schema, and reading it as `Draft` would quietly make it
	/// a nudge candidate. `None` lets the caller refuse it instead.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"draft" => Some(Self::Draft),
			"scheduled" => Some(Self::Scheduled),
			"active" => Some(Self::Active),
			"paused" => Some(Self::Paused),
			"completed" => Some(Self::Completed),
			_ => None,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
	Basic,
	Advanced,
}

impl LayoutMode {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Basic => "basic",
			Self::Advanced => "advanced",
		}
	}

	/// Parse the stored `TEXT`, falling back to `Basic` — an unrecognised
	/// layout mode costs a person the advanced editor's arrangement, which is
	/// recoverable, where refusing to read the row is not.
	#[must_use]
	pub fn parse(raw: &str) -> Self {
		if raw == "advanced" {
			Self::Advanced
		} else {
			Self::Basic
		}
	}
}

/// Distinguish "the key was absent" from "the key was explicitly `null`".
///
/// Serde folds both into `None` for a plain `Option<T>`. `layout` is the one
/// field where the difference is meaningful — absent means the client's naive
/// default tree applies, explicit null means someone cleared it — so it gets
/// the double-`Option` treatment through this helper plus `#[serde(default)]`.
fn deserialize_explicit_null<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
	D: Deserializer<'de>,
{
	serde_json::Value::deserialize(deserializer).map(Some)
}

/// The server's copy of `SessionRecord`.
///
/// Field names are `camelCase` on the wire because the client's type is the
/// contract and this is the side that moved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
	pub id: String,
	pub name: String,
	pub status: SessionStatus,
	/// Opaque to this crate; the client owns the shape.
	pub activities: Vec<serde_json::Value>,
	/// Opaque except for `start_time` and `duration`, which
	/// [`total_duration_of`] reads.
	pub scenes: Vec<serde_json::Value>,
	pub layout_mode: LayoutMode,
	#[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_explicit_null")]
	pub layout: Option<serde_json::Value>,
	pub total_duration_ms: i64,
	pub created_at: String,
	pub updated_at: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub started_at: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub completed_at: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub final_elapsed_ms: Option<i64>,
}

/// What `SessionsRepository.create(input)` sends.
///
/// Deliberately not a `SessionRecord`: the id, the timestamps, the initial
/// status, and `totalDurationMs` are all the server's to decide now, and a
/// client that could send them could disagree with itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSession {
	pub name: String,
	#[serde(default)]
	pub activities: Vec<serde_json::Value>,
	#[serde(default)]
	pub scenes: Vec<serde_json::Value>,
	pub layout_mode: LayoutMode,
	#[serde(default, deserialize_with = "deserialize_explicit_null")]
	pub layout: Option<serde_json::Value>,
}

/// What `SessionsRepository.update(id, patch)` sends: `Partial<Omit<SessionRecord,
/// "id" | "createdAt" | "updatedAt">>`. Every field absent means "leave it".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSession {
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub status: Option<SessionStatus>,
	#[serde(default)]
	pub activities: Option<Vec<serde_json::Value>>,
	#[serde(default)]
	pub scenes: Option<Vec<serde_json::Value>>,
	#[serde(default)]
	pub layout_mode: Option<LayoutMode>,
	/// Absent leaves the stored layout alone; an explicit `null` clears it.
	#[serde(default, deserialize_with = "deserialize_explicit_null")]
	pub layout: Option<serde_json::Value>,
	#[serde(default)]
	pub total_duration_ms: Option<i64>,
	#[serde(default)]
	pub started_at: Option<String>,
	#[serde(default)]
	pub completed_at: Option<String>,
	#[serde(default)]
	pub final_elapsed_ms: Option<i64>,
}

/// `max(start_time + duration)` across scenes, mirroring `totalDurationOf` in
/// `sessions-repository.ts`.
///
/// This moved server-side with the data on purpose. If the server stores
/// `total_duration_ms` but leaves its computation to the client, a client that
/// forgets stores a zero, and the nudge then cheerfully offers you a "~1 min"
/// session. A scene missing either field contributes nothing rather than
/// failing the write — the fields are the client's, and a schema mismatch
/// should cost a duration estimate, not the session.
#[must_use]
pub fn total_duration_of(scenes: &[serde_json::Value]) -> i64 {
	scenes
		.iter()
		.map(|scene| {
			let field = |key: &str| scene.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
			field("start_time").saturating_add(field("duration"))
		})
		.max()
		.unwrap_or(0)
		.max(0)
}

#[cfg(test)]
mod tests {
	use super::{total_duration_of, SessionRecord};
	use serde_json::json;

	#[test]
	fn total_duration_is_the_latest_scene_end() {
		let scenes = vec![json!({ "start_time": 0, "duration": 600_000 }), json!({ "start_time": 600_000, "duration": 300_000 })];
		assert_eq!(total_duration_of(&scenes), 900_000);
	}

	#[test]
	fn a_scene_missing_its_fields_contributes_nothing() {
		let scenes = vec![json!({ "start_time": 0, "duration": 60_000 }), json!({ "label": "no timing here" })];
		assert_eq!(total_duration_of(&scenes), 60_000);
	}

	#[test]
	fn no_scenes_is_zero_rather_than_a_panic() {
		assert_eq!(total_duration_of(&[]), 0);
	}

	#[test]
	fn an_absent_layout_is_distinguishable_from_an_explicit_null() {
		let absent: SessionRecord = serde_json::from_value(json!({
			"id": "session-1", "name": "n", "status": "draft", "activities": [], "scenes": [],
			"layoutMode": "basic", "totalDurationMs": 0, "createdAt": "a", "updatedAt": "b"
		}))
		.unwrap();
		assert!(absent.layout.is_none());

		let explicit: SessionRecord = serde_json::from_value(json!({
			"id": "session-1", "name": "n", "status": "draft", "activities": [], "scenes": [],
			"layoutMode": "basic", "totalDurationMs": 0, "createdAt": "a", "updatedAt": "b",
			"layout": null
		}))
		.unwrap();
		assert_eq!(explicit.layout, Some(serde_json::Value::Null));

		// And the distinction survives a round trip: absent stays absent.
		let reserialized = serde_json::to_value(&absent).unwrap();
		assert!(reserialized.get("layout").is_none());
	}
}
