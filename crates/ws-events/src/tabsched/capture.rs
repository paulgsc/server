#![cfg(feature = "tabsched")]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentKind(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
	pub kind: ContentKind,
	pub title: String,
	pub summary: String,
	pub headings: Vec<String>,
	pub keywords: Vec<String>,
	pub raw_length: u64,
	pub meta: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabCapture {
	pub tab_id: i64,
	pub url: String,
	pub tab_title: String,
	pub captured_at: String,
	pub extractor: String,
	pub domain: Domain,
	pub content: ExtractedContent,
	pub extraction_ok: bool,
	pub extraction_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedTab {
	pub tab_id: i64,
	pub url: String,
	pub reason: String,
}

impl std::fmt::Display for Domain {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSummary {
	pub tab_id: i64,
	pub url: String,
	pub tab_title: String,
	pub domain: String,
	pub last_seen_at: String,
	pub extraction_ok: bool,
}
