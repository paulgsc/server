//! Home's deploy of file_host, with the previous image put back when the new
//! one does not come up ready (AS18, #564). `make home-deploy` runs one pass
//! (`envctl deploy`), in the checkout (README.md, "Deploying to home"); the
//! on-demand unit (`nix/modules/home-deploy.nix`) runs `envctl deploy
//! --on-demand`. It is Rust, like everything else home runs: home has no
//! `python3` (server#PRNUM moved it here from `scripts/deploy.py`).
//!
//! In order:
//!
//! 1. **Records what serves now**: the image of the running `file-host`
//!    container, by image id.
//! 2. **Pulls** `pgathondu/server:latest` (`--no-pull` uses the image already
//!    here). Only that image: `rust-runner` and `orchestrator` are pulled by
//!    hand, and nothing third-party is pulled at all.
//! 3. **Skips an image that already failed here**: a pulled image whose id is
//!    in the state file's failed list is tagged back to the serving one, so a
//!    later pass, a reboot or a `make up` does not put a known-bad image up;
//!    the next publish (a new id) is tried.
//! 4. **Stops here when nothing changed**: the image is the serving one.
//! 5. **Keeps the old image**: tags the serving one `pgathondu/server:previous`,
//!    so a prune cannot take it before a rollback.
//! 6. **Brings the stack up**: `make up`, which is envctl's preflight on
//!    `deploy/<role>/`, then `docker compose up -d`. `file_host` applies its
//!    migrations as it starts, after a verified backup (AS16, #563,
//!    `apps/servers/file_host/src/migrate.rs`), and exits when it cannot.
//! 7. **Waits up to `--ready-timeout` seconds** for `file-host`'s healthcheck to
//!    say `healthy` (`file_host --health-check`: `/ready` ready or degraded).
//!    A container that exits, restarts or turns `unhealthy` fails at once.
//! 8. **Rolls back** when it does not come up and its image changed: tags the
//!    recorded image back as `pgathondu/server:latest`, recreates `file-host`
//!    alone from it (`up -d --no-deps --pull never file-host`), waits the same
//!    bound, and records the failed image so step 3 skips it.
//!
//! It never touches the database. A migration that ran stays applied: a
//! database ahead of its binary is one the old binary serves (`schema.rs`),
//! and the owner's answer (2026-11-10) is that a restore is never automatic.
//! README.md, "Deploying to home", is the manual restore. Nor does it update
//! the checkout (call C82): an image that needs a compose or `deploy/<role>/`
//! change the checkout lacks fails its preflight or its start, and is put
//! back, with that named as the likely cause.
//!
//! Exit status:
//!
//! - 0 deployed, or nothing to deploy
//! - 1 not deployed: the pull, the preflight or `up` failed; the old
//!   container still serves
//! - 2 rolled back: the new image did not come up; the previous one serves
//! - 3 nothing ready: the owner has to look (the message says what is known)
//!
//! The decisions (`judge`, `wait_ready`, `plan_after_failure`, `deploy` and
//! `on_demand` over a [`Host`] and [`Demand`] they are handed) are pure, so
//! the tests below run them against a simulated Docker and clock (`Lint`'s
//! `Deploy script` job).

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::{
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

/// file-host's compose service, its container_name and its image.
/// `the_service_is_the_compose_files` holds these to infra/compose/file_host.yml.
pub(crate) const FILE_HOST: &str = "file-host";
pub(crate) const CONTAINER: &str = "file-host-server";
pub(crate) const IMAGE: &str = "pgathondu/server:latest";

/// The bound on becoming ready, in seconds. file_host copies the database
/// and applies its migrations before it listens, so the healthcheck's
/// start_period (infra/compose/file_host.yml, 300 s) covers that work: no
/// failed probe counts during it, and the first that passes reads healthy.
/// The bound is above start_period, so a slow migrating start is never taken
/// for a bad image (which would be rolled back and then skipped). A start
/// that exits or restarts fails at once, whatever the bound, so only an image
/// that hangs costs all of it. `the_bound_outlasts_the_start_period` holds
/// the two together.
pub(crate) const DEFAULT_READY_TIMEOUT: f64 = 360.0;
pub(crate) const POLL_SECONDS: f64 = 2.0;

pub(crate) const EXIT_DEPLOYED: u8 = 0;
pub(crate) const EXIT_NOT_DEPLOYED: u8 = 1;
pub(crate) const EXIT_ROLLED_BACK: u8 = 2;
pub(crate) const EXIT_NOTHING_READY: u8 = 3;

/// What file_host prints at start when it took a backup
/// (apps/servers/file_host/src/main.rs, `migrate_on_start`).
pub(crate) const BACKUP_LINE: &str = "migrate: backed up to ";

/// How many failed image ids the state file remembers; older ones fall off.
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

impl Verdict {
	fn reason(&self) -> &str {
		match self {
			Self::Ready | Self::Wait => "",
			Self::Failed(reason) | Self::Unjudgeable(reason) => reason,
		}
	}
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
pub(crate) trait Clock {
	fn now(&self) -> f64;
	fn sleep(&self, seconds: f64);
}

/// Polls until a reading decides, or `timeout` seconds pass. Returns the
/// verdict and the seconds it took.
pub(crate) fn wait_ready(read: &mut dyn FnMut() -> Reading, clock: &dyn Clock, timeout: f64, poll: f64) -> (Verdict, f64) {
	let start = clock.now();
	let mut reading = read();
	let baseline = reading.restarts;
	loop {
		let verdict = judge(&reading, baseline);
		let elapsed = clock.now() - start;
		if verdict != Verdict::Wait {
			return (verdict, elapsed);
		}
		if elapsed >= timeout {
			let state = reading.health.as_deref().or(reading.status.as_deref()).unwrap_or("unknown");
			return (Verdict::Failed(format!("not ready within {timeout:.0} s (last seen: {state})")), elapsed);
		}
		clock.sleep(poll.min(timeout - elapsed));
		reading = read();
	}
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
	pub(crate) rollback: bool,
	pub(crate) reason: String,
}

/// Whether putting the previous image back can help.
///
/// Not when nothing served before (a first deploy), and not when the image
/// did not change: then the cause is the configuration or a dependency,
/// which the same image would meet again.
pub(crate) fn plan_after_failure(old_image: Option<&str>, new_image: Option<&str>) -> Plan {
	let Some(old) = old_image else {
		return Plan {
			rollback: false,
			reason: format!("no {FILE_HOST} container served before this deploy, so there is no previous image"),
		};
	};
	if new_image == Some(old) {
		return Plan {
			rollback: false,
			reason: "the image did not change, so the cause is not the image: look at deploy/<role>/, the config it refuses, or a dependency".to_owned(),
		};
	}
	Plan {
		rollback: true,
		reason: String::new(),
	}
}

/// `pgathondu/server:latest` → `pgathondu/server:previous`; a registry
/// port (`host:5000/x`) is not a tag.
pub(crate) fn previous_ref(image_ref: &str) -> String {
	let name = image_ref.split('@').next().unwrap_or(image_ref);
	let slash = name.rfind('/');
	let name = match name.rfind(':') {
		Some(colon) if slash.is_none_or(|slash| colon > slash) => &name[..colon],
		_ => name,
	};
	format!("{name}:previous")
}

/// The backup file_host took at this start, from its log, if it took one.
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

// ── The deploy, over a host it is handed ────────────────────────────────────

/// What a pass needs of Docker, `make` and the failed-image list.
pub(crate) trait Host {
	fn reading(&mut self, container: &str) -> Reading;
	fn image_id(&mut self, image_ref: &str) -> Option<String>;
	fn describe(&mut self, image_id: &str) -> String;
	fn tag(&mut self, image_id: &str, image_ref: &str) -> bool;
	fn pull(&mut self, image_ref: &str) -> bool;
	fn up(&mut self) -> bool;
	fn recreate_file_host(&mut self) -> bool;
	fn logs(&mut self, container: &str) -> Vec<String>;
	fn failed_images(&mut self) -> Vec<String>;
	fn remember_failed(&mut self, image_id: &str);
}

#[derive(Clone, Debug)]
pub(crate) struct Options {
	pub(crate) pull: bool,
	/// The image compose starts file-host from (infra/compose/file_host.yml);
	/// another only for a rehearsal against a local registry.
	pub(crate) image: String,
	pub(crate) ready_timeout: f64,
	pub(crate) poll: f64,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			pull: true,
			image: IMAGE.to_owned(),
			ready_timeout: DEFAULT_READY_TIMEOUT,
			poll: POLL_SECONDS,
		}
	}
}

