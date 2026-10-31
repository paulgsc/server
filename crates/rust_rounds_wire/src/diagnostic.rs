//! rustc's diagnostics, as a recording keeps them: read from
//! `--error-format=json` by `rust_runner::diagnostics`.

use serde::{Deserialize, Serialize};

/// One diagnostic as recorded: what a reader needs to point at the code,
/// without rustc's long-form `explanation` (`rustc --explain` has it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
	/// `error`, `warning`, `note`, `help`, `failure-note`, …
	pub level: String,
	/// `E0277`, or a lint's name (`unused_variables`); `None` for one with no
	/// code (`aborting due to 1 previous error`).
	pub code: Option<String>,
	pub message: String,
	pub spans: Vec<Span>,
	pub children: Vec<Child>,
	/// The text rustc prints for it without `--error-format=json`.
	pub rendered: Option<String>,
}

/// A note or help attached to a [`Diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Child {
	pub level: String,
	pub message: String,
	pub spans: Vec<Span>,
}

/// Where a diagnostic points, 1-based, as rustc reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
	/// The file, relative to the directory rustc ran in for the program's
	/// own (`main.rs`), or a path in the toolchain's sources.
	pub file: String,
	pub line_start: u64,
	pub line_end: u64,
	pub column_start: u64,
	pub column_end: u64,
	pub is_primary: bool,
	pub label: Option<String>,
	/// The text a suggestion would put there, when it is one.
	pub suggested_replacement: Option<String>,
}
