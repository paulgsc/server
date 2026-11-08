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
//! Accounting that outlives a day needs a key that doesn't rotate under it:
//! a WebSocket held open across midnight keeps its admission permit, and if
//! the next connection from the same address were counted under the new day's
//! key, it would get a fresh per-client allowance — a client could stack
//! connections up to the global limit by outlasting rotations. That is what
//! [`AdmissionKey`] is for: the same keyed digest over a secret that lives as
//! long as the process. Because it *doesn't* rotate, it would link a client
//! across days if it were logged, so it can't be: it has no `Display`, its
//! `Debug` is redacted, and `ws_conn_manager::ConnectionGuard` — the one thing
//! it is handed to — logs no client key at all.
//!
//! The digest is SHA-256 over `secret || address`. The input is fixed-length
//! and the output is only ever compared for equality, so length extension (the
//! usual reason to prefer HMAC) gives an attacker nothing here.

use axum::{
	extract::FromRequestParts,
	http::{request::Parts, HeaderMap},
};
use ipnet::IpNet;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
	convert::Infallible,
	fmt,
	net::{IpAddr, Ipv4Addr, SocketAddr},
	sync::{Arc, LazyLock, Mutex},
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
		Self { day, secret: random_secret() }
	}

	fn key(&mut self, ip: IpAddr, day: u64) -> PeerKey {
		if day != self.day {
			*self = Self::fresh(day);
		}
		PeerKey(keyed(&self.secret, ip))
	}
}

fn random_secret() -> [u8; 32] {
	let mut secret = [0u8; 32];
	rand::rng().fill_bytes(&mut secret);
	secret
}

fn keyed(secret: &[u8; 32], ip: IpAddr) -> String {
	let mut digest = Sha256::new();
	digest.update(secret);
	// Canonical form, so `::ffff:10.0.0.1` and `10.0.0.1` are one client. An
	// IPv6 client is its /64: one device holds a whole /64 and can rotate
	// through it (privacy addresses, or at will), so a full address would let
	// it mint a fresh bucket and admission allowance per address.
	match ip.to_canonical() {
		IpAddr::V4(v4) => digest.update(v4.octets()),
		IpAddr::V6(v6) => digest.update(&v6.octets()[..8]),
	}
	hex::encode(&digest.finalize()[..KEY_BYTES])
}

static KEYER: LazyLock<Mutex<Keyer>> = LazyLock::new(|| Mutex::new(Keyer::fresh(today())));

/// The admission secret: generated once, never rotated, never persisted.
static ADMISSION_SECRET: LazyLock<[u8; 32]> = LazyLock::new(random_secret);

/// A client's address, reduced to a key stable for this process's whole life.
///
/// For accounting that must not reset at midnight (see the module docs): the
/// WebSocket admission guard's, and the HTTP rate limiter's, whose buckets
/// would otherwise all refill at the UTC day boundary when `PeerKey` rotates.
/// Never logged: no `Display`, and `Debug` is redacted.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AdmissionKey(String);

impl AdmissionKey {
	/// For handing to `ConnectionGuard`, whose API is `String`-keyed and which
	/// logs no client key.
	#[must_use]
	pub fn into_string(self) -> String {
		self.0
	}
}

impl fmt::Debug for AdmissionKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("AdmissionKey([redacted])")
	}
}

/// The process-stable admission key for the socket's own peer address.
#[must_use]
pub fn admission_key(addr: SocketAddr) -> AdmissionKey {
	AdmissionKey(keyed(&ADMISSION_SECRET, addr.ip()))
}

/// The reverse proxies whose `X-Forwarded-For` names the client
/// (`TRUSTED_PROXIES`), installed on every request as an extension by
/// `main.rs`. None by default: a request's client is its socket's peer.
///
/// On home every client reaches `file_host` through Caddy on the host, so
/// without this every client is one peer: one rate-limit bucket and one
/// WebSocket admission allowance shared by the phone, the laptop and
/// everything else on the LAN.
///
/// Trust one address, not a range. Whoever is trusted can name any bucket it
/// likes, and a range trusts every container on the shared Docker network and
/// every LAN address in it too (a LAN client's own address then reads as a
/// proxy's, and the walk passes it to whatever the client wrote further left).
/// Home names `gateway`: this container's default gateway, which is where the
/// host's processes, Caddy among them, reach a published port from.
#[derive(Clone, Debug, Default)]
pub struct TrustedProxies(Arc<[IpNet]>);