/// Said with every failure a new image could owe to the checkout rather than
/// to itself: this pass does not update the checkout (the owner's call,
/// 2026-11-10), so a new variable or compose change the image needs is not
/// here yet.
pub(crate) const CHECKOUT_HINT: &str = "If the preflight or the container's last log lines name a missing or refused variable, \
	this image needs a newer checkout on home: `git pull` there, then `make home-deploy` again.";

/// One pass: see the module's documentation. Returns the exit status.
#[allow(clippy::too_many_lines)] // one pass, read top to bottom as the module's numbered steps
pub(crate) fn deploy(host: &mut dyn Host, options: &Options, clock: &dyn Clock, say: &mut dyn FnMut(&str)) -> u8 {
	let before = host.reading(CONTAINER);
	let old = if before.container.is_some() { before.image.clone() } else { None };
	match &old {
		Some(old) => {
			let described = host.describe(old);
			say(&format!("deploy: {FILE_HOST} serves {described}"));
		}
		None => say(&format!("deploy: no {FILE_HOST} container serves now")),
	}

	if options.pull && !host.pull(&options.image) {
		say(&format!("deploy: pulling {} failed; nothing was replaced", options.image));
		return EXIT_NOT_DEPLOYED;
	}

	let mut candidate = host.image_id(&options.image);
	if let (Some(found), Some(serving)) = (&candidate, &old) {
		if found != serving && host.failed_images().contains(found) {
			// Back to what serves, so a reboot or a `make up` by hand does not
			// start the image that failed either.
			host.tag(serving, &options.image);
			say(&format!(
				"deploy: {} is {found}, which failed here before; staying on {serving} until a newer one is published",
				options.image
			));
			candidate = Some(serving.clone());
		}
	}
	if candidate.is_some() && candidate == old {
		// Nothing to deploy only when what has that image is ready: a
		// container that exited, is restarting or reads unhealthy is not
		// serving, and `make up` below is the way to bring it back (with no
		// previous image to roll back to, since it is the same one).
		let state = judge(&before, before.restarts);
		if state == Verdict::Ready {
			say("deploy: the image is the one serving; nothing restarted");
			return EXIT_DEPLOYED;
		}
		let why = [Some(state.reason()), before.health.as_deref(), before.status.as_deref()]
			.into_iter()
			.flatten()
			.find(|text| !text.is_empty())
			.unwrap_or("None");
		say(&format!("deploy: the image is the container's, but it is not ready ({why}); bringing it up"));
	}

	if let (Some(serving), Some(found)) = (&old, &candidate) {
		if found != serving {
			let kept = previous_ref(&options.image);
			if host.tag(serving, &kept) {
				say(&format!("deploy: kept the serving {FILE_HOST} image as {kept}"));
			} else {
				say(&format!("deploy: could not tag the serving image as {kept}; it stays as {serving}"));
			}
		}
	}

	let up_ok = host.up();
	let after = host.reading(CONTAINER);
	let new = after.image;
	if !up_ok {
		// The preflight refused (it names each gap above), or compose did.
		if old.is_some() && new == old {
			say(&format!("deploy: make up failed (its reason is above); {FILE_HOST} was not replaced and still serves"));
			say(&format!("deploy: {CHECKOUT_HINT}"));
			return EXIT_NOT_DEPLOYED;
		}
		say("deploy: make up failed (its reason is above); judging what it left");
	}
	match &new {
		Some(new) => {
			let described = host.describe(new);
			say(&format!("deploy: started {described}; waiting up to {:.0} s for it to be ready", options.ready_timeout));
		}
		None => say(&format!("deploy: no {FILE_HOST} container after up")),
	}

	let (verdict, took) = wait_ready(&mut || host.reading(CONTAINER), clock, options.ready_timeout, options.poll);
	if verdict == Verdict::Ready {
		say(&format!("deploy: ready after {took:.0} s"));
		return EXIT_DEPLOYED;
	}
	say(&format!("deploy: the new {FILE_HOST} did not come up: {}", verdict.reason()));
	let lines = host.logs(CONTAINER);
	let backup = backup_from(&lines);
	for line in &lines[lines.len().saturating_sub(5)..] {
		say(&format!("deploy: its log: {line}"));
	}
	if matches!(verdict, Verdict::Unjudgeable(_)) {
		say(DATABASE_NOTE);
		return EXIT_NOTHING_READY;
	}

	let plan = plan_after_failure(old.as_deref(), new.as_deref());
	let (true, Some(old)) = (plan.rollback, old) else {
		say(&format!("deploy: not rolling back: {}", plan.reason));
		say(DATABASE_NOTE);
		say(&stuck_note(backup.as_deref()));
		return EXIT_NOTHING_READY;
	};

	// The image that failed: the container's, or, when `up` left no
	// container at all, the one it was asked to start. Recorded only when
	// known, and never the serving one.
	let failed_image = new.or(candidate);
	if let Some(failed) = failed_image.as_deref().filter(|failed| *failed != old) {
		host.remember_failed(failed);
	}
	let described = host.describe(&old);
	say(&format!("deploy: ROLLING BACK to {described}"));
	if !host.tag(&old, &options.image) || !host.recreate_file_host() {
		say(&format!("deploy: the rollback could not restart {old}"));
		say(DATABASE_NOTE);
		say(&stuck_note(backup.as_deref()));
		return EXIT_NOTHING_READY;
	}
	let (verdict, took_back) = wait_ready(&mut || host.reading(CONTAINER), clock, options.ready_timeout, options.poll);
	if verdict == Verdict::Ready {
		say(&format!(
			"deploy: ROLLED BACK: the previous image is ready after {took_back:.0} s. The image that failed is {}, and is skipped until a newer one is published.",
			failed_image.as_deref().unwrap_or("unknown")
		));
		say(&format!("deploy: {CHECKOUT_HINT}"));
		say(DATABASE_NOTE);
		return EXIT_ROLLED_BACK;
	}
	say(&format!("deploy: the previous image did not come up either: {}", verdict.reason()));
	say(DATABASE_NOTE);
	say(&stuck_note(backup.as_deref()));
	EXIT_NOTHING_READY
}

