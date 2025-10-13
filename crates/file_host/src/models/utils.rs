use regex::Regex; // You can use the `regex` crate for this
use serde::{Deserialize, Deserializer};

lazy_static::lazy_static! {
	static ref URL_REGEX: Regex = Regex::new(
		r"^(https?://)([a-zA-Z0-9.-]+\.[a-zA-Z]{2,})(/.*)?$"
	).unwrap();
}

lazy_static::lazy_static! {
	static ref COLOR_REGEX: Regex = Regex::new(
		r"^#([A-Fa-f0-9]{6}|[A-Fa-f0-9]{3})$"
	).unwrap();
}

fn validate_url(url: &str) -> Result<(), String> {
	if URL_REGEX.is_match(url) {
		Ok(())
	} else {
		Err(format!("Invalid URL: {}", url))
	}
}

pub fn deserialize_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
	D: Deserializer<'de>,
{
	let s = String::deserialize(deserializer)?;
	validate_url(&s).map_err(serde::de::Error::custom)?;
	Ok(s)
}

fn validate_color(color: &str) -> Result<(), String> {
	if COLOR_REGEX.is_match(color) {
		Ok(())
	} else {
		Err(format!("Invalid COLOR: {}", color))
	}
}

pub fn deserialize_color<'de, D>(deserializer: D) -> Result<String, D::Error>
where
	D: Deserializer<'de>,
{
	let s = String::deserialize(deserializer)?;
	validate_color(&s).map_err(serde::de::Error::custom)?;
	Ok(s)
}
