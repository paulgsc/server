//! Route registration that records what it registers.
//!
//! `axum::Router` is write-only, which is why [`super::inventory`] exists. It
//! used to be a second, hand-written list checked against the `.route(...)`
//! calls by a regex over their source text. Now a route is declared once, on a
//! [`RouteTable`]: the same call hands the handler to axum and appends
//! `(method, path)` to the table. The router this server serves and the
//! inventory `dump-routes` emits are two readings of one value, so they cannot
//! disagree.
//!
//! A [`Module`] is a table plus what a route module is allowed to wrap around
//! it: its name in the snapshot, whether `main.rs` nests it under
//! [`API_V1_BASE_PATH`](crate::API_V1_BASE_PATH), and an optional CORS layer.
//! Nothing else. A module cannot add a route anywhere except its table.
//!
//! Calling `Router::route` directly is a `clippy::disallowed_methods` error
//! (`clippy.toml`), so a route registered outside a table is a lint failure
//! rather than a route the client harness never hears about. The recorder's
//! own call in `RouteTable::on` is the only one on a served route; the
//! others (`/metrics` in `some_metrics`, throwaway test routers) each carry an
//! `#[allow]` saying why.

use super::inventory::RouteDescriptor;
use crate::Config;
use axum::{
	handler::Handler,
	routing::{delete, get, patch, post, MethodRouter},
	Router,
};
use tower_http::cors::CorsLayer;

/// The HTTP methods this server registers. The recorded method comes from
/// the same value that picks the axum helper, so the two cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
	Get,
	Post,
	Patch,
	Delete,
}

impl Verb {
	const fn as_str(self) -> &'static str {
		match self {
			Self::Get => "GET",
			Self::Post => "POST",
			Self::Patch => "PATCH",
			Self::Delete => "DELETE",
		}
	}

	fn method_router<H, T, S>(self, handler: H) -> MethodRouter<S>
	where
		H: Handler<T, S>,
		T: 'static,
		S: Clone + Send + Sync + 'static,
	{
		match self {
			Self::Get => get(handler),
			Self::Post => post(handler),
			Self::Patch => patch(handler),
			Self::Delete => delete(handler),
		}
	}
}

/// A router that remembers every route registered on it.
///
/// Registering `GET` and `POST` on the same path in two calls is fine, as it
/// is with axum: `Router::route` merges method routers on one path.
pub struct RouteTable<S> {
	router: Router<S>,
	routes: Vec<(Verb, &'static str)>,
}

impl<S> RouteTable<S>
where
	S: Clone + Send + Sync + 'static,
{
	#[must_use]
	pub fn new() -> Self {
		Self {
			router: Router::new(),
			routes: Vec::new(),
		}
	}

	#[must_use]
	pub fn get<H, T>(self, path: &'static str, handler: H) -> Self
	where
		H: Handler<T, S>,
		T: 'static,
	{
		self.on(Verb::Get, path, handler)
	}

	#[must_use]
	pub fn post<H, T>(self, path: &'static str, handler: H) -> Self
	where
		H: Handler<T, S>,
		T: 'static,
	{
		self.on(Verb::Post, path, handler)
	}

	#[must_use]
	pub fn patch<H, T>(self, path: &'static str, handler: H) -> Self
	where
		H: Handler<T, S>,
		T: 'static,
	{
		self.on(Verb::Patch, path, handler)
	}

	#[must_use]
	pub fn delete<H, T>(self, path: &'static str, handler: H) -> Self
	where
		H: Handler<T, S>,
		T: 'static,
	{
		self.on(Verb::Delete, path, handler)
	}

	fn on<H, T>(mut self, verb: Verb, path: &'static str, handler: H) -> Self
	where
		H: Handler<T, S>,
		T: 'static,
	{
		// The one sanctioned `Router::route` call: registration and record
		// happen together here, or not at all.
		#[allow(clippy::disallowed_methods)]
		let router = self.router.route(path, verb.method_router(handler));
		self.router = router;
		self.routes.push((verb, path));
		self
	}
}

impl<S> Default for RouteTable<S>
where
	S: Clone + Send + Sync + 'static,
{
	fn default() -> Self {
		Self::new()
	}
}

/// One route module: a [`RouteTable`] plus its name, nesting and CORS.
pub struct Module<S> {
	name: &'static str,
	versioned: bool,
	table: RouteTable<S>,
	cors: Option<fn(&Config) -> CorsLayer>,
}

impl<S> Module<S>
where
	S: Clone + Send + Sync + 'static,
{
	/// Served under `/api/v1`.
	#[must_use]
	pub const fn versioned(name: &'static str, table: RouteTable<S>) -> Self {
		Self {
			name,
			versioned: true,
			table,
			cors: None,
		}
	}

	/// Served at the root: load balancers, orchestrators and the WebSocket
	/// upgrade, which must not move when the API version does.
	#[must_use]
	pub const fn unversioned(name: &'static str, table: RouteTable<S>) -> Self {
		Self {
			name,
			versioned: false,
			table,
			cors: None,
		}
	}

	/// A function rather than a built layer: CORS reads `ALLOWED_ORIGINS` from
	/// [`Config`], and `dump-routes` must be able to read a module without one.
	#[must_use]
	pub fn with_cors(mut self, cors: fn(&Config) -> CorsLayer) -> Self {
		self.cors = Some(cors);
		self
	}

	#[must_use]
	pub const fn name(&self) -> &'static str {
		self.name
	}

	#[must_use]
	pub const fn is_versioned(&self) -> bool {
		self.versioned
	}

	/// What this module registered, in registration order.
	pub fn descriptors(&self) -> impl Iterator<Item = RouteDescriptor> + '_ {
		self.table.routes.iter().map(|(verb, path)| RouteDescriptor {
			method: verb.as_str(),
			path,
			versioned: self.versioned,
			module: self.name,
		})
	}

	/// The router `main.rs` serves: the recorded table, with its CORS applied.
	pub fn into_router(self, config: &Config) -> Router<S> {
		match self.cors {
			Some(cors) => self.table.router.layer(cors(config)),
			None => self.table.router,
		}
	}

	/// The bare table, without CORS. For tests that assemble the surface
	/// without a [`Config`].
	#[cfg(test)]
	pub(crate) fn into_table_router(self) -> Router<S> {
		self.table.router
	}
}