// ── On demand: a client needed a newer server ─────────────────────────────
//
// file_host writes ${STORAGE}/deploy/wanted when a client asks `/compat` for a
// contract epoch above its own (apps/servers/file_host/src/deploy.rs, which
// holds the bounds: one marker at a time, a gap between pulls, a ceiling, a
// need no published image met waits six hours). A systemd path unit
// (nix/modules/home-deploy.nix) runs `envctl deploy --on-demand` when it
// appears. This pass pulls, reads the pulled image's epoch, deploys it only
// when that is above the running one, records what it did in state.json
// beside the marker (which file_host reads for its answers and metrics, and
// the DeployFailed and DeployStuck alerts from those), and removes the
// marker. Nothing else ever pulls: with no client newer than home, home sends
// no request anywhere.

/// The shortest time between two pulls: deploy.rs's PULL_GAP, checked here
/// again so a marker written by hand cannot make home pull twice in a row.
pub(crate) const PULL_GAP_SECONDS: i64 = 600;

/// `${STORAGE}/deploy`: the marker file_host writes and the state it reads.
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
fn whole(seconds: f64) -> i64 {
	seconds as i64
}

/// The on-demand pass. An error is state.json that could not be written;
/// the caller records the failure and clears the marker
/// ([`record_crash`]).
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
		let serving_now = host.reading(CONTAINER);
		running = match (&serving_now.container, &serving_now.image) {
			(Some(_), Some(image)) => demand.image_epoch(image).filter(|epoch| *epoch != 0).unwrap_or(-1),
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
			let since = wall() - last_pull as f64;
			#[allow(clippy::cast_precision_loss)] // PULL_GAP_SECONDS is 600
			let gap = PULL_GAP_SECONDS as f64;
			if since < gap {
				say(&format!(
					"deploy: the last pull was {} s ago, under the {PULL_GAP_SECONDS} s gap; not pulling",
					whole(since)
				));
				return finish(demand, &mut state, &mut counts, EXIT_DEPLOYED, "deferred");
			}
		}
	}

	let before = host.reading(CONTAINER);
	let serving = if before.container.is_some() { before.image } else { None };
	let previous_id = host.image_id(&options.image);
	let pulled = host.pull(&options.image);
	bump(&mut counts, "pulls", 1);
	state.insert("last_pull_at".to_owned(), json!(whole(wall())));
	if !pulled {
		say(&format!("deploy: pulling {} failed; nothing was replaced", options.image));
		return finish(demand, &mut state, &mut counts, EXIT_NOT_DEPLOYED, "failed");
	}
	let candidate = host.image_id(&options.image);
	if let Some(found) = candidate.as_deref().filter(|found| Some(*found) != previous_id.as_deref()) {
		let bytes = demand.image_bytes(found);
		bump(&mut counts, "pulled_bytes", bytes);
	}
	let epoch = candidate.as_deref().and_then(|found| demand.image_epoch(found));
	state.insert("latest_seen_epoch".to_owned(), json!(epoch));
	let Some(epoch) = epoch.filter(|epoch| *epoch > running) else {
		// Lazily on the previous image (the owner, 2026-11-10): a newer build
		// of the same contract is not a reason to restart. :latest goes back
		// to what serves, so a reboot does not start it either.
		if let (Some(serving), Some(found)) = (&serving, &candidate) {
			if found != serving {
				host.tag(serving, &options.image);
			}
		}
		let seen = epoch.map_or_else(|| "an epoch it could not read".to_owned(), |epoch| format!("epoch {epoch}"));
		say(&format!("deploy: the newest image has {seen}, not above home's {running}; staying on the running one"));
		if epoch.is_some_and(|epoch| epoch < needs) {
			say(&format!(
				"deploy: nothing published serves epoch {needs} yet; home answers `unavailable` to it for six hours"
			));
		}
		return finish(demand, &mut state, &mut counts, EXIT_DEPLOYED, "no_newer_image");
	};

	say(&format!("deploy: the newest image has epoch {epoch}; deploying it"));
	let code = deploy(host, &Options { pull: false, ..options.clone() }, clock, say);
	finish(demand, &mut state, &mut counts, code, outcome_of_exit(code))
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
pub(crate) struct RealHost {
	repo: PathBuf,
	state: PathBuf,
	/// `make` (envctl, sops, cargo) runs as the checkout's owner when the
	/// on-demand unit runs this as root: `runuser -u <user> -- env HOME=…`
	/// (call C83).
	as_user: Vec<String>,
}

