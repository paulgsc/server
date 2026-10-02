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

/// The push services of the browsers' own vendors: Chrome, Edge, Brave and
/// Opera (FCM), Firefox, and Safari. A subscription on any of these is logged
/// under this host name; see [`PushSubscription::service`].
const KNOWN_SERVICES: [&str; 3] = ["fcm.googleapis.com", "updates.push.services.mozilla.com", "web.push.apple.com"];

/// Windows Notification Service endpoints are regional subdomains of this.
const WINDOWS_SUFFIX: &str = ".notify.windows.com";
const WINDOWS_SERVICE: &str = "notify.windows.com";

/// What [`PushSubscription::service`] says for any provider it does not
/// recognise, whatever the endpoint's host is.
pub const OTHER_SERVICE: &str = "other";

impl PushSubscription {
	/// The push service this subscription belongs to, as a fixed label, and
	/// nothing the client chose.
	///
	/// This is what a log line or a metric may name. The endpoint itself is a
	/// per-browser address: it is stable for one install, whoever holds it can
	/// send to that browser, and the service that issued it can tie it to that
	/// install. `validate()` accepts any `https://` host, so even the host can
	/// be a client's own choice (a device-specific name, a token in a
	/// subdomain). So only a *recognised* provider is named, by one of a fixed
	/// set of labels; anything else, including a lookalike such as
	/// `fcm.googleapis.com.example.test`, is [`OTHER_SERVICE`]. That says which
	/// provider answered when it is one of the browsers' own, which is what an
	/// operator reading a failure needs, and nothing more.
	#[must_use]
	pub fn service(&self) -> &'static str {
		let rest = self.endpoint.split_once("://").map_or(self.endpoint.as_str(), |(_, rest)| rest);
		let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
		let authority = authority.rsplit('@').next().unwrap_or_default();
		// A port, and an IPv6 literal (never a provider), are not part of the name.
		let host = authority
			.rsplit_once(':')
			.map_or(authority, |(host, port)| if port.bytes().all(|byte| byte.is_ascii_digit()) { host } else { authority });
		if let Some(known) = KNOWN_SERVICES.iter().find(|known| known.eq_ignore_ascii_case(host)) {
			return known;
		}
		if host.len() > WINDOWS_SUFFIX.len() && host.as_bytes()[host.len() - WINDOWS_SUFFIX.len()..].eq_ignore_ascii_case(WINDOWS_SUFFIX.as_bytes()) {
			return WINDOWS_SERVICE;
		}
		OTHER_SERVICE
	}

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
	fn the_service_is_a_recognised_provider_and_never_what_the_client_chose() {
		let keys = ("BLMbF9ffKBiWQLCKvTHb6LO8", "xS03Fi5ErfTNH_l9WHE9Ig");
		for (endpoint, service) in [
			// The browsers' own providers, whatever follows the host.
			(
				"https://updates.push.services.mozilla.com/wpush/v2/gAAAAB-per-browser-token",
				"updates.push.services.mozilla.com",
			),
			("https://fcm.googleapis.com/fcm/send/abc:APA91b?x=1#frag", "fcm.googleapis.com"),
			("https://web.push.apple.com/QGZ-token", "web.push.apple.com"),
			("https://wns2-par02p.notify.windows.com/w/?token=secret", "notify.windows.com"),
			// Port, user info and case do not change the provider.
			("https://user:secret@fcm.googleapis.com:443/p", "fcm.googleapis.com"),
			("https://FCM.GoogleAPIs.com/p", "fcm.googleapis.com"),
			// Anything else is one fixed word: a device-specific host, a token in a
			// subdomain, a lookalike, a port that is not one, an IP, no host at all.
			("https://SECRET-DEVICE.push.example/path", "other"),
			("https://push.example.test:8443/p", "other"),
			("https://fcm.googleapis.com.example.test/p", "other"),
			("https://example.test/fcm.googleapis.com", "other"),
			("https://127.0.0.1:3000/wpush/v2/token", "other"),
			("https://[::1]:3000/p", "other"),
			("https://notify.windows.com.example.test/p", "other"),
			("https://.notify.windows.com/p", "other"),
			// Not ASCII: a byte offset into these falls inside a character, and
			// `service()` runs on whatever a client stored (a panic here would take
			// down the waker task).
			("https://éééééééééé", "other"),
			("https://éééééééééééééééééééé/p", "other"),
			("https://日本語日本語日本語日本語.example/p", "other"),
			("https://é.notify.windows.com/p", "notify.windows.com"),
			("https://ééééééééééé.notifY.windows.cöm/p", "other"),
			("push.example.test/p", "other"),
			("", "other"),
		] {
			let subscription = subscription(endpoint, keys.0, keys.1);
			assert_eq!(subscription.service(), service, "{endpoint}");
			let named = subscription.service();
			assert!(
				!named.contains("per-browser") && !named.contains("secret") && !named.contains("SECRET") && !named.contains("example"),
				"{endpoint} -> {named}"
			);
		}
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
