//! `envctl import` (ENV2, #615): a role's deployment files written from
//! what its running containers use, straight into sops.
//!
//! A container's environment is the truth of what a host runs: the box's
//! `.env`, compose's own values and the image's `ENV`, merged. From it this
//! keeps each app's schema variables (and the OpenTelemetry SDK's own), and
//! leaves out compose's own wiring and anything else the image sets.
//! Compose's own `${...}` variables are read back from where compose put
//! them: a container's environment or a bind mount's source. Values live in
//! memory and reach disk only encrypted, through `sops encrypt`'s stdin; what
//! is printed is names, never a value.

use crate::{
	compose, dotenv,
	dotenv::Token,
	preflight::{DEPENDENCY, FROM_MAKE},
	schema::Schema,
	sops::{self, Plain},
};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde_json::Value;
use std::{
	fmt::Write as _,
	path::{Path, PathBuf},
	process::Command,
};

/// The label compose gives each container: its service's name.
const SERVICE_LABEL: &str = "com.docker.compose.service";

/// The label compose gives each container: its project's name.
const PROJECT_LABEL: &str = "com.docker.compose.project";

/// The running containers of `project` (not one-off `compose run` ones)
/// carrying `label`: compose's own, or ours (env-schema).
fn running(project: &str, label: &str) -> Result<Vec<String>> {
	let output = Command::new("docker")
		.args(["ps", "--filter", &format!("label={label}")])
		.args(["--filter", &format!("label={PROJECT_LABEL}={project}")])
		.args(["--filter", "label=com.docker.compose.oneoff=False", "--format", "{{.Names}}"])
		.output()
		.context("running docker ps")?;
	if !output.status.success() {
		bail!("docker ps failed: {}", String::from_utf8_lossy(&output.stderr).trim());
	}
	let mut names: Vec<String> = String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).filter(|name| !name.is_empty()).collect();
	names.sort();
	Ok(names)
}

/// `docker inspect` of one container, held in memory only.
fn inspect(container: &str) -> Result<Value> {
	let output = Command::new("docker")
		.args(["inspect", "--type", "container", container])
		.output()
		.context("running docker inspect")?;
	if !output.status.success() {
		bail!("docker inspect {container} failed: {}", String::from_utf8_lossy(&output.stderr).trim());
	}
	let all: Value = serde_json::from_slice(&output.stdout).with_context(|| format!("docker inspect {container} printed no JSON"))?;
	all.get(0).cloned().with_context(|| format!("docker inspect {container} described no container"))
}

/// A container's environment, by name.
pub(crate) fn environment(inspected: &Value) -> Plain {
	inspected
		.pointer("/Config/Env")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(Value::as_str)
		.filter_map(|line| line.split_once('='))
		.map(|(name, value)| (name.to_owned(), value.to_owned()))
		.collect()
}

fn label<'a>(inspected: &'a Value, name: &str) -> Option<&'a str> {
	inspected.pointer("/Config/Labels").and_then(|labels| labels.get(name)).and_then(Value::as_str)
}

/// The binary a container runs, and what its deployment file holds: every
/// schema variable (and the `OTel` SDK's own) it was given, but those its
/// service's own `environment:` sets, literal or from compose's variables.
pub(crate) fn app_values(inspected: &Value, schemas: &[Schema], raw: &Value) -> Result<(String, Plain)> {
	let Some(binary) = label(inspected, compose::LABEL) else {
		bail!("carries no {} label, so its binary is unknown", compose::LABEL);
	};
	let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
		bail!("runs {binary}, and infra/env/ has no {binary}.json");
	};
	let own = label(inspected, SERVICE_LABEL)
		.and_then(|service| compose::services(raw).into_iter().find(|(name, _)| *name == service))
		.map(|(_, service)| compose::own_environment(service))
		.unwrap_or_default();
	let values = environment(inspected)
		.into_iter()
		.filter(|(name, _)| (schema.variable(name).is_some() || DEPENDENCY.is_match(name)) && !own.contains_key(name))
		.collect();
	Ok((binary.to_owned(), values))
}

