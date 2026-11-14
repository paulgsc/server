//! Home's deploy: one release, applied whole or put back whole (RELEASE R4,
//! #793; epic #787; docs/release.md). `make home-deploy` runs one pass
//! (`envctl deploy`); the on-demand unit (`nix/modules/home-deploy.nix`)
//! runs `envctl deploy --on-demand`. It is Rust, like everything else home
//! runs: home has no `python3` (server#786).
//!
//! In order:
//!
//! 1. **Resolves the release**: `git fetch origin main` in the owner's
//!    checkout (only its object store moves, never its working tree), then
//!    `deploy/<role>/release.json` at that commit. A release that failed here
//!    before is not tried again: the release last applied is, until a newer
//!    one forms.
//! 2. **Pulls every image it names, by digest** (`--no-pull` uses the images
//!    already here).
//! 3. **Checks the set fits together** (RL8): every `io.paulgsc.requires`
//!    label is met by another image's `io.paulgsc.provides`, read from the
//!    pulled images (`release/constraints.rs`, the rule CI ran from the
//!    registry). It reads them after the pull, since home has no registry
//!    client: nothing has been replaced yet, so an unmet one stops the pass
//!    with exit 1.
//! 4. **Stops here when nothing differs**: every service runs the image the
//!    release names, and is ready.
//! 5. **Brings the release up** from its own tree: a worktree of the
//!    checkout at the release's commit, under the state directory's
//!    `releases/`, and `envctl up` on it under the stack's compose project.
//!    `up -d` recreates only what changed. Untracked state (file_host's data,
//!    logs and config, Grafana's built dashboards) is mounted from the
//!    owner's checkout, `STATE_ROOT` (C344; RL9). `file_host` applies its
//!    migrations as it starts, after a verified backup (AS16).
//! 6. **Gates every service, up to `--ready-timeout` seconds**: `healthy`
//!    where it has a healthcheck, otherwise running with no restart for
//!    [`STABLE_SECONDS`]. A container that exits, restarts or turns
//!    `unhealthy` fails at once.
//! 7. **Rolls back whole** when any service does not come up: the release
//!    last applied, its tree and its images, through the same `up` and the
//!    same gate. The failed release is remembered (by serial) and skipped.
//!
//! It never touches the database. A migration that ran stays applied: a
//! database ahead of its binary is one the old binary serves (`schema.rs`),
//! and the owner's answer (2026-11-10) is that a restore is never automatic.
//! README.md, "Deploying to home", is the manual restore.
//!
//! Exit status:
//!
//! - 0 deployed, or nothing to deploy
//! - 1 not deployed: main's release could not be read, a pull failed, or the
//!   set does not fit together; nothing was replaced
//! - 2 rolled back: the release did not come up; the previous one serves
//! - 3 nothing ready: the owner has to look (the message says what is known)
//!
//! The decisions (`judge_service`, `wait_all`, `apply`, `deploy` and
//! `on_demand` over a [`Host`] and [`Demand`] they are handed) are pure, so
//! the tests below run them against a simulated host and clock (`Lint`'s
//! `Deploy script` job).

use crate::release::{self, constraints, Release};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

/// file-host's compose service and its `container_name`.
/// `the_service_is_the_compose_files` holds these to
/// `infra/compose/file_host.yml`.
pub(crate) const FILE_HOST: &str = "file-host";
pub(crate) const CONTAINER: &str = "file-host-server";

/// The bound on every service becoming ready, in seconds. `file_host` copies
/// the database and applies its migrations before it listens, so the
/// healthcheck's `start_period` (`infra/compose/file_host.yml`, 300 s) covers
/// that work: no failed probe counts during it, and the first that passes
/// reads healthy. The bound is above `start_period`, so a slow migrating
/// start is never taken for a bad release. A start that exits or restarts
/// fails at once, whatever the bound, so only a release that hangs costs all
/// of it. `the_bound_outlasts_the_start_period` holds the two together.
pub(crate) const DEFAULT_READY_TIMEOUT: f64 = 360.0;
pub(crate) const POLL_SECONDS: f64 = 2.0;

pub(crate) const EXIT_DEPLOYED: u8 = 0;
pub(crate) const EXIT_NOT_DEPLOYED: u8 = 1;
pub(crate) const EXIT_ROLLED_BACK: u8 = 2;
pub(crate) const EXIT_NOTHING_READY: u8 = 3;

/// What `file_host` prints at start when it took a backup
/// (`apps/servers/file_host/src/main.rs`, `migrate_on_start`).
pub(crate) const BACKUP_LINE: &str = "migrate: backed up to ";

/// How many failed releases the state directory remembers.
const FAILED_KEPT: usize = 20;

// ── The decisions, pure ─────────────────────────────────────────────────────

/// One look at a container, as `docker inspect` gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reading {
	/// None: no such container.
	pub(crate) container: Option<String>,
	/// created, running, restarting, exited, dead, ...
	pub(crate) status: Option<String>,
	/// starting, healthy, unhealthy; None without a healthcheck.
	pub(crate) health: Option<String>,
	pub(crate) restarts: u64,
	pub(crate) image: Option<String>,
}

impl Reading {
	fn gone() -> Self {
		Self::default()
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
	Ready,
	Wait,
	Failed(String),
	Unjudgeable(String),
}

/// What one reading says about whether file-host came up.
///
/// Only `healthy` is ready. A container that is gone, has exited, is being
/// restarted or has restarted since the wait began has failed now: a server
/// that exits at start (a migration it refused, a config it refuses) will
/// not become ready by waiting. `unhealthy` is Docker giving up. No
/// healthcheck at all is not the image's fault, so it is unjudgeable.
pub(crate) fn judge(reading: &Reading, baseline_restarts: u64) -> Verdict {
	if reading.container.is_none() {
		return Verdict::Failed(format!("no {FILE_HOST} container"));
	}
	let status = reading.status.as_deref();
	if let Some(status @ ("exited" | "dead" | "restarting" | "removing" | "paused")) = status {
		return Verdict::Failed(format!("the container is {status}"));
	}
	if reading.restarts > baseline_restarts {
		return Verdict::Failed(format!("the container restarted {} time(s)", reading.restarts - baseline_restarts));
	}
	if status != Some("running") {
		return Verdict::Wait;
	}
	match reading.health.as_deref() {
		None => Verdict::Unjudgeable("the container has no healthcheck, so its readiness cannot be read".to_owned()),
		Some("healthy") => Verdict::Ready,
		Some("unhealthy") => Verdict::Failed("its healthcheck says unhealthy".to_owned()),
		Some(_) => Verdict::Wait,
	}
}

/// Monotonic seconds, and a way to let them pass.

/// Monotonic seconds, and a way to let them pass.
pub(crate) trait Clock {
	fn now(&self) -> f64;
	fn sleep(&self, seconds: f64);
}

/// A container without a healthcheck counts as ready once it has run this
/// many seconds of the wait with no restart (#793, "gate").
pub(crate) const STABLE_SECONDS: f64 = 60.0;

/// What one reading says about one service of a release, `observed`
/// seconds into the wait: [`judge`], except that a service without a
/// healthcheck is ready once it has run [`STABLE_SECONDS`] with no restart.
pub(crate) fn judge_service(reading: &Reading, baseline_restarts: u64, observed: f64) -> Verdict {
	match judge(reading, baseline_restarts) {
		Verdict::Unjudgeable(_) if observed >= STABLE_SECONDS => Verdict::Ready,
		Verdict::Unjudgeable(_) => Verdict::Wait,
		verdict => verdict,
	}
}

/// Polls every service until all are ready, one fails, or `timeout` seconds
/// pass. Returns the first failure, if any (service, reason), and the
/// seconds it took.
pub(crate) fn wait_all(services: &[String], read: &mut dyn FnMut(&str) -> Reading, clock: &dyn Clock, timeout: f64, poll: f64) -> (Option<(String, String)>, f64) {
	let start = clock.now();
	let baselines: Vec<u64> = services.iter().map(|service| read(service).restarts).collect();
	loop {
		let elapsed = clock.now() - start;
		let mut waiting = None;
		for (service, baseline) in services.iter().zip(&baselines) {
			let reading = read(service);
			match judge_service(&reading, *baseline, elapsed) {
				Verdict::Ready => {}
				Verdict::Wait => {
					waiting.get_or_insert_with(|| (service.clone(), reading.health.or(reading.status).unwrap_or_else(|| "unknown".to_owned())));
				}
				Verdict::Failed(reason) | Verdict::Unjudgeable(reason) => return (Some((service.clone(), reason)), elapsed),
			}
		}
		let Some((service, state)) = waiting else {
			return (None, elapsed);
		};
		if elapsed >= timeout {
			return (Some((service, format!("not ready within {timeout:.0} s (last seen: {state})"))), elapsed);
		}
		clock.sleep(poll.min(timeout - elapsed));
	}
}

// ── The deploy, over a host it is handed ────────────────────────────────────

/// A release, and the commit of main it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
	pub(crate) commit: String,
	pub(crate) release: Release,
}

impl Target {
	/// Its services, in name order.
	fn services(&self) -> Vec<String> {
		self.release.services.keys().cloned().collect()
	}

	/// `serial 7 (main 1a2b3c4)`.
	fn name(&self) -> String {
		format!("serial {} (main {})", self.release.serial, self.commit.get(..7).unwrap_or(&self.commit))
	}
}

