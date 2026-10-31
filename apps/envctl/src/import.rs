//! `envctl import`: a deployment file filled from what the running
//! containers use (CFG10, #606).
//!
//! A container's environment is the truth of what home runs: `.env`,
//! compose's own values and the image's `ENV`, merged, with nothing that did
//! not reach it. Only "ours" variables are read from it: a secret or private
//! value is never kept, printed or written (they go encrypted in CFG11, #607).

use crate::{
	compose,
	deploy::{self, Deployment, Stated},
	dotenv,
	schema::{Class, Schema, Variable},
};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde_json::Value;
use std::{path::Path, process::Command};

/// What a running container uses for one variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Running {
	/// No container was read.
	Unknown,
	/// Read, and unset there.
	Unset,
	Value(String),
}

/// One "ours" variable of a deployment file: what the file states, and what
/// the container runs on.
#[derive(Debug, Clone)]
pub(crate) struct Row {
	pub(crate) variable: Variable,
	pub(crate) file: Option<Stated>,
	pub(crate) live: Running,
}

impl Row {
	/// The statement that says what the container runs: its value, its
	/// default when it is unset, `#unset` when it has none.
	pub(crate) fn running_statement(&self) -> Option<Stated> {
		Some(match (&self.live, &self.variable.default) {
			(Running::Unknown, _) => return None,
			(Running::Value(value), _) => Stated::Value(value.clone()),
			(Running::Unset, Some(default)) => Stated::Value(default.clone()),
			(Running::Unset, None) => Stated::Unset,
		})
	}

	pub(crate) fn differs(&self) -> bool {
		self.running_statement().is_some_and(|live| self.file.as_ref() != Some(&live))
	}
}

/// The rows of `schema`'s deployment file `text`: every ours variable but
/// those compose's own `environment:` sets (`fixed`), with `live` beside each.
pub(crate) fn rows(schema: &Schema, text: &str, fixed: &IndexMap<String, String>, live: Option<&IndexMap<String, String>>) -> Vec<Row> {
	let stated = deploy::lines(text).stated;
	schema
		.variables
		.iter()
		.filter(|variable| variable.class == Class::Ours && !fixed.contains_key(&variable.name))
		.map(|variable| Row {
			variable: variable.clone(),
			file: stated.get(&variable.name).cloned(),
			live: live.map_or(Running::Unknown, |live| live.get(&variable.name).cloned().map_or(Running::Unset, Running::Value)),
		})
		.collect()
}

/// The containers running a binary of ours (labelled env-schema).
pub(crate) fn running() -> Result<Vec<String>> {
	let output = Command::new("docker")
		.args(["ps", "--filter", &format!("label={}", compose::LABEL), "--format", "{{.Names}}"])
		.output()
		.context("running docker ps")?;
	if !output.status.success() {
		bail!("docker ps failed: {}", String::from_utf8_lossy(&output.stderr).trim());
	}
	Ok(String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).filter(|name| !name.is_empty()).collect())
}

/// The binary a container runs and its "ours" values, from `docker inspect`.
/// Every other variable of its environment is dropped unread.
pub(crate) fn inspect(container: &str, schemas: &[Schema]) -> Result<(String, IndexMap<String, String>)> {
	let output = Command::new("docker")
		.args(["inspect", "--format", "{{json .Config}}", container])
		.output()
		.context("running docker inspect")?;
	if !output.status.success() {
		bail!("docker inspect {container} failed: {}", String::from_utf8_lossy(&output.stderr).trim());
	}
	let config: Value = serde_json::from_slice(&output.stdout).with_context(|| format!("docker inspect {container} printed no JSON"))?;
	ours_of(&config, schemas).with_context(|| format!("container {container}"))
}

/// [`inspect`] over a container's `.Config`.
pub(crate) fn ours_of(config: &Value, schemas: &[Schema]) -> Result<(String, IndexMap<String, String>)> {
	let Some(binary) = config.pointer(&format!("/Labels/{}", compose::LABEL)).and_then(Value::as_str) else {
		bail!("carries no {} label, so its binary is unknown", compose::LABEL);
	};
	let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
		bail!("runs {binary}, and infra/env/ has no {binary}.json");
	};
	let ours = config
		.get("Env")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(Value::as_str)
		.filter_map(|line| line.split_once('='))
		.filter(|(name, _)| schema.variable(name).is_some_and(|variable| variable.class == Class::Ours))
		.map(|(name, value)| (name.to_owned(), value.to_owned()))
		.collect();
	Ok((binary.to_owned(), ours))
}

/// `text` with every differing row set to what runs; the changes, as lines
/// a person reads.
pub(crate) fn apply(text: &str, rows: &[Row]) -> (String, Vec<String>) {
	let mut deployment = Deployment::parse(text);
	let mut changes = Vec::new();
	for row in rows.iter().filter(|row| row.differs()) {
		let Some(live) = row.running_statement() else { continue };
		let shown = |statement: Option<&Stated>| match statement {
			None => "not stated".to_owned(),
			Some(Stated::Unset) => "#unset".to_owned(),
			Some(Stated::Value(value)) => format!("{value:?}"),
		};
		changes.push(format!("{}: {} -> {}", row.variable.name, shown(row.file.as_ref()), shown(Some(&live))));
		deployment.set(&row.variable.name, &live, row.variable.group.as_deref());
	}
	(deployment.render(), changes)
}