/// `${NAME...}` alone, then literal text: the name, the default compose
/// would use without it, and the text after it.
fn lone(written: &str) -> Option<(&str, Option<&str>, &str)> {
	let start = written.find("${")?;
	let tokens = dotenv::tokens(&written[start..]);
	match tokens.as_slice() {
		[Token::Braced { name, operator, argument, .. }, rest @ ..] if rest.iter().all(|token| matches!(token, Token::Text(_))) => {
			let suffix = match rest {
				[Token::Text(text)] => *text,
				_ => "",
			};
			Some((name, (*operator == Some('-')).then_some(*argument), suffix))
		}
		_ => None,
	}
}

/// Compose's own variables, read back from the running containers: each
/// service's `environment:` entry that is one `${NAME...}`, and each bind
/// mount whose source starts with one. A value equal to the default compose
/// would use anyway is left out. Two containers that disagree are named.
pub(crate) fn compose_values(raw: &Value, containers: &[Value]) -> (Plain, Vec<String>, std::collections::BTreeSet<String>) {
	// Every place each variable reached, with the default that use declares:
	// all of them are weighed before any is left out, since two uses can
	// declare different defaults (file-host's RUST_LOG and orchestrator's).
	let mut seen: IndexMap<String, Vec<(String, Option<String>)>> = IndexMap::new();
	let mut observe = |name: &str, value: String, default: Option<&str>| {
		if !FROM_MAKE.contains(&name) {
			seen.entry(name.to_owned()).or_default().push((value, default.map(str::to_owned)));
		}
	};
	for (service_name, service) in compose::services(raw) {
		let Some(container) = containers.iter().find(|container| label(container, SERVICE_LABEL) == Some(service_name)) else {
			continue;
		};
		let given = environment(container);
		for (key, written) in compose::own_environment(service) {
			let Some((name, default, "")) = written.as_deref().and_then(lone) else { continue };
			if written.as_deref().is_some_and(|written| !written.starts_with("${")) {
				continue;
			}
			if let Some(value) = given.get(&key) {
				observe(name, value.clone(), default);
			}
		}
		let mounts: Vec<(String, String)> = container
			.get("Mounts")
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.filter_map(|mount| Some((mount.get("Destination")?.as_str()?.to_owned(), mount.get("Source")?.as_str()?.to_owned())))
			.collect();
		// A published port is read back from the binding docker made (Codex on
		// #622: OLLAMA_HOST_PORT).
		for (published, target) in compose::ports(service) {
			let Some((name, default, "")) = lone(&published) else { continue };
			let host_port = container
				.pointer("/HostConfig/PortBindings")
				.and_then(|bindings| bindings.get(&target))
				.and_then(Value::as_array)
				.and_then(|bindings| bindings.first())
				.and_then(|binding| binding.get("HostPort"))
				.and_then(Value::as_str);
			if let Some(value) = host_port.filter(|value| !value.is_empty()) {
				observe(name, value.to_owned(), default);
			}
		}
		for (source, target) in compose::volumes(service) {
			let Some((name, default, suffix)) = lone(&source) else { continue };
			let Some((_, mounted)) = mounts.iter().find(|(destination, _)| *destination == target) else {
				continue;
			};
			if let Some(value) = mounted.strip_suffix(suffix) {
				observe(name, value.to_owned(), default);
			}
		}
	}
	let mut found = Plain::new();
	let mut conflicts = Vec::new();
	let observed = seen.keys().cloned().collect();
	for (name, observations) in seen {
		// Every use already runs on its own default: compose gives it again.
		if observations.iter().all(|(value, default)| default.as_deref() == Some(value.as_str())) {
			continue;
		}
		let mut values: Vec<&String> = observations.iter().map(|(value, _)| value).collect();
		values.dedup();
		if values.iter().any(|value| *value != values[0]) {
			// One value for every use would change another's: the owner decides.
			conflicts.push(name.clone());
		}
		// The first value no use takes by default, for the owner to check.
		if let Some((value, _)) = observations.iter().find(|(value, default)| default.as_deref() != Some(value.as_str())) {
			found.insert(name, value.clone());
		}
	}
	conflicts.sort();
	(found, conflicts, observed)
}