/// What a pass needs of git, Docker, compose and its own records.
pub(crate) trait Host {
	/// `git fetch origin main`, then main's commit and the release it holds.
	fn newest(&mut self) -> Result<Target, String>;
	/// The release a commit holds.
	fn at(&mut self, commit: &str) -> Result<Target, String>;
	/// The commit whose release was last applied whole here, if one was.
	fn applied(&mut self) -> Option<String>;
	fn record_applied(&mut self, commit: &str);
	/// The service's container.
	fn reading(&mut self, service: &str) -> Reading;
	fn image_id(&mut self, image_ref: &str) -> Option<String>;
	/// An image's labels; None when it cannot be read.
	fn labels(&mut self, image_id: &str) -> Option<BTreeMap<String, String>>;
	fn describe(&mut self, image_id: &str) -> String;
	fn pull(&mut self, image_ref: &str) -> bool;
	/// The release's tree and images up: `docker compose up -d`, which
	/// recreates the services whose image or configuration changed.
	fn up(&mut self, target: &Target) -> bool;
	fn logs(&mut self, service: &str) -> Vec<String>;
	/// The serials of releases that failed here.
	fn failed_releases(&mut self) -> Vec<u64>;
	fn remember_failed(&mut self, serial: u64);
}

#[derive(Clone, Debug)]
pub(crate) struct Options {
	pub(crate) pull: bool,
	pub(crate) ready_timeout: f64,
	pub(crate) poll: f64,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			pull: true,
			ready_timeout: DEFAULT_READY_TIMEOUT,
			poll: POLL_SECONDS,
		}
	}
}

/// What the pass did, beyond its exit status: the releases state.json
/// records (`release`, `previous_release`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
	pub(crate) code: u8,
	/// The release serving when the pass ended, when one is known.
	pub(crate) serving: Option<u64>,
	/// The release that served before it, when it changed.
	pub(crate) previous: Option<u64>,
}

/// Pulls every image the release names (or, without `pull`, checks each is
/// here), and returns the image id of each service.
fn fetch(host: &mut dyn Host, target: &Target, pull: bool, say: &mut dyn FnMut(&str)) -> Option<BTreeMap<String, String>> {
	let mut ids = BTreeMap::new();
	for (service, entry) in &target.release.services {
		if pull && !host.pull(&entry.image) {
			say(&format!("deploy: pulling {} ({service}) failed; nothing was replaced", entry.image));
			return None;
		}
		let Some(id) = host.image_id(&entry.image) else {
			say(&format!("deploy: {} ({service}) is not here; nothing was replaced", entry.image));
			return None;
		};
		ids.insert(service.clone(), id);
	}
	Some(ids)
}

/// Every requirement the release's images state is met inside it
/// (docs/release.md, RL8), read from the images here, by their labels.
fn unmet_here(host: &mut dyn Host, ids: &BTreeMap<String, String>) -> Vec<String> {
	let mut images = BTreeMap::new();
	let mut problems = Vec::new();
	for (service, id) in ids {
		match host.labels(id).map(|labels| constraints::Constraints::from_labels(&labels)) {
			Some(Ok(found)) => {
				images.insert(service.clone(), found);
			}
			Some(Err(err)) => problems.push(format!("{service}: {err}")),
			None => problems.push(format!("{service}: its image's labels cannot be read")),
		}
	}
	problems.extend(constraints::unmet(&images));
	problems
}

/// One pass: see the module's documentation. Resolves main's release and
/// applies it.
pub(crate) fn deploy(host: &mut dyn Host, options: &Options, clock: &dyn Clock, say: &mut dyn FnMut(&str)) -> Outcome {
	let target = match host.newest() {
		Ok(target) => target,
		Err(err) => {
			say(&format!("deploy: main's release could not be read: {err}; nothing was replaced"));
			return Outcome {
				code: EXIT_NOT_DEPLOYED,
				..Outcome::default()
			};
		}
	};
	apply(host, &target, options, clock, say)
}

/// Applies one release whole, or puts the previous one back whole.
#[allow(clippy::too_many_lines)] // one pass, read top to bottom as the module's numbered steps
pub(crate) fn apply(host: &mut dyn Host, target: &Target, options: &Options, clock: &dyn Clock, say: &mut dyn FnMut(&str)) -> Outcome {
	let previous = match host.applied() {
		Some(commit) if commit == target.commit => Some(target.clone()),
		Some(commit) => match host.at(&commit) {
			Ok(found) => Some(found),
			Err(err) => {
				say(&format!("deploy: the release last applied (main {commit}) cannot be read ({err}), so there is none to roll back to"));
				None
			}
		},
		None => None,
	};
	match &previous {
		Some(previous) => say(&format!("deploy: home last applied release {}", previous.name())),
		None => say("deploy: no release is recorded as applied here"),
	}
	say(&format!("deploy: main's release is {}", target.name()));
	let not_deployed = |previous: &Option<Target>| Outcome {
		code: EXIT_NOT_DEPLOYED,
		serving: previous.as_ref().map(|previous| previous.release.serial),
		previous: None,
	};

	// A release that failed here is not tried again: the previous one is
	// applied instead, which brings back anything it left stopped.
	let mut target = target.clone();
	if previous.as_ref().is_some_and(|previous| previous.release != target.release) && host.failed_releases().contains(&target.release.serial) {
		let Some(previous) = previous.clone() else {
			return not_deployed(&previous);
		};
		say(&format!(
			"deploy: release {} failed here before; staying on {} until a newer release forms",
			target.name(),
			previous.name()
		));
		target = previous;
	}

	let Some(ids) = fetch(host, &target, options.pull, say) else {
		return not_deployed(&previous);
	};
	let problems = unmet_here(host, &ids);
	if !problems.is_empty() {
		say(&format!("deploy: release {} does not fit together; nothing was replaced:", target.name()));
		for problem in &problems {
			say(&format!("deploy:   {problem}"));
		}
		return not_deployed(&previous);
	}

	// Nothing to deploy only when every service runs its image and is ready.
	let before: BTreeMap<String, Reading> = target.services().into_iter().map(|service| (service.clone(), host.reading(&service))).collect();
	let changed: Vec<&String> = ids.iter().filter(|(service, id)| before[*service].image.as_ref() != Some(*id)).map(|(service, _)| service).collect();
	let stopped: Vec<&String> = before
		.iter()
		.filter(|(_, reading)| !matches!(judge_service(reading, reading.restarts, STABLE_SECONDS), Verdict::Ready))
		.map(|(service, _)| service)
		.collect();
	if changed.is_empty() && stopped.is_empty() {
		host.record_applied(&target.commit);
		say("deploy: every service runs this release's image; nothing restarted");
		return Outcome {
			code: EXIT_DEPLOYED,
			serving: Some(target.release.serial),
			previous: None,
		};
	}
	for service in &changed {
		let described = host.describe(&ids[*service]);
		say(&format!("deploy: {service} moves to {described}"));
	}
	for service in stopped.iter().filter(|service| !changed.contains(service)) {
		say(&format!("deploy: {service} runs this release's image but is not ready; bringing it up"));
	}

	let up_ok = host.up(&target);
	if !up_ok {
		say("deploy: up failed (its reason is above); judging what it left");
	}
	say(&format!("deploy: waiting up to {:.0} s for every service to be ready", options.ready_timeout));
	let services = target.services();
	let (failure, took) = wait_all(&services, &mut |service| host.reading(service), clock, options.ready_timeout, options.poll);
	let Some((service, reason)) = failure else {
		host.record_applied(&target.commit);
		say(&format!("deploy: release {} is ready after {took:.0} s", target.name()));
		return Outcome {
			code: EXIT_DEPLOYED,
			serving: Some(target.release.serial),
			previous: previous.filter(|previous| previous.release != target.release).map(|previous| previous.release.serial),
		};
	};
	say(&format!("deploy: {service} did not come up: {reason}"));
	let lines = host.logs(&service);
	let backup = backup_from(&host.logs(FILE_HOST));
	for line in &lines[lines.len().saturating_sub(5)..] {
		say(&format!("deploy: its log: {line}"));
	}

	let Some(previous) = previous.filter(|previous| previous.release != target.release) else {
		let why = if host.applied().is_some() {
			"the release did not change, so the cause is not the release: look at deploy/<role>/, the config it refuses, or a dependency"
		} else {
			"no release was applied here before this one, so there is none to put back"
		};
		say(&format!("deploy: not rolling back: {why}"));
		say(DATABASE_NOTE);
		say(&stuck_note(backup.as_deref()));
		return Outcome {
			code: EXIT_NOTHING_READY,
			serving: None,
			previous: None,
		};
	};

	host.remember_failed(target.release.serial);
	say(&format!("deploy: ROLLING BACK to release {}, whole", previous.name()));
	let back = fetch(host, &previous, options.pull, say).is_some() && host.up(&previous);
	let (failure, took_back) = if back {
		wait_all(&previous.services(), &mut |service| host.reading(service), clock, options.ready_timeout, options.poll)
	} else {
		(Some((String::from("the rollback"), String::from("its images or up failed"))), 0.0)
	};
	if let Some((service, reason)) = failure {
		say(&format!("deploy: the previous release did not come up either: {service}: {reason}"));
		say(DATABASE_NOTE);
		say(&stuck_note(backup.as_deref()));
		return Outcome {
			code: EXIT_NOTHING_READY,
			serving: None,
			previous: None,
		};
	}
	host.record_applied(&previous.commit);
	say(&format!(
		"deploy: ROLLED BACK: release {} is ready after {took_back:.0} s. Release {} failed ({service}) and is skipped until a newer release forms.",
		previous.name(),
		target.name()
	));
	say(DATABASE_NOTE);
	Outcome {
		code: EXIT_ROLLED_BACK,
		serving: Some(previous.release.serial),
		previous: None,
	}
}


