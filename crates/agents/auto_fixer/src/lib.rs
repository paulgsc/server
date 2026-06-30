#![allow(clippy::disallowed_macros)]
#![allow(clippy::items_after_statements)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::uninlined_format_args)]
use serde::{Deserialize, Serialize};

pub mod llm;
pub mod parsers;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClippyIssue {
	pub file_path: String,
	pub line: u32,
	pub column: u32,
	pub rule: String,
	pub message: String,
	pub suggestion: Option<String>,
	pub code_snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ESLintIssue {
	pub file_path: String,
	pub line: u32,
	pub column: u32,
	pub rule_id: String,
	pub message: String,
	pub fix: Option<ESLintFix>,
	pub code_snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ESLintFix {
	pub range: [u32; 2],
	pub text: String,
}

pub trait LintIssue {
	fn get_file_path(&self) -> &str;
	fn get_location(&self) -> (u32, u32);
	fn get_message(&self) -> &str;
	fn get_code_snippet(&self) -> &str;
}