/// How one file changes, by name only.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Change {
	pub(crate) added: Vec<String>,
	pub(crate) changed: Vec<String>,
	pub(crate) kept: usize,
	/// Stated at the binary's default, since nothing set them.
	pub(crate) defaulted: Vec<String>,
	/// Stated unset (an empty value), having no default and nothing setting
	/// them.
	pub(crate) unset: Vec<String>,
	/// How many variables the file states once written.
	pub(crate) total: usize,
	/// Taken out: what runs takes them by default, so a value the file held
	/// would change what runs.
	pub(crate) removed: Vec<String>,
}

impl Change {
	pub(crate) const fn is_empty(&self) -> bool {
		self.added.is_empty() && self.changed.is_empty() && self.defaulted.is_empty() && self.unset.is_empty() && self.removed.is_empty()
	}
}

/// The names compose's own `environment:` sets for `binary`'s labelled
/// services: compose's wiring, which a deployment file never states.
pub(crate) fn wired(raw: &Value, binary: &str) -> std::collections::BTreeSet<String> {
	compose::services(raw)
		.into_iter()
		.filter(|(_, service)| compose::labels(service).get(compose::LABEL).is_some_and(|label| label == binary))
		.flat_map(|(_, service)| compose::own_environment(service).into_keys())
		.collect()
}

/// `file` stating every variable of `schema` but compose's own (`wired`):
/// one it lacks takes the binary's default, or an empty value, "decided:
/// unset", when it has none (#616). The names each way, in schema order.
pub(crate) fn complete(schema: &Schema, wired: &std::collections::BTreeSet<String>, file: &mut Plain) -> (Vec<String>, Vec<String>) {
	let mut defaulted = Vec::new();
	let mut unset = Vec::new();
	for variable in &schema.variables {
		if wired.contains(&variable.name) || file.contains_key(&variable.name) {
			continue;
		}
		if let Some(default) = &variable.default {
			file.insert(variable.name.clone(), default.clone());
			defaulted.push(variable.name.clone());
		} else {
			file.insert(variable.name.clone(), String::new());
			unset.push(variable.name.clone());
		}
	}
	(defaulted, unset)
}

/// `existing` with `imported` over it, and what moved.
pub(crate) fn merge(existing: &Plain, imported: &Plain) -> (Plain, Change) {
	let mut merged = existing.clone();
	let mut change = Change::default();
	for (name, value) in imported {
		match existing.get(name) {
			None => change.added.push(name.clone()),
			Some(old) if old != value => change.changed.push(name.clone()),
			Some(_) => change.kept += 1,
		}
		merged.insert(name.clone(), value.clone());
	}
	(merged, change)
}

/// Each of our containers' deployment file, and the variables two
/// containers of one binary disagree on (the first is kept).
fn read_ours(ours: &[String], schemas: &[Schema], raw: &Value, role: &str) -> Result<(IndexMap<PathBuf, Plain>, Vec<String>)> {
	let mut files: IndexMap<PathBuf, Plain> = IndexMap::new();
	let mut conflicts = Vec::new();
	for container in ours {
		let inspected = inspect(container)?;
		let (binary, values) = app_values(&inspected, schemas, raw).with_context(|| format!("container {container}"))?;
		let file = files.entry(sops::relative(role, &binary)).or_default();
		for (name, value) in values {
			match file.get(&name) {
				Some(known) if *known != value => conflicts.push(format!("{binary}'s {name}")),
				Some(_) => {}
				None => {
					file.insert(name, value);
				}
			}
		}
	}
	Ok((files, conflicts))
}

/// Names in columns under a two-space indent, sorted, to fit 100 columns.
pub(crate) fn columns(names: &[String]) -> String {
	let mut sorted: Vec<&String> = names.iter().collect();
	sorted.sort();
	let width = sorted.iter().map(|name| name.len()).max().unwrap_or(0) + 2;
	let per_line = (96 / width.max(1)).max(1);
	let mut out = String::new();
	for line in sorted.chunks(per_line) {
		out.push_str("    ");
		let cells: Vec<String> = line.iter().map(|name| format!("{name:<width$}")).collect();
		out.push_str(cells.concat().trim_end());
		out.push('\n');
	}
	out
}

