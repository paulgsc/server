//! What a passkey looks like on the wire and at rest.
//!
//! The WebAuthn library's defaults are close to what docs/identity.md asks for
//! and differ in two places, both fixed here rather than trusted to a client:
//!
//! - **Registration options require a discoverable credential.** The library
//!   asks for a non-resident one, which would mean typing a username to sign
//!   in. [`registration_options`] asks for `residentKey: "required"`, so the
//!   browser lists the passkeys it holds for this site and sign-in needs no
//!   identifier at all.
//! - **Nothing identifying an authenticator is stored.** The options already
//!   say `attestation: "none"`, but a client can send attestation anyway, and
//!   the library keeps whatever it parsed, certificates included, and the
//!   transports the authenticator reported. [`for_storage`] drops both before
//!   a passkey reaches the database, so what is stored is the public key, its
//!   counter and backup flags, and nothing that names a device model.

use crate::FileHostError;
use auth_repo::StoredPasskey;
use serde_json::Value;
use webauthn_rs::prelude::{AttestationFormat, CreationChallengeResponse, Credential, ParsedAttestation, Passkey, RequestChallengeResponse};

/// The options a browser's `navigator.credentials.create` needs, as JSON.
///
/// # Errors
/// Serialization failure.
pub(crate) fn registration_options(options: &CreationChallengeResponse) -> Result<Value, FileHostError> {
	let mut json = serde_json::to_value(options)?;
	if let Some(selection) = json.pointer_mut("/publicKey/authenticatorSelection").and_then(Value::as_object_mut) {
		selection.insert(String::from("residentKey"), Value::from("required"));
		selection.insert(String::from("requireResidentKey"), Value::from(true));
	}
	Ok(json)
}

/// The options a browser's `navigator.credentials.get` needs, as JSON.
///
/// The library marks discoverable sign-in as conditional mediation (the
/// browser's autofill prompt), which needs a username field to attach to. The
/// app signs in from a button instead, so mediation is left to the default.
///
/// # Errors
/// Serialization failure.
pub(crate) fn sign_in_options(mut options: RequestChallengeResponse) -> Result<Value, FileHostError> {
	options.mediation = None;
	Ok(serde_json::to_value(&options)?)
}

/// A verified passkey, reduced to what a sign-in needs and serialised for the
/// database.
///
/// # Errors
/// Serialization failure.
pub(crate) fn for_storage(passkey: Passkey) -> Result<StoredPasskey, FileHostError> {
	let mut credential = Credential::from(passkey);
	credential.attestation = ParsedAttestation::default();
	credential.attestation_format = AttestationFormat::None;
	credential.transports = None;
	let credential_id = credential.cred_id.to_vec();
	// Disallowed for tracing; this is the stored column itself.
	#[allow(clippy::disallowed_methods)]
	let passkey = serde_json::to_string(&Passkey::from(credential))?;
	Ok(StoredPasskey { credential_id, passkey })
}

/// A stored passkey, back as the library's type.
///
/// # Errors
/// A row that does not deserialise, which would mean it was written by
/// something other than [`for_storage`].
pub(crate) fn from_storage(stored: &StoredPasskey) -> Result<Passkey, FileHostError> {
	Ok(serde_json::from_str(&stored.passkey)?)
}
