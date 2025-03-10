use regex::Regex; // You can use the `regex` crate for this
use serde::{Deserialize, Deserializer};

lazy_static::lazy_static! {
			static ref URL_REGEX: Regex = Regex::new(
							r"^(https?://)([a-zA-Z0-9.-]+\.[a-zA-Z]{2,})(/.*)?$"
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
