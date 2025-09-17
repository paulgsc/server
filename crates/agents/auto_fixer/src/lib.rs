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
