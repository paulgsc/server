use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabsBody<T> {
	browser_tab: T,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserTabs {
	pub id: i32,
	pub status: Option<String>,
	#[serde(rename = "index")]
	pub tab_index: i32,
	pub opener_tab_id: Option<i32>,
	pub title: Option<String>,
	pub url: Option<String>,
	pub pending_url: Option<String>,
	pub pinned: Option<bool>,
	pub highlighted: Option<bool>,
	pub window_id: i32,
	pub active: Option<bool>,
	pub favicon_url: Option<String>,
	pub incognito: Option<bool>,
	pub selected: Option<bool>,
	pub audible: Option<bool>,
	pub discarded: Option<bool>,
	pub auto_discardable: Option<bool>,
	pub width: Option<i32>,
	pub height: Option<i32>,
	pub session_id: Option<String>,
	pub group_id: i32,
	pub last_accessed: Option<NaiveDateTime>,
}
