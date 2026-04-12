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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSession {
	pub session_id: String,
	pub captured_at: String,
	pub extension_version: String,
	pub total_open_tabs: u64,
	pub captures: Vec<TabCapture>,
	pub skipped: Vec<SkippedTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSummary {
	pub session_id: String,
	pub captured_at: String,
	pub total_tabs: u64,
	pub captured_ok: usize,
	pub captured_fail: usize,
	pub skipped: usize,
}

impl std::fmt::Display for Domain {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl From<&CaptureSession> for CaptureSummary {
	fn from(s: &CaptureSession) -> Self {
		Self {
			session_id: s.session_id.clone(),
			captured_at: s.captured_at.clone(),
			total_tabs: s.total_open_tabs,
			captured_ok: s.captures.iter().filter(|c| c.extraction_ok).count(),
			captured_fail: s.captures.iter().filter(|c| !c.extraction_ok).count(),
			skipped: s.skipped.len(),
		}
	}
}
