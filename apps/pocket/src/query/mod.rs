/// Query — fuzzy matching and interactive picking.
///
/// Separate from the store (no file I/O) and from the CLI (no arg parsing).
/// Receives a slice of register data and returns a selection or an error.
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use skim::prelude::*;
use std::sync::Arc;

use crate::{
	error::{PocketError, Result},
	model::RegisterData,
};

// ── Scored match ──────────────────────────────────────────────────────────────

/// A register paired with its fuzzy match score.
#[derive(Debug, Clone)]
pub struct ScoredMatch<'a> {
	pub register: &'a RegisterData,
	pub score: i64,
}

/// Score every register against `query` and return matches sorted by score
/// descending.  Empty query returns all registers unscored (score = 0).
pub fn score_all<'a>(registers: &'a [RegisterData], query: &str) -> Vec<ScoredMatch<'a>> {
	let matcher = SkimMatcherV2::default();

	if query.is_empty() {
		return registers.iter().map(|r| ScoredMatch { register: r, score: 0 }).collect();
	}

	let mut matches: Vec<ScoredMatch<'a>> = registers
		.iter()
		.filter_map(|r| matcher.fuzzy_match(&r.label, query).map(|score| ScoredMatch { register: r, score }))
		.collect();

	matches.sort_by(|a, b| b.score.cmp(&a.score));
	matches
}

// ── Skim item wrapper ─────────────────────────────────────────────────────────

/// Wraps a `RegisterData` reference for skim's trait requirements.
struct SkimRegister {
	label: String,
	value: String,
}

impl SkimItem for SkimRegister {
	fn text(&self) -> Cow<'_, str> {
		Cow::Borrowed(&self.label)
	}

	fn preview(&self, _context: PreviewContext) -> ItemPreview {
		ItemPreview::Text(self.value.clone())
	}
}

// ── Interactive picker ────────────────────────────────────────────────────────

/// Present an interactive fuzzy picker and return the selected register's value.
///
/// Returns:
///   - `Ok(String)` — the value of the selected register.
///   - `Err(PickerCancelled)` — user pressed Escape or closed without selecting.
///   - `Err(PickerNoSelection)` — picker returned but produced no output item.
pub fn pick_interactive(registers: &[RegisterData]) -> Result<String> {
	if registers.is_empty() {
		return Err(PocketError::PickerNoSelection);
	}

	let options = SkimOptionsBuilder::default()
		.height(Some("40%"))
		.preview(Some("")) // enable preview window
		.preview_window(Some("right:50%"))
		.multi(false)
		.build()
		.expect("skim options are valid");

	let (tx, rx): (SkimItemSender, SkimItemReceiver) = unbounded();

	for r in registers {
		tx.send(Arc::new(SkimRegister {
			label: r.label.clone(),
			value: r.value.clone(),
		}))
		.expect("skim channel is open during send");
	}
	drop(tx); // signal EOF to skim

	let output = Skim::run_with(&options, Some(rx));

	match output {
		Some(out) if out.is_abort => Err(PocketError::PickerCancelled),

		Some(out) => {
			let selected = out.selected_items.into_iter().next().ok_or(PocketError::PickerNoSelection)?;

			// Downcast back to our wrapper.
			let item = selected.as_any().downcast_ref::<SkimRegister>().ok_or(PocketError::PickerNoSelection)?;

			Ok(item.value.clone())
		}

		None => Err(PocketError::PickerCancelled),
	}
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::Utc;

	fn reg(label: &str, value: &str) -> RegisterData {
		RegisterData {
			id: "test-id".to_string(),
			label: label.to_string(),
			value: value.to_string(),
			created_at: Utc::now(),
			last_used_at: Utc::now(),
		}
	}

	#[test]
	fn empty_query_returns_all() {
		let regs = vec![reg("pdf dark mode", "filter: invert(1)"), reg("cargo watch", "cargo watch -x test")];
		let results = score_all(&regs, "");
		assert_eq!(results.len(), 2);
	}

	#[test]
	fn query_filters_by_label() {
		let regs = vec![reg("pdf dark mode", "filter: invert(1)"), reg("cargo watch", "cargo watch -x test")];
		let results = score_all(&regs, "pdf");
		assert_eq!(results.len(), 1);
		assert_eq!(results[0].register.label, "pdf dark mode");
	}

	#[test]
	fn fuzzy_subsequence_match() {
		let regs = vec![reg("pdf dark mode", "filter: invert(1)")];
		// "pdm" is a subsequence of "pdf dark mode"
		let results = score_all(&regs, "pdm");
		assert_eq!(results.len(), 1);
	}

	#[test]
	fn no_match_returns_empty() {
		let regs = vec![reg("pdf dark mode", "filter: invert(1)")];
		let results = score_all(&regs, "zzz");
		assert!(results.is_empty());
	}

	#[test]
	fn results_sorted_by_score_descending() {
		let regs = vec![
			reg("pdf dark", "a"),
			reg("pdf dark mode", "b"), // more chars, potentially different score
			reg("pdf", "c"),
		];
		// All match "pdf"; just check ordering is stable and non-empty.
		let results = score_all(&regs, "pdf");
		assert!(!results.is_empty());
		// Scores should be non-increasing.
		let scores: Vec<i64> = results.iter().map(|m| m.score).collect();
		for w in scores.windows(2) {
			assert!(w[0] >= w[1]);
		}
	}
}