/// One file's part of the report: what it states, grouped by where each
/// value came from, names only.
pub(crate) fn report(file: &str, existed: bool, change: &Change) -> String {
	if existed && change.is_empty() {
		return format!("{file}\n  already what runs: {} variable(s), nothing to change\n\n", change.total);
	}
	let mut out = format!("{file}{}\n", if existed { "" } else { "  (new)" });
	for (label, names) in [
		("from what runs now", &change.added),
		("changed, to what runs now", &change.changed),
		("at the binary's default (nothing set them)", &change.defaulted),
		("unset (an empty value; no default)", &change.unset),
		("removed (what runs takes its default)", &change.removed),
	] {
		if !names.is_empty() {
			let _ = writeln!(out, "  {label}: {}", names.len());
			out.push_str(&columns(names));
		}
	}
	if change.kept > 0 {
		let _ = writeln!(out, "  already in the file, unchanged: {}", change.kept);
	}
	out.push('\n');
	out
}

/// What is left to do, numbered.
pub(crate) fn next_steps(repo: &Path, role: &str, conflicts: &[String], unverified: &[String], unfound: &[String], dry_run: bool, wrote: bool) -> String {
	let mut out = String::new();
	if !conflicts.is_empty() {
		let _ = write!(
			out,
			"Containers disagree on {}: no one value keeps every one as it runs. The file holds the first value\nno use takes by default; decide each with `sops edit` before `make up`.\n\n",
			conflicts.join(", ")
		);
	}
	let compose = sops::relative(role, sops::COMPOSE).display().to_string();
	let mut steps: Vec<String> = Vec::new();
	if dry_run {
		steps.push("make import          write the files above, through sops (nothing was written)".to_owned());
	}
	if !unverified.is_empty() {
		let mut step = format!("check these by hand before make up, with sops edit {compose}:");
		for name in unverified {
			let _ = write!(step, "\n       {name}");
		}
		steps.push(step);
	}
	if !unfound.is_empty() {
		steps.push(format!(
			"set what no container or .env shows, each with sops set {compose} '[\"NAME\"]' '\"value\"'\n{}",
			columns(unfound).trim_end_matches('\n').replace("\n    ", "\n       ").replacen("    ", "       ", 1)
		));
	}
	if dry_run || wrote || !unfound.is_empty() {
		steps.push(format!(
			"make up{}              start the stack from deploy/{role}/",
			if role == "home" { String::new() } else { format!(" DEPLOY={role}") }
		));
		steps.push(format!("git add deploy/{role} && git commit && git push"));
	}
	if repo.join(".env").exists() {
		steps.push(format!("rm {}   once make up has run from the encrypted files", repo.join(".env").display()));
	}
	if !steps.is_empty() {
		out.push_str("Next:\n");
		for (number, step) in steps.iter().enumerate() {
			let _ = writeln!(out, "  {}. {step}", number + 1);
		}
	}
	out
}