/// The backup `file_host` took at this start, from its log, if it took one.
pub(crate) fn backup_from(lines: &[String]) -> Option<String> {
	let mut found = None;
	for line in lines {
		if let Some(at) = line.find(BACKUP_LINE) {
			let rest = &line[at + BACKUP_LINE.len()..];
			found = Some(rest.strip_suffix(" (verified)").unwrap_or(rest).trim().to_owned());
		}
	}
	found
}

pub(crate) const DATABASE_NOTE: &str = "The database stays where the migrations put it: the old image serves a database ahead of it \
	(apps/servers/file_host/src/schema.rs). Nothing was restored or deleted.";

pub(crate) fn stuck_note(backup: Option<&str>) -> String {
	let restore = backup.map_or_else(
		|| "This start took no backup that its log names. ".to_owned(),
		|backup| format!("This start backed the database up to {backup} (on the host, under ${{STORAGE}}/backups). "),
	);
	format!(
		"{restore}It is never restored automatically: every write since it would leave the live database. \
		 If restoring is the way out, move the live file aside first, never delete it \
		 (README.md, \"Deploying to home\")."
	)
}


// ── On demand: a client needed a newer server ─────────────────────────────
//
// file_host writes ${STORAGE}/deploy/wanted when a client asks `/compat` for a
// contract epoch above its own (apps/servers/file_host/src/deploy.rs, which
// holds the bounds: one marker at a time, a gap between pulls, a ceiling, a
// need no published image met waits six hours). A systemd path unit
// (nix/modules/home-deploy.nix) runs `envctl deploy --on-demand` when it
// appears. This pass resolves main's release, pulls its file_host image,
// reads that image's epoch, applies the release (`apply`, whole) only when
// the epoch is above the running one, records what it did in state.json
// beside the marker (which file_host reads for its answers and metrics, and
// the DeployFailed and DeployStuck alerts from those), and removes the
// marker. Nothing else ever pulls: with no client newer than home, home sends
// no request anywhere.

/// The shortest time between two pulls: `deploy.rs`'s `PULL_GAP`, checked here
/// again so a marker written by hand cannot make home pull twice in a row.
pub(crate) const PULL_GAP_SECONDS: i64 = 600;

/// `${STORAGE}/deploy`: the marker `file_host` writes and the state it reads.
pub(crate) trait Demand {
	/// None: no marker. An unreadable or malformed one is an empty object.
	fn read_marker(&mut self) -> Option<Map<String, Value>>;
	fn clear_marker(&mut self);
	fn read_state(&mut self) -> Map<String, Value>;
	fn write_state(&mut self, state: &Map<String, Value>) -> Result<()>;
	fn image_epoch(&mut self, image_id: &str) -> Option<i64>;
	fn image_bytes(&mut self, image_id: &str) -> i64;
}

/// What each exit of `deploy` is counted as in state.json's `counts`.
const fn outcome_of_exit(code: u8) -> &'static str {
	match code {
		EXIT_DEPLOYED => "deployed",
		EXIT_ROLLED_BACK => "rolled_back",
		_ => "failed",
	}
}

/// An integer, never a bool or anything else (which reads as `default`).
fn as_int(value: Option<&Value>, default: i64) -> i64 {
	match value {
		Some(Value::Number(number)) => number.as_i64().unwrap_or(default),
		_ => default,
	}
}

/// state.json's `counts`, each value an integer.
fn counts_of(state: &Map<String, Value>) -> Map<String, Value> {
	match state.get("counts") {
		Some(Value::Object(raw)) => raw.iter().map(|(key, value)| (key.clone(), json!(as_int(Some(value), 0)))).collect(),
		_ => Map::new(),
	}
}

fn bump(counts: &mut Map<String, Value>, key: &str, by: i64) {
	let next = as_int(counts.get(key), 0) + by;
	counts.insert(key.to_owned(), json!(next));
}

/// Seconds since the epoch, whole, as state.json records them.
#[allow(clippy::cast_possible_truncation)] // Unix seconds, far inside i64
const fn whole(seconds: f64) -> i64 {
	seconds as i64
}


/// The contract epoch an image provides: its `contract` label (R3), or,
/// for an image from before the labels, what the binary itself says.
fn epoch_of(host: &mut dyn Host, demand: &mut dyn Demand, image_id: &str) -> Option<i64> {
	let labelled = host
		.labels(image_id)
		.and_then(|labels| constraints::Constraints::from_labels(&labels).ok())
		.and_then(|found| found.provides.get("contract").and_then(|epoch| epoch.parse().ok()));
	labelled.or_else(|| demand.image_epoch(image_id))
}

/// The on-demand pass. An error is state.json that could not be written;
/// the caller records the failure and clears the marker
/// ([`record_crash`]).
#[allow(clippy::too_many_lines)] // one pass, read top to bottom
pub(crate) fn on_demand(host: &mut dyn Host, demand: &mut dyn Demand, options: &Options, wall: &dyn Fn() -> f64, clock: &dyn Clock, say: &mut dyn FnMut(&str)) -> Result<u8> {
	let Some(marker) = demand.read_marker() else {
		say("deploy: no deploy is wanted; nothing to do");
		return Ok(EXIT_DEPLOYED);
	};
	let needs = as_int(marker.get("needs"), 0);
	let mut running = as_int(marker.get("running"), -1);
	if running < 0 {
		// A marker without what home ran (unreadable, or written by hand):
		// ask the serving image itself, never assume 0, which would make any
		// pulled image look newer. (An image that says 0 is as good as none.)
		let serving_now = host.reading(FILE_HOST);
		running = match (&serving_now.container, &serving_now.image) {
			(Some(_), Some(image)) => epoch_of(host, demand, image).filter(|epoch| *epoch != 0).unwrap_or(-1),
			_ => -1,
		};
	}
	if running < 0 {
		say("deploy: the marker does not say which epoch home runs, and the serving image cannot either; not deploying");
		demand.clear_marker();
		return Ok(EXIT_NOT_DEPLOYED);
	}
	say(&format!("deploy: a client needs contract epoch {needs}; home runs {running}"));

	let mut state = demand.read_state();
	let mut counts = counts_of(&state);

	let finish = |demand: &mut dyn Demand, state: &mut Map<String, Value>, counts: &mut Map<String, Value>, code: u8, outcome: &str| -> Result<u8> {
		bump(counts, outcome, 1);
		state.insert("counts".to_owned(), Value::Object(counts.clone()));
		state.insert("last_run".to_owned(), json!({"at": whole(wall()), "exit": code, "outcome": outcome, "needs": needs}));
		demand.write_state(state)?;
		demand.clear_marker();
		Ok(code)
	};

	if let Some(Value::Number(last_pull)) = state.get("last_pull_at") {
		if let Some(last_pull) = last_pull.as_i64() {
			#[allow(clippy::cast_precision_loss)] // Unix seconds and 600, far below 2^52
			let (since, gap) = (wall() - last_pull as f64, PULL_GAP_SECONDS as f64);
			if since < gap {
				say(&format!(
					"deploy: the last pull was {} s ago, under the {PULL_GAP_SECONDS} s gap; not pulling",
					whole(since)
				));
				return finish(demand, &mut state, &mut counts, EXIT_DEPLOYED, "deferred");
			}
		}
	}

	// The newest release is main's: an older one never provides more.
	let target = match host.newest() {
		Ok(target) => target,
		Err(err) => {
			say(&format!("deploy: main's release could not be read: {err}; nothing was replaced"));
			return finish(demand, &mut state, &mut counts, EXIT_NOT_DEPLOYED, "failed");
		}
	};
	let Some(file_host) = target.release.services.get(FILE_HOST).map(|entry| entry.image.clone()) else {
		say(&format!("deploy: release {} names no {FILE_HOST}; nothing was replaced", target.name()));
		return finish(demand, &mut state, &mut counts, EXIT_NOT_DEPLOYED, "failed");
	};
	let previous_id = host.image_id(&file_host);
	let pulled = host.pull(&file_host);
	bump(&mut counts, "pulls", 1);
	state.insert("last_pull_at".to_owned(), json!(whole(wall())));
	if !pulled {
		say(&format!("deploy: pulling {file_host} failed; nothing was replaced"));
		return finish(demand, &mut state, &mut counts, EXIT_NOT_DEPLOYED, "failed");
	}
	let candidate = host.image_id(&file_host);
	if let Some(found) = candidate.as_deref().filter(|found| Some(*found) != previous_id.as_deref()) {
		let bytes = demand.image_bytes(found);
		bump(&mut counts, "pulled_bytes", bytes);
	}
	let epoch = candidate.as_deref().and_then(|found| epoch_of(host, demand, found));
	state.insert("latest_seen_epoch".to_owned(), json!(epoch));
	let Some(epoch) = epoch.filter(|epoch| *epoch > running) else {
		// Lazily on the running release (the owner, 2026-11-10): a newer
		// build of the same contract is not a reason to restart. It waits
		// for a `make home-deploy`.
		let seen = epoch.map_or_else(|| "an epoch it could not read".to_owned(), |epoch| format!("epoch {epoch}"));
		say(&format!(
			"deploy: release {}'s {FILE_HOST} has {seen}, not above home's {running}; staying on the running release",
			target.name()
		));
		if epoch.is_some_and(|epoch| epoch < needs) {
			say(&format!(
				"deploy: nothing published serves epoch {needs} yet; home answers `unavailable` to it for six hours"
			));
		}
		return finish(demand, &mut state, &mut counts, EXIT_DEPLOYED, "no_newer_image");
	};

	say(&format!("deploy: release {}'s {FILE_HOST} has epoch {epoch}; applying the release", target.name()));
	let outcome = apply(host, &target, options, clock, say);
	if let Some(serving) = outcome.serving {
		state.insert("release".to_owned(), json!(serving));
	}
	if let Some(previous) = outcome.previous {
		state.insert("previous_release".to_owned(), json!(previous));
	}
	finish(demand, &mut state, &mut counts, outcome.code, outcome_of_exit(outcome.code))
}

