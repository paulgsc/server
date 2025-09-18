fn main() {
	unimplemented!();
}

// use lint_parser::{clippy, eslint, ollama::OllamaClient};
// use std::env;
// use tokio;
//
// #[tokio::main]
// async fn main() -> Result<(), Box<dyn std::error::Error>> {
// 	let args: Vec<String> = env::args().collect();
//
// 	if args.len() < 3 {
// 		eprintln!("Usage: {} <clippy|eslint> <json_file_or_stdin>", args[0]);
// 		std::process::exit(1);
// 	}
//
// 	let lint_type = &args[1];
// 	let input = &args[2];
//
// 	let json_content = if input == "-" {
// 		// Read from stdin
// 		use std::io::{self, Read};
// 		let mut buffer = String::new();
// 		io::stdin().read_to_string(&mut buffer)?;
// 		buffer
// 	} else {
// 		// Read from file
// 		std::fs::read_to_string(input)?
// 	};
//
// 	let ollama = OllamaClient::default();
//
// 	// Check if Ollama is available
// 	if !ollama.health_check().await? {
// 		eprintln!("Warning: Ollama is not available. Fixes will not be generated.");
// 	}
//
// 	match lint_type.as_str() {
// 		"clippy" => {
// 			let issues = clippy::parse_clippy_output(&json_content)?;
// 			println!("Found {} clippy issues:", issues.len());
//
// 			for (i, issue) in issues.iter().enumerate() {
// 				println!("\n--- Issue {} ---", i + 1);
// 				println!("File: {}", issue.file_path);
// 				println!("Location: {}:{}", issue.line, issue.column);
// 				println!("Rule: {}", issue.rule);
// 				println!("Message: {}", issue.message);
//
// 				if let Some(suggestion) = &issue.suggestion {
// 					println!("Suggestion: {}", suggestion);
// 				}
//
// 				println!("Code snippet:\n{}", issue.code_snippet);
//
// 				// Generate fix if Ollama is available
// 				if ollama.health_check().await.unwrap_or(false) {
// 					match ollama.fix_clippy_issue(issue).await {
// 						Ok(fix) => {
// 							println!("AI-generated fix:\n{}", fix);
// 						}
// 						Err(e) => {
// 							eprintln!("Failed to generate fix: {}", e);
// 						}
// 					}
// 				}
// 			}
// 		}
// 		"eslint" => {
// 			let issues = eslint::parse_eslint_output(&json_content)?;
// 			println!("Found {} ESLint issues:", issues.len());
//
// 			for (i, issue) in issues.iter().enumerate() {
// 				println!("\n--- Issue {} ---", i + 1);
// 				println!("File: {}", issue.file_path);
// 				println!("Location: {}:{}", issue.line, issue.column);
// 				println!("Rule: {}", issue.rule_id);
// 				println!("Message: {}", issue.message);
//
// 				if let Some(fix) = &issue.fix {
// 					println!("ESLint fix available: range {:?} -> '{}'", fix.range, fix.text);
// 				}
//
// 				println!("Code snippet:\n{}", issue.code_snippet);
//
// 				// Generate fix if Ollama is available
// 				if ollama.health_check().await.unwrap_or(false) {
// 					match ollama.fix_eslint_issue(issue).await {
// 						Ok(fix) => {
// 							println!("AI-generated fix:\n{}", fix);
// 						}
// 						Err(e) => {
// 							eprintln!("Failed to generate fix: {}", e);
// 						}
// 					}
// 				}
// 			}
// 		}
// 		_ => {
// 			eprintln!("Unknown lint type: {}. Use 'clippy' or 'eslint'", lint_type);
// 			std::process::exit(1);
// 		}
// 	}
//
// 	Ok(())
// }
//
// #[cfg(test)]
// mod integration_tests {
// 	use super::*;
// 	use lint_parser::{clippy, eslint};
//
// 	#[test]
// 	fn test_clippy_integration() {
// 		let sample_output = r#"{"message":{"message":"variable does not need to be mutable","code":{"code":"clippy::unused_mut"},"level":"warning","spans":[{"file_name":"src/test.rs","line_start":5,"line_end":5,"column_start":9,"column_end":18,"text":[{"text":"    let mut x = 5;"}]}],"children":[{"message":"remove this `mut`","spans":[{"file_name":"src/test.rs","line_start":5,"line_end":5,"column_start":9,"column_end":13,"text":[{"text":"    let mut x = 5;","highlight_start":9,"highlight_end":13}]}]}]}}"#;
//
// 		let issues = clippy::parse_clippy_output(sample_output).unwrap();
// 		assert_eq!(issues.len(), 1);
// 		assert_eq!(issues[0].rule, "clippy::unused_mut");
// 	}
//
// 	#[test]
// 	fn test_eslint_integration() {
// 		let sample_output = r#"[{"filePath":"test.ts","messages":[{"ruleId":"no-unused-vars","severity":2,"message":"'test' is defined but never used.","line":1,"column":7}]}]"#;
//
// 		let issues = eslint::parse_eslint_output(sample_output).unwrap();
// 		assert_eq!(issues.len(), 1);
// 		assert_eq!(issues[0].rule_id, "no-unused-vars");
// 	}
// }
