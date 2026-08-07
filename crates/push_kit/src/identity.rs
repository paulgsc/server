//! VAPID identity — one keypair, and the care it needs.
//!
//! VAPID (RFC 8292) is how a push service knows the sender is the one the
//! browser subscribed to. One ECDSA P-256 keypair identifies this deployment:
//! the public key goes to the browser at subscribe time as
//! `applicationServerKey`, and the private key signs every send.
//!
//! ## Why this is a module and not a line of setup
//!
//! **The public key is baked into every subscription made with it.** Rotating
//! it does not re-key those subscriptions — it silently invalidates all of
//! them, and the failure presents as "notifications just stopped" with a `403`
//! buried in a log. Recovery is every browser visiting the site and
//! subscribing again; there is no server-side fix. So the keys are validated at
//! startup, together, and a mismatched pair is a boot failure rather than a
//! discovery made hours later at the first send.
//!
//! ## Generating a pair
//!
//! Either of these produces the base64url pair the config wants:
//!
//! ```text
//! npx web-push generate-vapid-keys
//! ```
//!
//! ```text
//! openssl ecparam -name prime256v1 -genkey -noout -out vapid_private.pem
//! openssl ec -in vapid_private.pem -outform DER | tail -c +8 | head -c 32 \
//!   | base64 | tr -d '=' | tr '/+' '_-'                       # VAPID_PRIVATE_KEY
//! openssl ec -in vapid_private.pem -pubout -outform DER | tail -c 65 \
//!   | base64 | tr -d '=' | tr '/+' '_-'                       # VAPID_PUBLIC_KEY
//! ```
//!
//! The PEM is a secret. Keep it outside the repository — `.gitignore` covers
//! `*.pem` — and put only the base64url strings wherever this crate's host
//! reads its configuration.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use web_push::{PartialVapidSignatureBuilder, VapidSignatureBuilder};

/// A validated VAPID identity, ready to sign.
#[derive(Clone)]
pub struct VapidIdentity {
	signer: PartialVapidSignatureBuilder,
	public_key: String,
	subject: String,
}

/// Written by hand rather than derived, and the reason is the omission: the
/// signer holds the private key, and a derived `Debug` would put it into the
/// first `tracing::error!(?identity)` anyone reaches for.
#[allow(clippy::missing_fields_in_debug)] // The omission is the point.
impl std::fmt::Debug for VapidIdentity {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("VapidIdentity")
			.field("public_key", &self.public_key)
			.field("subject", &self.subject)
			.field("private_key", &"<redacted>")
			.finish()
	}
}

#[derive(Debug)]
pub enum VapidError {
	MissingPrivateKey,
	MissingPublicKey,
	UnreadablePrivateKey,
	/// The configured public key is not the one this private key derives.
	/// Every existing subscription was made with one of them; sending with the
	/// other returns `403` on all of them.
	KeyPairMismatch {
		configured: String,
		derived: String,
	},
	/// The `sub` claim must be a `mailto:` or an `https:` URL. Push services
	/// reject or deprioritise senders without a usable one.
	InvalidSubject(String),
}

impl std::fmt::Display for VapidError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::MissingPrivateKey => write!(f, "VAPID_PRIVATE_KEY is not set; the nudge cannot sign anything"),
			Self::MissingPublicKey => write!(f, "VAPID_PUBLIC_KEY is not set; the browser has nothing to subscribe with"),
			Self::UnreadablePrivateKey => write!(f, "VAPID_PRIVATE_KEY is not a base64url-encoded P-256 private key"),
			Self::KeyPairMismatch { configured, derived } => write!(
				f,
				"VAPID_PUBLIC_KEY does not belong to VAPID_PRIVATE_KEY (configured {configured}, derived {derived}); \
				 every existing subscription was made with one of these and would fail with 403"
			),
			Self::InvalidSubject(subject) => write!(f, "VAPID_SUBJECT must be a mailto: or https: URL, got {subject}"),
		}
	}
}

impl std::error::Error for VapidError {}

