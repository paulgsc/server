use serde::{Deserialize, Serialize};

/// The four pre-built layout trees an activity can default to.
///
/// `LayoutTreeId` in `packages/activity-catalog/src/lib/types.ts`
/// (`paulgsc/some-ui`). Closed, the same way `SessionStatus` is: a row
/// claiming a fifth tree is a row this schema did not write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutTree {
	Study,
	Topik,
	Drama,
	Voice,
}

impl LayoutTree {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Study => "study",
			Self::Topik => "topik",
			Self::Drama => "drama",
			Self::Voice => "voice",
		}
	}

	/// Parse the stored `TEXT`.
	///
	/// `None` on anything outside the four-value vocabulary, matching
	/// `SessionStatus::parse`'s reasoning: a layout tree this table has never
	/// heard of is not a safe thing to fall back to a default for, since
	/// there is no "naive" tree the way there is a naive session layout.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"study" => Some(Self::Study),
			"topik" => Some(Self::Topik),
			"drama" => Some(Self::Drama),
			"voice" => Some(Self::Voice),
			_ => None,
		}
	}
}

/// How finished an activity is, in a person's terms rather than a
/// developer's (`ActivityMaturity` in `types.ts`).
///
/// The recommender filters on this directly -- an `early` construction zone
/// is a poor thing to propose unprompted -- so an unrecognised value is
/// refused rather than quietly treated as `Ready`, the same argument
/// `SessionStatus::parse` makes for not defaulting to `Draft`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityMaturity {
	Ready,
	Preview,
	Early,
}

impl ActivityMaturity {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Ready => "ready",
			Self::Preview => "preview",
			Self::Early => "early",
		}
	}

	/// Parse the stored `TEXT`. `None` on anything outside the three-value
	/// vocabulary -- see the type's own doc comment for why defaulting one
	/// silently would be the wrong failure mode here specifically.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"ready" => Some(Self::Ready),
			"preview" => Some(Self::Preview),
			"early" => Some(Self::Early),
			_ => None,
		}
	}
}

/// The server's row shape for one catalogued activity.
///
/// Mirrors `ActivityDefinition` (`packages/activity-catalog/src/lib/types.ts`,
/// `paulgsc/some-ui`) field for field, with three deliberate differences: no
/// `toSceneProps` (a closure -- CAT4/#272 decides its server-side
/// replacement), `min_duration_ms` added (hoisted out of `fields` -- see
/// `20260822000600_create_activities.up.sql`), and `published_at`/`version`
/// added (server-only bookkeeping the client type has no need of). `fields`,
/// `default_config`, and `audio` stay opaque `serde_json::Value` the same way
/// `SessionRecord::activities`/`scenes` do: this crate persists them, it does
/// not interpret them.
///
/// `camelCase` on the wire, same reasoning `SessionRecord` gives for its own
/// rename: the client's type is the contract, and this is the side that
/// moved. No route serves this yet (#271), but the casing is a property of
/// the field names, not of whether a handler exists, so there is nothing for
/// that story to decide here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecord {
	pub id: String,
	pub name: String,
	pub description: String,
	pub icon: String,
	pub registry_key: String,
	pub layout_tree: LayoutTree,
	pub maturity: ActivityMaturity,
	/// `None` means this activity declares no duration field at all -- see
	/// the migration's comment on why that is a real case, not a gap.
	pub min_duration_ms: Option<i64>,
	pub published_at: String,
	pub version: i64,
	/// Opaque; a heterogeneous `SelectField | DurationField` union the
	/// client owns the shape of.
	pub fields: Vec<serde_json::Value>,
	/// Opaque; values keyed by `fields`, meaningless without it.
	pub default_config: serde_json::Value,
	/// Opaque; absent means the activity is silent and needs no disclosure.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub audio: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
	use super::{ActivityMaturity, ActivityRecord, LayoutTree};
	use serde_json::json;

	/// The bug the Codex review on this PR caught: without
	/// `#[serde(rename_all = "camelCase")]`, this struct would serialize as
	/// `registry_key`/`layout_tree`/`default_config` -- fields
	/// `ActivityDefinition` on the client does not have, so a `GET
	/// /activities` handler built on this record (#271) would hand a
	/// `some-ui` caller a shape it cannot read.
	#[test]
	fn the_record_serializes_with_the_clients_field_names() {
		let record = ActivityRecord {
			id: "honeycomb".to_owned(),
			name: "Hangul Honeycomb".to_owned(),
			description: "d".to_owned(),
			icon: "hexagon".to_owned(),
			registry_key: "hangul".to_owned(),
			layout_tree: LayoutTree::Study,
			maturity: ActivityMaturity::Ready,
			min_duration_ms: Some(300_000),
			published_at: "2026-08-01T00:00:00Z".to_owned(),
			version: 1,
			fields: Vec::new(),
			default_config: json!({}),
			audio: None,
		};

		let wire = serde_json::to_value(&record).expect("serialize the record");
		for camel_case_key in ["registryKey", "layoutTree", "minDurationMs", "publishedAt", "defaultConfig"] {
			assert!(wire.get(camel_case_key).is_some(), "expected `{camel_case_key}` in {wire}");
		}
		// `audio` is optional and absent here; the snake_case name must not
		// leak through as an empty/null field either.
		assert!(wire.get("audio").is_none());
		assert!(wire.get("registry_key").is_none());
	}

	#[test]
	fn layout_tree_round_trips_and_an_unknown_one_is_refused() {
		for tree in [LayoutTree::Study, LayoutTree::Topik, LayoutTree::Drama, LayoutTree::Voice] {
			assert_eq!(LayoutTree::parse(tree.as_str()), Some(tree));
		}
		assert_eq!(LayoutTree::parse("cinematic"), None);
	}

	#[test]
	fn maturity_round_trips_and_an_unknown_one_is_refused() {
		for maturity in [ActivityMaturity::Ready, ActivityMaturity::Preview, ActivityMaturity::Early] {
			assert_eq!(ActivityMaturity::parse(maturity.as_str()), Some(maturity));
		}
		assert_eq!(ActivityMaturity::parse("beta"), None);
	}
}
