//! Terminal rendering helpers.
//!
//! All `owo-colors` and `tabled` usage is confined to this module.
//! Command handlers call these functions; they never format strings
//! directly. This makes it trivial to swap the output format later
//! (e.g., JSON stdout for a future TUI or web layer).

use owo_colors::{OwoColorize, Style};
use tabsched::Session;

use crate::config::LabelIndex;

const STYLE_ACCENT: Style = Style::new().bright_cyan().bold();
const STYLE_DIM: Style = Style::new().bright_black();
const STYLE_OK: Style = Style::new().bright_green();
const STYLE_WARN: Style = Style::new().bright_yellow();

// ── Colours ────────────────────────────────────────────────────────────────

fn accent(s: impl ToString) -> String {
	s.to_string().style(STYLE_ACCENT).to_string()
}

fn dim(s: impl ToString) -> String {
	s.to_string().style(STYLE_DIM).to_string()
}

fn ok(s: impl ToString) -> String {
	s.to_string().style(STYLE_OK).to_string()
}

fn warn(s: impl ToString) -> String {
	s.to_string().style(STYLE_WARN).to_string()
}

// ── Session card ───────────────────────────────────────────────────────────

/// Print the "now work on this" card shown by `next` and `step`.
pub fn print_session_card(session: &Session, index: &LabelIndex) {
	let track_label = index.track_label(session.track);
	let resource_label = index.resource_label(session.resource);
	let slot = session.slot_index.as_u64();

	println!();
	println!("  {} {}", dim("slot"), accent(&slot.to_string()));
	println!("  {} {}", dim("track   "), accent(track_label));
	println!("  {} {}", dim("resource"), accent(resource_label));
	println!();
}

// ── Status table ───────────────────────────────────────────────────────────

/// One row in the status table.
pub struct StatusRow {
	pub track: String,
	pub target: u32,
	pub recent: u32,
	pub deficit: i32,
	pub total: usize,
}

/// Print the per-track status table.
pub fn print_status_table(rows: &[StatusRow]) {
	// Column widths derived from content.
	let w_track = rows.iter().map(|r| r.track.len()).max().unwrap_or(5).max(5);

	// Header
	println!("\n  {:<w$}  {:>6}  {:>6}  {:>7}  {:>7}", "TRACK", "TARGET", "RECENT", "DEFICIT", "TOTAL", w = w_track,);
	println!("  {}", "-".repeat(w_track + 34).bright_black());

	for row in rows {
		let deficit_str = if row.deficit > 0 {
			format!("+{}", row.deficit).bright_red().to_string()
		} else if row.deficit < 0 {
			format!("{}", row.deficit).bright_green().to_string()
		} else {
			format!("{}", row.deficit).bright_black().to_string()
		};

		println!(
			"  {:<w$}  {:>6}  {:>6}  {:>7}  {:>7}",
			row.track.bright_white(),
			row.target,
			row.recent,
			deficit_str,
			row.total,
			w = w_track,
		);
	}
	println!();
}

// ── History table ──────────────────────────────────────────────────────────

pub struct HistoryRow {
	pub slot: u64,
	pub track: String,
	pub resource: String,
	pub outcome: String,
}

pub fn print_history_table(rows: &[HistoryRow]) {
	if rows.is_empty() {
		println!("\n  {}\n", dim("no sessions recorded yet"));
		return;
	}

	let w_track = rows.iter().map(|r| r.track.len()).max().unwrap_or(5).max(5);
	let w_res = rows.iter().map(|r| r.resource.len()).max().unwrap_or(8).max(8);

	println!("\n  {:>5}  {:<w_t$}  {:<w_r$}  {}", "SLOT", "TRACK", "RESOURCE", "OUTCOME", w_t = w_track, w_r = w_res,);
	println!("  {}", "-".repeat(5 + 2 + w_track + 2 + w_res + 2 + 10).bright_black());

	for row in rows {
		println!(
			"  {:>5}  {:<w_t$}  {:<w_r$}  {}",
			row.slot.bright_black(),
			row.track,
			row.resource,
			row.outcome,
			w_t = w_track,
			w_r = w_res,
		);
	}
	println!();
}

// ── Inline messages ────────────────────────────────────────────────────────

pub fn print_ok(msg: &str) {
	println!("  {} {}", ok("✓"), msg);
}

pub fn print_warn(msg: &str) {
	println!("  {} {}", warn("!"), msg);
}

pub fn format_outcome(outcome: &tabsched::Outcome) -> &'static str {
	match outcome {
		tabsched::Outcome::Unrecorded => "—",
		tabsched::Outcome::Progress => "progress",
		tabsched::Outcome::Stuck => "stuck",
		tabsched::Outcome::Review => "review",
	}
}

pub fn print_step_hint() {
	println!("  run {} when done\n", "ts done <outcome>".bright_black());
}