impl VapidIdentity {
	/// Validate the configured keypair.
	///
	/// # Errors
	/// Returns a [`VapidError`] if either key is missing, the private key does
	/// not decode, the pair does not match, or the subject is unusable.
	pub fn from_config(private_key: Option<&str>, public_key: Option<&str>, subject: &str) -> Result<Self, VapidError> {
		let private_key = private_key.filter(|key| !key.is_empty()).ok_or(VapidError::MissingPrivateKey)?;
		let public_key = public_key.filter(|key| !key.is_empty()).ok_or(VapidError::MissingPublicKey)?;

		if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
			return Err(VapidError::InvalidSubject(subject.to_owned()));
		}

		let signer = VapidSignatureBuilder::from_base64_no_sub(private_key).map_err(|_| VapidError::UnreadablePrivateKey)?;

		// The check that earns this module its existence. A pair that does not
		// match is indistinguishable from a working one until a browser fails
		// to receive something, hours or days later.
		let derived = URL_SAFE_NO_PAD.encode(signer.get_public_key());
		let configured = public_key.trim_end_matches('=').to_owned();
		if derived != configured {
			return Err(VapidError::KeyPairMismatch { configured, derived });
		}

		Ok(Self {
			signer,
			public_key: derived,
			subject: subject.to_owned(),
		})
	}

	/// The `applicationServerKey` the browser subscribes with, base64url.
	#[must_use]
	pub fn public_key(&self) -> &str {
		&self.public_key
	}

	#[must_use]
	pub fn subject(&self) -> &str {
		&self.subject
	}

	#[must_use]
	pub fn signer(&self) -> PartialVapidSignatureBuilder {
		self.signer.clone()
	}
}

#[cfg(test)]
mod tests {
	use super::{VapidError, VapidIdentity};

	/// The test vector from the `web-push` crate's own suite.
	const PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";

	fn derived_public() -> String {
		// Deliberately derived rather than pasted, so this test cannot drift
		// from the encoding `from_config` actually compares against.
		use base64::engine::general_purpose::URL_SAFE_NO_PAD;
		use base64::Engine as _;
		let signer = web_push::VapidSignatureBuilder::from_base64_no_sub(PRIVATE).unwrap();
		URL_SAFE_NO_PAD.encode(signer.get_public_key())
	}

	#[test]
	fn a_matching_pair_is_accepted() {
		let identity = VapidIdentity::from_config(Some(PRIVATE), Some(&derived_public()), "mailto:a@example.com").unwrap();
		assert_eq!(identity.public_key(), derived_public());
	}

	#[test]
	fn a_mismatched_pair_fails_at_startup_rather_than_at_the_first_send() {
		let wrong = "BLMbF9ffKBiWQLCKvTHb6LO8Nb6dcUh6TItC455vu2kElga6PQvUmaFyCdykxY2nOSSL3yKgfbmFLRTUaGv4yV8";
		let err = VapidIdentity::from_config(Some(PRIVATE), Some(wrong), "mailto:a@example.com").unwrap_err();
		assert!(matches!(err, VapidError::KeyPairMismatch { .. }));
	}

	#[test]
	fn a_missing_private_key_is_named_precisely() {
		let err = VapidIdentity::from_config(None, Some(&derived_public()), "mailto:a@example.com").unwrap_err();
		assert!(matches!(err, VapidError::MissingPrivateKey));

		let err = VapidIdentity::from_config(Some(""), Some(&derived_public()), "mailto:a@example.com").unwrap_err();
		assert!(matches!(err, VapidError::MissingPrivateKey));
	}

	#[test]
	fn an_unreadable_private_key_does_not_panic() {
		let err = VapidIdentity::from_config(Some("not a key"), Some(&derived_public()), "mailto:a@example.com").unwrap_err();
		assert!(matches!(err, VapidError::UnreadablePrivateKey));
	}

	#[test]
	fn a_subject_push_services_would_reject_is_refused_here() {
		let err = VapidIdentity::from_config(Some(PRIVATE), Some(&derived_public()), "admin@example.com").unwrap_err();
		assert!(matches!(err, VapidError::InvalidSubject(_)));
	}
}
