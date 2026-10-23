use std::fmt;
use std::future::Future;

/// A mailbox bearer token, which never prints.
///
/// Its `Debug` prints `[redacted]` and it has no `Display`, so a `{:?}` of
/// anything holding one leaks nothing; the value is reached only through
/// [`Token::expose`], which is greppable.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
	#[must_use]
	pub const fn new(value: String) -> Self {
		Self(value)
	}

	/// The token itself, for the `Authorization` header and nothing else.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl fmt::Debug for Token {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[redacted]")
	}
}

/// The methods the mailbox's `/v1` API answers. `DELETE` archives an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
	Get,
	Put,
	Delete,
}

/// One request, ready for the wire. A `PUT` carries a JSON body; a `DELETE`
/// names the bytes it archives in `If-Match`.
#[derive(Debug, Clone)]
pub struct Request {
	pub method: Method,
	pub url: String,
	pub token: Token,
	pub body: Option<Vec<u8>>,
	/// An `If-Match` value, already quoted.
	pub if_match: Option<String>,
}

/// What the mailbox answered: everything [`crate::Mailbox`] reads.
#[derive(Debug, Clone)]
pub struct Response {
	pub status: u16,
	pub body: Vec<u8>,
}

/// Put a request on the wire.
///
/// A generic bound, never a trait object, as `push_kit::PushTransport` is,
/// and for the same reasons. Implementors do not
/// interpret the response: mapping a status to a meaning is
/// [`crate::Mailbox`]'s job.
pub trait MailboxTransport: Send + Sync {
	type Error: fmt::Display + Send;

	fn send(&self, request: Request) -> impl Future<Output = Result<Response, Self::Error>> + Send;
}

#[cfg(feature = "reqwest-transport")]
pub mod reqwest_transport {
	use super::{MailboxTransport, Method, Request, Response};

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

	impl MailboxTransport for ReqwestTransport {
		type Error = reqwest::Error;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			let outgoing = match request.method {
				Method::Get => self.client.get(&request.url),
				Method::Put => self.client.put(&request.url).header(reqwest::header::CONTENT_TYPE, "application/json"),
				Method::Delete => self.client.delete(&request.url),
			};
			let outgoing = outgoing.bearer_auth(request.token.expose());
			let outgoing = match request.if_match {
				Some(tag) => outgoing.header(reqwest::header::IF_MATCH, tag),
				None => outgoing,
			};
			let outgoing = match request.body {
				Some(body) => outgoing.body(body),
				None => outgoing,
			};
			// `without_url`: an error's `Display` names the URL, and the caller
			// logs that text. The URL names the owner's Worker; it stays out.
			let response = outgoing.send().await.map_err(reqwest::Error::without_url)?;
			let status = response.status().as_u16();
			let body = response.bytes().await.map_err(reqwest::Error::without_url)?;
			Ok(Response { status, body: body.to_vec() })
		}
	}
}
