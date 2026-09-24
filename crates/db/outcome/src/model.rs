use serde::{Deserialize, Serialize};

/// How one activity block ended.
///
/// Closed, the same way `SessionStatus` is: a row claiming a fourth outcome is
/// a row this schema did not write, and there is no safe default to fall back
/// to — defaulting to `Completed` would invent attendance, and defaulting to
/// `Abandoned` would drain momentum for something nobody did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeKind {
	/// The block ran to its end. Says nothing about how well — that is
	/// `score`, and only when the activity assessed it.
	Completed,
	/// The person left the block before its end.
	Abandoned,
	/// The block was passed over without being started.
	Skipped,
}

impl OutcomeKind {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Completed => "completed",
			Self::Abandoned => "abandoned",
			Self::Skipped => "skipped",
		}
	}

	/// Parse the stored (or posted) `TEXT`. `None` on anything outside the
	/// three-value vocabulary — see the type's own doc comment.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"completed" => Some(Self::Completed),
			"abandoned" => Some(Self::Abandoned),
			"skipped" => Some(Self::Skipped),
			_ => None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::OutcomeKind;

	#[test]
	fn outcome_round_trips_and_an_unknown_one_is_refused() {
		for kind in [OutcomeKind::Completed, OutcomeKind::Abandoned, OutcomeKind::Skipped] {
			assert_eq!(OutcomeKind::parse(kind.as_str()), Some(kind));
		}
		assert_eq!(OutcomeKind::parse("finished"), None);
		assert_eq!(OutcomeKind::parse("Completed"), None, "case matters: the stored vocabulary is lowercase");
		assert_eq!(OutcomeKind::parse(""), None);
	}
}
