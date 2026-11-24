//! `GET /compat?needs=<epoch>`: does home serve a client of epoch `needs`?
//!
//! And if not, is a newer server coming? The decision and its bounds are `crate::deploy`'s; this reads and writes
//! `${STORAGE}/deploy` for it and says what it decided.

use crate::contract::{CONTRACT_EPOCH, OLDEST_SUPPORTED};
use crate::deploy::{self, Deploy};
use crate::metrics::deploy as metrics;
use crate::AppState;
use axum::{
	extract::{Query, State},
	response::Json,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::instrument;

#[derive(Debug, Deserialize)]
pub struct CompatQuery {
	/// The contract epoch the client was built against
	/// (`SERVER_CONTRACT_EPOCH` in the route snapshot it bundles).
	needs: u32,
}

#[derive(Debug, Serialize)]
pub struct Running {
	/// This build's contract epoch.
	epoch: u32,
	/// The lowest epoch it still serves.
	oldest_supported: u32,
	/// The crate version, for a person reading the answer.
	version: &'static str,
}

/// What home's limiter lets each client send (AD1).
///
/// `burst` requests at once, refilling to `per_minute` a minute, one each
/// `60 / per_minute` seconds. A client paces itself to it rather than
/// assume a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RateLimit {
	per_minute: u32,
	burst: u32,
}

impl RateLimit {
	/// What `RATE_LIMIT` means, read from the quota the limiter is built
	/// with (`rate_limiter::quota`).
	#[must_use]
	pub fn of(rate_limit: u32) -> Option<Self> {
		crate::rate_limiter::quota(rate_limit).map(|quota| Self {
			per_minute: rate_limit,
			burst: quota.burst(),
		})
	}
}

#[derive(Debug, Serialize)]
pub struct CompatResponse {
	running: Running,
	needs: u32,
	/// `oldest_supported <= needs <= epoch`.
	supported: bool,
	/// What home does about it (`deploy::Deploy`): `none`, `client_behind`
	/// (the client must update), `refused`, `in_flight`, `unavailable`,
	/// `cooldown`, `wanted` or `cannot_request`.
	deploy: Deploy,
	/// The newest contract epoch home's last pull found, when it read one.
	#[serde(skip_serializing_if = "Option::is_none")]
	latest_seen_epoch: Option<u32>,
	/// When asking again could answer differently (`unavailable`,
	/// `cooldown`).
	#[serde(skip_serializing_if = "Option::is_none")]
	retry_after_s: Option<u64>,
	/// Each client's limit (epoch 14). Absent only on a configuration the
	/// server refuses to start with, a limit of 0.
	#[serde(skip_serializing_if = "Option::is_none")]
	rate_limit: Option<RateLimit>,
}

#[axum::debug_handler]
#[instrument(name = "compat", skip_all, fields(needs = query.needs))]
pub async fn compat(State(state): State<AppState>, Query(query): Query<CompatQuery>) -> Json<CompatResponse> {
	let rate_limit = RateLimit::of(state.core.config.rate_limit);
	Json(answer(Path::new(deploy::FOLDER), query.needs, chrono::Utc::now().timestamp(), rate_limit))
}

