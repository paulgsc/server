//! Firebase Cloud Messaging actuation, and nothing else.
//!
//! This crate knows how to prove who is sending (a service account's OAuth
//! token, from an RS256 assertion) and how to hand FCM's HTTP v1 API one
//! notice for one device. It does not know what a morning run is, what
//! decided to send, or which devices asked: those are `file_host`'s
//! `jobs_notice` and `notice_repo`.
//!
//! ## The seam
//!
//! [`Sender`] turns `(device, notice)` into [`Request`]s, and an
//! [`FcmTransport`] puts each on the wire. As in `push_kit` and
//! `mailbox_kit`, the transport is a generic parameter, never a trait object.
//!
//! ## What never prints
//!
//! A [`DeviceToken`] names one phone to Google, and an access token is a
//! credential; both `Debug` as `[redacted]`, and no [`SendOutcome`] carries
//! either. The service account's private key never leaves [`ServiceAccount`].
//!
//! ## What is deliberately absent
//!
//! **The words.** [`Notice`] carries a title, a body and a tag the caller
//! chose. **Any notion of when.** [`Sender::send`] sends when told to.

pub mod transport;

pub use transport::{Bearer, FcmTransport, Request, Response};

#[cfg(feature = "reqwest-transport")]
pub use transport::reqwest_transport::ReqwestTransport;

use jwt_simple::prelude::{Claims, Duration, RS256KeyPair, RSAKeyPairLike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

/// The scope FCM's send API asks for.
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// How long an assertion asks a token to last: Google's ceiling.
const ASSERTION_HOURS: u64 = 1;

/// A token this close to expiring is fetched afresh rather than used.
const REFRESH_MARGIN_SECS: u64 = 300;

/// Why a service-account key file can't be used. Says which part, never the
/// key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
	/// Not a service account's JSON key.
	NotAKeyFile,
	/// A field the key file must carry is empty.
	Missing(&'static str),
	/// `private_key` is not an RSA key in PEM.
	PrivateKey,
}

impl fmt::Display for KeyError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NotAKeyFile => f.write_str("not a service account's JSON key"),
			Self::Missing(field) => write!(f, "the key file has no {field}"),
			Self::PrivateKey => f.write_str("the key file's private_key is not an RSA key in PEM"),
		}
	}
}

impl std::error::Error for KeyError {}

#[derive(Deserialize)]
struct KeyFile {
	#[serde(rename = "type")]
	kind: String,
	project_id: String,
	client_email: String,
	private_key: String,
	token_uri: String,
}

/// Who sends: a Google service account allowed to send through FCM.
pub struct ServiceAccount {
	project_id: String,
	client_email: String,
	token_uri: String,
	key: RS256KeyPair,
}

impl fmt::Debug for ServiceAccount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ServiceAccount").field("project_id", &self.project_id).finish_non_exhaustive()
	}
}

impl ServiceAccount {
	/// Read a service account's JSON key, as Google's console downloads it.
	///
	/// # Errors
	/// [`KeyError`] when it is not such a key, lacks a field, or its private
	/// key does not parse.
	pub fn from_json(text: &str) -> Result<Self, KeyError> {
		let file: KeyFile = serde_json::from_str(text).map_err(|_| KeyError::NotAKeyFile)?;
		if file.kind != "service_account" {
			return Err(KeyError::NotAKeyFile);
		}
		for (field, value) in [("project_id", &file.project_id), ("client_email", &file.client_email), ("token_uri", &file.token_uri)] {
			if value.trim().is_empty() {
				return Err(KeyError::Missing(field));
			}
		}
		let key = RS256KeyPair::from_pem(&file.private_key).map_err(|_| KeyError::PrivateKey)?;
		Ok(Self {
			project_id: file.project_id,
			client_email: file.client_email,
			token_uri: file.token_uri,
			key,
		})
	}

	/// The Firebase project sends go to.
	#[must_use]
	pub fn project_id(&self) -> &str {
		&self.project_id
	}

	/// A signed assertion asking `token_uri` for an access token.
	fn assertion(&self) -> Option<String> {
		#[derive(Serialize, Deserialize)]
		struct Scope {
			scope: String,
		}
		let claims = Claims::with_custom_claims(Scope { scope: SCOPE.to_owned() }, Duration::from_hours(ASSERTION_HOURS))
			.with_issuer(&self.client_email)
			.with_audience(&self.token_uri);
		self.key.sign(claims).ok()
	}
}

/// One phone, as FCM names it. Never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceToken(String);

impl DeviceToken {
	#[must_use]
	pub const fn new(value: String) -> Self {
		Self(value)
	}