/// Never leave the marker behind a crash: the path unit would start the pass
/// again at once. The failure is recorded for the alert.
pub(crate) fn record_crash(demand: &mut dyn Demand, wall: f64) {
	let mut state = demand.read_state();
	let mut counts = counts_of(&state);
	bump(&mut counts, "failed", 1);
	state.insert("counts".to_owned(), Value::Object(counts));
	state.insert("last_run".to_owned(), json!({"at": whole(wall), "exit": 1, "outcome": "failed"}));
	if let Err(err) = demand.write_state(&state) {
		println!("deploy: could not record the failure in state.json: {err:#}");
	}
	demand.clear_marker();
}

// ── The real host ───────────────────────────────────────────────────────────

/// The failed images are remembered under the user's own state directory.
pub(crate) fn default_state_dir(xdg_state_home: Option<&str>, home: Option<&str>) -> PathBuf {
	let base = match (xdg_state_home.filter(|dir| !dir.is_empty()), home) {
		(Some(dir), _) => PathBuf::from(dir),
		(None, Some(home)) => Path::new(home).join(".local/state"),
		(None, None) => PathBuf::from(".local/state"),
	};
	base.join("server-deploy")
}

fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<(bool, String, String)> {
	let mut command = Command::new(program);
	command.args(args).stdin(Stdio::null());
	if let Some(cwd) = cwd {
		command.current_dir(cwd);
	}
	let out = command.output().ok()?;
	Some((
		out.status.success(),
		String::from_utf8_lossy(&out.stdout).into_owned(),
		String::from_utf8_lossy(&out.stderr).into_owned(),
	))
}

/// Docker and `make` on this machine.

/// Git, Docker and compose on this machine. Everything that writes into
/// the owner's checkout or the state directory runs as the owner when the
/// on-demand unit runs this as root (`as_user`, call C83), so neither ends
/// up holding files the owner's own `make home-deploy` cannot replace.
pub(crate) struct RealHost {
	/// The owner's checkout: the git repository releases are read from, and
	/// `STATE_ROOT`, whose untracked state compose mounts (C344).
	checkout: PathBuf,
	/// `applied.json`, `failed-releases.json` and `releases/<commit>/`.
	state: PathBuf,
	role: String,
	/// compose's project: the one the stack already runs under.
	project: String,
	envctl: PathBuf,
	/// The owner, when this runs as root for them (the on-demand unit).
	owner: Option<String>,
	as_user: Vec<String>,
}