fn header(role: &str, binary: &str) -> String {
	format!(
		"# {binary} on {role}: every \"ours\" variable of infra/env/{binary}.json, decided here\n# (CFG9, #604). Written by `envctl import` from the running container; review\n# it as any other change.\n"
	)
}

/// Every named (or every running) container's ours values into
/// `deploy/<role>/<binary>.env`.
pub(crate) fn run(repo: &Path, schemas: &[Schema], role: &str, containers: &[String], dry_run: bool) -> Result<()> {
	let containers = if containers.is_empty() {
		eprintln!("envctl import: asking docker for the running containers labelled {}...", compose::LABEL);
		running()?
	} else {
		containers.to_vec()
	};
	if containers.is_empty() {
		bail!("no running container carries the {} label; name one with --container", compose::LABEL);
	}
	let composed = deploy::COMPOSE_ROLES.contains(&role);
	let raw = if composed {
		eprintln!("envctl import: reading docker compose config for compose's own values...");
		compose::config(repo).context("docker compose config failed, so compose's own values are unknown")?
	} else {
		Value::Null
	};
	let (services, _) = deploy::labelled(schemas, &raw);
	let mut changed = false;
	for container in containers {
		eprintln!("envctl import: reading {container} (its \"ours\" values only)...");
		let (binary, live) = inspect(&container, schemas)?;
		let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
			continue;
		};
		let fixed = services.get(&binary).map(|(_, service)| compose::sets(service)).unwrap_or_default();
		let path = repo.join("deploy").join(role).join(format!("{binary}.env"));
		let text = std::fs::read_to_string(&path).unwrap_or_else(|_| header(role, &binary));
		let planned = rows(schema, &text, &fixed, Some(&live));
		let (written, edits) = apply(&text, &planned);
		let shown = path.strip_prefix(repo).unwrap_or(&path).display().to_string();
		if edits.is_empty() {
			println!("{shown}: already what {container} runs.");
			continue;
		}
		changed = true;
		println!("{shown}, from {container}:");
		for edit in &edits {
			println!("  {edit}");
		}
		if !dry_run {
			std::fs::create_dir_all(path.parent().unwrap_or(repo))?;
			std::fs::write(&path, written).with_context(|| format!("writing {}", path.display()))?;
		}
	}
	let env = dotenv::parse(&std::fs::read_to_string(repo.join(".env")).unwrap_or_default());
	let stale: Vec<&str> = schemas
		.iter()
		.flat_map(|schema| &schema.variables)
		.filter(|variable| variable.class == Class::Ours && env.contains_key(&variable.name))
		.map(|variable| variable.name.as_str())
		.collect::<indexmap::IndexSet<_>>()
		.into_iter()
		.collect();
	if !stale.is_empty() {
		println!("Once this is committed, remove these lines from .env (the preflight refuses them): {}", stale.join(", "));
	}
	if dry_run && changed {
		println!("Dry run: nothing written.");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{apply, ours_of, rows};
	use crate::{
		deploy::{lines, Stated},
		schema::fixture::ours,
	};
	use indexmap::IndexMap;
	use serde_json::json;

	#[test]
	fn a_container_gives_only_its_ours_values() {
		let config = json!({"Labels": {"env-schema": "probe"}, "Env": ["ON=true", "KEY=hunter2", "ACCOUNT=me", "PORT=1", "WIRE=w", "PATH=/bin"]});
		let (binary, live) = ours_of(&config, &[ours()]).unwrap();
		assert_eq!(binary, "probe");
		assert_eq!(live.keys().collect::<Vec<_>>(), ["ON", "WIRE"], "no secret, private, neutral or foreign value is kept");
		assert!(ours_of(&json!({"Env": []}), &[ours()]).is_err());
	}

	#[test]
	fn import_states_what_runs_and_nothing_else() {
		let text = "# head\n\nON=false\nDAYS=30\n";
		let live = IndexMap::from([
			("ON".to_owned(), "true".to_owned()),
			("URL".to_owned(), "https://x".to_owned()),
			("WIRE".to_owned(), "w".to_owned()),
		]);
		let fixed = IndexMap::from([("WIRE".to_owned(), "w".to_owned())]);
		let planned = rows(&ours(), text, &fixed, Some(&live));
		assert_eq!(
			planned.iter().map(|row| row.variable.name.as_str()).collect::<Vec<_>>(),
			["ON", "URL", "WHO", "DAYS"],
			"compose's own wiring is not the file's"
		);
		let (written, changes) = apply(text, &planned);
		assert_eq!(
			changes,
			[
				"ON: \"false\" -> \"true\"",
				"URL: not stated -> \"https://x\"",
				"WHO: not stated -> #unset",
				"DAYS: \"30\" -> \"90\""
			]
		);
		let read = lines(&written).stated;
		assert_eq!(read["ON"], Stated::Value("true".into()));
		assert_eq!(read["WHO"], Stated::Unset);
		assert_eq!(read["DAYS"], Stated::Value("90".into()), "unset where it runs means its default");
		let (again, none) = apply(&written, &rows(&ours(), &written, &fixed, Some(&live)));
		assert!(none.is_empty() && again == written, "a second import changes nothing");
	}
}
