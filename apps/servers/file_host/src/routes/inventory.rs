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
//! # The declaration is checked, not trusted
//!
//! A hand-maintained list of routes rots exactly as fast as the thing it
//! describes, so this one is not taken on faith. The test at the bottom of this
//! file parses the sibling `routes/*.rs` sources at compile time (via
//! `include_str!`) and asserts set equality with what is declared here, in both
//! directions. Adding a `.route(...)` call without adding it here fails the
//! test, and so does the reverse.
//!
//! That check is source-level on purpose: verifying against a live `Router`
//! would require a fully built `AppState` (SQLite, NATS, Google clients), which
//! would make the cheapest invariant in the crate the most expensive one to
//! run.

use crate::API_V1_BASE_PATH;
use serde::Serialize;

/// Bump when the emitted JSON shape changes in a way consumers must react to.
/// The client harness refuses a snapshot it does not know how to read rather
/// than silently misinterpreting one.
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

/// The complete surface. Every `.route(...)` call reachable from `main.rs`
/// appears here exactly once; the source-parity test enforces it.
///
/// Ordering is irrelevant — [`snapshot`] sorts before emitting so the JSON is
/// stable across edits to this list.
pub const ROUTES: &[RouteDescriptor] = &[
	// ── unversioned ─────────────────────────────────────────────────────────
	// Load balancers and orchestrators track these, so they must not move when
	// the API version bumps. See the doc comment on `API_V1_BASE_PATH`.
	RouteDescriptor { method: "GET", path: "/health", versioned: false, module: "health" },
	RouteDescriptor { method: "GET", path: "/ws", versioned: false, module: "websocket" },
	// ── sheets ──────────────────────────────────────────────────────────────
	RouteDescriptor { method: "GET", path: "/get_attributions/:sheet_id", versioned: true, module: "sheets" },
	RouteDescriptor { method: "GET", path: "/get_video_chapters/:sheet_id", versioned: true, module: "sheets" },
	RouteDescriptor { method: "GET", path: "/get_gantt/:sheet_id", versioned: true, module: "sheets" },
	RouteDescriptor { method: "GET", path: "/get_nfl_tennis/:sheet_id", versioned: true, module: "sheets" },
	RouteDescriptor { method: "GET", path: "/get_nfl_roster/:sheet_id", versioned: true, module: "sheets" },
	// ── gdrive ──────────────────────────────────────────────────────────────
	RouteDescriptor { method: "GET", path: "/gdrive/image/:image_id", versioned: true, module: "gdrive" },
	RouteDescriptor { method: "GET", path: "/gdrive/list", versioned: true, module: "gdrive" },
	RouteDescriptor { method: "GET", path: "/gdrive/list/:folder_id", versioned: true, module: "gdrive" },
	RouteDescriptor { method: "GET", path: "/gdrive/json/:file_id", versioned: true, module: "gdrive" },
	RouteDescriptor { method: "PUT", path: "/gdrive/write/:folder_id/:name", versioned: true, module: "gdrive" },
	// ── github ──────────────────────────────────────────────────────────────
	RouteDescriptor { method: "GET", path: "/get_github_repos", versioned: true, module: "github" },
	// ── mood events ─────────────────────────────────────────────────────────
	RouteDescriptor { method: "POST", path: "/mood_events", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "GET", path: "/mood_events", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "GET", path: "/mood_events/:id", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "PATCH", path: "/mood_events/:id", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "DELETE", path: "/mood_events/:id", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "POST", path: "/mood_events/batch", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "PATCH", path: "/mood_events/batch", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "DELETE", path: "/mood_events/batch", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "GET", path: "/mood_events/week/:week", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "GET", path: "/mood_events/team/:team", versioned: true, module: "mood_events" },
	RouteDescriptor { method: "GET", path: "/mood_events/stats", versioned: true, module: "mood_events" },
	// ── tabs ────────────────────────────────────────────────────────────────
	RouteDescriptor { method: "POST", path: "/tabs", versioned: true, module: "tabs" },
	RouteDescriptor { method: "GET", path: "/tabs", versioned: true, module: "tabs" },
	RouteDescriptor { method: "GET", path: "/tabs/:tab_id", versioned: true, module: "tabs" },
	RouteDescriptor { method: "DELETE", path: "/tabs/:tab_id", versioned: true, module: "tabs" },
	RouteDescriptor { method: "POST", path: "/tabs/batch", versioned: true, module: "tabs" },
	RouteDescriptor { method: "DELETE", path: "/tabs/batch", versioned: true, module: "tabs" },
	RouteDescriptor { method: "POST", path: "/tabs/prune", versioned: true, module: "tabs" },
	RouteDescriptor { method: "POST", path: "/tabs/reconcile", versioned: true, module: "tabs" },
	RouteDescriptor { method: "GET", path: "/tabs/summaries", versioned: true, module: "tabs" },
	RouteDescriptor { method: "POST", path: "/tabs/pipeline", versioned: true, module: "tabs" },
	// ── audio ───────────────────────────────────────────────────────────────
	RouteDescriptor { method: "GET", path: "/get_audio/:id", versioned: true, module: "audio_files" },
	RouteDescriptor { method: "GET", path: "/search_audio", versioned: true, module: "audio_files" },
	RouteDescriptor { method: "POST", path: "/search_audio", versioned: true, module: "audio_files" },
	// ── misc ────────────────────────────────────────────────────────────────
	RouteDescriptor { method: "POST", path: "/now-playing", versioned: true, module: "tab_metadata" },
	RouteDescriptor { method: "POST", path: "/utter", versioned: true, module: "utterance" },
];

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
	let mut routes: Vec<RouteEntry> = ROUTES
		.iter()
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
	use super::{RouteDescriptor, ROUTES};
	use regex::Regex;
	use std::collections::BTreeSet;

	/// Every source file that registers routes, paired with the module name
	/// used in [`ROUTES`] and whether `main.rs` nests it under `/api/v1`.
	///
	/// Adding a new `routes/*.rs` module means adding it here too — otherwise
	/// its routes are unaudited. The `module_coverage` test below catches the
	/// half of that mistake it can see.
	const SOURCES: &[(&str, &str, bool)] = &[
		("health", include_str!("health.rs"), false),
		("websocket", include_str!("../websocket.rs"), false),
		("sheets", include_str!("sheets.rs"), true),
		("gdrive", include_str!("gdrive.rs"), true),
		("github", include_str!("github.rs"), true),
		("mood_events", include_str!("db/hopium.rs"), true),
		("tabs", include_str!("db/tab.rs"), true),
		("audio_files", include_str!("audio_files.rs"), true),
		("tab_metadata", include_str!("tab_metadata.rs"), true),
		("utterance", include_str!("utterance.rs"), true),
	];

	/// A route as recovered from source: `(module, METHOD, path, versioned)`.
	type ParsedRoute = (String, String, String, bool);

	/// Recovers every `.route("<path>", <method>(...))` registration from the
	/// sources above.
	///
	/// This reads the literal text rather than the built router, which means it
	/// sees what a reviewer sees. The one thing it cannot see is a route
	/// registered somewhere not listed in `SOURCES` — hence `module_coverage`.
	fn parse_sources() -> BTreeSet<ParsedRoute> {
		let pattern = Regex::new(r#"\.route\(\s*"([^"]+)"\s*,\s*(get|post|put|patch|delete)\("#).unwrap();

		let mut found = BTreeSet::new();
		for (module, source, versioned) in SOURCES {
			for capture in pattern.captures_iter(source) {
				let path = capture.get(1).unwrap().as_str().to_owned();
				let method = capture.get(2).unwrap().as_str().to_uppercase();
				found.insert(((*module).to_owned(), method, path, *versioned));
			}
		}
		found
	}

	fn declared() -> BTreeSet<ParsedRoute> {
		ROUTES
			.iter()
			.map(|route: &RouteDescriptor| ((route.module).to_owned(), (route.method).to_owned(), (route.path).to_owned(), route.versioned))
			.collect()
	}

	/// The load-bearing invariant: the declaration and the router agree.
	///
	/// Both directions matter. A route in source but not declared means the
	/// client harness is blind to it. A route declared but not in source means
	/// the harness is testing something that no longer exists, and will report
	/// a 404 as a client bug.
	#[test]
	fn inventory_matches_route_sources() {
		let in_source = parse_sources();
		let in_inventory = declared();

		let undeclared: Vec<_> = in_source.difference(&in_inventory).collect();
		let phantom: Vec<_> = in_inventory.difference(&in_source).collect();

		assert!(
			undeclared.is_empty(),
			"routes registered in source but missing from ROUTES — add them, or the contract harness cannot see them: {undeclared:#?}"
		);
		assert!(
			phantom.is_empty(),
			"routes declared in ROUTES but absent from source — they were renamed or removed; update ROUTES and regenerate the client snapshot: {phantom:#?}"
		);
	}

	/// Guards the blind spot in `parse_sources`: a `routes/*.rs` module that
	/// nobody added to `SOURCES` would silently contribute zero routes and the
	/// parity test would still pass.
	#[test]
	fn module_coverage() {
		let source_modules: BTreeSet<&str> = SOURCES.iter().map(|(module, _, _)| *module).collect();
		let declared_modules: BTreeSet<&str> = ROUTES.iter().map(|route| route.module).collect();

		assert_eq!(
			source_modules, declared_modules,
			"SOURCES and ROUTES disagree about which modules exist; a whole route module is probably unaudited"
		);
	}

	/// Each `SOURCES` entry must actually yield routes. Catches a stale
	/// `include_str!` path that still compiles but points at a file which no
	/// longer registers anything.
	#[test]
	fn every_source_contributes_routes() {
		let parsed = parse_sources();
		for (module, _, _) in SOURCES {
			assert!(
				parsed.iter().any(|(parsed_module, _, _, _)| parsed_module == module),
				"source module `{module}` parsed to zero routes — wrong include_str! path, or the routes moved"
			);
		}
	}

	#[test]
	fn versioned_routes_carry_the_api_prefix() {
		for route in ROUTES {
			let full = route.full_path();
			if route.versioned {
				assert!(full.starts_with("/api/v1/"), "versioned route {} did not resolve under /api/v1: {full}", route.path);
			} else {
				assert!(!full.starts_with("/api/v1"), "unversioned route {} unexpectedly resolved under /api/v1: {full}", route.path);
			}
		}
	}

	/// `main.rs` merges these routers into one `Router`; axum panics at startup
	/// on a duplicate method+path. Catching it here turns a boot-time crash into
	/// a test failure.
	#[test]
	fn no_duplicate_method_path_pairs() {
		let mut seen = BTreeSet::new();
		for route in ROUTES {
			let key = (route.method, route.full_path());
			assert!(seen.insert(key), "duplicate route registration: {} {}", route.method, route.full_path());
		}
	}

	#[test]
	fn snapshot_is_deterministic() {
		let first = super::snapshot();
		let second = super::snapshot();
		let rendered = |inventory: &super::RouteInventory| serde_json::to_string_pretty(inventory).unwrap();
		assert_eq!(rendered(&first), rendered(&second));
	}
}