	/// The token itself, for the message's `token` field and nothing else.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl fmt::Debug for DeviceToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[redacted]")
	}
}

/// What one notification says, and what it carries for the app.
#[derive(Debug, Clone, Copy)]
pub struct Notice<'a> {
	pub title: &'a str,
	pub body: &'a str,
	/// Replaces an earlier notification with the same tag on the phone.
	pub tag: &'a str,
	/// Key-value pairs the app reads when it is opened from the notice.
	pub data: &'a [(&'a str, &'a str)],
}

/// What came of one send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
	/// FCM took it.
	Delivered,
	/// FCM no longer knows the device: the app was removed, or its token
	/// changed. Forget the token.
	Unregistered,
	/// FCM, or Google's token endpoint, refused with this status.
	Refused(u16),
	/// The request never got an answer; the text is the transport's, which
	/// names no token.
	Transport(String),
}

impl SendOutcome {
	/// A label for logs and metrics.
	#[must_use]
	pub const fn as_str(&self) -> &'static str {
		match self {
			Self::Delivered => "delivered",
			Self::Unregistered => "unregistered",
			Self::Refused(_) => "refused",
			Self::Transport(_) => "transport",
		}
	}
}

#[derive(Deserialize)]
struct TokenAnswer {
	access_token: String,
	expires_in: u64,
}

struct Cached {
	bearer: Bearer,
	expires_at: u64,
}

/// Sends notices as one service account, keeping its access token until it
/// is near expiry.
pub struct Sender<T> {
	account: ServiceAccount,
	transport: T,
	token: Mutex<Option<Cached>>,
}

impl<T: FcmTransport> Sender<T> {
	#[must_use]
	pub const fn new(account: ServiceAccount, transport: T) -> Self {
		Self {
			account,
			transport,
			token: Mutex::new(None),
		}
	}

	/// Hand FCM `notice` for `device`, `now` being the Unix time in seconds.
	pub async fn send(&self, device: &DeviceToken, notice: &Notice<'_>, now: u64) -> SendOutcome {
		let bearer = match self.bearer(now).await {
			Ok(bearer) => bearer,
			Err(outcome) => return outcome,
		};
		let request = Request {
			url: String::from("https://fcm.googleapis.com/v1/projects/") + &self.account.project_id + "/messages:send",
			bearer: Some(bearer),
			content_type: "application/json",
			body: message(device, notice),
		};
		match self.transport.send(request).await {
			Err(err) => SendOutcome::Transport(err.to_string()),
			Ok(response) if (200..300).contains(&response.status) => SendOutcome::Delivered,
			Ok(response) if response.status == 404 || unregistered(&response.body) => SendOutcome::Unregistered,
			Ok(response) => {
				if response.status == 401 {
					// The token was refused: fetch another next time.
					self.forget_token();
				}
				SendOutcome::Refused(response.status)
			}
		}
	}

	fn forget_token(&self) {
		if let Ok(mut cached) = self.token.lock() {
			*cached = None;
		}
	}

	/// The cached access token, or a fresh one from `token_uri`.
	async fn bearer(&self, now: u64) -> Result<Bearer, SendOutcome> {
		if let Ok(cached) = self.token.lock() {
			if let Some(cached) = cached.as_ref().filter(|cached| now + REFRESH_MARGIN_SECS < cached.expires_at) {
				return Ok(cached.bearer.clone());
			}
		}
		let Some(assertion) = self.account.assertion() else {
			return Err(SendOutcome::Refused(0));
		};
		let form = String::from("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion=") + &assertion;
		let request = Request {
			url: self.account.token_uri.clone(),
			bearer: None,
			content_type: "application/x-www-form-urlencoded",
			body: form.into_bytes(),
		};
		let response = self.transport.send(request).await.map_err(|err| SendOutcome::Transport(err.to_string()))?;
		if !(200..300).contains(&response.status) {
			return Err(SendOutcome::Refused(response.status));
		}
		let answer: TokenAnswer = serde_json::from_slice(&response.body).map_err(|_| SendOutcome::Refused(response.status))?;
		let bearer = Bearer::new(answer.access_token);
		if let Ok(mut cached) = self.token.lock() {
			*cached = Some(Cached {
				bearer: bearer.clone(),
				expires_at: now + answer.expires_in,
			});
		}
		Ok(bearer)
	}
}