impl TrustedProxies {
	#[must_use]
	pub fn new(proxies: &[IpNet]) -> Self {
		Self(proxies.into())
	}

	/// From `Config::trusted_proxies`, whose entries its parser has already
	/// checked, resolving `gateway` from `/proc/net/route`. An entry that does
	/// not resolve is left out, and said so: trusting fewer proxies only ever
	/// shares buckets, never splits them.
	#[must_use]
	pub fn from_config(entries: &[String]) -> Self {
		let routes = std::fs::read_to_string("/proc/net/route").ok();
		Self::resolve(entries, routes.as_deref())
	}

	fn resolve(entries: &[String], routes: Option<&str>) -> Self {
		let proxies = entries
			.iter()
			.filter_map(|entry| {
				if entry.trim() == GATEWAY {
					let gateway = routes.and_then(default_gateway);
					if gateway.is_none() {
						tracing::warn!("TRUSTED_PROXIES names the gateway, but this host has no default IPv4 route to read it from; trusting no gateway");
					}
					return gateway.map(|gateway| IpNet::from(IpAddr::V4(gateway)));
				}
				parse_trusted_proxy(entry).ok()
			})
			.collect();
		Self(proxies)
	}

	fn trusts(&self, ip: IpAddr) -> bool {
		let ip = ip.to_canonical();
		self.0.iter().any(|net| net.contains(&ip))
	}
}

/// The `TRUSTED_PROXIES` entry naming this host's default gateway.
pub const GATEWAY: &str = "gateway";

/// Checks one `TRUSTED_PROXIES` entry, as `Config`'s parser.
///
/// # Errors
///
/// When the entry is neither `gateway`, an address nor a CIDR range.
pub fn validate_trusted_proxy(entry: &str) -> Result<String, String> {
	if entry.trim() == GATEWAY {
		return Ok(GATEWAY.to_owned());
	}
	parse_trusted_proxy(entry).map(|_| entry.trim().to_owned())
}

/// The default IPv4 gateway in a `/proc/net/route` table: the row whose
/// destination and mask are both `00000000`. Its addresses are the kernel's
/// network-order bytes printed as a little-endian word, as on every host this
/// runs on (`x86_64`, `aarch64`).
fn default_gateway(routes: &str) -> Option<Ipv4Addr> {
	routes.lines().skip(1).find_map(|line| {
		let fields: Vec<&str> = line.split_whitespace().collect();
		let (destination, gateway, mask) = (fields.get(1)?, fields.get(2)?, fields.get(7)?);
		if *destination != "00000000" || *mask != "00000000" {
			return None;
		}
		let gateway = u32::from_str_radix(gateway, 16).ok().filter(|word| *word != 0)?;
		Some(Ipv4Addr::from(gateway.to_le_bytes()))
	})
}

/// Parses one `TRUSTED_PROXIES` entry: a CIDR range, or a bare address.
///
/// # Errors
///
/// When the entry is neither.
pub fn parse_trusted_proxy(entry: &str) -> Result<IpNet, String> {
	let entry = entry.trim();
	entry
		.parse::<IpNet>()
		.or_else(|_| entry.parse::<IpAddr>().map(IpNet::from))
		.map_err(|_| entry.to_owned() + " is neither an address nor a CIDR range")
}

