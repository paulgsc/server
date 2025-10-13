#![allow(dead_code)]

use crate::{ESLintFix, ESLintIssue, LintIssue};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct ESLintOutput {
	#[serde(rename = "filePath")]
	file_path: String,
	messages: Vec<ESLintMessage>,
}

#[derive(Deserialize)]
struct ESLintMessage {
	#[serde(rename = "ruleId")]
	rule_id: Option<String>,
	severity: u32,
	message: String,
	line: u32,
	column: u32,
	#[serde(rename = "nodeType")]
	node_type: Option<String>,
	#[serde(rename = "messageId")]
	message_id: Option<String>,
	#[serde(rename = "endLine")]
	end_line: Option<u32>,
	#[serde(rename = "endColumn")]
	end_column: Option<u32>,
	fix: Option<ESLintFixRaw>,
}

#[derive(Deserialize)]
struct ESLintFixRaw {
	range: [u32; 2],
	text: String,
}

impl LintIssue for ESLintIssue {
	fn get_file_path(&self) -> &str {
		&self.file_path
	}

	fn get_location(&self) -> (u32, u32) {
		(self.line, self.column)
	}

	fn get_message(&self) -> &str {
		&self.message
	}

	fn get_code_snippet(&self) -> &str {
		&self.code_snippet
	}
}

pub fn parse_eslint_output(output: &str) -> Result<Vec<ESLintIssue>> {
	let eslint_results: Vec<ESLintOutput> = serde_json::from_str(output).context("Failed to parse ESLint JSON output")?;

	let mut issues = Vec::new();

	for result in eslint_results {
		for message in result.messages {
			let code_snippet = read_code_context(&result.file_path, message.line, 2).unwrap_or_else(|_| format!("Could not read context from {}", result.file_path));

			let fix = message.fix.map(|f| ESLintFix { range: f.range, text: f.text });

			issues.push(ESLintIssue {
				file_path: result.file_path.clone(),
				line: message.line,
				column: message.column,
				rule_id: message.rule_id.unwrap_or_else(|| "unknown".to_string()),
				message: message.message,
				fix,
				code_snippet,
			});
		}
	}

	Ok(issues)
}

fn read_code_context(file_path: &str, line: u32, context_lines: u32) -> Result<String> {
	let content = fs::read_to_string(file_path).with_context(|| format!("Failed to read file: {}", file_path))?;

	let lines: Vec<&str> = content.lines().collect();
	let start = line.saturating_sub(context_lines + 1) as usize;
	let end = ((line + context_lines) as usize).min(lines.len());

	Ok(lines[start..end].join("\n"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_eslint_output() {
		let json_output = r#"[
									{
													"filePath": "/path/to/file.ts",
																	"messages": [
																						{
																												"ruleId": "@typescript-eslint/no-unused-vars",
																																		"severity": 2,
																																								"message": "'unusedVar' is defined but never used.",
																																														"line": 5,
																																																				"column": 7,
																																																										"nodeType": "Identifier",
																																																																"messageId": "unusedVar",
																																																																						"endLine": 5,
																																																																												"endColumn": 16
																																																																																	}
																																																																																					]
																																																																																								}
																																																																																										]"#;

		let issues = parse_eslint_output(json_output).unwrap();
		assert_eq!(issues.len(), 1);

		let issue = &issues[0];
		assert_eq!(issue.file_path, "/path/to/file.ts");
		assert_eq!(issue.line, 5);
		assert_eq!(issue.rule_id, "@typescript-eslint/no-unused-vars");
		assert_eq!(issue.message, "'unusedVar' is defined but never used.");
	}
}
