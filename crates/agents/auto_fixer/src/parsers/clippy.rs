#![allow(dead_code)]

use crate::{ClippyIssue, LintIssue};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct ClippyMessage {
	message: MessageData,
	target: Option<Target>,
}

#[derive(Deserialize)]
struct MessageData {
	message: String,
	code: Option<Code>,
	spans: Vec<Span>,
	children: Vec<Child>,
}

#[derive(Deserialize)]
struct Code {
	code: String,
}

#[derive(Deserialize)]
struct Span {
	file_name: String,
	line_start: u32,
	column_start: u32,
	text: Vec<SpanText>,
}

#[derive(Deserialize)]
struct SpanText {
	text: String,
}

#[derive(Deserialize)]
struct Child {
	message: String,
	spans: Vec<Span>,
}

#[derive(Deserialize)]
struct Target {
	name: String,
}

impl LintIssue for ClippyIssue {
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

pub fn parse_clippy_output(output: &str) -> Result<Vec<ClippyIssue>> {
	let mut issues = Vec::new();

	for line in output.lines() {
		if line.trim().is_empty() {
			continue;
		}

		let msg: ClippyMessage = serde_json::from_str(line).with_context(|| format!("Failed to parse clippy JSON: {}", line))?;

		if let Some(span) = msg.message.spans.first() {
			// Fixed: Use itertools or collect to String directly
			let code_snippet = span.text.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join("");

			let suggestion = msg.message.children.iter().find(|child| child.message.contains("help:")).map(|child| child.message.clone());

			let rule = msg.message.code.map(|c| c.code).unwrap_or_else(|| "unknown".to_string());

			issues.push(ClippyIssue {
				file_path: span.file_name.clone(),
				line: span.line_start,
				column: span.column_start,
				rule,
				message: msg.message.message,
				suggestion,
				code_snippet,
			});
		}
	}

	Ok(issues)
}

pub fn read_code_context(file_path: &str, line: u32, context_lines: u32) -> Result<String> {
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
	fn test_parse_clippy_output() {
		let json_output = r#"{"message":{"message":"this function has too many arguments (8/7)","code":{"code":"clippy::too_many_arguments","explanation":null},"level":"warning","spans":[{"file_name":"src/main.rs","byte_start":123,"byte_end":456,"line_start":10,"line_end":10,"column_start":1,"column_end":20,"is_primary":true,"text":[{"text":"fn complex_function(","highlight_start":1,"highlight_end":20}],"label":null,"suggested_replacement":null,"suggestion_applicability":null,"expansion":null}],"children":[{"message":"help: consider using a struct","code":null,"level":"help","spans":[],"children":[],"rendered":null}],"rendered":"warning: this function has too many arguments (8/7)\n  --> src/main.rs:10:1\n   |\n10 | fn complex_function(\n   | ^^^^^^^^^^^^^^^^^^^\n   |\n   = help: consider using a struct\n"},"target":{"kind":["bin"],"crate_types":["bin"],"name":"test","src_path":"/path/to/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true}}"#;

		let issues = parse_clippy_output(json_output).unwrap();
		assert_eq!(issues.len(), 1);

		let issue = &issues[0];
		assert_eq!(issue.file_path, "src/main.rs");
		assert_eq!(issue.line, 10);
		assert_eq!(issue.rule, "clippy::too_many_arguments");
		assert!(issue.suggestion.is_some());
	}
}
