use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MoodEvent {
	pub id: i64,
	pub index: i64,
	pub week: i64,
	pub label: String,
	pub description: String,
	pub team: String,
	pub category: String,
	pub delta: i64,
	pub mood: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMoodEvent {
	pub week: i64,
	pub label: String,
	pub description: String,
	pub team: String,
	pub category: String,
	pub delta: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMoodEvent {
	pub week: Option<i64>,
	pub label: Option<String>,
	pub description: Option<String>,
	pub team: Option<String>,
	pub category: Option<String>,
	pub delta: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoodStats {
	pub total_events: i64,
	pub min_mood: Option<i64>,
	pub max_mood: Option<i64>,
	pub avg_mood: Option<f64>,
	pub positive_events: i64,
	pub negative_events: i64,
	pub neutral_events: i64,
}