/// The address a request came from.
///
/// The socket's peer, unless that peer is a trusted proxy: then the nearest
/// `X-Forwarded-For` hop that is not one, walking from the right, since each
/// proxy appends the address it heard from and only the hops our own proxies
/// added can be believed. Anything left of the first untrusted hop was written
/// by the client and is ignored. A hop that does not parse ends the walk at
/// the last address that was believable, which is a proxy's: when in doubt the
/// request shares its proxy's bucket rather than getting a fresh one.
fn client_ip(socket: IpAddr, headers: &HeaderMap, trusted: &TrustedProxies) -> IpAddr {
	if !trusted.trusts(socket) {
		return socket;
	}
	let mut hops = Vec::new();
	for value in headers.get_all("x-forwarded-for") {
		let Ok(value) = value.to_str() else { return socket };
		hops.extend(value.split(','));
	}
	let mut client = socket;
	for hop in hops.into_iter().rev() {
		let Ok(hop) = hop.trim().parse::<IpAddr>() else { return client };
		client = hop;
		if !trusted.trusts(hop) {
			break;
		}
	}
	client
}

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

/// Extracts the requesting peer's keys: the daily [`PeerKey`] for fairness and
/// logs, and the process-stable [`AdmissionKey`] for accounting that outlives a
/// day.
///
/// The one place in the crate allowed to read `ConnectInfo` — `clippy.toml`
/// disallows the type everywhere else, so a handler can't go around this and
/// hold the raw address.
pub struct Peer {
	pub key: PeerKey,
	pub admission: AdmissionKey,
}

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
		let trusted = parts.extensions.get::<TrustedProxies>().cloned().unwrap_or_default();
		Ok(addr.map_or_else(
			|| Self {
				key: PeerKey(UNKNOWN_PEER.to_owned()),
				admission: AdmissionKey(UNKNOWN_PEER.to_owned()),
			},
			|addr| {
				let client = client_ip(addr.ip(), &parts.headers, &trusted);
				Self {
					key: key_for(client),
					admission: AdmissionKey(keyed(&ADMISSION_SECRET, client)),
				}
			},
		))
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
	///
	/// The needles are whole renderings of the address, never fragments: a key
	/// is random hex, so a two-digit fragment like `77` turns up in about 6% of
	/// keys by chance (#377). The dotted forms contain a `.`, which hex never
	/// does. The 8-digit byte encodings turn up by chance about once in 10^9.
	#[test]
	fn a_key_never_contains_the_address_it_came_from() {
		use std::fmt::Write;
		let key = peer_key(addr("192.168.1.77"));
		let mut debug = String::new();
		write!(debug, "{key:?}").unwrap();
		for rendering in [key.to_string(), debug, key.as_str().to_owned()] {
			for needle in ["192.168.1.77", "192.168.", "c0a8014d", "C0A8014D"] {
				assert!(!rendering.contains(needle), "{rendering} contains {needle}");
			}
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

	/// Codex P2 on #374: the admission key must not reset at midnight, or a
	/// client that holds connections across the rotation gets a fresh
	/// per-client allowance each day.
	#[test]
	fn the_admission_key_survives_the_daily_rotation_that_the_peer_key_does_not() {
		let mut keyer = Keyer::fresh(10);
		let ip: IpAddr = "10.0.0.1".parse().unwrap();
		let monday = keyer.key(ip, 10);
		let tuesday = keyer.key(ip, 11);
		assert_ne!(monday, tuesday, "sanity check: the peer key rotated");
		assert_eq!(admission_key(addr("10.0.0.1")), admission_key(addr("10.0.0.1")));
		assert_ne!(admission_key(addr("10.0.0.1")), admission_key(addr("10.0.0.2")));
	}

	#[test]
	fn an_admission_key_is_unrelated_to_the_peer_key_and_redacted_in_debug() {
		use std::fmt::Write;
		let admission = admission_key(addr("10.0.0.1"));
		let mut debug = String::new();
		write!(debug, "{admission:?}").unwrap();
		assert_eq!(debug, "AdmissionKey([redacted])");
		assert_ne!(admission.into_string(), peer_key(addr("10.0.0.1")).as_str());
	}

	#[test]
	fn the_first_forwarded_hop_is_keyed_like_a_socket_peer() {
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", "10.0.0.9, 172.16.0.1".parse().unwrap());
		assert_eq!(forwarded_peer_key(&headers), Some(peer_key(addr("10.0.0.9"))));
	}

	fn proxies(entries: &[&str]) -> TrustedProxies {
		TrustedProxies::new(&entries.iter().map(|entry| parse_trusted_proxy(entry).unwrap()).collect::<Vec<_>>())
	}

	fn forwarded(value: &str) -> HeaderMap {
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", value.parse().unwrap());
		headers
	}

	fn ip(text: &str) -> IpAddr {
		text.parse().unwrap()
	}

	async fn peer(socket: &str, headers: HeaderMap, trusted: Option<TrustedProxies>) -> Peer {
		#[allow(clippy::disallowed_types)] // the sanctioned reader's own test
		let mut request = axum::http::Request::builder().extension(axum::extract::ConnectInfo(addr(socket)));
		if let Some(trusted) = trusted {
			request = request.extension(trusted);
		}
		let (mut parts, ()) = request.body(()).unwrap().into_parts();
		parts.headers = headers;
		Peer::from_request_parts(&mut parts, &()).await.unwrap()
	}

	/// Home's case: every request arrives from Caddy, which names the client in
	/// `X-Forwarded-For`. Two phones behind it are two clients, each with its
	/// own rate-limit bucket and WebSocket allowance, not one shared by the LAN.
	#[tokio::test]
	async fn clients_behind_a_trusted_proxy_are_told_apart() {
		let caddy = Some(proxies(&["172.16.0.0/12"]));
		let one = peer("172.18.0.1", forwarded("192.168.1.20"), caddy.clone()).await;
		let other = peer("172.18.0.1", forwarded("192.168.1.21"), caddy.clone()).await;
		assert_ne!(one.key, other.key);
		assert_ne!(one.admission, other.admission);
		assert_eq!(one.key, peer_key(addr("192.168.1.20")), "keyed as if the client had connected directly");
		assert_eq!(
			peer("172.18.0.1", HeaderMap::new(), caddy).await.key,
			peer_key(addr("172.18.0.1")),
			"no header: the proxy itself"
		);
	}

	/// The inverse, which must not regress: a peer that is not a trusted proxy
	/// cannot name its own bucket by sending the header.
	#[tokio::test]
	async fn an_untrusted_peer_cannot_choose_its_bucket() {
		let caddy = Some(proxies(&["172.16.0.0/12"]));
		assert_eq!(peer("203.0.113.5", forwarded("192.168.1.20"), caddy).await.key, peer_key(addr("203.0.113.5")));
		assert_eq!(
			peer("172.18.0.1", forwarded("192.168.1.20"), None).await.key,
			peer_key(addr("172.18.0.1")),
			"no proxies trusted by default"
		);
	}

	/// Only hops our own proxies appended are believed: whatever the client
	/// put further left is ignored.
	#[test]
	fn only_the_hops_trusted_proxies_added_are_believed() {
		let trusted = proxies(&["172.16.0.0/12", "10.0.0.7"]);
		let socket = ip("172.18.0.1");
		assert_eq!(client_ip(socket, &forwarded("6.6.6.6, 192.168.1.20"), &trusted), ip("192.168.1.20"));
		assert_eq!(client_ip(socket, &forwarded("192.168.1.20, 10.0.0.7"), &trusted), ip("192.168.1.20"), "through two of ours");
		let mut two_headers = forwarded("6.6.6.6");
		two_headers.append("x-forwarded-for", "192.168.1.20".parse().unwrap());
		assert_eq!(client_ip(socket, &two_headers, &trusted), ip("192.168.1.20"), "repeated headers are one list, in order");
	}

	/// A hop that does not read as an address stops the walk at the last
	/// proxy: the request shares that proxy's bucket, never a fresh one.
	#[test]
	fn an_unreadable_hop_falls_back_to_the_proxy() {
		let trusted = proxies(&["172.16.0.0/12"]);
		let socket = ip("172.18.0.1");
		assert_eq!(client_ip(socket, &forwarded("not-an-address"), &trusted), socket);
		assert_eq!(client_ip(socket, &forwarded("192.168.1.20, junk"), &trusted), socket);
		assert_eq!(client_ip(socket, &forwarded("192.168.1.20:5555"), &trusted), socket, "a port is not an address");
	}

	/// `/proc/net/route` as a container on a Docker bridge sees it: the
	/// default route through the bridge's gateway, 172.18.0.1.
	const ROUTES: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
		eth0\t00000000\t010012AC\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
		eth0\t000012AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n";

	#[test]
	fn the_gateway_is_read_from_the_default_route() {
		assert_eq!(default_gateway(ROUTES), Some(Ipv4Addr::new(172, 18, 0, 1)));
		assert_eq!(default_gateway("Iface\tDestination\tGateway\n"), None, "no default route");
		let trusted = TrustedProxies::resolve(&[GATEWAY.to_owned()], Some(ROUTES));
		assert!(trusted.trusts(ip("172.18.0.1")));
		assert!(!trusted.trusts(ip("172.18.0.5")), "another container on the bridge is not the gateway");
		assert!(TrustedProxies::resolve(&[GATEWAY.to_owned()], None).0.is_empty(), "unreadable: trust nothing");
	}

	/// Home's configuration exactly (`TRUSTED_PROXIES=gateway`): Caddy is
	/// believed, and nobody else can name a bucket, whether a container on the
	/// shared network or a LAN client writing its own header for Caddy to append to.
	#[test]
	fn under_homes_configuration_nobody_but_caddy_names_a_client() {
		let trusted = TrustedProxies::resolve(&[GATEWAY.to_owned()], Some(ROUTES));
		let caddy = ip("172.18.0.1");
		assert_eq!(client_ip(caddy, &forwarded("192.168.1.20"), &trusted), ip("192.168.1.20"));
		assert_eq!(
			client_ip(caddy, &forwarded("6.6.6.6, 192.168.1.20"), &trusted),
			ip("192.168.1.20"),
			"what the client wrote is ignored"
		);
		assert_eq!(
			client_ip(ip("172.18.0.5"), &forwarded("6.6.6.6"), &trusted),
			ip("172.18.0.5"),
			"a container is its own address"
		);
		assert_eq!(
			client_ip(ip("192.168.1.20"), &forwarded("6.6.6.6"), &trusted),
			ip("192.168.1.20"),
			"a LAN host reaching the port directly is its own address"
		);
	}

	/// One device holds a whole /64; keying the full address would let it
	/// rotate into a fresh bucket per address.
	#[test]
	fn an_ipv6_client_is_its_slash_64() {
		let a = SocketAddr::new(ip("2001:db8:1:2::1"), 1);
		let b = SocketAddr::new(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), 1);
		let other = SocketAddr::new(ip("2001:db8:1:3::1"), 1);
		assert_eq!(peer_key(a), peer_key(b));
		assert_eq!(admission_key(a), admission_key(b));
		assert_ne!(peer_key(a), peer_key(other));
	}

	#[test]
	fn a_trusted_proxy_entry_is_a_range_or_an_address() {
		assert_eq!(parse_trusted_proxy("10.0.0.0/8").unwrap().to_string(), "10.0.0.0/8");
		assert_eq!(parse_trusted_proxy(" 127.0.0.1 ").unwrap().to_string(), "127.0.0.1/32");
		assert_eq!(parse_trusted_proxy("::1").unwrap().to_string(), "::1/128");
		assert!(parse_trusted_proxy("caddy").is_err());
		assert_eq!(validate_trusted_proxy(" gateway ").unwrap(), GATEWAY);
		assert!(validate_trusted_proxy("caddy").is_err());
		assert!(proxies(&["172.16.0.0/12"]).trusts(ip("::ffff:172.18.0.1")), "a v4-mapped peer is its v4 form");
	}

	#[test]
	fn an_unparseable_forwarded_hop_is_no_key_rather_than_a_key_of_garbage() {
		let mut headers = HeaderMap::new();
		headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
		assert_eq!(forwarded_peer_key(&headers), None);
		assert_eq!(forwarded_peer_key(&HeaderMap::new()), None);
	}
}