impl RealHost {
	pub(crate) fn new(repo: PathBuf, state_dir: &Path, as_user: Vec<String>) -> Self {
		Self {
			repo,
			state: state_dir.join("failed-images.json"),
			as_user,
		}
	}

	fn docker(&self, args: &[&str]) -> Option<(bool, String, String)> {
		run("docker", args, Some(&self.repo))
	}

	/// `make -s <args>`, as the user when one is given, its output shown as
	/// it comes.
	fn make(&self, args: &[&str]) -> bool {
		let mut words: Vec<&str> = self.as_user.iter().map(String::as_str).collect();
		words.extend(["make", "-s"]);
		words.extend(args);
		let (program, rest) = words.split_first().unwrap_or((&"make", &[]));
		Command::new(program).args(rest).current_dir(&self.repo).status().is_ok_and(|status| status.success())
	}
}

impl Host for RealHost {
	fn reading(&mut self, container: &str) -> Reading {
		let Some((true, stdout, _)) = self.docker(&["inspect", "--format", "{{json .}}", container]) else {
			return Reading::gone();
		};
		let Ok(info) = serde_json::from_str::<Value>(&stdout) else {
			return Reading::gone();
		};
		let text = |value: &Value| value.as_str().map(str::to_owned);
		Reading {
			container: Some(container.to_owned()),
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

	fn tag(&mut self, image_id: &str, image_ref: &str) -> bool {
		matches!(self.docker(&["tag", image_id, image_ref]), Some((true, _, _)))
	}

	fn pull(&mut self, image_ref: &str) -> bool {
		matches!(self.docker(&["pull", "-q", image_ref]), Some((true, _, _)))
	}

	fn up(&mut self) -> bool {
		self.make(&["up"])
	}

	fn recreate_file_host(&mut self) -> bool {
		// --no-deps: nothing else is the image that failed. --pull never: the
		// tag just moved back to the old image; a pull would undo that.
		self.make(&["compose", &format!("ARGS=up -d --no-deps --pull never {FILE_HOST}")])
	}

	fn logs(&mut self, container: &str) -> Vec<String> {
		self
			.docker(&["logs", "--tail", "200", container])
			.map(|(_, stdout, stderr)| (stdout + &stderr).lines().map(str::to_owned).collect())
			.unwrap_or_default()
	}

	fn failed_images(&mut self) -> Vec<String> {
		let Ok(text) = std::fs::read_to_string(&self.state) else {
			return Vec::new();
		};
		match serde_json::from_str::<Value>(&text) {
			Ok(Value::Array(items)) => items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), str::to_owned)).collect(),
			_ => Vec::new(),
		}
	}

	fn remember_failed(&mut self, image_id: &str) {
		let mut kept: Vec<String> = self.failed_images().into_iter().filter(|item| item != image_id).collect();
		let drop = kept.len().saturating_sub(FAILED_KEPT - 1);
		kept.drain(..drop);
		kept.push(image_id.to_owned());
		let written = self
			.state
			.parent()
			.map_or(Ok(()), std::fs::create_dir_all)
			.and_then(|()| std::fs::write(&self.state, Value::from(kept).to_string()));
		if let Err(err) = written {
			println!("deploy: could not record {image_id} as failed in {}: {err}", self.state.display());
		}
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
		let mut text = serde_json::to_string(state)?;
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

/// A non-negative integer written in digits only, as file_host prints them.
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
//
// Ported from scripts/deploy.py's `--self-test` with the same names, less the
// `test_` prefix: some-ui's assurance ledger cites them.

#[cfg(test)]
mod tests {
	use super::*;
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

	/// How a file-host image behaves once started: seconds since start ->
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
		// A migration file_host refuses: it exits, and `restart: unless-stopped`
		// brings it back to exit again.
		Rc::new(move |t| {
			if t < seconds {
				("running", Some("starting"), 0)
			} else {
				("restarting", Some("starting"), 1)
			}
		})
	}

	fn exited() -> Behaviour {
		Rc::new(|_| ("exited", None, 0))
	}

	const FH_REF: &str = IMAGE;

	#[derive(Default)]
	struct FakeHost {
		clock: FakeClock,
		behaviours: HashMap<String, Behaviour>,
		tags: HashMap<String, String>,
		serving: Option<String>,
		/// What a pull fetches.
		registry: HashMap<String, String>,
		pull_fails: bool,
		up_fails: bool,
		/// `up` removed the old container and created none.
		up_removes: bool,
		log: Vec<String>,
		failed: Vec<String>,
		calls: Vec<String>,
		started_at: f64,
	}

	impl FakeHost {
		fn start(&mut self) {
			let image = self.tags[FH_REF].clone();
			if Some(&image) != self.serving.as_ref() {
				self.serving = Some(image);
				self.started_at = self.clock.now();
			}
		}
	}

	impl Host for FakeHost {
		fn reading(&mut self, container: &str) -> Reading {
			let Some(serving) = &self.serving else {
				return Reading::gone();
			};
			let (status, health, restarts) = self.behaviours[serving](self.clock.now() - self.started_at);
			Reading {
				container: Some(container.to_owned()),
				status: Some(status.to_owned()),
				health: health.map(str::to_owned),
				restarts,
				image: Some(serving.clone()),
			}
		}

		fn image_id(&mut self, image_ref: &str) -> Option<String> {
			self.tags.get(image_ref).cloned()
		}

		fn describe(&mut self, image_id: &str) -> String {
			image_id.to_owned()
		}

		fn tag(&mut self, image_id: &str, image_ref: &str) -> bool {
			self.calls.push(format!("tag {image_id} {image_ref}"));
			self.tags.insert(image_ref.to_owned(), image_id.to_owned());
			true
		}

		fn pull(&mut self, image_ref: &str) -> bool {
			self.calls.push(format!("pull {image_ref}"));
			if !self.pull_fails {
				if let Some(id) = self.registry.get(image_ref) {
					self.tags.insert(image_ref.to_owned(), id.clone());
				}
			}
			!self.pull_fails
		}

		fn up(&mut self) -> bool {
			self.calls.push("up".to_owned());
			if self.up_removes {
				self.serving = None;
				return false;
			}
			if !self.up_fails {
				self.start();
			}
			!self.up_fails
		}

		fn recreate_file_host(&mut self) -> bool {
			self.calls.push("recreate".to_owned());
			self.start();
			true
		}

		fn logs(&mut self, _container: &str) -> Vec<String> {
			self.log.clone()
		}

		fn failed_images(&mut self) -> Vec<String> {
			self.failed.clone()
		}

		fn remember_failed(&mut self, image_id: &str) {
			self.failed.push(image_id.to_owned());
		}
	}

	#[derive(Default)]
	struct FakeDemand {
		marker: Option<Map<String, Value>>,
		state: Map<String, Value>,
		epochs: HashMap<String, Option<i64>>,
		cleared: bool,
	}

	impl FakeDemand {
		fn new(marker: Option<Value>) -> Self {
			Self {
				marker: marker.map(|marker| marker.as_object().unwrap().clone()),
				..Self::default()
			}
		}

		fn epoch(mut self, image: &str, epoch: Option<i64>) -> Self {
			self.epochs.insert(image.to_owned(), epoch);
			self
		}

		fn with_state(mut self, state: Value) -> Self {
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

		fn image_epoch(&mut self, image_id: &str) -> Option<i64> {
			self.epochs.get(image_id).copied().flatten()
		}

		fn image_bytes(&mut self, _image_id: &str) -> i64 {
			1000
		}
	}

	/// The serving image `old` (None: nothing serves), and a registry whose
	/// :latest is `new` when `publish`.
	fn world(old: Option<Behaviour>, new: Behaviour, publish: bool) -> FakeHost {
		let mut host = FakeHost::default();
		host.behaviours.insert("sha256:new".to_owned(), new);
		if let Some(old) = old {
			host.behaviours.insert("sha256:old".to_owned(), old);
			host.tags.insert(FH_REF.to_owned(), "sha256:old".to_owned());
			host.serving = Some("sha256:old".to_owned());
		}
		let published = if publish {
			"sha256:new".to_owned()
		} else {
			host.tags.get(FH_REF).cloned().unwrap_or_else(|| "sha256:new".to_owned())
		};
		host.registry.insert(FH_REF.to_owned(), published);
		host
	}

	fn run_deploy_with(host: &mut FakeHost, options: &Options) -> (u8, Vec<String>) {
		let mut said = Vec::new();
		let clock = host.clock.clone();
		let code = deploy(host, options, &clock, &mut |line| said.push(line.to_owned()));
		(code, said)
	}

	fn run_deploy(host: &mut FakeHost) -> (u8, Vec<String>) {
		run_deploy_with(host, &Options::default())
	}

	fn said_any(said: &[String], text: &str) -> bool {
		said.iter().any(|line| line.contains(text))
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

	// judge, wait_ready

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
	fn waiting_is_bounded() {
		let clock = FakeClock::default();
		let (verdict, took) = wait_ready(&mut || reading(Some("running"), Some("starting"), 0), &clock, 90.0, 2.0);
		assert!(is_failed(&verdict));
		assert!(verdict.reason().contains("within 90 s"), "{verdict:?}");
		assert!((took - 90.0).abs() < f64::EPSILON, "{took}");
	}

	#[test]
	fn rollback_only_to_a_different_previous_image() {
		assert!(plan_after_failure(Some("a"), Some("b")).rollback);
		assert!(!plan_after_failure(Some("a"), Some("a")).rollback);
		assert!(!plan_after_failure(None, Some("b")).rollback);
	}

	#[test]
	fn previous_ref_keeps_the_name() {
		assert_eq!(previous_ref("pgathondu/server:latest"), "pgathondu/server:previous");
		assert_eq!(previous_ref("host:5000/x/server:latest"), "host:5000/x/server:previous");
		assert_eq!(previous_ref("host:5000/server"), "host:5000/server:previous");
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
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), false);
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		assert!(said.last().unwrap().contains("nothing restarted"));
	}

	#[test]
	fn the_same_image_not_serving_is_brought_up_not_called_deployed() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), false);
		host.behaviours.insert("sha256:old".to_owned(), exited());
		let (code, said) = run_deploy(&mut host);
		assert!(host.calls.contains(&"up".to_owned()));
		assert_ne!(code, EXIT_DEPLOYED);
		assert!(!said_any(&said, "nothing restarted"));
		assert!(!host.calls.contains(&"recreate".to_owned()), "no previous image to roll back to");
	}

	#[test]
	fn skipping_a_failed_image_still_brings_a_stopped_container_up() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		host.failed = vec!["sha256:new".to_owned()];
		host.behaviours.insert("sha256:old".to_owned(), exited());
		let (code, said) = run_deploy(&mut host);
		assert!(said_any(&said, "failed here before"));
		assert_eq!(host.tags[FH_REF], "sha256:old");
		assert!(host.calls.contains(&"up".to_owned()));
		assert_ne!(code, EXIT_DEPLOYED);
	}

	#[test]
	fn only_file_hosts_image_is_pulled() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), false);
		run_deploy(&mut host);
		let pulls: Vec<&String> = host.calls.iter().filter(|call| call.starts_with("pull")).collect();
		assert_eq!(pulls, [&format!("pull {IMAGE}")]);
	}

	#[test]
	fn a_ready_image_is_deployed_and_the_old_one_kept() {
		let mut host = world(Some(ready_after(10.0)), ready_after(12.0), true);
		let (code, _) = run_deploy(&mut host);
		assert_eq!(code, EXIT_DEPLOYED);
		assert_eq!(host.serving.as_deref(), Some("sha256:new"));
		assert_eq!(host.tags["pgathondu/server:previous"], "sha256:old");
		assert_eq!(host.calls[host.calls.len() - 2..], ["tag sha256:old pgathondu/server:previous", "up"]);
	}

	#[test]
	fn an_image_that_never_becomes_ready_ends_with_the_previous_one_serving() {
		let timeout = DEFAULT_READY_TIMEOUT;
		let mut host = world(Some(ready_after(10.0)), never_ready(), true);
		let (code, said) = run_deploy_with(
			&mut host,
			&Options {
				ready_timeout: timeout,
				..Options::default()
			},
		);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert_eq!(host.tags[FH_REF], "sha256:old");
		assert_eq!(host.calls[host.calls.len() - 2..], ["tag sha256:old pgathondu/server:latest", "recreate"]);
		assert!(host.clock.now() <= 2.0f64.mul_add(timeout, POLL_SECONDS));
		assert_eq!(host.failed, ["sha256:new"]);
		assert!(said_any(&said, "ROLLED BACK"));
		assert!(said_any(&said, "needs a newer checkout on home"));
		assert!(said_any(&said, "Nothing was restored or deleted"));
	}

	#[test]
	fn a_failed_image_is_not_tried_again() {
		let mut host = world(Some(ready_after(0.0)), crashes_after(3.0), true);
		assert_eq!(run_deploy(&mut host).0, EXIT_ROLLED_BACK);
		host.calls.clear();
		let (code, said) = run_deploy(&mut host); // the registry still has the bad one
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		assert_eq!(host.tags[FH_REF], "sha256:old", "a reboot starts the old image, not the failed one");
		assert!(said_any(&said, "failed here before"));
		// A newer publish is tried.
		host.behaviours.insert("sha256:newer".to_owned(), ready_after(5.0));
		host.registry.insert(FH_REF.to_owned(), "sha256:newer".to_owned());
		assert_eq!(run_deploy(&mut host).0, EXIT_DEPLOYED);
		assert_eq!(host.serving.as_deref(), Some("sha256:newer"));
		assert!(host.failed.contains(&"sha256:new".to_owned()));
	}

	#[test]
	fn a_refused_migration_rolls_back_without_waiting_out_the_bound() {
		let mut host = world(Some(ready_after(4.0)), crashes_after(3.0), true);
		host.log = vec!["Error: migrate: refusing to migrate: failed: [20261110000100]".to_owned()];
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert!(host.clock.now() < 15.0);
		assert!(said_any(&said, "its log: Error: migrate: refusing"));
	}

	#[test]
	fn when_the_previous_image_fails_too_it_says_so_and_names_the_backup() {
		let backup = "/app/dbs/backups/hopium-20261110T120000Z-before-20261110000100.db";
		let mut host = world(Some(never_ready()), never_ready(), true);
		host.log = vec![format!("{BACKUP_LINE}{backup} (verified)")];
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOTHING_READY);
		let text = said.join("\n");
		assert!(text.contains(backup));
		assert!(text.contains("never restored automatically"));
		assert!(!host.calls.iter().any(|call| call.contains("restore")));
	}

	#[test]
	fn a_first_deploy_has_nothing_to_roll_back_to() {
		let mut host = world(None, never_ready(), true);
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOTHING_READY);
		assert!(said_any(&said, "no previous image"));
	}

	#[test]
	fn a_failed_pull_replaces_nothing() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		host.pull_fails = true;
		let (code, _) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOT_DEPLOYED);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert_eq!(host.calls, [format!("pull {FH_REF}")]);
	}

	#[test]
	fn a_refused_preflight_leaves_the_old_one_serving_and_names_the_checkout() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		host.up_fails = true;
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_NOT_DEPLOYED);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert!(host.failed.is_empty());
		assert!(said_any(&said, "still serves"));
		assert!(said_any(&said, "needs a newer checkout on home"));
	}

	#[test]
	fn an_up_that_leaves_no_container_still_rolls_back() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		host.up_removes = true;
		let (code, said) = run_deploy(&mut host);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert_eq!(host.tags[FH_REF], "sha256:old");
		assert_eq!(host.failed, ["sha256:new"], "the image up was asked to start");
		assert!(said_any(&said, "ROLLED BACK"));
	}

	// On demand

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

	#[test]
	fn no_marker_no_pull() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		let mut demand = FakeDemand::new(None);
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(host.calls.is_empty());
		assert!(demand.state.is_empty());
	}

	#[test]
	fn a_newer_epoch_is_deployed_and_recorded() {
		let mut host = world(Some(ready_after(0.0)), ready_after(5.0), true);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).epoch("sha256:new", Some(2));
		let (code, _) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_DEPLOYED);
		assert_eq!(host.serving.as_deref(), Some("sha256:new"));
		assert!(demand.cleared);
		assert_eq!(demand.state["latest_seen_epoch"], json!(2));
		assert_eq!(demand.state["counts"], json!({"pulls": 1, "pulled_bytes": 1000, "deployed": 1}));
		assert_eq!(demand.state["last_run"], json!({"at": wall(), "exit": 0, "outcome": "deployed", "needs": 2}));
		assert!(!host.calls[1..].contains(&format!("pull {IMAGE}")), "pulled once, not again by deploy");
	}

	#[test]
	fn the_same_epoch_republished_is_not_deployed() {
		// Lazily on the previous image: a newer build of the same contract.
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).epoch("sha256:new", Some(1));
		let (code, said) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		assert_eq!(host.tags[FH_REF], "sha256:old", "a reboot does not start it");
		assert_eq!(demand.state["counts"]["no_newer_image"], json!(1));
		assert_eq!(demand.state["latest_seen_epoch"], json!(1));
		assert!(said_any(&said, "unavailable"));
	}

	#[test]
	fn an_image_whose_epoch_cannot_be_read_is_not_deployed() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).epoch("sha256:new", None);
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		assert_eq!(demand.state["latest_seen_epoch"], Value::Null);
	}

	#[test]
	fn a_malformed_marker_never_reads_as_epoch_zero() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		// The serving image says 2; the published one is 2 as well: nothing newer.
		let mut demand = FakeDemand::new(Some(json!({}))).epoch("sha256:old", Some(2)).epoch("sha256:new", Some(2));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		// And when nothing can say what runs, nothing is deployed.
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		let mut demand = FakeDemand::new(Some(json!({}))).epoch("sha256:new", Some(9));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_NOT_DEPLOYED);
		assert!(!host.calls.contains(&"up".to_owned()));
		assert!(demand.cleared);
	}

	#[test]
	fn a_pull_within_the_gap_is_deferred() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).with_state(json!({"last_pull_at": wall() - 60}));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_DEPLOYED);
		assert!(host.calls.is_empty());
		assert_eq!(demand.state["counts"], json!({"deferred": 1}));
		assert!(demand.cleared);
	}

	#[test]
	fn a_newer_epoch_that_fails_is_rolled_back_and_counted() {
		let mut host = world(Some(ready_after(0.0)), crashes_after(3.0), true);
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})))
			.with_state(json!({"counts": {"pulls": 4, "deployed": 3}}))
			.epoch("sha256:new", Some(2));
		let (code, _) = run_on_demand(&mut host, &mut demand);
		assert_eq!(code, EXIT_ROLLED_BACK);
		assert_eq!(host.serving.as_deref(), Some("sha256:old"));
		assert_eq!(demand.state["counts"], json!({"pulls": 5, "pulled_bytes": 1000, "deployed": 3, "rolled_back": 1}));
		assert_eq!(demand.state["last_run"]["exit"], json!(EXIT_ROLLED_BACK));
	}

	#[test]
	fn a_failed_pull_is_counted_and_clears_the_marker() {
		let mut host = world(Some(ready_after(0.0)), ready_after(0.0), true);
		host.pull_fails = true;
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1})));
		assert_eq!(run_on_demand(&mut host, &mut demand).0, EXIT_NOT_DEPLOYED);
		assert_eq!(demand.state["counts"], json!({"pulls": 1, "failed": 1}));
		assert!(demand.cleared);
		assert_eq!(demand.state["last_pull_at"], json!(wall()), "a failed pull keeps the gap too");
	}

	#[test]
	fn a_crash_is_counted_and_clears_the_marker() {
		let mut demand = FakeDemand::new(Some(json!({"needs": 2, "running": 1}))).with_state(json!({"counts": {"pulls": 4}}));
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
		let mut host = RealHost::new(dir.clone(), &dir.join("state"), Vec::new());
		for n in 0..25 {
			host.remember_failed(&format!("sha256:{n}"));
		}
		host.remember_failed("sha256:10");
		let kept = host.failed_images();
		assert_eq!(kept.len(), FAILED_KEPT);
		assert_eq!(kept.last().map(String::as_str), Some("sha256:10"));
		assert_eq!(kept.first().map(String::as_str), Some("sha256:5"));
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
	}

	#[test]
	fn the_service_is_the_compose_files() {
		let text = compose_file();
		let pattern = format!(r"(?m)^  {}:\n((?:    .*\n|[ \t]*\n)+)", regex::escape(FILE_HOST));
		let block = regex::Regex::new(&pattern).unwrap().captures(&text).unwrap();
		assert!(block[1].contains(&format!("image: {IMAGE}\n")));
		assert!(block[1].contains(&format!("container_name: {CONTAINER}\n")));
	}
}
