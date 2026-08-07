use crate::identity::VapidIdentity;
use crate::outcome::SendOutcome;
use crate::subscription::PushSubscription;
use crate::transport::PushTransport;
use web_push::{ContentEncoding, SubscriptionInfo, WebPushMessageBuilder};

/// Push services cap payloads below this; `413 Payload Too Large` is the
/// failure. Checked before sending so an oversized body is a caller's bug
/// rather than a nightly mystery.
pub const MAX_PAYLOAD_BYTES: usize = 4096;

/// An encrypted, signed request, ready for any [`PushTransport`].
///
/// Plain data on purpose: no client type appears in it, which is what makes the
/// transport seam narrow enough to implement in a test in six lines.
#[derive(Debug, Clone)]
pub struct PreparedPush {
	pub endpoint: String,
	pub headers: Vec<(&'static str, String)>,
	pub body: Vec<u8>,
}

/// Encrypts and signs, then hands off.
///
/// Generic over the transport rather than holding a boxed one — see
/// [`PushTransport`] for why.
pub struct Sender<T> {
	identity: VapidIdentity,
	transport: T,
}

impl<T: PushTransport> Sender<T> {
	pub const fn new(identity: VapidIdentity, transport: T) -> Self {
		Self { identity, transport }
	}

	#[must_use]
	pub const fn identity(&self) -> &VapidIdentity {
		&self.identity
	}

	/// Encrypt per RFC 8291 and sign per RFC 8292.
	///
	/// Split from [`Self::deliver`] so the expensive, fallible half can be
	/// exercised — and its output inspected — without a network.
	///
	/// # Errors
	/// Returns [`SendOutcome::NotSent`] or [`SendOutcome::TooLarge`]. Both mean
	/// nothing was transmitted, which is why they are outcomes rather than a
	/// separate error type: the caller records them exactly as it records a
	/// failed send.
	pub fn prepare(&self, subscription: &PushSubscription, payload: &[u8]) -> Result<PreparedPush, SendOutcome> {
		if payload.len() > MAX_PAYLOAD_BYTES {
			return Err(SendOutcome::TooLarge);
		}

		let info = SubscriptionInfo::new(subscription.endpoint.clone(), subscription.keys.p256dh.clone(), subscription.keys.auth.clone());

		let mut signature = self.identity.signer().add_sub_info(&info);
		signature.add_claim("sub", self.identity.subject());
		let signature = signature.build().map_err(|err| SendOutcome::NotSent(err.to_string()))?;

		let mut builder = WebPushMessageBuilder::new(&info);
		builder.set_payload(ContentEncoding::Aes128Gcm, payload);
		builder.set_vapid_signature(signature);

		let message = builder.build().map_err(|err| SendOutcome::NotSent(err.to_string()))?;
		let encrypted = message.payload.ok_or_else(|| SendOutcome::NotSent("message built without a payload".to_owned()))?;

		let mut headers = vec![
			("TTL", message.ttl.to_string()),
			("Content-Encoding", encrypted.content_encoding.to_str().to_owned()),
			("Content-Type", "application/octet-stream".to_owned()),
		];
		headers.extend(encrypted.crypto_headers);

		Ok(PreparedPush {
			endpoint: message.endpoint.to_string(),
			headers,
			body: encrypted.content,
		})
	}

	/// Prepare and send.
	///
	/// Never returns `Err`: every way this can fail is a [`SendOutcome`] the
	/// caller has to record anyway, and an error type alongside it would just be
	/// a second spelling of the same thing.
	pub async fn deliver(&self, subscription: &PushSubscription, payload: &[u8]) -> SendOutcome {
		let request = match self.prepare(subscription, payload) {
			Ok(request) => request,
			Err(outcome) => return outcome,
		};

		match self.transport.deliver(request).await {
			Err(err) => SendOutcome::Transport(err.to_string()),
			Ok(response) => SendOutcome::from_status(response.status, response.retry_after, response.body),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{PreparedPush, Sender, MAX_PAYLOAD_BYTES};
	use crate::identity::VapidIdentity;
	use crate::outcome::SendOutcome;
	use crate::subscription::{PushSubscription, SubscriptionKeys};
	use crate::transport::{PushResponse, PushTransport};
	use std::sync::Mutex;

	const PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";

	/// Six lines, no HTTP stack, no `dyn`. The point of the seam.
	struct Recorder {
		status: u16,
		seen: Mutex<Vec<PreparedPush>>,
	}

	impl PushTransport for Recorder {
		type Error = std::convert::Infallible;

		async fn deliver(&self, request: PreparedPush) -> Result<PushResponse, Self::Error> {
			self.seen.lock().unwrap().push(request);
			Ok(PushResponse {
				status: self.status,
				retry_after: None,
				body: String::new(),
			})
		}
	}

	fn identity() -> VapidIdentity {
		use base64::engine::general_purpose::URL_SAFE_NO_PAD;
		use base64::Engine as _;
		let derived = URL_SAFE_NO_PAD.encode(web_push::VapidSignatureBuilder::from_base64_no_sub(PRIVATE).unwrap().get_public_key());
		VapidIdentity::from_config(Some(PRIVATE), Some(&derived), "mailto:a@example.com").unwrap()
	}

	fn subscription() -> PushSubscription {
		PushSubscription {
			endpoint: "https://updates.push.services.mozilla.com/wpush/v2/abc".to_owned(),
			keys: SubscriptionKeys {
				p256dh: "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8".to_owned(),
				auth: "xS03Fi5ErfTNH_l9WHE9Ig".to_owned(),
			},
		}
	}

	#[test]
	fn a_prepared_push_carries_the_encryption_and_authorization_headers() {
		let sender = Sender::new(
			identity(),
			Recorder {
				status: 201,
				seen: Mutex::new(Vec::new()),
			},
		);
		let request = sender.prepare(&subscription(), b"{}").unwrap();

		let names: Vec<&str> = request.headers.iter().map(|(name, _)| *name).collect();
		assert!(names.contains(&"Content-Encoding"));
		assert!(names.contains(&"TTL"));
		assert!(names.iter().any(|name| name.eq_ignore_ascii_case("authorization")), "got {names:?}");
		// The ciphertext is not the plaintext — the one property that, if it
		// silently stopped holding, would still return 201 forever.
		assert_ne!(request.body, b"{}");
	}

	#[test]
	fn an_oversized_payload_never_reaches_the_transport() {
		let sender = Sender::new(
			identity(),
			Recorder {
				status: 201,
				seen: Mutex::new(Vec::new()),
			},
		);
		let outcome = sender.prepare(&subscription(), &vec![b'x'; MAX_PAYLOAD_BYTES + 1]).unwrap_err();
		assert_eq!(outcome, SendOutcome::TooLarge);
	}

	#[tokio::test]
	async fn a_410_becomes_expired_and_the_request_was_still_made() {
		let sender = Sender::new(
			identity(),
			Recorder {
				status: 410,
				seen: Mutex::new(Vec::new()),
			},
		);
		assert_eq!(sender.deliver(&subscription(), b"{}").await, SendOutcome::Expired);
	}
}
