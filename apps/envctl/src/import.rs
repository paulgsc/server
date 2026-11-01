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
pub(crate) fn compose_values(raw: &Value, containers: &[Value]) -> (Plain, Vec<String>) {
	let mut found = Plain::new();
	let mut conflicts = Vec::new();
	let mut keep = |name: &str, value: String, default: Option<&str>, found: &mut Plain| {
		if FROM_MAKE.contains(&name) || default == Some(value.as_str()) {
			return;
		}
		match found.get(name) {
			Some(known) if *known != value => conflicts.push(name.to_owned()),
			Some(_) => {}
			None => {
				found.insert(name.to_owned(), value);
			}
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
				keep(name, value.clone(), default, &mut found);
			}
		}
		let mounts: Vec<(String, String)> = container
			.get("Mounts")
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.filter_map(|mount| Some((mount.get("Destination")?.as_str()?.to_owned(), mount.get("Source")?.as_str()?.to_owned())))
			.collect();
		for (source, target) in compose::volumes(service) {
			let Some((name, default, suffix)) = lone(&source) else { continue };
			let Some((_, mounted)) = mounts.iter().find(|(destination, _)| *destination == target) else {
				continue;
			};
			if let Some(value) = mounted.strip_suffix(suffix) {
				keep(name, value.to_owned(), default, &mut found);
			}
		}
	}
	conflicts.sort();
	conflicts.dedup();
	(found, conflicts)
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
}

impl Change {
	pub(crate) const fn is_empty(&self) -> bool {
		self.added.is_empty() && self.changed.is_empty() && self.defaulted.is_empty() && self.unset.is_empty()
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
		eprintln!("envctl import: reading {container} (names are shown, values never)...");
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

/// Each file `files` holds, merged into what is there, completed (every
/// variable stated), and written through sops unless `dry_run`; the names
/// of each change printed. Whether anything was written.
fn write(repo: &Path, schemas: &[Schema], raw: &Value, files: &IndexMap<PathBuf, Plain>, dry_run: bool, sops_program: &[String]) -> Result<bool> {
	let mut wrote = false;
	for (relative, imported) in files {
		let path = repo.join(relative);
		let existing = if path.exists() { sops::decrypt(sops_program, repo, relative)? } else { Plain::new() };
		let (mut merged, mut change) = merge(&existing, imported);
		let app = relative
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(|name| name.strip_suffix(sops::SUFFIX))
			.unwrap_or_default();
		if let Some(schema) = schemas.iter().find(|schema| schema.binary == app) {
			let (defaulted, unset) = complete(schema, &wired(raw, app), &mut merged);
			change.defaulted = defaulted;
			change.unset = unset;
		}
		let shown = relative.display();
		if path.exists() && change.is_empty() {
			println!("{shown}: already what runs ({} variable(s)).", change.kept);
			continue;
		}
		println!(
			"{shown}: {} new, {} changed, {} the same, {} at its default, {} unset.",
			change.added.len(),
			change.changed.len(),
			change.kept,
			change.defaulted.len(),
			change.unset.len()
		);
		for name in &change.added {
			println!("  + {name}");
		}
		for name in &change.changed {
			println!("  ~ {name}");
		}
		for name in &change.defaulted {
			println!("  + {name} (its default; nothing set it)");
		}
		for name in &change.unset {
			println!("  + {name} (unset: an empty value)");
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
	eprintln!("envctl import: docker compose config, for what compose itself sets...");
	let raw = compose::config(repo).context("docker compose config failed, so compose's own wiring is unknown")?;
	let Some(project) = raw.get("name").and_then(Value::as_str).map(str::to_owned) else {
		bail!("docker compose config names no project");
	};
	eprintln!("envctl import: docker ps, for project {project}'s running containers labelled {}...", compose::LABEL);
	let ours = if named.is_empty() { running(&project, compose::LABEL)? } else { named.to_vec() };
	if ours.is_empty() {
		bail!("no running container of project {project} carries the {} label; name one with --container", compose::LABEL);
	}
	// Each deployment file, relative to the checkout, and what it will hold.
	let (mut files, app_conflicts) = read_ours(&ours, schemas, &raw, role)?;
	eprintln!("envctl import: docker ps, for every running container of project {project} (compose's own variables)...");
	let mut containers = Vec::new();
	for container in running(&project, SERVICE_LABEL)? {
		eprintln!("envctl import: reading {container}...");
		containers.push(inspect(&container)?);
	}
	let (mut compose_found, mut conflicts) = compose_values(&raw, &containers);
	conflicts.extend(app_conflicts);
	// What no running container shows (a service that is stopped), from the
	// box's .env while it is still there.
	let env = dotenv::parse(&std::fs::read_to_string(repo.join(".env")).unwrap_or_default());
	let compose_variables = compose::read(repo)?;
	let mut unfound = Vec::new();
	for variable in compose_variables.values() {
		if FROM_MAKE.contains(&variable.name.as_str()) || compose_found.contains_key(&variable.name) {
			continue;
		}
		match env.get(&variable.name) {
			Some(value) => {
				compose_found.insert(variable.name.clone(), value.clone());
			}
			None if variable.required => unfound.push(variable.name.clone()),
			None => {}
		}
	}
	files.insert(sops::relative(role, sops::COMPOSE), compose_found);
	let wrote = write(repo, schemas, &raw, &files, dry_run, sops_program)?;
	if !conflicts.is_empty() {
		println!(
			"Two containers disagree on {}; the first was kept. Check with `sops edit` before committing.",
			conflicts.join(", ")
		);
	}
	if !unfound.is_empty() {
		println!(
			"No running container or .env shows {}; set each with `sops set {} '[\"NAME\"]' '\"value\"'`.",
			unfound.join(", "),
			sops::relative(role, sops::COMPOSE).display()
		);
	}
	if dry_run {
		println!("Dry run: nothing written.");
	} else if wrote {
		println!("Commit deploy/{role}/, then `make up DEPLOY={role}` starts the stack from it.");
	}
	if repo.join(".env").exists() {
		println!(
			"Once `make up` has run from the encrypted files, delete the box's plain-text .env: rm {}",
			repo.join(".env").display()
		);
	}
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
		let (found, conflicts) = compose_values(&raw(), &[app, grafana]);
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
	fn a_merge_reports_names_only() {
		let (merged, change) = merge(&plain(&[("A", "1"), ("B", "2")]), &plain(&[("B", "3"), ("C", "4"), ("A", "1")]));
		assert_eq!(merged, plain(&[("A", "1"), ("B", "3"), ("C", "4")]));
		assert_eq!((change.added, change.changed, change.kept), (vec!["C".to_owned()], vec!["B".to_owned()], 1));
	}
}