/// Each file `files` holds, merged into what is there, completed (every
/// variable stated), and written through sops unless `dry_run`; the names
/// of each change printed. Whether anything was written.
fn write(
	repo: &Path,
	schemas: &[Schema],
	raw: &Value,
	files: &IndexMap<PathBuf, Plain>,
	on_default: &IndexMap<PathBuf, std::collections::BTreeSet<String>>,
	dry_run: bool,
	sops_program: &[String],
) -> Result<bool> {
	let mut wrote = false;
	for (relative, imported) in files {
		let path = repo.join(relative);
		let existing = if path.exists() { sops::decrypt(sops_program, repo, relative)? } else { Plain::new() };
		let app = relative
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(|name| name.strip_suffix(sops::SUFFIX))
			.unwrap_or_default();
		// What runs, whole, before it meets the file: a variable the container
		// does not carry runs on its default (or unset), so a value the file
		// still holds for it is replaced, not kept (Codex on #622).
		let mut what_runs = imported.clone();
		let (filled_default, filled_unset) = schemas
			.iter()
			.find(|schema| schema.binary == app)
			.map_or_else(Default::default, |schema| complete(schema, &wired(raw, app), &mut what_runs));
		let (mut merged, mut change) = merge(&existing, &what_runs);
		// Names the completion filled are reported as such, not as values the
		// containers gave.
		for list in [&mut change.added, &mut change.changed] {
			list.retain(|name| {
				if filled_default.contains(name) {
					change.defaulted.push(name.clone());
					false
				} else if filled_unset.contains(name) {
					change.unset.push(name.clone());
					false
				} else {
					true
				}
			});
		}
		// Every use runs on its own default: a value the file still holds would
		// change what runs at the next make up (Codex on #622).
		for name in on_default.get(relative).into_iter().flatten() {
			if merged.shift_remove(name).is_some() {
				change.removed.push(name.clone());
			}
		}
		change.total = merged.len();
		print!("{}", report(&relative.display().to_string(), path.exists(), &change));
		if path.exists() && change.is_empty() {
			continue;
		}
		if dry_run {
			continue;
		}
		let encrypted = sops::encrypt(sops_program, repo, relative, &merged)?;
		std::fs::create_dir_all(path.parent().unwrap_or(repo)).with_context(|| format!("creating {}", path.display()))?;
		std::fs::write(&path, encrypted).with_context(|| format!("writing {}", path.display()))?;
		wrote = true;
	}
	Ok(wrote)
}

/// Every running container of ours (or those named) into
/// `deploy/<role>/`, through `sops`.
pub(crate) fn run(repo: &Path, schemas: &[Schema], role: &str, named: &[String], dry_run: bool, sops_program: &[String]) -> Result<()> {
	eprintln!("· docker compose config (what compose sets itself)");
	let raw = compose::config(repo).context("docker compose config failed, so compose's own wiring is unknown")?;
	let Some(project) = raw.get("name").and_then(Value::as_str).map(str::to_owned) else {
		bail!("docker compose config names no project");
	};
	eprintln!("· docker ps (project {project}'s containers labelled {})", compose::LABEL);
	let ours = if named.is_empty() { running(&project, compose::LABEL)? } else { named.to_vec() };
	if ours.is_empty() {
		bail!("no running container of project {project} carries the {} label; name one with --container", compose::LABEL);
	}
	eprintln!("· docker inspect {} (names are shown, values never)", ours.join(", "));
	// Each deployment file, relative to the checkout, and what it will hold.
	let (mut files, app_conflicts) = read_ours(&ours, schemas, &raw, role)?;
	let every = running(&project, SERVICE_LABEL)?;
	eprintln!("· docker inspect {} more of project {project} (compose's own variables)", every.len());
	let mut containers = Vec::new();
	for container in every {
		containers.push(inspect(&container)?);
	}
	eprintln!();
	let (mut compose_found, mut conflicts, observed) = compose_values(&raw, &containers);
	conflicts.extend(app_conflicts);
	// What no running container shows (a service that is stopped), from the
	// box's .env while it is still there.
	let env = dotenv::parse(&std::fs::read_to_string(repo.join(".env")).unwrap_or_default());
	let compose_variables = compose::read(repo)?;
	let mut unfound = Vec::new();
	// What no running container showed is never taken silently: a value from
	// .env, and a variable compose uses where import cannot read it back, are
	// named for the owner to check (Codex on #622, four findings in this one
	// mechanism: reading compose's variables back is best effort).
	let unread: std::collections::BTreeSet<String> = compose::services(&raw).into_iter().flat_map(|(_, service)| compose::unread_uses(service)).collect();
	let mut unverified = Vec::new();
	for variable in compose_variables.values() {
		// Only what no running container showed: one seen on its default runs
		// on that, whatever an edited .env says now (Codex on #622).
		if FROM_MAKE.contains(&variable.name.as_str()) || compose_found.contains_key(&variable.name) || observed.contains(&variable.name) {
			continue;
		}
		match env.get(&variable.name) {
			Some(value) => {
				compose_found.insert(variable.name.clone(), value.clone());
				unverified.push(format!("{} (from .env; no running container shows it)", variable.name));
			}
			None if variable.required => unfound.push(variable.name.clone()),
			None if unread.contains(&variable.name) => {
				unverified.push(format!("{} (compose uses it where import cannot read it back)", variable.name));
			}
			None => {}
		}
	}
	let compose_file = sops::relative(role, sops::COMPOSE);
	let on_default = IndexMap::from([(compose_file.clone(), observed.iter().filter(|name| !compose_found.contains_key(*name)).cloned().collect())]);
	files.insert(compose_file, compose_found);
	let wrote = write(repo, schemas, &raw, &files, &on_default, dry_run, sops_program)?;
	print!("{}", next_steps(repo, role, &conflicts, &unverified, &unfound, dry_run, wrote));
	Ok(())
}

