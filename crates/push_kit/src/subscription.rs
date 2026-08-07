use serde::{Deserialize, Serialize};

/// The `PushSubscription` shape a browser produces.
///
/// Accepted verbatim so a client can `JSON.stringify(subscription)` with no
/// reshaping. Browsers send more than these fields (`expirationTime`, and on
/// some engines an `options` block); serde drops them, which is the intended
/// reading — sending needs an address and two keys and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscription {
	pub endpoint: String,
	pub keys: SubscriptionKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionKeys {
	pub p256dh: String,
	pub auth: String,
}

/// Why a subscription could never be delivered to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidSubscription {
	/// Push endpoints are always `https:`. Anything else is a client bug or a
	/// forgery, and either way nothing can be sent to it.
	InsecureEndpoint,
	MissingKey(&'static str),
	MalformedKey(&'static str),
}

impl std::fmt::Display for InvalidSubscription {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::InsecureEndpoint => write!(f, "endpoint must be an absolute https: URL"),
			Self::MissingKey(name) => write!(f, "`keys.{name}` is missing"),
			Self::MalformedKey(name) => write!(f, "`keys.{name}` is not base64url"),
		}
	}
}

impl std::error::Error for InvalidSubscription {}

impl PushSubscription {
	/// Reject a subscription that could never be sent to.
	///
	/// Validating on the way *in* rather than at send time is the difference
	/// between a `4xx` a developer sees immediately and a row that fails
	/// silently every evening for a month.
	///
	/// # Errors
	/// Returns the first problem found.
	pub fn validate(&self) -> Result<(), InvalidSubscription> {
		use base64::engine::general_purpose::URL_SAFE_NO_PAD;
		use base64::Engine as _;

		if !self.endpoint.starts_with("https://") {
			return Err(InvalidSubscription::InsecureEndpoint);
		}

		for (name, key) in [("p256dh", &self.keys.p256dh), ("auth", &self.keys.auth)] {
			if key.is_empty() {
				return Err(InvalidSubscription::MissingKey(name));
			}
			if URL_SAFE_NO_PAD.decode(key.trim_end_matches('=')).is_err() {
				return Err(InvalidSubscription::MalformedKey(name));
			}
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{InvalidSubscription, PushSubscription, SubscriptionKeys};

	fn subscription(endpoint: &str, p256dh: &str, auth: &str) -> PushSubscription {
		PushSubscription {
			endpoint: endpoint.to_owned(),
			keys: SubscriptionKeys {
				p256dh: p256dh.to_owned(),
				auth: auth.to_owned(),
			},
		}
	}

	#[test]
	fn a_real_subscription_is_accepted() {
		let real = subscription(
			"https://updates.push.services.mozilla.com/wpush/v2/abc",
			"BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8",
			"xS03Fi5ErfTNH_l9WHE9Ig",
		);
		assert_eq!(real.validate(), Ok(()));
	}

	#[test]
	fn an_insecure_endpoint_is_refused() {
		let insecure = subscription("http://example.com/push", "BLMbF9ffKBiWQLCKvTHb6LO8", "xS03Fi5ErfTNH_l9WHE9Ig");
		assert_eq!(insecure.validate(), Err(InvalidSubscription::InsecureEndpoint));
	}

	#[test]
	fn a_missing_key_names_itself() {
		let missing = subscription("https://example.com/push", "", "xS03Fi5ErfTNH_l9WHE9Ig");
		assert_eq!(missing.validate(), Err(InvalidSubscription::MissingKey("p256dh")));
	}

	#[test]
	fn a_key_that_is_not_base64url_is_refused_now_rather_than_nightly_at_send_time() {
		let bad = subscription("https://example.com/push", "BLMbF9ffKBiWQLCKvTHb6LO8", "not base64!!");
		assert_eq!(bad.validate(), Err(InvalidSubscription::MalformedKey("auth")));
	}

	#[test]
	fn the_browsers_extra_fields_are_dropped_rather_than_refused() {
		let raw = r#"{
			"endpoint": "https://example.com/push",
			"expirationTime": null,
			"keys": { "p256dh": "abc", "auth": "def" }
		}"#;
		let parsed: PushSubscription = serde_json::from_str(raw).unwrap();
		assert_eq!(parsed.keys.auth, "def");
	}
}
