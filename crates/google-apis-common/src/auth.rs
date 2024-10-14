/// This file is adapted @see
/// https://github.com/Byron/google-apis-rs/blob/main/google-apis-common/src/lib.rs
/// The original source is Licensed under MIT
use std::future::Future;
use std::pin::Pin;

type GetTokenOutput<'a> = Pin<Box<dyn Future<Output = Result<Option<String>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;

pub trait GetToken: GetTokenClone + Send + Sync {
	/// Called whenever an API call requires authentication via an oauth2 token.
	/// Returns `Ok(None)` if a token is not necessary - otherwise, returns an error
	/// indicating the reason why a token could not be produced.
	fn get_token<'a>(&'a self, _scopes: &'a [&str]) -> GetTokenOutput<'a>;
}

pub trait GetTokenClone {
	fn clone_box(&self) -> Box<dyn GetToken>;
}

impl<T> GetTokenClone for T
where
	T: 'static + GetToken + Clone,
{
	fn clone_box(&self) -> Box<dyn GetToken> {
		Box::new(self.clone())
	}
}

impl Clone for Box<dyn GetToken> {
	fn clone(&self) -> Box<dyn GetToken> {
		self.clone_box()
	}
}

impl GetToken for String {
	fn get_token<'a>(&'a self, _scopes: &'a [&str]) -> GetTokenOutput<'a> {
		Box::pin(async move { Ok(Some(self.clone())) })
	}
}

/// In the event that the API endpoint does not require an oauth2 token, `NoToken` should be provided to the hub to avoid specifying an
/// authenticator.
#[derive(Default, Clone)]
pub struct NoToken;

impl GetToken for NoToken {
	fn get_token<'a>(&'a self, _scopes: &'a [&str]) -> GetTokenOutput<'a> {
		Box::pin(async move { Ok(None) })
	}
}

#[cfg(feature = "yup-oauth2")]
mod yup_oauth2_impl {
	use super::{GetToken, GetTokenOutput};

	use http::Uri;
	use hyper::client::connect::Connection;
	use tokio::io::{AsyncRead, AsyncWrite};
	use tower_service::Service;
	use yup_oauth2::authenticator::Authenticator;

	impl<S> GetToken for Authenticator<S>
	where
		S: Service<Uri> + Clone + Send + Sync + 'static,
		S::Response: Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
		S::Future: Send + Unpin + 'static,
		S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
	{
		fn get_token<'a>(&'a self, scopes: &'a [&str]) -> GetTokenOutput<'a> {
			Box::pin(async move { self.token(scopes).await.map(|t| t.token().map(|t| t.to_owned())).map_err(|e| e.into()) })
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn dyn_get_token_is_send() {
		fn with_send(_x: impl Send) {}

		let mut gt = String::new();
		let dgt: &mut dyn GetToken = &mut gt;
		with_send(dgt);
	}
}