/// One more reading of a file a pass keeps beside the state directory.
fn read_json(path: &Path) -> Option<Value> {
	serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

impl RealHost {
	pub(crate) fn new(checkout: PathBuf, state_dir: &Path, role: &str, owner: Option<String>, as_user: Vec<String>) -> Result<Self> {
		let envctl = std::env::current_exe().context("finding envctl's own binary")?;
		let mut host = Self {
			project: String::new(),
			checkout,
			state: state_dir.to_owned(),
			role: role.to_owned(),
			envctl,
			owner,
			as_user,
		};
		host.project = host.running_project().unwrap_or_else(|| {
			// compose's own default for the checkout: its directory's name.
			host.checkout.file_name().map_or_else(|| "server".to_owned(), |name| name.to_string_lossy().to_lowercase())
		});
		Ok(host)
	}

	fn docker(&self, args: &[&str]) -> Option<(bool, String, String)> {
		run("docker", args, None)
	}

	/// A command as the owner (when one is given), in the checkout, its
	/// output kept.
	fn as_owner(&self, program: &str, args: &[&str]) -> Option<(bool, String, String)> {
		let mut words: Vec<&str> = self.as_user.iter().map(String::as_str).collect();
		words.push(program);
		words.extend(args);
		let (first, rest) = words.split_first()?;
		run(first, rest, Some(&self.checkout))
	}

	fn git(&self, args: &[&str]) -> Result<String, String> {
		let mut all = vec!["-C", self.checkout.to_str().unwrap_or(".")];
		all.extend(args);
		match self.as_owner("git", &all) {
			Some((true, stdout, _)) => Ok(stdout),
			Some((false, _, stderr)) => Err(format!("git {}: {}", args.join(" "), stderr.trim())),
			None => Err("git could not run".to_owned()),
		}
	}

	/// The project the running file-host belongs to, so `up` replaces the
	/// stack's containers instead of meeting their names.
	fn running_project(&self) -> Option<String> {
		let (ok, stdout, _) = self.docker(&["inspect", "--format", "{{index .Config.Labels \"com.docker.compose.project\"}}", CONTAINER])?;
		Some(stdout.trim().to_owned()).filter(|project| ok && !project.is_empty() && project != "<no value>")
	}

	fn container(&self, service: &str) -> Option<String> {
		let project = format!("label=com.docker.compose.project={}", self.project);
		let named = format!("label=com.docker.compose.service={service}");
		let (ok, stdout, _) = self.docker(&["ps", "-a", "-q", "--filter", &project, "--filter", &named])?;
		stdout.lines().next().map(str::to_owned).filter(|id| ok && !id.is_empty())
	}

	/// Writes a file of the state directory, whole, as the owner's.
	fn write_state_file(&self, name: &str, value: &Value) {
		let path = self.state.join(name);
		let temp = self.state.join(format!(".{name}.tmp"));
		let written = std::fs::create_dir_all(&self.state)
			.and_then(|()| std::fs::write(&temp, value.to_string()))
			.and_then(|()| std::fs::rename(&temp, &path));
		if let Err(err) = written {
			println!("deploy: could not write {}: {err}", path.display());
		}
		self.hand_to_owner(&self.state);
		self.hand_to_owner(&path);
	}

	/// `chown` to the owner when running for one, so their own pass can
	/// write it next.
	fn hand_to_owner(&self, path: &Path) {
		if let Some(owner) = &self.owner {
			let _ = run("chown", &[owner.as_str(), path.to_str().unwrap_or_default()], None);
		}
	}

	/// The release's tree: a worktree of the owner's checkout at its commit,
	/// made once and kept while it is the applied or the previous release.
	fn tree(&self, commit: &str) -> Result<PathBuf, String> {
		let releases = self.state.join("releases");
		let tree = releases.join(commit);
		if tree.join(".git").exists() {
			return Ok(tree);
		}
		std::fs::create_dir_all(&releases).map_err(|err| format!("creating {}: {err}", releases.display()))?;
		self.hand_to_owner(&self.state);
		self.hand_to_owner(&releases);
		self.git(&["worktree", "add", "--detach", "--force", tree.to_str().unwrap_or_default(), commit])?;
		Ok(tree)
	}

	/// Worktrees of releases neither applied nor previous are removed.
	fn prune_trees(&self, keep: &[&str]) {
		let Ok(entries) = std::fs::read_dir(self.state.join("releases")) else {
			return;
		};
		for entry in entries.flatten() {
			let name = entry.file_name().to_string_lossy().into_owned();
			if !keep.contains(&name.as_str()) {
				let _ = self.git(&["worktree", "remove", "--force", entry.path().to_str().unwrap_or_default()]);
			}
		}
		let _ = self.git(&["worktree", "prune"]);
	}
}

impl Host for RealHost {
	fn newest(&mut self) -> Result<Target, String> {
		self.git(&["fetch", "-q", "origin", "main"])?;
		let commit = self.git(&["rev-parse", "FETCH_HEAD"])?.trim().to_owned();
		self.at(&commit)
	}

	fn at(&mut self, commit: &str) -> Result<Target, String> {
		let path = format!("{commit}:{}", release::relative(&self.role).display());
		let text = self.git(&["show", &path])?;
		let release = release::parse(&text).map_err(|err| format!("{path}: {err:#}"))?;
		Ok(Target {
			commit: commit.to_owned(),
			release,
		})
	}

	fn applied(&mut self) -> Option<String> {
		read_json(&self.state.join("applied.json"))?.get("commit")?.as_str().map(str::to_owned)
	}

	fn record_applied(&mut self, commit: &str) {
		let previous = self.applied().filter(|previous| previous != commit);
		self.write_state_file("applied.json", &json!({"commit": commit, "previous": previous}));
		let mut keep = vec![commit];
		if let Some(previous) = &previous {
			keep.push(previous);
		}
		self.prune_trees(&keep);
	}

	fn reading(&mut self, service: &str) -> Reading {
		let Some(id) = self.container(service) else {
			return Reading::gone();
		};
		let Some((true, stdout, _)) = self.docker(&["inspect", "--format", "{{json .}}", &id]) else {
			return Reading::gone();
		};
		let Ok(info) = serde_json::from_str::<Value>(&stdout) else {
			return Reading::gone();
		};
		let text = |value: &Value| value.as_str().map(str::to_owned);
		Reading {
			container: Some(id),
			status: text(&info["State"]["Status"]),
			health: text(&info["State"]["Health"]["Status"]),
			restarts: info["RestartCount"].as_u64().unwrap_or(0),
			image: text(&info["Image"]),
		}
	}

	fn image_id(&mut self, image_ref: &str) -> Option<String> {
		match self.docker(&["image", "inspect", "--format", "{{.Id}}", image_ref]) {
			Some((true, stdout, _)) => Some(stdout.trim().to_owned()).filter(|id| !id.is_empty()),
			_ => None,
		}
	}

	fn labels(&mut self, image_id: &str) -> Option<BTreeMap<String, String>> {
		let Some((true, stdout, _)) = self.docker(&["image", "inspect", "--format", "{{json .Config.Labels}}", image_id]) else {
			return None;
		};
		match serde_json::from_str::<Value>(stdout.trim()).ok()? {
			Value::Null => Some(BTreeMap::new()),
			Value::Object(labels) => Some(labels.into_iter().filter_map(|(key, value)| Some((key, value.as_str()?.to_owned()))).collect()),
			_ => None,
		}
	}

	fn describe(&mut self, image_id: &str) -> String {
		let digests = match self.docker(&["image", "inspect", "--format", "{{join .RepoDigests \", \"}}", image_id]) {
			Some((true, stdout, _)) => stdout.trim().to_owned(),
			_ => String::new(),
		};
		if digests.is_empty() {
			image_id.to_owned()
		} else {
			format!("{image_id} ({digests})")
		}
	}

	fn pull(&mut self, image_ref: &str) -> bool {
		matches!(self.docker(&["pull", "-q", image_ref]), Some((true, _, _)))
	}

	fn up(&mut self, target: &Target) -> bool {
		let tree = match self.tree(&target.commit) {
			Ok(tree) => tree,
			Err(err) => {
				println!("deploy: the release's tree could not be made: {err}");
				return false;
			}
		};
		// envctl's own `up` on the release's tree: its preflight, then
		// `docker compose up -d`, which recreates what changed. Untracked
		// state is mounted from the owner's checkout (STATE_ROOT, C344).
		let project = format!("COMPOSE_PROJECT_NAME={}", self.project);
		let mut words: Vec<String> = self.as_user.clone();
		if words.is_empty() {
			words.push("env".to_owned());
		}
		words.push(project);
		words.extend([
			self.envctl.display().to_string(),
			"--repo".to_owned(),
			tree.display().to_string(),
			"--state-root".to_owned(),
			self.checkout.display().to_string(),
			"up".to_owned(),
			"--role".to_owned(),
			self.role.clone(),
		]);
		let (program, rest) = words.split_first().map_or(("env", &[][..]), |(first, rest)| (first.as_str(), rest));
		Command::new(program).args(rest).current_dir(&tree).status().is_ok_and(|status| status.success())
	}

	fn logs(&mut self, service: &str) -> Vec<String> {
		let Some(id) = self.container(service) else {
			return Vec::new();
		};
		self
			.docker(&["logs", "--tail", "200", &id])
			.map(|(_, stdout, stderr)| (stdout + &stderr).lines().map(str::to_owned).collect())
			.unwrap_or_default()
	}

	fn failed_releases(&mut self) -> Vec<u64> {
		match read_json(&self.state.join("failed-releases.json")) {
			Some(Value::Array(items)) => items.iter().filter_map(Value::as_u64).collect(),
			_ => Vec::new(),
		}
	}

	fn remember_failed(&mut self, serial: u64) {
		let mut kept: Vec<u64> = self.failed_releases().into_iter().filter(|item| *item != serial).collect();
		let drop = kept.len().saturating_sub(FAILED_KEPT - 1);
		kept.drain(..drop);
		kept.push(serial);
		self.write_state_file("failed-releases.json", &json!(kept));
	}
}

/// `${STORAGE}/deploy` on the host: the marker and state.json.
pub(crate) struct RealDemand {
	folder: PathBuf,
}

impl RealDemand {
	pub(crate) const fn new(folder: PathBuf) -> Self {
		Self { folder }
	}

	fn marker(&self) -> PathBuf {
		self.folder.join("wanted")
	}

	fn state(&self) -> PathBuf {
		self.folder.join("state.json")
	}
}

impl Demand for RealDemand {
	fn read_marker(&mut self) -> Option<Map<String, Value>> {
		match std::fs::read_to_string(self.marker()) {
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
			Err(_) => Some(Map::new()),
			Ok(text) => match serde_json::from_str::<Value>(&text) {
				Ok(Value::Object(marker)) => Some(marker),
				_ => Some(Map::new()),
			},
		}
	}

	fn clear_marker(&mut self) {
		let _ = std::fs::remove_file(self.marker());
	}

	fn read_state(&mut self) -> Map<String, Value> {
		match std::fs::read_to_string(self.state()).map(|text| serde_json::from_str::<Value>(&text)) {
			Ok(Ok(Value::Object(state))) => state,
			_ => Map::new(),
		}
	}

	fn write_state(&mut self, state: &Map<String, Value>) -> Result<()> {
		use std::os::unix::fs::PermissionsExt as _;
		// Whole or not at all: file_host reads it every 10 s. Keys sorted, as
		// serde_json's map keeps them.
		let temp = self.folder.join(".state.json.tmp");
		let mut text = Value::Object(state.clone()).to_string();
		text.push('\n');
		std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
		std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o644))?;
		std::fs::rename(&temp, self.state()).with_context(|| format!("replacing {}", self.state().display()))
	}

	fn image_epoch(&mut self, image_id: &str) -> Option<i64> {
		// An image from before the epoch existed refuses the flag: None.
		let args = ["run", "--rm", "--network", "none", "--entrypoint", "/usr/local/bin/file_host", image_id, "--contract-epoch"];
		match run("docker", &args, None) {
			Some((true, stdout, _)) => digits(stdout.trim()),
			_ => None,
		}
	}

	fn image_bytes(&mut self, image_id: &str) -> i64 {
		match run("docker", &["image", "inspect", "--format", "{{.Size}}", image_id], None) {
			Some((true, stdout, _)) => digits(stdout.trim()).unwrap_or(0),
			_ => 0,
		}
	}
}

/// A non-negative integer written in digits only, as `file_host` prints them.
fn digits(text: &str) -> Option<i64> {
	if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}
	text.parse().ok()
}

/// A user's uid, gid and home, from /etc/passwd (home's users are local).
fn passwd_entry(passwd: &str, user: &str) -> Option<(u32, u32, String)> {
	passwd.lines().find_map(|line| {
		let fields: Vec<&str> = line.split(':').collect();
		match fields.as_slice() {
			[name, _, uid, gid, _, home, ..] if *name == user => Some((uid.parse().ok()?, gid.parse().ok()?, (*home).to_owned())),
			_ => None,
		}
	})
}

/// A user's home, from /etc/passwd.
pub(crate) fn home_of(user: &str) -> Result<String> {
	let passwd = std::fs::read_to_string("/etc/passwd").context("reading /etc/passwd")?;
	passwd_entry(&passwd, user).map(|(_, _, home)| home).with_context(|| format!("no user {user} in /etc/passwd"))
}

/// `make` as `user`, with its home (sops' key) and, when given, a tmpfs
/// runtime directory it owns (envctl's decrypted files).
pub(crate) fn runuser_prefix(user: &str, runtime_dir: Option<&Path>) -> Result<Vec<String>> {
	use std::os::unix::fs::DirBuilderExt as _;
	let passwd = std::fs::read_to_string("/etc/passwd").context("reading /etc/passwd")?;
	let (uid, gid, home) = passwd_entry(&passwd, user).with_context(|| format!("no user {user} in /etc/passwd"))?;
	let mut prefix: Vec<String> = ["runuser", "-u", user, "--", "env"].map(str::to_owned).to_vec();
	prefix.push(format!("HOME={home}"));
	if let Some(dir) = runtime_dir {
		match std::fs::DirBuilder::new().mode(0o700).create(dir) {
			Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
				return Err(err).with_context(|| format!("creating {}", dir.display()));
			}
			_ => {}
		}
		std::os::unix::fs::chown(dir, Some(uid), Some(gid)).with_context(|| format!("handing {} to {user}", dir.display()))?;
		prefix.push(format!("XDG_RUNTIME_DIR={}", dir.display()));
	}
	Ok(prefix)
}

/// The wall clock, in seconds since the epoch.
pub(crate) fn wall_now() -> f64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |since| since.as_secs_f64())
}

/// The real monotonic clock.
pub(crate) struct Monotonic(std::time::Instant);

impl Monotonic {
	pub(crate) fn new() -> Self {
		Self(std::time::Instant::now())
	}
}

impl Clock for Monotonic {
	fn now(&self) -> f64 {
		self.0.elapsed().as_secs_f64()
	}

	fn sleep(&self, seconds: f64) {
		if seconds > 0.0 {
			std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
		}
	}
}


// ── Tests: a simulated host and clock ───────────────────────────────────────

#[cfg(test)]
mod tests {
	use super::*;
	use crate::release::Service;
	use std::{cell::Cell, collections::HashMap, rc::Rc};

	#[derive(Clone, Default)]
	struct FakeClock(Rc<Cell<f64>>);

	impl Clock for FakeClock {
		fn now(&self) -> f64 {
			self.0.get()
		}

