//! The peer's address, and the one thing about it that leaves this module.
//!
//! Rate limiting and WebSocket admission both need to tell one client from
//! another, and the address is the only handle a request arrives with. Neither
//! needs to know *which* address it is. So the raw address never leaves this
//! module: callers get a [`PeerKey`], a keyed hash that is stable for a day and
//! then unlinkable.
//!
//! - **Keyed**, with a random secret that exists only in this process's
//!   memory. The IPv4 space is small enough to hash exhaustively, so an
//!   unkeyed hash would be an IP address with extra steps.
//! - **Rotated daily** (UTC day boundary) to a *fresh* random secret, not one
//!   derived from the previous day's. Once a day's secret is dropped, nothing,
//!   including a later memory dump of this process, can recompute that day's
//!   keys. A key in yesterday's logs can no longer be tied to an address.
//!
//! A `PeerKey` is not an identity. It groups requests for fairness (one
//! client's burst must not starve another), and nothing may treat it as "who".
//! Identity is [`crate::subject::SubjectId`]'s alone. See `docs/identity.md`.
//!
//! The digest is SHA-256 over `secret || address`. The input is fixed-length
//! and the output is only ever compared for equality, so length extension (the
//! usual reason to prefer HMAC) gives an attacker nothing here.

use axum::{extract::FromRequestParts, http::request::Parts};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
	convert::Infallible,
	fmt,
	net::{IpAddr, SocketAddr},
	sync::{LazyLock, Mutex},
	time::{SystemTime, UNIX_EPOCH},
};

const SECONDS_PER_DAY: u64 = 86_400;

/// How many bytes of the digest a key keeps. Eight bytes (sixteen hex digits)
/// is far past collision range for the number of concurrent clients one
/// process sees, and short enough to read in a log line.
const KEY_BYTES: usize = 8;

/// A client's address, reduced to something safe to compare, count and log.
///
/// Stable within one UTC day of one process; unrelated across days and across
/// restarts.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PeerKey(String);

impl PeerKey {
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl fmt::Display for PeerKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl fmt::Debug for PeerKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "PeerKey({})", self.0)
	}
}

/// The current day's secret. Replaced, never derived, when the day changes.
struct Keyer {
	day: u64,
	secret: [u8; 32],
}

impl Keyer {
	fn fresh(day: u64) -> Self {
		let mut secret = [0u8; 32];
		rand::rng().fill_bytes(&mut secret);
		Self { day, secret }
	}

	fn key(&mut self, ip: IpAddr, day: u64) -> PeerKey {
		if day != self.day {
			*self = Self::fresh(day);
		}
		let mut digest = Sha256::new();
		digest.update(self.secret);
		// Canonical form, so `::ffff:10.0.0.1` and `10.0.0.1` are one client.
		match ip.to_canonical() {
			IpAddr::V4(v4) => digest.update(v4.octets()),
			IpAddr::V6(v6) => digest.update(v6.octets()),
		}
		PeerKey(hex::encode(&digest.finalize()[..KEY_BYTES]))
	}
}

static KEYER: LazyLock<Mutex<Keyer>> = LazyLock::new(|| Mutex::new(Keyer::fresh(today())));

fn today() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() / SECONDS_PER_DAY)
}

fn key_for(ip: IpAddr) -> PeerKey {
	// A poisoned lock means a panic mid-`key`, which leaves the keyer either
	// on the old secret or a fresh one — both valid. Keep serving.
	let mut keyer = KEYER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
	keyer.key(ip, today())
}

/// The key for the socket's own peer address.
#[must_use]
pub fn peer_key(addr: SocketAddr) -> PeerKey {
	key_for(addr.ip())
}

/// The key for the first hop named in `X-Forwarded-For`, when the header is
/// present and names a parseable address.
///
/// Behind a reverse proxy every socket peer is the proxy itself, so this is
/// the only way WebSocket clients behind one are told apart. The header is
/// client-controlled, which is acceptable only because a `PeerKey` groups
/// connections and is never an identity: forging it buys a caller a different
/// fairness bucket, nothing else.
#[must_use]
pub fn forwarded_peer_key(headers: &axum::http::HeaderMap) -> Option<PeerKey> {
	let first_hop = headers.get("x-forwarded-for")?.to_str().ok()?.split(',').next()?.trim();
	first_hop.parse::<IpAddr>().ok().map(key_for)
}