#[cfg(test)]
// `${...}` is compose's syntax under test here, not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
	use super::{app_values, compose_values, merge};
	use crate::{
		schema::{
			fixture::{schema, var},
			Kind,
		},
		sops::Plain,
	};
	use serde_json::json;

	fn plain(pairs: &[(&str, &str)]) -> Plain {
		pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
	}

	fn raw() -> serde_json::Value {
		json!({"services": {
			"app": {
				"labels": {"env-schema": "probe"},
				"environment": ["WIRE=w://wire", "LOG=${LOG:-info}", "THREADS=${THREADS:-4}", "MIXED=x${M:-m}"],
				"volumes": [
					{"source": "/repo/infra/compose/${STORAGE:?set STORAGE}", "target": "/app/dbs"},
					{"source": "${MODELS:-/default/models}/sub", "target": "/models"},
					{"source": "${REPO_ROOT:?x}/infra/x", "target": "/x"},
				],
			},
			"grafana": {"environment": ["GF_PASSWORD=${GRAFANA_PASSWORD:?set it}"]},
		}})
	}

	fn container(service: &str, env: &[&str], mounts: &serde_json::Value) -> serde_json::Value {
		json!({
			"Config": {"Labels": {"env-schema": "probe", "com.docker.compose.service": service}, "Env": env},
			"Mounts": mounts,
		})
	}

	#[test]
	fn a_container_gives_its_schema_variables_and_not_compose_wiring() {
		let mut key = var("KEY", Kind::String);
		key.secret = true;
		let probe = schema("probe", vec![var("ON", Kind::Bool), key, var("WIRE", Kind::String), var("LOG", Kind::String)]);
		let app = container(
			"app",
			&["ON=true", "KEY=k", "WIRE=w://wire", "LOG=debug", "PATH=/bin", "OTEL_BSP_MAX_QUEUE_SIZE=9"],
			&json!([]),
		);
		let (binary, values) = app_values(&app, std::slice::from_ref(&probe), &raw()).unwrap();
		assert_eq!(binary, "probe");
		assert_eq!(
			values.keys().collect::<Vec<_>>(),
			["ON", "KEY", "OTEL_BSP_MAX_QUEUE_SIZE"],
			"every schema variable, secrets too; not compose's own environment, nor the image's PATH"
		);
		assert!(app_values(&json!({"Config": {"Env": []}}), &[probe], &raw()).is_err());
	}

	#[test]
	fn compose_variables_come_back_from_where_compose_put_them() {
		let app = container(
			"app",
			&["LOG=debug", "THREADS=4", "MIXED=xq"],
			&json!([
				{"Destination": "/app/dbs", "Source": "/mnt/db"},
				{"Destination": "/models", "Source": "/mnt/models/sub"},
				{"Destination": "/x", "Source": "/srv/repo/infra/x"},
			]),
		);
		let grafana = container("grafana", &["GF_PASSWORD=pw"], &json!([]));
		let (found, conflicts, observed) = compose_values(&raw(), &[app, grafana]);
		assert!(observed.contains("THREADS"), "a variable on its default is still one a container showed");
		assert_eq!(
			found,
			plain(&[("LOG", "debug"), ("STORAGE", "/mnt/db"), ("MODELS", "/mnt/models"), ("GRAFANA_PASSWORD", "pw")]),
			"a default compose would use anyway is left out, and REPO_ROOT is make's"
		);
		assert!(conflicts.is_empty());
		let other = container("app", &["LOG=trace"], &json!([]));
		let again = container("grafana", &["GF_PASSWORD=pw"], &json!([]));
		let raw_twice = json!({"services": {
			"app": {"environment": ["LOG=${LOG:-info}"]},
			"grafana": {"environment": ["GF_LOG=${LOG:-info}", "GF_PASSWORD=${GRAFANA_PASSWORD:?x}"]},
		}});
		let mut disagree = again;
		disagree["Config"]["Env"] = json!(["GF_LOG=warn", "GF_PASSWORD=pw"]);
		assert_eq!(compose_values(&raw_twice, &[other, disagree]).1, ["LOG"]);
		// Codex on #622: one use on its own default and another set otherwise
		// is a disagreement too, not a value to write for both.
		let defaults_differ = json!({"services": {
			"app": {"environment": ["LOG=${LOG:-info,nats=warn}"]},
			"grafana": {"environment": ["GF_LOG=${LOG:-info}"]},
		}});
		let on_default = container("app", &["LOG=info,nats=warn"], &json!([]));
		let mut set = container("grafana", &[], &json!([]));
		set["Config"]["Env"] = json!(["GF_LOG=debug"]);
		let (found, conflicts, _) = compose_values(&defaults_differ, &[on_default, set]);
		assert_eq!((found, conflicts), (plain(&[("LOG", "debug")]), vec!["LOG".to_owned()]));
		let both_default = [container("app", &["LOG=info,nats=warn"], &json!([])), {
			let mut grafana = container("grafana", &[], &json!([]));
			grafana["Config"]["Env"] = json!(["GF_LOG=info"]);
			grafana
		}];
		let (found, conflicts, observed) = compose_values(&defaults_differ, &both_default);
		assert_eq!((found, conflicts), (Plain::new(), Vec::new()), "every use on its own default");
		assert!(observed.contains("LOG"), "and so .env is not asked for it");
	}

	#[test]
	fn every_variable_is_stated_its_default_or_unset_but_compose_wiring() {
		let mut on = var("ON", Kind::Bool);
		on.default = Some("false".into());
		let probe = schema("probe", vec![on, var("URL", Kind::String), var("WIRE", Kind::String), var("SET", Kind::String)]);
		let wired = std::collections::BTreeSet::from(["WIRE".to_owned()]);
		let mut file = plain(&[("SET", "x")]);
		let (defaulted, unset) = super::complete(&probe, &wired, &mut file);
		assert_eq!((defaulted, unset), (vec!["ON".to_owned()], vec!["URL".to_owned()]));
		assert_eq!(file, plain(&[("SET", "x"), ("ON", "false"), ("URL", "")]), "compose's wiring is never stated");
		assert_eq!(super::wired(&raw(), "probe"), ["LOG", "MIXED", "THREADS", "WIRE"].into_iter().map(str::to_owned).collect());
	}

	#[test]
	fn the_report_groups_names_in_columns_and_numbers_what_is_left() {
		let change = super::Change {
			added: vec!["B".into(), "A".into()],
			defaulted: vec!["ON".into()],
			unset: vec!["URL".into()],
			..super::Change::default()
		};
		assert_eq!(
			super::report("deploy/home/p.sops.env", false, &change),
			"deploy/home/p.sops.env  (new)\n  from what runs now: 2\n    A  B\n  at the binary's default (nothing set them): 1\n    ON\n  unset (an empty value; no default): 1\n    URL\n\n"
		);
		let left = super::next_steps(std::path::Path::new("/nowhere"), "home", &[], &[], &["X".into()], true, false);
		assert!(
			left.starts_with("Next:\n  1. make import") && left.contains("  2. set what no container") && left.contains("       X"),
			"{left}"
		);
	}

	#[test]
	fn a_merge_reports_names_only() {
		let (merged, change) = merge(&plain(&[("A", "1"), ("B", "2")]), &plain(&[("B", "3"), ("C", "4"), ("A", "1")]));
		assert_eq!(merged, plain(&[("A", "1"), ("B", "3"), ("C", "4")]));
		assert_eq!((change.added, change.changed, change.kept), (vec!["C".to_owned()], vec!["B".to_owned()], 1));
	}
}
