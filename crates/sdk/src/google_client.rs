//! Shared auth/token/connector foundation for the Google service clients
//! (`gdrive`, `gsheets`, `gmail`). Every service previously built its own
//! TLS connector, hyper client, and `ServiceAccountAuthenticator` from
//! scratch; this module centralizes that construction plus a process-wide
//! client cache so services constructed with the same credentials share one
//! authenticated hub instead of each re-authenticating independently.

use crate::{GoogleServiceFilePath, SecretFilePathError};
use google_apis_common as common;
// All four generated Google API crates (drive3, sheets4, gmail1, youtube3)
// resolve to the same yup-oauth2 major version, so any one of their
// re-exports is the canonical type every hub expects.
use google_drive3::yup_oauth2;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::OnceCell as AsyncOnceCell;
use yup_oauth2::authenticator::Authenticator;
use yup_oauth2::ServiceAccountAuthenticator;

pub type HttpsConnectorType = HttpsConnector<HttpConnector>;
pub type GoogleAuthenticator = Authenticator<HttpsConnectorType>;
pub type GoogleHttpClient = common::Client<HttpsConnectorType>;
type LegacyClient<B> = hyper_util::client::legacy::Client<HttpsConnectorType, B>;

#[derive(Debug, thiserror::Error)]
pub enum GoogleClientError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] yup_oauth2::Error),

	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),
}

pub fn build_https_connector() -> Result<HttpsConnectorType, GoogleClientError> {
	Ok(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots()?.https_or_http().enable_http1().build())
}

pub fn build_http_client() -> Result<GoogleHttpClient, GoogleClientError> {
	build_legacy_client()
}

/// Same connector, generic over the response body type. The generated hub
/// clients (`DriveHub`, `Sheets`, `Gmail`, ...) all need `common::Body`
/// (aliased above as [`GoogleHttpClient`]), but `yup_oauth2`'s interactive
/// installed-flow authenticator needs a client whose body is `String`. Both
/// go through the same connector-building logic.
pub fn build_legacy_client<B>() -> Result<LegacyClient<B>, GoogleClientError>
where
	B: http_body::Body + Send,
	B::Data: Send,
{
	let connector = build_https_connector()?;
	Ok(hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(connector))
}

pub async fn build_service_account_authenticator(secret_path: &GoogleServiceFilePath) -> Result<GoogleAuthenticator, GoogleClientError> {
	let secret = yup_oauth2::read_service_account_key(secret_path.as_ref()).await?;
	Ok(ServiceAccountAuthenticator::builder(secret).build().await?)
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
struct CacheKey {
	kind: &'static str,
	user_email: String,
	secret_path: String,
}

/// Process-wide memoized cache keyed by `(kind, user_email, secret_path)`.
/// Multiple service handles built from the same credentials (e.g. a reader
/// and a writer) resolve to the same lazily-initialized `Arc<T>` instead of
/// each performing their own auth handshake.
pub struct ClientCache<T: ?Sized> {
	entries: StdMutex<HashMap<CacheKey, Arc<AsyncOnceCell<Arc<T>>>>>,
}

impl<T: ?Sized> Default for ClientCache<T> {
	fn default() -> Self {
		Self {
			entries: StdMutex::new(HashMap::new()),
		}
	}
}

impl<T: ?Sized> ClientCache<T> {
	pub fn new() -> Self {
		Self::default()
	}

	pub async fn get_or_try_init<E, F, Fut>(&self, kind: &'static str, user_email: &str, secret_path: &str, init: F) -> Result<Arc<T>, E>
	where
		F: FnOnce() -> Fut,
		Fut: Future<Output = Result<Arc<T>, E>>,
	{
		let key = CacheKey {
			kind,
			user_email: user_email.to_string(),
			secret_path: secret_path.to_string(),
		};

		let cell = {
			let mut guard = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
			Arc::clone(guard.entry(key).or_insert_with(|| Arc::new(AsyncOnceCell::new())))
		};

		let value = cell.get_or_try_init(init).await?;
		Ok(Arc::clone(value))
	}
}
