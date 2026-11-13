//! The server contract's epoch: which clients this build serves, as one
//! ordered number a client can carry at build time (home's on-demand
//! deploy; README.md, "Deploying to home").
//!
//! Each APK and web build bundles the epoch of the route snapshot it was
//! built against (`dump-routes` emits [`CONTRACT_EPOCH`] in the snapshot
//! paulgsc/some-ui already copies). It asks `GET /compat?needs=<epoch>`;
//! home serves it when `OLDEST_SUPPORTED <= needs <= CONTRACT_EPOCH`, and
//! asks for a newer image when `needs` is above (`handlers::compat`).
//!
//! # When to bump
//!
//! - [`CONTRACT_EPOCH`] goes up by one with every change a client can depend
//!   on: a route added, removed, renamed or moved, which
//!   `the_fingerprint_is_this_route_surface` makes a failing test until the
//!   epoch and [`ROUTE_FINGERPRINT`] move together; and by hand for a payload
//!   change a client needs, which the snapshot cannot see. Never down:
//!   `scripts/check_contract_epoch.py` (Lint) fails a PR whose fingerprint
//!   changed without the epoch rising above the base branch's.
//! - [`OLDEST_SUPPORTED`] goes up to the new epoch when a change breaks
//!   older clients (a route removed or renamed: the PR is labelled
//!   `client-breaking-route`). A client below it is told it is behind home,
//!   and pulling cannot help it.

/// This build's contract epoch. See the module doc for when it moves.
pub const CONTRACT_EPOCH: u32 = 4;

/// The lowest epoch this build still serves. Raised when a client-breaking
/// route change lands; at most [`CONTRACT_EPOCH`].
pub const OLDEST_SUPPORTED: u32 = 1;

/// The route surface [`CONTRACT_EPOCH`] was last set for.
///
/// FNV-1a over the sorted `METHOD full_path` lines of every route this build
/// serves, as [`route_fingerprint`] computes it. A route change fails `the_fingerprint_is_this_route_surface`,
/// whose message names the new value, until this and the epoch are updated.
pub const ROUTE_FINGERPRINT: &str = "a96921f1b5471362";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// The fingerprint of `routes`, `(method, full_path)` pairs in any order.
///
/// FNV-1a 64 over each `METHOD full_path\n`, sorted, as 16 hex digits.
/// Stable across platforms and builds, and needs no dependency.
#[must_use]
pub fn fingerprint_of(routes: &[(String, String)]) -> String {
	let mut lines: Vec<String> = routes
		.iter()
		.map(|(method, path)| {
			let mut line = String::with_capacity(method.len() + path.len() + 2);
			line.push_str(method);
			line.push(' ');
			line.push_str(path);
			line.push('\n');
			line
		})
		.collect();
	lines.sort_unstable();
	let mut hash = FNV_OFFSET;
	for byte in lines.iter().flat_map(|line| line.bytes()) {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(FNV_PRIME);
	}
	let mut out = String::with_capacity(16);
	for shift in (0..16).rev() {
		let nibble = u8::try_from((hash >> (shift * 4)) & 0xf).unwrap_or(0);
		out.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
	}
	out
}

/// The fingerprint of the route surface this build serves.
#[must_use]
pub fn route_fingerprint() -> String {
	let routes: Vec<(String, String)> = crate::routes::inventory::routes()
		.iter()
		.map(|route| (route.method.to_owned(), route.full_path()))
		.collect();
	fingerprint_of(&routes)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A route change is a contract change: this fails, naming the new
	/// fingerprint, until `ROUTE_FINGERPRINT` is set to it and
	/// `CONTRACT_EPOCH` raised by one (module doc, "When to bump").
	#[test]
	fn the_fingerprint_is_this_route_surface() {
		let now = route_fingerprint();
		assert_eq!(
			now, ROUTE_FINGERPRINT,
			"the route surface changed: set contract::ROUTE_FINGERPRINT to \"{now}\" and raise contract::CONTRACT_EPOCH by one \
			 (and OLDEST_SUPPORTED to it if this removes or renames a route a client calls)"
		);
	}

	#[test]
	fn the_oldest_supported_is_at_most_the_epoch() {
		const { assert!(OLDEST_SUPPORTED >= 1 && OLDEST_SUPPORTED <= CONTRACT_EPOCH) };
	}

	#[test]
	fn the_fingerprint_ignores_order_and_sees_every_change() {
		let pair = |m: &str, p: &str| (m.to_owned(), p.to_owned());
		let a = fingerprint_of(&[pair("GET", "/a"), pair("POST", "/b")]);
		assert_eq!(a, fingerprint_of(&[pair("POST", "/b"), pair("GET", "/a")]));
		assert_ne!(a, fingerprint_of(&[pair("GET", "/a"), pair("PUT", "/b")]));
		assert_ne!(a, fingerprint_of(&[pair("GET", "/a")]));
		assert_eq!(a.len(), 16);
	}
}