		fn sleep(&self, seconds: f64) {
			self.0.set(self.0.get() + seconds);
		}
	}

	/// How an image behaves once started: seconds since start ->
	/// (status, health, restarts).
	type Behaviour = Rc<dyn Fn(f64) -> (&'static str, Option<&'static str>, u64)>;

	fn ready_after(seconds: f64) -> Behaviour {
		Rc::new(move |t| ("running", Some(if t >= seconds { "healthy" } else { "starting" }), 0))
	}

	fn never_ready() -> Behaviour {
		// start_period 300 s, then 5 failed probes 30 s apart.
		Rc::new(|t| ("running", Some(if t >= 450.0 { "unhealthy" } else { "starting" }), 0))
	}

	fn crashes_after(seconds: f64) -> Behaviour {
		Rc::new(move |t| if t < seconds { ("running", Some("starting"), 0) } else { ("restarting", Some("starting"), 1) })
	}

	/// No healthcheck: running, never restarting.
	fn plain() -> Behaviour {
		Rc::new(|_| ("running", None, 0))
	}

	/// An image: its id, labels and behaviour.
	struct Image {
		id: String,
		labels: BTreeMap<String, String>,
		behaviour: Behaviour,
	}

	#[derive(Default)]
	struct FakeHost {
		clock: FakeClock,
		/// commit → release, and main's commit.
		releases: HashMap<String, Release>,
		main: String,
		/// What a pull fetches: reference → image.
		registry: HashMap<String, Image>,
		/// What is here: reference → id.
		local: HashMap<String, String>,
		images: HashMap<String, (BTreeMap<String, String>, Behaviour)>,
		/// service → (image id, started at).
		running: BTreeMap<String, (String, f64)>,
		applied: Option<String>,
		failed: Vec<u64>,
		pull_fails: bool,
		up_fails: bool,
		log: Vec<String>,
		calls: Vec<String>,
	}

	impl FakeHost {
		fn bring_up(&mut self, target: &Target) {
			for (service, entry) in &target.release.services {
				let Some(id) = self.local.get(&entry.image).cloned() else {
					self.running.remove(service);
					continue;
				};
				if self.running.get(service).map(|(running, _)| running) != Some(&id) {
					self.running.insert(service.clone(), (id, self.clock.now()));
				}
			}
		}
	}

	impl Host for FakeHost {
		fn newest(&mut self) -> Result<Target, String> {
			self.calls.push("fetch".to_owned());
			let main = self.main.clone();
			self.at(&main)
		}

		fn at(&mut self, commit: &str) -> Result<Target, String> {
			let release = self.releases.get(commit).cloned().ok_or_else(|| format!("no commit {commit}"))?;
			Ok(Target {
				commit: commit.to_owned(),
				release,
			})
		}

		fn applied(&mut self) -> Option<String> {
			self.applied.clone()
		}

		fn record_applied(&mut self, commit: &str) {
			self.applied = Some(commit.to_owned());
		}

		fn reading(&mut self, service: &str) -> Reading {
			let Some((id, started)) = self.running.get(service) else {
				return Reading::gone();
			};
			let (status, health, restarts) = (self.images[id].1)(self.clock.now() - started);
			Reading {
				container: Some(service.to_owned()),
				status: Some(status.to_owned()),
				health: health.map(str::to_owned),
				restarts,
				image: Some(id.clone()),
			}
		}

		fn image_id(&mut self, image_ref: &str) -> Option<String> {
			self.local.get(image_ref).cloned()
		}

		fn labels(&mut self, image_id: &str) -> Option<BTreeMap<String, String>> {
			self.images.get(image_id).map(|(labels, _)| labels.clone())
		}

		fn describe(&mut self, image_id: &str) -> String {
			image_id.to_owned()
		}

		fn pull(&mut self, image_ref: &str) -> bool {
			self.calls.push(format!("pull {image_ref}"));
			if self.pull_fails {
				return false;
			}
			if let Some(image) = self.registry.get(image_ref) {
				self.local.insert(image_ref.to_owned(), image.id.clone());
				self.images.insert(image.id.clone(), (image.labels.clone(), image.behaviour.clone()));
			}
			true
		}

		fn up(&mut self, target: &Target) -> bool {
			self.calls.push(format!("up {}", target.release.serial));
			if self.up_fails {
				return false;
			}
			self.bring_up(target);
			true
		}

		fn logs(&mut self, _service: &str) -> Vec<String> {
			self.log.clone()
		}

		fn failed_releases(&mut self) -> Vec<u64> {
			self.failed.clone()
		}

		fn remember_failed(&mut self, serial: u64) {
			self.failed.push(serial);
		}
	}

	/// `(service, behaviour, labels)` for each service of a release.
	type Spec<'a> = &'a [(&'a str, Behaviour, &'a str, &'a str)];

	fn labels(provides: &str, requires: &str) -> BTreeMap<String, String> {
		[(constraints::PROVIDES, provides), (constraints::REQUIRES, requires)]
			.into_iter()
			.filter(|(_, value)| !value.is_empty())
			.map(|(label, value)| (label.to_owned(), value.to_owned()))
			.collect()
	}

	/// Publishes a release at `commit`, serial `serial`: each service's image
	/// `<service>:<serial>`, id `sha256:<service>-<serial>`.
	fn publish(host: &mut FakeHost, commit: &str, serial: u64, spec: Spec<'_>) {
		let mut services = BTreeMap::new();
		for (service, behaviour, provides, requires) in spec {
			let reference = format!("pgathondu/{service}:{serial}@sha256:x");
			let id = format!("sha256:{service}-{serial}");
			host.registry.insert(
				reference.clone(),
				Image {
					id,
					labels: labels(provides, requires),
					behaviour: behaviour.clone(),
				},
			);
			services.insert((*service).to_owned(), Service { image: reference });
		}
		host.releases.insert(
			commit.to_owned(),
			Release {
				serial,
				server_commit: commit.to_owned(),
				services,
			},
		);
		host.main = commit.to_owned();
	}

	/// A host already running release 1 (commit "c1") whole, ready.
	fn serving(spec: Spec<'_>) -> FakeHost {
		let mut host = FakeHost::default();
		publish(&mut host, "c1", 1, spec);
		let target = host.at("c1").unwrap();
		for entry in target.release.services.values() {
			host.pull(&entry.image);
		}
		host.bring_up(&target);
		host.applied = Some("c1".to_owned());
		host.calls.clear();
		host
	}

	fn three(file_host: Behaviour, render: Behaviour) -> Vec<(&'static str, Behaviour, &'static str, &'static str)> {
		vec![
			("file-host", file_host, "contract=6,render-folders=1", "rust-runner-protocol=1"),
			("render", render, "", "render-folders=1"),
			("rust-runner", plain(), "rust-runner-protocol=1", ""),
		]
	}

	fn run_deploy_with(host: &mut FakeHost, options: &Options) -> (Outcome, Vec<String>) {
		let mut said = Vec::new();
		let clock = host.clock.clone();
		let outcome = deploy(host, options, &clock, &mut |line| said.push(line.to_owned()));
		(outcome, said)
	}

	fn run_deploy(host: &mut FakeHost) -> (u8, Vec<String>) {
		let (outcome, said) = run_deploy_with(host, &Options::default());
		(outcome.code, said)
	}

	fn said_any(said: &[String], text: &str) -> bool {
		said.iter().any(|line| line.contains(text))
	}

	fn ups(host: &FakeHost) -> Vec<&str> {
		host.calls.iter().filter(|call| call.starts_with("up")).map(String::as_str).collect()
	}

	fn reading(status: Option<&str>, health: Option<&str>, restarts: u64) -> Reading {
		Reading {
			container: Some("c".to_owned()),
			status: status.map(str::to_owned),
			health: health.map(str::to_owned),
			restarts,
			image: None,
		}
	}

	fn is_failed(verdict: &Verdict) -> bool {
		matches!(verdict, Verdict::Failed(_))
	}

	// judge, judge_service, wait_all

	#[test]
	fn only_healthy_is_ready() {
		assert_eq!(judge(&reading(Some("running"), Some("healthy"), 0), 0), Verdict::Ready);
		assert_eq!(judge(&reading(Some("running"), Some("starting"), 0), 0), Verdict::Wait);
		assert_eq!(judge(&reading(Some("created"), None, 0), 0), Verdict::Wait);
	}

	#[test]
	fn a_container_that_exits_or_restarts_has_failed_now() {
		for status in ["exited", "dead", "restarting"] {
			assert!(is_failed(&judge(&reading(Some(status), Some("starting"), 0), 0)), "{status}");
		}
		assert!(is_failed(&judge(&reading(Some("running"), Some("starting"), 3), 2)));
		assert_eq!(judge(&reading(Some("running"), Some("starting"), 3), 3), Verdict::Wait);
		assert!(is_failed(&judge(&reading(Some("running"), Some("unhealthy"), 0), 0)));
		assert!(is_failed(&judge(&Reading::gone(), 0)));
		assert!(matches!(judge(&reading(Some("running"), None, 0), 0), Verdict::Unjudgeable(_)));
	}

	#[test]
	fn a_service_without_a_healthcheck_is_ready_once_it_has_run_a_minute() {
		let plain = reading(Some("running"), None, 0);
		assert_eq!(judge_service(&plain, 0, STABLE_SECONDS - 1.0), Verdict::Wait);
		assert_eq!(judge_service(&plain, 0, STABLE_SECONDS), Verdict::Ready);
		assert!(is_failed(&judge_service(&reading(Some("running"), None, 1), 0, STABLE_SECONDS)), "a restart fails it");
	}

	#[test]
	fn waiting_is_bounded() {
		let clock = FakeClock::default();
		let services = ["a".to_owned(), "b".to_owned()];
		let (failure, took) = wait_all(
			&services,
			&mut |service| if service == "a" { reading(Some("running"), Some("healthy"), 0) } else { reading(Some("running"), Some("starting"), 0) },
			&clock,
			90.0,
			2.0,
		);
		let (service, reason) = failure.unwrap();
		assert_eq!(service, "b");
		assert!(reason.contains("within 90 s"), "{reason}");
		assert!((took - 90.0).abs() < f64::EPSILON, "{took}");
	}

	#[test]
	fn backup_from_reads_file_hosts_start() {
		let lines = [
			"migrate: backed up to /app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db (verified)",
			"migrate: applied 2 (…)",
		]
		.map(str::to_owned);
		assert_eq!(backup_from(&lines).as_deref(), Some("/app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db"));
		assert_eq!(backup_from(&["migrate: current, nothing to apply".to_owned()]), None);
	}

	// One pass, end to end against the simulation

	#[test]
	fn nothing_published_restarts_nothing() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		host.clock.sleep(STABLE_SECONDS);
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert!(said.last().unwrap().contains("nothing restarted"));
	}

	#[test]
	fn the_same_release_not_serving_is_brought_up_not_called_deployed() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		host.running.remove("render");
		let (code, said) = run_deploy(&mut host);
		assert_eq!(ups(&host), ["up 1"]);
		assert_eq!(code, EXIT_DEPLOYED, "up brought render back");
		assert!(said_any(&said, "render moves to sha256:render-1"), "{said:?}");
		// A container that exited, with the release's image, is brought up too.
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		host.registry.get_mut("pgathondu/render:1@sha256:x").unwrap().behaviour = Rc::new(|_| ("exited", None, 0));
		let (_, said) = run_deploy(&mut host);
		assert_eq!(ups(&host), ["up 1"]);
		assert!(said_any(&said, "render runs this release's image but is not ready"), "{said:?}");
	}