/// The answer for `needs` at `now`, against the deploy folder `folder`.
fn answer(folder: &Path, needs: u32, now: i64, rate_limit: Option<RateLimit>) -> CompatResponse {
	// HD3 (#1208): a record or marker that can't be read is decided on as if
	// absent, and said here only at debug: the gauge sweep reads both every
	// tick and holds one that stays unreadable open at `home-deploy`'s
	// `read_state`/`read_marker` (`metrics::deploy`), so a lasting one is
	// told once as a level, not once per ask (docs/home-compute.md,
	// invariant 9). One that fails only at this ask is in the debug line. With no record there is no last pull, so neither
	// `PULL_GAP` nor `UNMET_RETRY` holds a newer client back: the deploy it
	// asks for runs and, when envctl can replace the record, writes it anew.
	// A marker that is there after all can't be linked over (`request`),
	// which is `in_flight`; one that can't be written is `cannot_request`,
	// which alerts.
	let recorded = match deploy::read_recorded(folder) {
		Ok(recorded) => recorded,
		Err(err) => {
			tracing::debug!(needs, error = %err, "home's deploy record can't be read: deciding as if nothing were recorded");
			deploy::Recorded::default()
		}
	};
	let marker_waiting = match deploy::marker_since(folder) {
		Ok(since) => since.is_some(),
		Err(err) => {
			tracing::debug!(needs, error = %err, "home's deploy marker can't be read: deciding as if none were waiting");
			false
		}
	};
	let mut decision = deploy::decide(needs, now, &recorded, marker_waiting);
	if decision.deploy == Deploy::Wanted {
		decision.deploy = match deploy::request(folder, needs, now) {
			Ok(true) => {
				tracing::info!(needs, running = CONTRACT_EPOCH, "a client needs a newer server: deploy requested");
				Deploy::Wanted
			}
			Ok(false) => Deploy::InFlight,
			Err(err) => {
				tracing::warn!(needs, error = %err, "a client needs a newer server, but the deploy request could not be written");
				Deploy::CannotRequest
			}
		};
	}
	metrics::record_compat(decision.deploy);
	CompatResponse {
		running: Running {
			epoch: CONTRACT_EPOCH,
			oldest_supported: OLDEST_SUPPORTED,
			version: env!("CARGO_PKG_VERSION"),
		},
		needs,
		supported: decision.supported,
		deploy: decision.deploy,
		latest_seen_epoch: recorded.latest_seen_epoch,
		retry_after_s: decision.retry_after.map(|after| after.as_secs()),
		rate_limit,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_newer_client_writes_the_marker_once_and_then_reads_in_flight() {
		let dir = tempfile::tempdir().unwrap();
		let first = answer(dir.path(), CONTRACT_EPOCH + 1, 1_800_000_000, RateLimit::of(60));
		assert_eq!(first.deploy, Deploy::Wanted);
		assert!(!first.supported);
		assert!(dir.path().join(deploy::WANTED).exists());
		let second = answer(dir.path(), CONTRACT_EPOCH + 1, 1_800_000_001, RateLimit::of(60));
		assert_eq!(second.deploy, Deploy::InFlight);
	}

	#[test]
	fn a_supported_client_touches_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let answer = answer(dir.path(), CONTRACT_EPOCH, 1_800_000_000, RateLimit::of(60));
		assert!(answer.supported);
		assert_eq!(answer.deploy, Deploy::None);
		assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
	}

	#[test]
	fn no_storage_is_said_not_hidden() {
		let answer = answer(Path::new("/proc/no-such-folder/deploy"), CONTRACT_EPOCH + 1, 1_800_000_000, RateLimit::of(60));
		assert_eq!(answer.deploy, Deploy::CannotRequest);
	}

	#[test]
	fn the_answer_reads_as_the_client_spec_says() {
		let dir = tempfile::tempdir().unwrap();
		let value = serde_json::to_value(answer(dir.path(), CONTRACT_EPOCH, 1_800_000_000, RateLimit::of(60))).unwrap();
		assert_eq!(value["running"]["epoch"], CONTRACT_EPOCH);
		assert_eq!(value["running"]["oldest_supported"], OLDEST_SUPPORTED);
		assert_eq!(value["supported"], true);
		assert_eq!(value["deploy"], "none");
		assert!(value.get("retry_after_s").is_none());
		assert_eq!(value["rate_limit"], serde_json::json!({ "per_minute": 60, "burst": 60 }));
	}

	/// The limit told is the one the limiter enforces, for any `RATE_LIMIT`.
	#[test]
	fn the_rate_limit_told_is_the_limiters() {
		for rate_limit in [1, 30, 60, 600] {
			let told = RateLimit::of(rate_limit).unwrap();
			let quota = crate::rate_limiter::quota(rate_limit).unwrap();
			assert_eq!(told.per_minute, rate_limit);
			assert_eq!(told.burst, quota.burst());
			assert_eq!(quota.interval() * rate_limit, std::time::Duration::from_secs(60));
		}
		assert_eq!(RateLimit::of(0), None);
	}

	/// HD3 (#1208): a garbled `state.json` never holds a newer client back:
	/// it still asks for the deploy, which writes the record anew, and an
	/// ordinary ask is answered as before. Neither logs at warn or error: the
	/// sweep holds it open once, not once per ask (invariant 9).
	#[test]
	fn an_unreadable_deploy_record_still_asks_for_the_deploy_and_is_not_said_per_ask() {
		let (captured, _guard) = crate::privacy::capture_where(|meta| *meta.level() <= tracing::Level::WARN);
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(deploy::STATE), b"{ not json").unwrap();
		let ordinary = answer(dir.path(), CONTRACT_EPOCH, 1_800_000_000, RateLimit::of(60));
		assert_eq!(ordinary.deploy, Deploy::None);
		assert!(!dir.path().join(deploy::WANTED).exists(), "an ordinary ask asks for nothing");
		let answered = answer(dir.path(), CONTRACT_EPOCH + 1, 1_800_000_000, RateLimit::of(60));
		assert_eq!(answered.deploy, Deploy::Wanted);
		assert!(dir.path().join(deploy::WANTED).exists(), "the deploy is asked for");
		assert!(captured.lines().is_empty(), "{:#?}", captured.lines());
	}

	/// HD3: when the marker can't be read (here its folder is a file), the
	/// ask that follows can't write one either, which is `cannot_request`
	/// (`DeployCannotRequest` alerts on it), never a silent `in_flight`.
	#[test]
	fn an_unreadable_marker_reaches_an_alert() {
		let dir = tempfile::tempdir().unwrap();
		let not_a_folder = dir.path().join("deploy");
		std::fs::write(&not_a_folder, b"").unwrap();
		assert!(deploy::marker_since(&not_a_folder).is_err());
		let answered = answer(&not_a_folder, CONTRACT_EPOCH + 1, 1_800_000_000, RateLimit::of(60));
		assert_eq!(answered.deploy, Deploy::CannotRequest);
	}
}
