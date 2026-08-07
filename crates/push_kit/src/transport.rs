use crate::sender::PreparedPush;
use std::future::Future;

/// What the push service said.
///
/// Deliberately not a `reqwest::Response`, nor any other client's type: the
/// three fields below are everything [`crate::SendOutcome`] reads, and keeping
/// the seam this narrow is what lets a test transport be six lines.
#[derive(Debug, Clone)]
pub struct PushResponse {
	pub status: u16,
	/// `Retry-After` verbatim when the service sent one.
	pub retry_after: Option<String>,
	pub body: String,
}

/// Put a prepared request on the wire.
///
/// A **generic bound, never a trait object.** A given binary has one transport,
/// known when it is compiled; a vtable to reach one implementation buys nothing
/// and costs the ability to inline. `impl Future` in return position keeps that
/// true through the async boundary as well — no `async_trait`, no boxing.
///
/// Implementors do not interpret the response. Mapping a status to a meaning is
/// [`crate::SendOutcome::from_status`]'s job, and a transport that also decided
/// what a `410` meant would be a second place for that to be wrong.
///
/// `Sync` is required rather than merely `Send`: a `Sender` lives in shared
/// application state and is sent to by several tasks at once, so `&Sender<T>`
/// has to cross an await point. Requiring it here means a transport that
/// cannot is rejected at its definition rather than at every call site.
pub trait PushTransport: Send + Sync {
	type Error: std::fmt::Display + Send;

	fn deliver(&self, request: PreparedPush) -> impl Future<Output = Result<PushResponse, Self::Error>> + Send;
}

#[cfg(feature = "reqwest-transport")]
pub mod reqwest_transport {
	use super::{PreparedPush, PushResponse, PushTransport};

	/// The adapter for hosts that already have a `reqwest` client.
	///
	/// Assembled by hand rather than through `web_push::request_builder`, which
	/// returns an `http 0.2` request that `reqwest 0.12` (on `http 1`) cannot
	/// consume. The RFC 8291/8292 work has already happened by the time a
	/// [`PreparedPush`] exists; what is left is four headers.
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

	impl Default for ReqwestTransport {
		fn default() -> Self {
			Self::new(reqwest::Client::new())
		}
	}

	impl PushTransport for ReqwestTransport {
		type Error = reqwest::Error;

		async fn deliver(&self, request: PreparedPush) -> Result<PushResponse, Self::Error> {
			let mut outgoing = self.client.post(&request.endpoint);
			for (name, value) in request.headers {
				outgoing = outgoing.header(name, value);
			}

			let response = outgoing.body(request.body).send().await?;
			let status = response.status().as_u16();
			let retry_after = response
				.headers()
				.get(reqwest::header::RETRY_AFTER)
				.and_then(|value| value.to_str().ok())
				.map(ToOwned::to_owned);

			Ok(PushResponse {
				status,
				retry_after,
				body: response.text().await.unwrap_or_default(),
			})
		}
	}
}