	#[test]
	fn every_image_of_the_release_is_pulled_by_its_reference() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(ready_after(5.0), ready_after(5.0)));
		run_deploy(&mut host);
		let pulls: Vec<&String> = host.calls.iter().filter(|call| call.starts_with("pull")).collect();
		assert_eq!(pulls.len(), 3, "{pulls:?}");
		assert!(pulls.iter().all(|pull| pull.ends_with(":2@sha256:x")), "{pulls:?}");
	}

	#[test]
	fn a_ready_release_is_deployed_and_recorded() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(ready_after(12.0), ready_after(3.0)));
		let (outcome, said) = run_deploy_with(&mut host, &Options::default());
		assert_eq!(
			outcome,
			Outcome {
				code: EXIT_DEPLOYED,
				serving: Some(2),
				previous: Some(1)
			}
		);
		assert_eq!(host.applied.as_deref(), Some("c2"));
		assert_eq!(host.running["file-host"].0, "sha256:file-host-2");
		assert!(host.clock.now() >= STABLE_SECONDS, "rust-runner has no healthcheck: it ran a minute first");
		assert!(said_any(&said, "is ready after"));
	}

	#[test]
	fn a_render_only_failure_rolls_back_every_service() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(ready_after(5.0), crashes_after(3.0)));
		let (outcome, said) = run_deploy_with(&mut host, &Options::default());
		assert_eq!(outcome.code, EXIT_ROLLED_BACK);
		assert_eq!(outcome.serving, Some(1));
		assert_eq!(ups(&host), ["up 2", "up 1"]);
		for (service, (id, _)) in &host.running {
			assert_eq!(id, &format!("sha256:{service}-1"), "{service} is back on release 1");
		}
		assert_eq!(host.applied.as_deref(), Some("c1"));
		assert_eq!(host.failed, [2]);
		assert!(said_any(&said, "render did not come up"));
		assert!(said_any(&said, "ROLLED BACK"));
		assert!(said_any(&said, "Nothing was restored or deleted"));
	}

	#[test]
	fn an_unmet_constraint_replaces_nothing() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		let mut spec = three(ready_after(0.0), ready_after(0.0));
		spec[1].3 = "render-folders=2";
		publish(&mut host, "c2", 2, &spec);
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert_eq!(host.running["render"].0, "sha256:render-1");
		assert_eq!(host.applied.as_deref(), Some("c1"));
		assert!(host.failed.is_empty(), "nothing ran, so nothing failed here");
		assert!(said_any(&said, "does not fit together"));
		assert!(said_any(&said, "render requires render-folders=2, but file-host provides render-folders=1"));
	}

	#[test]
	fn a_failed_release_is_not_tried_again() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(crashes_after(3.0), ready_after(0.0)));
		assert_eq!(run_deploy(&mut host).0, EXIT_ROLLED_BACK);
		host.calls.clear();
		host.clock.sleep(STABLE_SECONDS);
		let (code, said) = run_deploy(&mut host); // main still holds release 2
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert!(said_any(&said, "failed here before"));
		// A newer release is tried.
		publish(&mut host, "c3", 3, &three(ready_after(5.0), ready_after(0.0)));
		assert_eq!(run_deploy(&mut host).0, EXIT_DEPLOYED);
		assert_eq!(host.running["file-host"].0, "sha256:file-host-3");
	}

	#[test]
	fn a_refused_migration_rolls_back_without_waiting_out_the_bound() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(crashes_after(3.0), ready_after(0.0)));
		host.log = vec!["Error: migrate: refusing to migrate: failed: [20261110000100]".to_owned()];
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert!(host.clock.now() < 15.0 + STABLE_SECONDS * 2.0);
		assert!(said_any(&said, "its log: Error: migrate: refusing"));
	}

	#[test]
	fn an_image_that_never_becomes_ready_ends_with_the_previous_release_serving() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(never_ready(), ready_after(0.0)));
		let (code, _) = run_deploy(&mut host);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.running["file-host"].0, "sha256:file-host-1");
		assert!(host.clock.now() <= 2.0f64.mul_add(DEFAULT_READY_TIMEOUT, POLL_SECONDS));
	}

	#[test]
	fn when_the_previous_release_fails_too_it_says_so_and_names_the_backup() {
		let backup = "/app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db";
		let mut host = serving(&three(never_ready(), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(never_ready(), ready_after(0.0)));
		host.log = vec![format!("{BACKUP_LINE}{backup} (verified)")];
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOTHING_READY);
		let text = said.join("\n");
		assert!(text.contains(backup));
		assert!(text.contains("never restored automatically"));
	}

	#[test]
	fn a_first_deploy_has_nothing_to_roll_back_to() {
		let mut host = FakeHost::default();
		publish(&mut host, "c1", 1, &three(never_ready(), ready_after(0.0)));
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOTHING_READY);
		assert!(said_any(&said, "no release was applied here before"));
	}

	#[test]
	fn a_failed_pull_replaces_nothing() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(ready_after(0.0), ready_after(0.0)));
		host.pull_fails = true;
		let (code, _) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert_eq!(host.running["file-host"].0, "sha256:file-host-1");
	}

	#[test]
	fn an_up_that_fails_rolls_back_whole() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		publish(&mut host, "c2", 2, &three(ready_after(0.0), ready_after(0.0)));
		host.up_fails = true;
		host.running.remove("render"); // up stopped it, then failed
		let (outcome, said) = run_deploy_with(&mut host, &Options::default());
		assert_eq!(outcome.code, EXIT_NOTHING_READY, "the rollback's up fails as well here");
		assert!(said_any(&said, "up failed"));
		assert_eq!(ups(&host), ["up 2", "up 1"]);
	}

	#[test]
	fn an_unreadable_main_replaces_nothing() {
		let mut host = serving(&three(ready_after(0.0), ready_after(0.0)));
		host.main = "gone".to_owned();
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert!(said_any(&said, "could not be read"));
	}

	// On demand

	#[derive(Default)]
	struct FakeDemand {
		marker: Option<Map<String, Value>>,
		state: Map<String, Value>,
		cleared: bool,
	}

	impl FakeDemand {
		fn new(marker: Option<Value>) -> Self {
			Self {
				marker: marker.map(|marker| marker.as_object().unwrap().clone()),
				..Self::default()
			}
		}

		fn with_state(mut self, state: &Value) -> Self {
			self.state = state.as_object().unwrap().clone();
			self
		}
	}

	impl Demand for FakeDemand {
		fn read_marker(&mut self) -> Option<Map<String, Value>> {
			self.marker.clone()
		}

		fn clear_marker(&mut self) {
			self.marker = None;
			self.cleared = true;
		}

		fn read_state(&mut self) -> Map<String, Value> {
			self.state.clone()
		}

		fn write_state(&mut self, state: &Map<String, Value>) -> Result<()> {
			self.state = state.clone();
			Ok(())
		}

		fn image_epoch(&mut self, _image_id: &str) -> Option<i64> {
			None
		}

		fn image_bytes(&mut self, _image_id: &str) -> i64 {
			1000
		}
	}

	const WALL: f64 = 1_800_000_000.0;

	fn run_on_demand(host: &mut FakeHost, demand: &mut FakeDemand) -> (u8, Vec<String>) {
		let mut said = Vec::new();
		let clock = host.clock.clone();
		let code = on_demand(host, demand, &Options::default(), &|| WALL, &clock, &mut |line| said.push(line.to_owned())).unwrap();
		(code, said)
	}

	fn wall() -> i64 {
		whole(WALL)
	}

	fn with_contract(epoch: u32) -> Vec<(&'static str, Behaviour, String, &'static str)> {
		vec![
			("file-host", ready_after(5.0), format!("contract={epoch},render-folders=1"), "rust-runner-protocol=1"),
			("render", ready_after(0.0), String::new(), "render-folders=1"),
			("rust-runner", plain(), "rust-runner-protocol=1".to_owned(), ""),
		]
	}

	fn publish_contract(host: &mut FakeHost, commit: &str, serial: u64, epoch: u32) {
		let spec = with_contract(epoch);
		let borrowed: Vec<(&str, Behaviour, &str, &str)> = spec.iter().map(|(service, behaviour, provides, requires)| (*service, behaviour.clone(), provides.as_str(), *requires)).collect();
		publish(host, commit, serial, &borrowed);
	}

	fn serving_contract(epoch: u32) -> FakeHost {
		let spec = with_contract(epoch);
		let borrowed: Vec<(&str, Behaviour, &str, &str)> = spec.iter().map(|(service, behaviour, provides, requires)| (*service, behaviour.clone(), provides.as_str(), *requires)).collect();
		serving(&borrowed)
	}

	#[test]
	fn no_marker_no_pull() {
		let mut host = serving_contract(1);
		let mut demand = FakeDemand::new(None);
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(host.calls.is_empty());
		assert!(demand.state.is_empty());
	}

	#[test]
	fn a_newer_epoch_is_deployed_and_recorded() {
		let mut host = serving_contract(1);
		publish_contract(&mut host, "c2", 2, 2);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})));
		let (code, _) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_DEPLOYED);
		assert_eq!(host.running["file-host"].0, "sha256:file-host-2");
		assert!(demand.cleared);
		assert_eq!(demand.state["latest_seen_epoch"], json!(2));
		assert_eq!(demand.state["counts"], json!({"pulls": 1, "pulled_bytes": 1000, "deployed": 1}));
		assert_eq!(demand.state["last_run"], json!({"at": wall(), "exit": 0, "outcome": "deployed", "needs": 2}));
		assert_eq!((demand.state["release"].clone(), demand.state["previous_release"].clone()), (json!(2), json!(1)));
	}

	#[test]
	fn the_same_epoch_republished_is_not_deployed() {
		// Lazily on the running release: a newer build of the same contract.
		let mut host = serving_contract(1);
		publish_contract(&mut host, "c2", 2, 1);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})));
		let (code, said) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert_eq!(host.applied.as_deref(), Some("c1"), "the release applied stays");
		assert_eq!(demand.state["counts"]["no_newer_image"], json!(1));
		assert_eq!(demand.state["latest_seen_epoch"], json!(1));
		assert!(said_any(&said, "unavailable"));
	}

	#[test]
	fn an_image_whose_epoch_cannot_be_read_is_not_deployed() {
		let mut host = serving_contract(1);
		publish(&mut host, "c2", 2, &three(ready_after(0.0), ready_after(0.0)));
		host.registry.values_mut().for_each(|image| image.labels.clear());
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert_eq!(demand.state["latest_seen_epoch"], Value::Null);
	}

	#[test]
	fn a_malformed_marker_never_reads_as_epoch_zero() {
		// The serving image says 2, and so does the newest: nothing newer.
		let mut host = serving_contract(2);
		publish_contract(&mut host, "c2", 2, 2);
		let mut demand = FakeDemand::new(Some(json!({})));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(ups(&host).is_empty());
		// And when nothing can say what runs, nothing is deployed.
		let mut host = FakeHost::default();
		publish_contract(&mut host, "c2", 2, 9);
		let mut demand = FakeDemand::new(Some(json!({})));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_NOT_DEPLOYED);
		assert!(ups(&host).is_empty());
		assert!(demand.cleared);
	}

	#[test]
	fn a_pull_within_the_gap_is_deferred() {
		let mut host = serving_contract(1);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).with_state(&json!({"last_pull_at": wall() - 60}));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(host.calls.is_empty());
		assert_eq!(demand.state["counts"], json!({"deferred": 1}));
		assert!(demand.cleared);
	}

	#[test]
	fn a_newer_epoch_that_fails_is_rolled_back_and_counted() {
		let mut host = serving_contract(1);
		publish_contract(&mut host, "c2", 2, 2);
		host.registry.get_mut("pgathondu/render:2@sha256:x").unwrap().behaviour = crashes_after(3.0);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).with_state(&json!({"counts": {"pulls": 4, "deployed": 3}}));
		let (code, _) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.running["file-host"].0, "sha256:file-host-1");
		assert_eq!(demand.state["counts"], json!({"pulls": 5, "pulled_bytes": 1000, "deployed": 3, "rolled_back": 1}));
		assert_eq!(demand.state["last_run"]["exit"], json!(EXIT_ROLLED_BACK));
		assert_eq!(demand.state["release"], json!(1));
	}

	#[test]
	fn a_failed_pull_is_counted_and_clears_the_marker() {
		let mut host = serving_contract(1);
		publish_contract(&mut host, "c2", 2, 2);
		host.pull_fails = true;
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_NOT_DEPLOYED);
		assert_eq!(demand.state["counts"], json!({"pulls": 1, "failed": 1}));
		assert!(demand.cleared);
		assert_eq!(demand.state["last_pull_at"], json!(wall()), "a failed pull keeps the gap too");
	}

	#[test]
	fn a_crash_is_counted_and_clears_the_marker() {
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).with_state(&json!({"counts": {"pulls": 4}}));
		record_crash(&mut demand, WALL);
		assert!(demand.cleared);
		assert_eq!(demand.state["counts"], json!({"pulls": 4, "failed": 1}));
		assert_eq!(demand.state["last_run"], json!({"at": wall(), "exit": 1, "outcome": "failed"}));
	}

	#[test]
	fn the_real_state_file_is_written_whole_and_read_back() {
		let dir = std::env::temp_dir().join(format!("envctl-deploy-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut demand = RealDemand::new(dir.clone());
		assert_eq!(demand.read_marker(), None);
		std::fs::write(dir.join("wanted"), "not json").unwrap();
		assert_eq!(demand.read_marker(), Some(Map::new()), "a malformed marker is an empty one");
		std::fs::write(dir.join("wanted"), r#"{"needs": 3, "running": 2}"#).unwrap();
		assert_eq!(demand.read_marker().unwrap()["needs"], json!(3));
		demand.clear_marker();
		assert!(!dir.join("wanted").exists());
		let state = json!({"latest_seen_epoch": null, "counts": {"pulls": 1}}).as_object().unwrap().clone();
		demand.write_state(&state).unwrap();
		assert_eq!(demand.read_state(), state);
		assert!(!dir.join(".state.json.tmp").exists());
		std::fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn the_failed_list_keeps_the_last_twenty() {
		let dir = std::env::temp_dir().join(format!("envctl-deploy-failed-{}", std::process::id()));
		let mut host = RealHost::new(dir.clone(), &dir.join("state"), "home", None, Vec::new()).unwrap();
		for serial in 0..25 {
			host.remember_failed(serial);
		}
		host.remember_failed(10);
		let kept = host.failed_releases();
		assert_eq!(kept.len(), FAILED_KEPT);
		assert_eq!(kept.last(), Some(&10));
		assert_eq!(kept.first(), Some(&5));
		std::fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn the_users_home_is_read_from_passwd() {
		let passwd = "root:x:0:0:root:/root:/bin/sh\ndev:x:1000:100:Dev:/home/dev:/run/current-system/sw/bin/bash\n";
		assert_eq!(passwd_entry(passwd, "dev"), Some((1000, 100, "/home/dev".to_owned())));
		assert_eq!(passwd_entry(passwd, "nobody"), None);
	}

	#[test]
	fn the_state_dir_is_the_users_own() {
		assert_eq!(default_state_dir(Some("/x/state"), Some("/home/dev")), Path::new("/x/state/server-deploy"));
		assert_eq!(default_state_dir(None, Some("/home/dev")), Path::new("/home/dev/.local/state/server-deploy"));
	}

	fn compose_file() -> String {
		std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/compose/file_host.yml")).unwrap()
	}

	#[test]
	fn the_bound_outlasts_the_start_period() {
		let text = compose_file();
		let found = regex::Regex::new(r"(?m)^      start_period: (\d+)s$").unwrap().captures(&text).unwrap();
		let start_period: f64 = found[1].parse().unwrap();
		assert!(DEFAULT_READY_TIMEOUT > start_period);
		assert!(DEFAULT_READY_TIMEOUT > STABLE_SECONDS);
	}

	#[test]
	fn the_service_is_the_compose_files() {
		let text = compose_file();
		let pattern = ["(?m)^  ", &regex::escape(FILE_HOST), ":\n((?:    .*\n|[ \t]*\n)+)"].concat();
		let block = regex::Regex::new(&pattern).unwrap().captures(&text).unwrap();
		// The image is home's release's (deploy/home/release.json, #788).
		assert!(block[1].contains("image: ${IMAGE_FILE_HOST:?"));
		assert!(block[1].contains(&["container_name: ", CONTAINER, "\n"].concat()));
		let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let release = crate::release::read(&repo, "home").unwrap().unwrap();
		assert!(release.services.contains_key(FILE_HOST));
	}
}
