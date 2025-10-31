use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UtterancePrompt {
	pub text: String,
	pub metadata: UtteranceMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ElementInfo {
	pub tag_name: String,
	#[serde(rename = "type")]
	pub element_type: Option<String>,
	pub id: Option<String>,
	pub name: Option<String>,
	pub class_name: Option<String>,
	pub placeholder: Option<String>,
	#[serde(rename = "formAction")]
	pub form_action: Option<String>,
	#[serde(rename = "formMethod")]
	pub form_method: Option<String>,
	#[serde(rename = "formId")]
	pub form_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UtteranceMetadata {
	pub url: String,
	pub domain: String,
	pub title: String,
	pub timestamp: String,
	pub element: ElementInfo,
}