/// FCM v1's message for `device`: a notification Android shows, high
/// priority so it arrives while the phone dozes, with `tag` replacing an
/// earlier one.
fn message(device: &DeviceToken, notice: &Notice<'_>) -> Vec<u8> {
	let data: BTreeMap<&str, &str> = notice.data.iter().copied().collect();
	serde_json::json!({
		"message": {
			"token": device.expose(),
			"notification": { "title": notice.title, "body": notice.body },
			"android": {
				"priority": "high",
				"collapse_key": notice.tag,
				"notification": { "tag": notice.tag },
			},
			"data": data,
		}
	})
	.to_string()
	.into_bytes()
}

/// Whether an FCM error body says the device is gone (`UNREGISTERED`).
fn unregistered(body: &[u8]) -> bool {
	let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
		return false;
	};
	value["error"]["details"]
		.as_array()
		.is_some_and(|details| details.iter().any(|detail| detail["errorCode"] == "UNREGISTERED"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use jwt_simple::prelude::{RSAPublicKeyLike, VerificationOptions};
	use std::collections::{HashSet, VecDeque};
	use std::sync::Arc;

	#[derive(Clone, Default)]
	struct Fake {
		answers: Arc<Mutex<VecDeque<Result<Response, String>>>>,
		seen: Arc<Mutex<Vec<Request>>>,
	}

	impl Fake {
		fn answer(&self, status: u16, body: &str) {
			self.answers.lock().unwrap().push_back(Ok(Response {
				status,
				body: body.as_bytes().to_vec(),
			}));
		}

		fn fail(&self) {
			self.answers.lock().unwrap().push_back(Err(String::from("connection reset")));
		}

		fn urls(&self) -> Vec<String> {
			self.seen.lock().unwrap().iter().map(|request| request.url.clone()).collect()
		}
	}

	impl FcmTransport for Fake {
		type Error = String;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			self.seen.lock().unwrap().push(request);
			self.answers.lock().unwrap().pop_front().unwrap()
		}
	}

	const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
	const SEND_URL: &str = "https://fcm.googleapis.com/v1/projects/some-ui/messages:send";

	fn key_file(key: &RS256KeyPair) -> String {
		serde_json::json!({
			"type": "service_account",
			"project_id": "some-ui",
			"client_email": "fcm@some-ui.iam.gserviceaccount.com",
			"private_key": key.to_pem().unwrap(),
			"token_uri": TOKEN_URI,
		})
		.to_string()
	}

	fn sender(key: &RS256KeyPair, fake: &Fake) -> Sender<Fake> {
		Sender::new(ServiceAccount::from_json(&key_file(key)).unwrap(), fake.clone())
	}

	const NOTICE: Notice<'static> = Notice {
		title: "This morning's picks are at home",
		body: "Open Jobs on the home Wi-Fi to see them.",
		tag: "jobs-run",
		data: &[("open", "/jobs")],
	};

	fn device() -> DeviceToken {
		DeviceToken::new(String::from("device-token-abc"))
	}

	#[test]
	fn a_key_file_must_be_a_service_accounts() {
		let key = RS256KeyPair::generate(2048).unwrap();
		assert_eq!(ServiceAccount::from_json("{}").unwrap_err(), KeyError::NotAKeyFile);
		let mut file: serde_json::Value = serde_json::from_str(&key_file(&key)).unwrap();
		file["type"] = "authorized_user".into();
		assert_eq!(ServiceAccount::from_json(&file.to_string()).unwrap_err(), KeyError::NotAKeyFile);
		file["type"] = "service_account".into();
		file["client_email"] = "".into();
		assert_eq!(ServiceAccount::from_json(&file.to_string()).unwrap_err(), KeyError::Missing("client_email"));
		file["client_email"] = "fcm@some-ui.iam.gserviceaccount.com".into();
		file["private_key"] = "-----BEGIN PRIVATE KEY-----\nnot a key\n-----END PRIVATE KEY-----".into();
		assert_eq!(ServiceAccount::from_json(&file.to_string()).unwrap_err(), KeyError::PrivateKey);
	}

	#[test]
	fn nothing_secret_prints() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let account = ServiceAccount::from_json(&key_file(&key)).unwrap();
		let printed = [
			std::fmt::format(format_args!("{account:?}")),
			std::fmt::format(format_args!("{:?}", device())),
			std::fmt::format(format_args!("{:?}", Bearer::new(String::from("ya29.secret")))),
		]
		.concat();
		assert!(
			!printed.contains("PRIVATE KEY") && !printed.contains("device-token-abc") && !printed.contains("ya29"),
			"{printed}"
		);
	}

	#[tokio::test]
	async fn a_send_gets_a_token_once_then_reuses_it_until_near_expiry() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let fake = Fake::default();
		let sender = sender(&key, &fake);
		fake.answer(200, r#"{"access_token":"ya29.first","expires_in":3600}"#);
		fake.answer(200, r#"{"name":"projects/some-ui/messages/1"}"#);
		fake.answer(200, r#"{"name":"projects/some-ui/messages/2"}"#);
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Delivered);
		assert_eq!(sender.send(&device(), &NOTICE, 1_000 + 3_000).await, SendOutcome::Delivered);
		assert_eq!(fake.urls(), [TOKEN_URI, SEND_URL, SEND_URL]);

		// Within five minutes of expiry, a fresh token first.
		fake.answer(200, r#"{"access_token":"ya29.second","expires_in":3600}"#);
		fake.answer(200, r#"{"name":"projects/some-ui/messages/3"}"#);
		assert_eq!(sender.send(&device(), &NOTICE, 1_000 + 3_400).await, SendOutcome::Delivered);
		let seen = std::mem::take(&mut *fake.seen.lock().unwrap());
		assert_eq!(seen[3].url, TOKEN_URI);
		assert_eq!(seen[4].bearer.as_ref().unwrap().expose(), "ya29.second");
	}

	#[tokio::test]
	async fn the_assertion_is_signed_by_the_key_and_names_the_account_and_the_scope() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let fake = Fake::default();
		fake.answer(200, r#"{"access_token":"ya29.first","expires_in":3600}"#);
		fake.answer(200, "{}");
		sender(&key, &fake).send(&device(), &NOTICE, 1_000).await;
		let seen = std::mem::take(&mut *fake.seen.lock().unwrap());
		let form = String::from_utf8(seen[0].body.clone()).unwrap();
		let assertion = form.split("assertion=").nth(1).unwrap();
		assert!(form.starts_with("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&"));
		let options = VerificationOptions {
			allowed_issuers: Some(HashSet::from([String::from("fcm@some-ui.iam.gserviceaccount.com")])),
			allowed_audiences: Some(HashSet::from([String::from(TOKEN_URI)])),
			..VerificationOptions::default()
		};
		let claims = key.public_key().verify_token::<serde_json::Value>(assertion, Some(options)).unwrap();
		assert_eq!(claims.custom["scope"], SCOPE);
	}

	#[tokio::test]
	async fn the_message_names_the_device_and_carries_the_notice() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let fake = Fake::default();
		fake.answer(200, r#"{"access_token":"ya29.first","expires_in":3600}"#);
		fake.answer(200, "{}");
		sender(&key, &fake).send(&device(), &NOTICE, 1_000).await;
		let seen = std::mem::take(&mut *fake.seen.lock().unwrap());
		assert_eq!(seen[1].bearer.as_ref().unwrap().expose(), "ya29.first");
		let body: serde_json::Value = serde_json::from_slice(&seen[1].body).unwrap();
		assert_eq!(
			body,
			serde_json::json!({ "message": {
				"token": "device-token-abc",
				"notification": { "title": NOTICE.title, "body": NOTICE.body },
				"android": { "priority": "high", "collapse_key": "jobs-run", "notification": { "tag": "jobs-run" } },
				"data": { "open": "/jobs" },
			}})
		);
	}

	#[tokio::test]
	async fn each_answer_maps_to_an_outcome() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let fake = Fake::default();
		let sender = sender(&key, &fake);
		fake.answer(200, r#"{"access_token":"ya29.first","expires_in":3600}"#);
		fake.answer(404, "{}");
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Unregistered);
		fake.answer(
			400,
			r#"{"error":{"details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#,
		);
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Unregistered);
		fake.answer(400, r#"{"error":{"details":[{"errorCode":"INVALID_ARGUMENT"}]}}"#);
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Refused(400));
		fake.fail();
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Transport(String::from("connection reset")));

		// A refused token is dropped, so the next send asks for another.
		fake.answer(401, "{}");
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Refused(401));
		fake.answer(200, r#"{"access_token":"ya29.second","expires_in":3600}"#);
		fake.answer(200, "{}");
		assert_eq!(sender.send(&device(), &NOTICE, 1_000).await, SendOutcome::Delivered);
		assert_eq!(fake.urls().iter().filter(|url| *url == TOKEN_URI).count(), 2);
	}

	#[tokio::test]
	async fn a_token_endpoint_that_refuses_sends_nothing() {
		let key = RS256KeyPair::generate(2048).unwrap();
		let fake = Fake::default();
		fake.answer(400, r#"{"error":"invalid_grant"}"#);
		assert_eq!(sender(&key, &fake).send(&device(), &NOTICE, 1_000).await, SendOutcome::Refused(400));
		assert_eq!(fake.urls(), [TOKEN_URI]);
	}
}
