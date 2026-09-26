//! Machine-readable inventory of the HTTP surface `file_host` exposes.
//!
//! # Why this exists
//!
//! `axum::Router` is write-only: once routes are registered there is no way to
//! ask it what it accepts. That makes the server's boundary invisible to
//! anything outside this process — including the client that has to agree with
//! it. When a path is renamed here, the only thing that notices is a 404 in
//! somebody's browser, days later.
//!
//! This module makes the surface an artifact. `dump-routes` emits it as JSON,
//! the client repo checks that snapshot into its contract harness, and a
//! renamed or dropped route shows up as a reviewable diff instead of a runtime
//! surprise.
//!
//! # Recorded, not declared
//!
//! There is no second list of routes to keep in step with the first. Each
//! route module registers its routes on a [`RouteTable`](super::table::RouteTable),
//! which hands each one to axum and records it in the same call, and
//! [`modules`] lists the modules. `main.rs` serves the routers built from that
//! list; [`snapshot`] reads the records from the same list. A route cannot be
//! served and missing from the snapshot, or in the snapshot and not served.
//!
//! Building the tables needs no `AppState` and no `Config`: handlers are
//! registered, never called, and CORS (the one thing that reads `Config`) is
//! applied only when `main.rs` asks for the router. So `dump-routes` still
//! runs without a provisioned environment.
//!
//! # Why no route carries a subject
//!
//! None of the paths this inventory lists look like `/subjects/:id/sessions`, and that is not
//! an oversight this inventory should flag. Whose request this is gets
//! resolved once, by the [`SubjectId`](crate::subject::SubjectId) extractor
//! every subject-scoped handler takes — an extractor concern, not a routing
//! one. Putting the subject in the path would mean every route in this list
//! grows a segment the day auth lands and shrinks back the day it is
//! refactored; putting it in the extractor means this file, and the surface it
//! describes, never changes when that happens.

use super::table::Module;
use super::{db, health, outcomes, presence, push, readiness, signals, subjects, tab_metadata, utterance};
use crate::{websocket, AppState, Config, API_V1_BASE_PATH};
use axum::{extract::FromRef, Router};
use serde::Serialize;

/// Bump when the emitted JSON shape changes in a way consumers must react to.
/// The client harness refuses a snapshot it does not know how to read rather
/// than silently misinterpreting one.
///
/// The TypeScript emitter added for #266 ([`super::ts_emitter`]) does not
/// bump this. It is a second artefact, not a new shape of this one —
/// [`RouteInventory`]'s fields are unchanged, and the `.ts` module carries no
/// version field of its own to go stale: a route it no longer contains is a
/// `tsc` compile error in the client, not a runtime value this constant would
/// otherwise need to describe.
pub const INVENTORY_SCHEMA_VERSION: u32 = 1;

/// One route as registered on the router, before the `/api/v1` nest is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RouteDescriptor {
	/// Uppercase HTTP method, matching the `axum::routing` helper used.
	pub method: &'static str,
	/// Path exactly as written in the `.route(...)` call, so the source-parity
	/// test can compare literals without normalising anything away. Path
	/// parameters keep their axum `:name` form.
	pub path: &'static str,
	/// Whether `main.rs` nests this route under [`API_V1_BASE_PATH`].
	/// `/health` and `/ws` are the deliberate exceptions.
	pub versioned: bool,
	/// The `routes` submodule that registers it. Groups the emitted snapshot
	/// so a diff reads as "the tabs surface moved", not thirty loose lines.
	pub module: &'static str,
}

impl RouteDescriptor {
	/// The path a client actually has to request, `/api/v1` prefix included.
	#[must_use]
	pub fn full_path(&self) -> String {
		let mut out = String::with_capacity(API_V1_BASE_PATH.len() + self.path.len());
		if self.versioned {
			out.push_str(API_V1_BASE_PATH);
		}
		out.push_str(self.path);
		out
	}
}

/// Every route module `main.rs` serves, in one list.
///
/// `main.rs` builds its router from this and [`snapshot`] builds the inventory
/// from it, so a module is either on both or on neither. Adding a route module
/// means adding it here; forgetting to means it is not served, which is loud,
/// rather than served but missing from the snapshot, which is not.
///
/// `S` is the router state. `main.rs` passes [`AppState`]; so does
/// [`snapshot`], which builds the tables without ever constructing one.
///
/// Not here: `GET /metrics`. It is built inside `some_metrics` on its own
/// state and merged separately in `main.rs`, and it is a Prometheus scrape
/// target rather than anything the client calls, so it has never been part
/// of the client-facing inventory.
#[must_use]
pub fn modules<S>() -> Vec<Module<S>>
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	vec![
		// ── unversioned ─────────────────────────────────────────────────────
		// Load balancers and orchestrators track these, so they must not move
		// when the API version bumps. See the doc comment on `API_V1_BASE_PATH`.
		health::get_health(),
		readiness::get_readiness(),
		websocket::routes(),
		// ── versioned ───────────────────────────────────────────────────────
		db::mood_events(),
		db::tabs(),
		db::sessions(),
		db::activities(),
		db::curriculum(),
		push::push(),
		presence::presence(),
		signals::signals(),
		outcomes::outcomes(),
		subjects::subjects(),
		tab_metadata::post_now_playing(),
		utterance::post_utterance(),
	]
}

