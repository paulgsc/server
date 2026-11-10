//! `GET /compat?needs=<epoch>`: does home serve a client of epoch `needs`?
//!
//! And if not, is a newer server coming? The decision and its bounds are `crate::deploy`'s; this reads and writes
//! `${STORAGE}/deploy` for it and says what it decided.

use crate::contract::{CONTRACT_EPOCH, OLDEST_SUPPORTED};
use crate::deploy::{self, Deploy};
use crate::metrics::deploy as metrics;
use axum::{extract::Query, response::Json};
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
}

#[axum::debug_handler]
#[instrument(name = "compat", skip_all, fields(needs = query.needs))]
pub async fn compat(Query(query): Query<CompatQuery>) -> Json<CompatResponse> {
	Json(answer(Path::new(deploy::FOLDER), query.needs, chrono::Utc::now().timestamp()))
}

/// The answer for `needs` at `now`, against the deploy folder `folder`.
fn answer(folder: &Path, needs: u32, now: i64) -> CompatResponse {
	let recorded = deploy::read_recorded(folder);
	let mut decision = deploy::decide(needs, now, &recorded, deploy::marker_since(folder).is_some());
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
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_newer_client_writes_the_marker_once_and_then_reads_in_flight() {
		let dir = tempfile::tempdir().unwrap();
		let first = answer(dir.path(), CONTRACT_EPOCH + 1, 1_800_000_000);
		assert_eq!(first.deploy, Deploy::Wanted);
		assert!(!first.supported);
		assert!(dir.path().join(deploy::WANTED).exists());
		let second = answer(dir.path(), CONTRACT_EPOCH + 1, 1_800_000_001);
		assert_eq!(second.deploy, Deploy::InFlight);
	}

	#[test]
	fn a_supported_client_touches_nothing() {
		let dir = tempfile::tempdir().unwrap();
		let answer = answer(dir.path(), CONTRACT_EPOCH, 1_800_000_000);
		assert!(answer.supported);
		assert_eq!(answer.deploy, Deploy::None);
		assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
	}

	#[test]
	fn no_storage_is_said_not_hidden() {
		let answer = answer(Path::new("/proc/no-such-folder/deploy"), CONTRACT_EPOCH + 1, 1_800_000_000);
		assert_eq!(answer.deploy, Deploy::CannotRequest);
	}

	#[test]
	fn the_answer_reads_as_the_client_spec_says() {
		let dir = tempfile::tempdir().unwrap();
		let value = serde_json::to_value(answer(dir.path(), CONTRACT_EPOCH, 1_800_000_000)).unwrap();
		assert_eq!(value["running"]["epoch"], CONTRACT_EPOCH);
		assert_eq!(value["running"]["oldest_supported"], OLDEST_SUPPORTED);
		assert_eq!(value["supported"], true);
		assert_eq!(value["deploy"], "none");
		assert!(value.get("retry_after_s").is_none());
	}
}