/// The key every request shares when the router has no connection info.
pub const UNKNOWN_PEER: &str = "unknown-peer";

/// Extracts the requesting peer's [`PeerKey`].
///
/// The one place in the crate allowed to read `ConnectInfo` — `clippy.toml`
/// disallows the type everywhere else, so a handler can't go around this and
/// hold the raw address.
pub struct Peer(pub PeerKey);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for Peer {
	type Rejection = Infallible;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		// Through axum's own extractor rather than a raw extension lookup, so
		// `MockConnectInfo` (which tests install instead of a real listener)
		// is honoured exactly as it is for `ConnectInfo` itself.
		#[allow(clippy::disallowed_types)] // the sanctioned reader; see the type's doc comment
		let addr = axum::extract::ConnectInfo::<SocketAddr>::from_request_parts(parts, state).await.ok().map(|info| info.0);
		// Neither present means a router built without
		// `into_make_service_with_connect_info` — a test harness, in practice.
		// One shared bucket is the honest answer: there is no peer to key.
		Ok(Self(addr.map_or_else(|| PeerKey(UNKNOWN_PEER.to_owned()), peer_key)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use axum::http::HeaderMap;

	fn addr(ip: &str) -> SocketAddr {
		SocketAddr::new(ip.parse().unwrap(), 4242)
	}

	#[test]
	fn a_key_is_stable_for_one_address_within_a_day() {
		assert_eq!(peer_key(addr("10.0.0.1")), peer_key(addr("10.0.0.1")));
	}

	#[test]
	fn the_port_is_not_part_of_the_client() {
		let a = SocketAddr::new("10.0.0.1".parse().unwrap(), 1);
		let b = SocketAddr::new("10.0.0.1".parse().unwrap(), 2);
		assert_eq!(peer_key(a), peer_key(b));
	}

	#[test]
	fn different_addresses_get_different_keys() {
		assert_ne!(peer_key(addr("10.0.0.1")), peer_key(addr("10.0.0.2")));
	}

	#[test]
	fn a_v4_mapped_v6_address_is_the_same_client_as_its_v4_form() {
		assert_eq!(peer_key(addr("::ffff:10.0.0.1")), peer_key(addr("10.0.0.1")));
	}

	/// The property the whole module exists for: nothing that leaves it
	/// contains the address, in any rendering.
	#[test]
	fn a_key_never_contains_the_address_it_came_from() {
		use std::fmt::Write;
		let key = peer_key(addr("192.168.1.77"));
		let mut debug = String::new();
		write!(debug, "{key:?}").unwrap();
		for rendering in [key.to_string(), debug, key.as_str().to_owned()] {
			assert!(!rendering.contains("192.168"), "{rendering}");
			assert!(!rendering.contains("77"), "{rendering}");
		}
	}

	#[test]
	fn a_new_day_is_a_fresh_secret_and_so_an_unrelated_key() {
		let mut keyer = Keyer::fresh(10);
		let before = keyer.secret;
		let monday = keyer.key("10.0.0.1".parse().unwrap(), 10);
		let tuesday = keyer.key("10.0.0.1".parse().unwrap(), 11);
		assert_ne!(monday, tuesday);
		assert_ne!(before, keyer.secret, "the secret is replaced, not reused");
	}

	#[test]
	fn the_first_forwarded_hop_is_keyed_like_a_socket_peer() {
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", "10.0.0.9, 172.16.0.1".parse().unwrap());
		assert_eq!(forwarded_peer_key(&headers), Some(peer_key(addr("10.0.0.9"))));
	}

	#[test]
	fn an_unparseable_forwarded_hop_is_no_key_rather_than_a_key_of_garbage() {
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
		assert_eq!(forwarded_peer_key(&headers), None);
		assert_eq!(forwarded_peer_key(&HeaderMap::new()), None);
	}
}
