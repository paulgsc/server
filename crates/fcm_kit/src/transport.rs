//! The wire, as a seam: a request goes out, a status and bytes come back.

use std::fmt;
use std::future::Future;

/// A bearer the transport puts on a request, and nothing prints.
#[derive(Clone)]
pub struct Bearer(String);

impl Bearer {
	#[must_use]
	pub const fn new(value: String) -> Self {
		Self(value)
	}

	/// The value itself, for the `Authorization` header and nothing else.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl fmt::Debug for Bearer {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[redacted]")
	}
}

/// One `POST`: to the token endpoint (a form, no bearer) or to FCM (JSON,
/// with the access token).
#[derive(Debug)]
pub struct Request {
	pub url: String,
	pub bearer: Option<Bearer>,
	pub content_type: &'static str,
	pub body: Vec<u8>,
}

/// What came back, uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
	pub status: u16,
	pub body: Vec<u8>,
}

/// Puts a [`Request`] on the wire.
///
/// A generic bound, never a trait object, as `mailbox_kit::MailboxTransport`
/// is. Implementors do not interpret the response: mapping a status to a
/// meaning is [`crate::Sender`]'s job. An error's `Display` must name neither
/// a device token nor a bearer, since the caller logs it.
pub trait FcmTransport: Send + Sync {
	type Error: fmt::Display + Send;

	fn send(&self, request: Request) -> impl Future<Output = Result<Response, Self::Error>> + Send;
}

#[cfg(feature = "reqwest-transport")]
pub mod reqwest_transport {
	use super::{FcmTransport, Request, Response};
	use std::fmt;

	/// Why a request got no answer. Names neither a token nor a URL.
	#[derive(Debug)]
	pub enum TransportError {
		/// The URL is not `https://`: a bearer or a token never travels in the
		/// clear, so nothing was sent.
		NotHttps,
		/// The request was sent and failed; the URL is stripped.
		Http(reqwest::Error),
	}

	impl fmt::Display for TransportError {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			match self {
				Self::NotHttps => f.write_str("refused to send outside https"),
				Self::Http(err) => err.fmt(f),
			}
		}
	}

	/// The adapter for hosts that already have a `reqwest` client.
	#[derive(Debug, Clone)]
	pub struct ReqwestTransport {
		client: reqwest::Client,
	}

	impl ReqwestTransport {
		#[must_use]
		pub const fn new(client: reqwest::Client) -> Self {
			Self { client }
		}
	}

	impl FcmTransport for ReqwestTransport {
		type Error = TransportError;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			let url = reqwest::Url::parse(&request.url).map_err(|_| TransportError::NotHttps)?;
			if url.scheme() != "https" {
				return Err(TransportError::NotHttps);
			}
			let outgoing = self.client.post(url).header(reqwest::header::CONTENT_TYPE, request.content_type);
			let outgoing = match request.bearer {
				Some(bearer) => outgoing.bearer_auth(bearer.expose()),
				None => outgoing,
			};
			// `without_url`: the send URL names the owner's Firebase project.
			let response = outgoing.body(request.body).send().await.map_err(|err| TransportError::Http(err.without_url()))?;
			let status = response.status().as_u16();
			let body = response.bytes().await.map_err(|err| TransportError::Http(err.without_url()))?;
			Ok(Response { status, body: body.to_vec() })
		}
	}
}
