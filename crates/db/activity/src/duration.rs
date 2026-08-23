use serde_json::Value;

/// Derives `min_duration_ms` from an activity's `fields` blob, the same
/// blob the `fields` column stores verbatim.
///
/// Finds the entry with `"kind":"duration"` (`DurationField` in
/// `packages/activity-catalog/src/lib/types.ts`, `paulgsc/some-ui`) and turns
/// its `minMinutes` into milliseconds -- the unit `min_duration_ms` is named
/// for, see `20260822000600_create_activities.up.sql`'s note on why this is
/// hoisted out of the JSON blob into its own column at all.
///
/// `None` when no field declares `"kind":"duration"`, or when the one found
/// carries a `minMinutes` that is missing or not a whole number. An activity
/// with no duration field is a real, expected shape (the migration's own
/// comment names it); a malformed one is refused rather than guessed at, the
/// same refuse-don't-default reasoning `LayoutTree::parse`/
/// `ActivityMaturity::parse` use for an unrecognised value.
#[must_use]
pub fn derive_min_duration_ms(fields: &[Value]) -> Option<i64> {
	fields
		.iter()
		.find(|field| field.get("kind").and_then(Value::as_str) == Some("duration"))
		.and_then(|field| field.get("minMinutes"))
		.and_then(Value::as_i64)
		.map(|min_minutes| min_minutes * 60_000)
}

#[cfg(test)]
mod tests {
	use super::derive_min_duration_ms;
	use serde_json::json;

	/// `leetype`'s own field, post-some-ui#1130: `minMinutes: 5` derives to
	/// the 300_000 the seed migration claims for it.
	#[test]
	fn a_duration_fields_minimum_becomes_milliseconds() {
		let fields = vec![json!({"kind": "duration", "key": "durationMinutes", "minMinutes": 5, "maxMinutes": 60})];
		assert_eq!(derive_min_duration_ms(&fields), Some(300_000));
	}

	/// A `select` field never contributes a duration, even when it is the
	/// only field present.
	#[test]
	fn no_duration_field_derives_to_none() {
		let fields = vec![json!({"kind": "select", "key": "level"})];
		assert_eq!(derive_min_duration_ms(&fields), None);
	}

	/// The shape `leetype` used to be the sole example of, before
	/// some-ui#1130 -- still a real case for whichever activity is next to
	/// not need a duration field.
	#[test]
	fn an_empty_fields_array_derives_to_none() {
		assert_eq!(derive_min_duration_ms(&[]), None);
	}

	/// A `minMinutes` that is not a whole number is refused rather than
	/// coerced or truncated into a plausible-looking guess.
	#[test]
	fn a_non_numeric_min_minutes_is_refused_rather_than_guessed() {
		let fields = vec![json!({"kind": "duration", "minMinutes": "five"})];
		assert_eq!(derive_min_duration_ms(&fields), None);
	}

	/// The duration field is found regardless of where it sits among its
	/// siblings -- the client's own catalogue puts it last in every entry,
	/// but nothing here should depend on that.
	#[test]
	fn the_duration_field_is_found_even_when_not_first() {
		let fields = vec![
			json!({"kind": "select", "key": "level"}),
			json!({"kind": "select", "key": "category"}),
			json!({"kind": "duration", "minMinutes": 10}),
		];
		assert_eq!(derive_min_duration_ms(&fields), Some(600_000));
	}
}
