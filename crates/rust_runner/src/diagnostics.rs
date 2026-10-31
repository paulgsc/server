//! rustc's `--error-format=json` diagnostics, read back.
//!
//! Each line rustc writes to stderr under `--error-format=json` is one
//! diagnostic: its level, its code (`E0277`), its message, its spans and its
//! children, and `rendered`, the text rustc would have printed without the
//! flag. Concatenating every `rendered` gives that text byte for byte
//! (`the_rendered_text_is_what_rustc_prints_without_json`), so asking for JSON
//! loses nothing a reader of the plain output had, and keeps the spans and
//! codes a reader of the plain output would have to parse back out.
//!
//! A line that is not a diagnostic (an internal compiler error's own text,
//! say) is kept in the rendered text as it came.

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

/// rustc's JSON, as it writes it. Only the fields kept are read; the rest
/// (`explanation`, `expansion`, `text`, …) are ignored.
#[derive(Deserialize)]
struct Wire {
	#[serde(rename = "$message_type")]
	message_type: Option<String>,
	message: String,
	code: Option<WireCode>,
	level: String,
	#[serde(default)]
	spans: Vec<WireSpan>,
	#[serde(default)]
	children: Vec<Self>,
	rendered: Option<String>,
}

#[derive(Deserialize)]
struct WireCode {
	code: String,
}

#[derive(Deserialize)]
struct WireSpan {
	file_name: String,
	line_start: u64,
	line_end: u64,
	column_start: u64,
	column_end: u64,
	is_primary: bool,
	label: Option<String>,
	suggested_replacement: Option<String>,
}

impl From<WireSpan> for Span {
	fn from(span: WireSpan) -> Self {
		Self {
			file: span.file_name,
			line_start: span.line_start,
			line_end: span.line_end,
			column_start: span.column_start,
			column_end: span.column_end,
			is_primary: span.is_primary,
			label: span.label,
			suggested_replacement: span.suggested_replacement,
		}
	}
}

/// rustc's stderr, read back: every diagnostic, and the text it would have
/// printed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
	pub diagnostics: Vec<Diagnostic>,
	/// Every diagnostic's `rendered`, and every line that was not one, in
	/// order.
	pub rendered: String,
}

/// Read `stderr` as rustc wrote it under `--error-format=json`. `cut` says
/// the stream was kept to a ceiling, so its last line, without a newline, is
/// a part of one and is dropped rather than misread.
#[must_use]
pub fn read(stderr: &[u8], cut: bool) -> Read {
	let mut read = Read::default();
	let text = String::from_utf8_lossy(stderr);
	for line in text.split_inclusive('\n') {
		if cut && !line.ends_with('\n') {
			break;
		}
		match serde_json::from_str::<Wire>(line) {
			Ok(wire) if wire.message_type.as_deref().is_none_or(|kind| kind == "diagnostic") => {
				if let Some(rendered) = &wire.rendered {
					read.rendered.push_str(rendered);
				}
				read.diagnostics.push(diagnostic(wire));
			}
			// Another kind of JSON message (an artifact notification, say):
			// not printed without the flag either.
			Ok(_) => {}
			Err(_) => read.rendered.push_str(line),
		}
	}
	read
}

fn diagnostic(wire: Wire) -> Diagnostic {
	Diagnostic {
		level: wire.level,
		code: wire.code.map(|code| code.code),
		message: wire.message,
		spans: wire.spans.into_iter().map(Span::from).collect(),
		children: wire
			.children
			.into_iter()
			.map(|child| Child {
				level: child.level,
				message: child.message,
				spans: child.spans.into_iter().map(Span::from).collect(),
			})
			.collect(),
		rendered: wire.rendered,
	}
}

#[cfg(test)]
mod tests {
	use super::read;

	const E0277: &str = r#"{"$message_type":"diagnostic","message":"the type `str` cannot be indexed by `usize`","code":{"code":"E0277","explanation":"long"},"level":"error","spans":[{"file_name":"main.rs","byte_start":1,"byte_end":2,"line_start":4,"line_end":4,"column_start":15,"column_end":16,"is_primary":true,"text":[],"label":"string indices are ranges of `usize`","suggested_replacement":null,"suggestion_applicability":null,"expansion":null}],"children":[{"message":"the trait `SliceIndex<str>` is not implemented for `usize`","code":null,"level":"help","spans":[],"children":[],"rendered":null}],"rendered":"error[E0277]: the type `str` cannot be indexed by `usize`\n"}"#;
	const ABORTING: &str = r#"{"$message_type":"diagnostic","message":"aborting due to 1 previous error","code":null,"level":"error","spans":[],"children":[],"rendered":"error: aborting due to 1 previous error\n\n"}"#;

	#[test]
	fn a_diagnostic_keeps_its_code_spans_and_children_and_its_rendered_text() {
		let stderr = String::from(E0277) + "\n" + ABORTING + "\n";
		let read = read(stderr.as_bytes(), false);
		assert_eq!(
			read.rendered,
			"error[E0277]: the type `str` cannot be indexed by `usize`\nerror: aborting due to 1 previous error\n\n"
		);
		assert_eq!(read.diagnostics.len(), 2);
		let first = &read.diagnostics[0];
		assert_eq!(first.code.as_deref(), Some("E0277"));
		assert_eq!(first.level, "error");
		assert_eq!((first.spans[0].line_start, first.spans[0].column_start), (4, 15));
		assert!(first.spans[0].is_primary);
		assert_eq!(first.spans[0].file, "main.rs");
		assert_eq!(first.children[0].level, "help");
		assert_eq!(read.diagnostics[1].code, None);
	}

	#[test]
	fn a_line_that_is_not_json_is_kept_as_printed_and_a_cut_line_is_dropped() {
		let stderr = String::from("thread 'rustc' panicked at compiler/x.rs\n") + E0277 + "\n" + &ABORTING[..40];
		let read = read(stderr.as_bytes(), true);
		assert_eq!(read.diagnostics.len(), 1, "the cut diagnostic is not read");
		assert_eq!(
			read.rendered,
			"thread 'rustc' panicked at compiler/x.rs\nerror[E0277]: the type `str` cannot be indexed by `usize`\n"
		);
	}
}