/// The routers `main.rs` serves, built from [`modules`].
///
/// Returns the versioned half, which the caller nests under
/// [`API_V1_BASE_PATH`] after adding its own layers, and the unversioned
/// half, which it merges at the root.
pub fn routers<S>(config: &Config) -> (Router<S>, Router<S>)
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	split(|module| module.into_router(config))
}

fn split<S>(build: impl Fn(Module<S>) -> Router<S>) -> (Router<S>, Router<S>)
where
	S: Clone + Send + Sync + 'static,
	AppState: FromRef<S>,
{
	modules().into_iter().fold((Router::new(), Router::new()), |(versioned, unversioned), module| {
		if module.is_versioned() {
			(versioned.merge(build(module)), unversioned)
		} else {
			(versioned, unversioned.merge(build(module)))
		}
	})
}

/// Every route [`modules`] registers, unsorted.
#[must_use]
pub fn routes() -> Vec<RouteDescriptor> {
	modules::<AppState>().iter().flat_map(Module::descriptors).collect()
}

/// One route in the emitted JSON. Carries `full_path` already resolved so
/// consumers never have to reimplement the `/api/v1` nesting rule.
#[derive(Debug, Clone, Serialize)]
pub struct RouteEntry {
	pub method: String,
	pub path: String,
	pub full_path: String,
	pub versioned: bool,
	pub module: String,
}

/// The emitted document.
#[derive(Debug, Clone, Serialize)]
pub struct RouteInventory {
	pub schema_version: u32,
	pub api_base_path: String,
	/// Server crate version, so a snapshot can be traced back to a build.
	pub server_version: String,
	pub routes: Vec<RouteEntry>,
}

/// Builds the emittable snapshot, sorted so that regenerating it without
/// changing any route produces a byte-identical file.
#[must_use]
pub fn snapshot() -> RouteInventory {
	let mut routes: Vec<RouteEntry> = routes()
		.into_iter()
		.map(|route| RouteEntry {
			method: route.method.to_owned(),
			path: route.path.to_owned(),
			full_path: route.full_path(),
			versioned: route.versioned,
			module: route.module.to_owned(),
		})
		.collect();

	routes.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.path.cmp(&b.path)).then_with(|| a.method.cmp(&b.method)));

	RouteInventory {
		schema_version: INVENTORY_SCHEMA_VERSION,
		api_base_path: API_V1_BASE_PATH.to_owned(),
		server_version: env!("CARGO_PKG_VERSION").to_owned(),
		routes,
	}
}

#[cfg(test)]
mod tests {
	use super::{modules, routes};
	use crate::routes::table::Module;
	use crate::{AppState, API_V1_BASE_PATH};
	use axum::Router;
	use std::collections::BTreeSet;

	#[test]
	fn versioned_routes_carry_the_api_prefix() {
		for route in routes() {
			let full = route.full_path();
			if route.versioned {
				assert!(full.starts_with("/api/v1/"), "versioned route {} did not resolve under /api/v1: {full}", route.path);
			} else {
				assert!(!full.starts_with("/api/v1"), "unversioned route {} unexpectedly resolved under /api/v1: {full}", route.path);
			}
		}
	}

	/// Two modules registering the same method and path each build fine on
	/// their own; axum only panics when `main.rs` merges them, at boot.
	#[test]
	fn no_duplicate_method_path_pairs() {
		let mut seen = BTreeSet::new();
		for route in routes() {
			let key = (route.method, route.full_path());
			assert!(seen.insert(key), "duplicate route registration: {} {}", route.method, route.full_path());
		}
	}

	/// The snapshot groups routes by module name, so two modules sharing one
	/// would read as a single surface in the client's diff.
	#[test]
	fn module_names_are_unique() {
		let mut seen = BTreeSet::new();
		for module in modules::<AppState>() {
			assert!(seen.insert(module.name()), "two route modules are both named `{}`", module.name());
		}
	}

	/// A module with an empty table is a registry entry whose routes went
	/// somewhere else.
	#[test]
	fn every_module_registers_a_route() {
		for module in modules::<AppState>() {
			assert!(module.descriptors().next().is_some(), "route module `{}` registers no routes", module.name());
		}
	}

	/// Merges and nests every table through the same [`super::split`]
	/// `main.rs` reaches via [`super::routers`]. Anything axum rejects only
	/// once routers are combined (a duplicate registration, two captures with
	/// different names in one position) panics here instead of at boot. CORS
	/// is left off: it needs a `Config`, and a layer cannot conflict.
	#[test]
	fn the_surface_assembles_like_main_does() {
		let (versioned, unversioned) = super::split::<AppState>(Module::into_table_router);
		let _app: Router<AppState> = Router::new().nest(API_V1_BASE_PATH, versioned).merge(unversioned);
	}

	#[test]
	fn snapshot_is_deterministic() {
		let first = super::snapshot();
		let second = super::snapshot();
		let rendered = |inventory: &super::RouteInventory| serde_json::to_string_pretty(inventory).unwrap();
		assert_eq!(rendered(&first), rendered(&second));
	}
}
