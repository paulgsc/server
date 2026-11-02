//! `make preflight`, before `docker compose up` (CFG8 #504, on #614's
//! files): every required variable a deployment's files miss, every value
//! its binary would refuse, all in one pass, from the files decrypted in
//! memory. It names variables and never prints a value (#613).

use crate::{
	compose::{self, ComposeVariables},
	dotenv,
	schema::Schema,
	sops::{self, Plain},
	values::{bound_refusal, refusal},
};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeSet, fmt::Write as _, sync::LazyLock};

/// Read by a dependency, not declared by any `Config`: the OpenTelemetry
/// SDK's own variables (an authenticated collector's headers, resource
/// attributes, the batch processor's limits). An app's file may set them,
/// and its container gets them as it gets the rest of the file.
pub(crate) static DEPENDENCY: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^(RUST_BACKTRACE|OTEL_RESOURCE_ATTRIBUTES|OTEL_EXPORTER_OTLP_HEADERS|OTEL_BSP_\w+)$"));
/// Read by docker compose itself, in the compose file.
static COMPOSE_OWN: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^(COMPOSE_\w+|DOCKER_\w+)$"));

/// What `make` gives compose itself, never a deployment file: the
/// checkout's path and where the decrypted files are.
pub(crate) const FROM_MAKE: [&str; 2] = ["REPO_ROOT", "ENV_DIR"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gap {
	pub(crate) name: String,
	pub(crate) reader: String,
	pub(crate) help: String,
	pub(crate) problem: String,
}

#[derive(Debug, Default)]
pub(crate) struct Report {
	pub(crate) missing: Vec<Gap>,
	pub(crate) invalid: Vec<Gap>,
	pub(crate) errors: Vec<String>,
	pub(crate) warnings: Vec<String>,
}

impl Report {
	pub(crate) const fn failed(&self) -> bool {
		!(self.missing.is_empty() && self.invalid.is_empty() && self.errors.is_empty())
	}
}

fn gap(name: &str, reader: &str, help: &str, problem: &str) -> Gap {
	Gap {
		name: name.to_owned(),
		reader: reader.to_owned(),
		help: help.to_owned(),
		problem: problem.to_owned(),
	}
}

/// One deployment, decrypted: compose's own variables (its
/// `compose.sops.env` and what `make` adds) and each app's file, None
/// where the role has none.
#[derive(Debug, Default)]
pub(crate) struct Deployment {
	pub(crate) role: String,
	pub(crate) compose: Plain,
	pub(crate) apps: IndexMap<String, Option<Plain>>,
}

/// What the stack would start on, given one deployment's files and
/// compose's uninterpolated config.
pub(crate) fn preflight(schemas: &[Schema], compose_variables: &ComposeVariables, raw: &Value, deployment: &Deployment) -> Report {
	let mut report = Report::default();
	let role = &deployment.role;
	let compose_file = sops::relative(role, sops::COMPOSE).display().to_string();
	let given = |name: &str| deployment.compose.get(name).cloned();
	for variable in compose_variables.values() {
		let value = given(&variable.name);
		if variable.required && value.as_deref().is_none_or(|value| variable.nonempty && value.is_empty()) {
			let help = if variable.message.is_empty() { &variable.help } else { &variable.message };
			let reader = format!("docker compose ({}), in {compose_file}", variable.files.join(", "));
			report.missing.push(gap(&variable.name, &reader, help, "missing"));
		}
	}
	for name in deployment.compose.keys() {
		if !compose_variables.contains_key(name) && !COMPOSE_OWN.is_match(name) && !FROM_MAKE.contains(&name.as_str()) {
			report
				.warnings
				.push(format!("{compose_file} sets {name}, which no compose file reads (renamed or retired?)"));
		}
	}
	let mut seen = BTreeSet::new();
	for (name, service) in compose::services(raw) {
		let Some(binary) = compose::labels(service).get(compose::LABEL).cloned() else {
			continue;
		};
		let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
			report.errors.push(format!("service {name} names env-schema {binary}, and infra/env/ has no {binary}.json"));
			continue;
		};
		if let Some(problem) = compose::wired(service, &binary) {
			report.errors.push(format!("service {name} {problem}"));
		}
		let file_name = sops::relative(role, &binary).display().to_string();
		let Some(Some(file)) = deployment.apps.get(&binary) else {
			report.errors.push(format!(
				"service {name}: {file_name} does not exist; `make import` writes it from a running container, or `sops edit {file_name}`"
			));
			continue;
		};
		let environment = compose::environment_of(service, file, &given);
		let reader = format!("{binary} (service {name})");
		let current = |wanted: &str| {
			environment
				.get(wanted)
				.cloned()
				.or_else(|| schema.variable(wanted).and_then(|variable| variable.default.clone()))
		};
		for variable in &schema.variables {
			// Unset and empty differ: clap hands an empty value to its parser,
			// which takes it as text and refuses it as anything else.
			let value = environment.get(&variable.name);
			if let Some(condition) = &variable.required_when {
				if current(&condition.variable).as_deref() == Some(condition.value.as_str()) && value.is_none_or(|value| value.trim().is_empty()) {
					let why = format!("required while {} is {}. {}", condition.variable, condition.value, variable.help);
					report.missing.push(gap(&variable.name, &reader, &why, "missing"));
					continue;
				}
			}
			let Some(value) = value else {
				if variable.required {
					report.missing.push(gap(&variable.name, &reader, &variable.help, "missing"));
				}
				continue;
			};
			if let Some(problem) = refusal(variable, value).or_else(|| bound_refusal(variable, value, &current)) {
				report.invalid.push(gap(&variable.name, &reader, &variable.help, &problem));
			}
		}
		if !seen.insert(binary.clone()) {
			continue;
		}
		let own = compose::own_environment(service);
		for variable in file.keys() {
			if own.contains_key(variable) {
				report.warnings.push(format!(
					"{file_name} sets {variable}, and compose's own environment: for service {name} wins over it; remove it from one"
				));
			} else if schema.variable(variable).is_none() && !DEPENDENCY.is_match(variable) {
				report
					.warnings
					.push(format!("{file_name} sets {variable}, which {binary} does not read (renamed or retired?)"));
			}
		}
	}
	for (binary, file) in &deployment.apps {
		if file.is_some() && !seen.contains(binary) && !compose::services(raw).is_empty() {
			report.warnings.push(format!(
				"{} exists, and no compose service is labelled env-schema: {binary}",
				sops::relative(role, binary).display()
			));
		}
	}
	report
}

/// The first sentence of `text`'s first paragraph, at most 100 characters.
pub(crate) fn first_line(text: &str) -> String {
	let line = text.trim().lines().next().unwrap_or_default();
	let characters: Vec<char> = line.chars().collect();
	let end = (1..characters.len())
		.find(|&at| characters[at].is_whitespace() && matches!(characters[at - 1], '.' | ';'))
		.unwrap_or(characters.len());
	let sentence = &characters[..end];
	if sentence.len() <= 100 {
		sentence.iter().collect()
	} else {
		format!("{}...", sentence[..97].iter().collect::<String>().trim_end())
	}
}

pub(crate) fn render(report: &Report, role: &str) -> String {
	let mut out = String::new();
	for error in &report.errors {
		let _ = writeln!(out, "error: {error}");
	}
	if !report.missing.is_empty() {
		let _ = writeln!(out, "{} required variable(s) missing from deploy/{role}/:", report.missing.len());
		for gap in &report.missing {
			let _ = writeln!(out, "  {}  [{}]  {}", gap.name, gap.reader, first_line(&gap.help));
		}
	}
	if !report.invalid.is_empty() {
		let _ = writeln!(out, "{} value(s) their binary would refuse at start:", report.invalid.len());
		for gap in &report.invalid {
			let _ = writeln!(out, "  {}  [{}]  {}", gap.name, gap.reader, gap.problem);
		}
	}
	for warning in &report.warnings {
		let _ = writeln!(out, "warning: {warning}");
	}
	if !report.failed() {
		let _ = writeln!(out, "preflight: deploy/{role}/ sets every required variable, and every value parses.");
	}
	out
}

/// How a role's files state one variable, read by name without a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stated {
	/// A value (encrypted), or compose's own wiring.
	Set,
	/// An empty value: decided, unset (sops leaves an empty value as it is).
	Unset,
	/// Not in the file at all.
	Absent,
}

/// Every variable the stack reads, who reads it, its default, and how the
/// role's files state it, by name (`make env`).
pub(crate) fn render_list(schemas: &[Schema], compose_variables: &ComposeVariables, stated: &dyn Fn(&str, &str) -> Stated) -> String {
	let mut rows: Vec<(String, String, Stated, String, String, String)> = Vec::new();
	let mut seen = BTreeSet::new();
	for schema in schemas {
		for variable in &schema.variables {
			if !seen.insert(variable.name.clone()) {
				continue;
			}
			let readers: Vec<&str> = schemas
				.iter()
				.filter(|other| other.variable(&variable.name).is_some())
				.map(|other| other.binary.as_str())
				.collect();
			let by_reader: Vec<Stated> = readers.iter().map(|reader| stated(reader, &variable.name)).collect();
			let set = if by_reader.iter().all(|state| *state == Stated::Set) {
				Stated::Set
			} else if by_reader.iter().all(|state| *state != Stated::Absent) {
				Stated::Unset
			} else {
				Stated::Absent
			};
			let when = variable
				.required_when
				.as_ref()
				.map(|condition| format!("  required while {} is {}", condition.variable, condition.value))
				.unwrap_or_default();
			let default = if variable.required {
				"required".to_owned()
			} else {
				variable.default.clone().unwrap_or_default()
			};
			rows.push((variable.name.clone(), readers.join(", "), set, default, when, variable.help.clone()));
		}
	}
	let mut rest: Vec<_> = compose_variables
		.values()
		.filter(|variable| !seen.contains(&variable.name) && !FROM_MAKE.contains(&variable.name.as_str()))
		.collect();
	rest.sort_by(|a, b| a.name.cmp(&b.name));
	for variable in rest {
		let default = if variable.required {
			"required".to_owned()
		} else {
			variable.default.clone().unwrap_or_default()
		};
		let set = stated(sops::COMPOSE, &variable.name);
		rows.push((variable.name.clone(), "docker compose".to_owned(), set, default, String::new(), variable.help.clone()));
	}
	let mut out = String::new();
	for (name, readers, set, default, when, help) in rows {
		let state = match set {
			Stated::Set => "set    ",
			Stated::Unset => "unset  ",
			Stated::Absent if default == "required" => "MISSING",
			Stated::Absent => "       ",
		};
		let shown = match default.as_str() {
			"" => String::new(),
			"required" => "  required".to_owned(),
			default => format!("  default: {default}"),
		};
		let _ = writeln!(out, "{state} {name}  [{readers}]{shown}{when}");
		let first = first_line(&help);
		if !first.is_empty() {
			let _ = writeln!(out, "        {first}");
		}
	}
	out
}

#[cfg(test)]
// `${...}` is compose's syntax under test here, not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
	use super::{first_line, preflight, render, render_list, Deployment};
	use crate::{
		compose::{variables, LABEL},
		schema::{
			fixture::{schema, var, when},
			Kind,
		},
		sops::Plain,
	};
	use indexmap::IndexMap;
	use serde_json::json;

	fn plain(pairs: &[(&str, &str)]) -> Plain {
		pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
	}

	fn probe() -> crate::schema::Schema {
		let mut db = var("DB", Kind::String);
		db.required = true;
		let mut token = var("TOKEN", Kind::String);
		token.required = true;
		let mut on = var("ON", Kind::Bool);
		on.default = Some("false".into());
		let mut port = var("PORT", Kind::U16);
		port.default = Some("3000".into());
		let mut rp = var("RP_ID", Kind::String);
		rp.required_when = Some(when("AUTH", "true"));
		let mut auth = var("AUTH", Kind::Bool);
		auth.default = Some("true".into());
		let mut log = var("LOG", Kind::String);
		log.default = Some("info".into());
		schema("probe", vec![db, token, on, port, rp, auth, log])
	}

	fn compose() -> crate::compose::ComposeVariables {
		variables(&[
			("a.yml".to_owned(), "      - path: ${ENV_DIR:?set ENV_DIR}/probe.env\n      - LOG=${LOG:-info}\n".to_owned()),
			("b.yml".to_owned(), "      - ${STORAGE:?set STORAGE}:/data\n      - ${BARE?set BARE}\n".to_owned()),
		])
	}

	fn raw() -> serde_json::Value {
		json!({"services": {
			"app": {
				"labels": {LABEL: "probe"},
				"env_file": [{"path": "/repo/infra/compose/${ENV_DIR:?x}/probe.env", "required": true}],
				"environment": ["TOKEN=t://wired", "LOG=${LOG:-info}"],
			},
			"db": {"environment": {}},
		}})
	}

	const SECRET: &str = "hunter2-never-printed";

	#[test]
	fn every_gap_is_named_at_once_and_no_value_is_shown() {
		let deployment = Deployment {
			role: "home".into(),
			compose: plain(&[("ENV_DIR", "/run/user/1/server"), ("UNUSED", SECRET)]),
			apps: IndexMap::from([(
				"probe".to_owned(),
				Some(plain(&[
					("PORT", "70000"),
					("ON", SECRET),
					("TOKEN", SECRET),
					("OLD", SECRET),
					("OTEL_BSP_MAX_QUEUE_SIZE", "1"),
				])),
			)]),
		};
		let report = preflight(&[probe()], &compose(), &raw(), &deployment);
		let names = |gaps: &[super::Gap]| gaps.iter().map(|gap| gap.name.clone()).collect::<Vec<_>>();
		assert_eq!(names(&report.missing), ["STORAGE", "BARE", "DB", "RP_ID"], "a condition's default counts");
		assert_eq!(names(&report.invalid), ["ON", "PORT"]);
		let shown = render(&report, "home");
		assert!(!shown.contains(SECRET), "the preflight printed a value");
		assert!(shown.contains("deploy/home/compose.sops.env sets UNUSED, which no compose file reads"), "{shown}");
		assert!(shown.contains("deploy/home/probe.sops.env sets OLD, which probe does not read"), "{shown}");
		assert!(shown.contains("sets TOKEN, and compose's own environment:"), "{shown}");
		assert!(!shown.contains("OTEL_BSP"), "the OTel SDK's own variables reach the container: {shown}");
	}

	#[test]
	fn a_complete_deployment_passes_and_a_missing_file_is_named() {
		let complete = Deployment {
			role: "home".into(),
			compose: plain(&[("ENV_DIR", "/run"), ("STORAGE", "/s"), ("BARE", "")]),
			apps: IndexMap::from([("probe".to_owned(), Some(plain(&[("DB", "sqlite://x"), ("AUTH", "false")])))]),
		};
		let report = preflight(&[probe()], &compose(), &raw(), &complete);
		assert!(!report.failed(), "{report:?}");
		let absent = Deployment {
			apps: IndexMap::from([("probe".to_owned(), None)]),
			..complete
		};
		let report = preflight(&[probe()], &compose(), &raw(), &absent);
		assert!(report.errors[0].contains("deploy/home/probe.sops.env does not exist"), "{:?}", report.errors);
	}

	#[test]
	fn a_service_reading_more_than_its_own_file_is_refused() {
		let raw = json!({"services": {"app": {
			"labels": {LABEL: "probe"},
			"env_file": [{"path": "/repo/infra/compose/${REPO_ROOT:?x}/.env"}],
		}}});
		let deployment = Deployment {
			role: "dev".into(),
			apps: IndexMap::from([("probe".to_owned(), Some(plain(&[("DB", "x"), ("TOKEN", "t"), ("AUTH", "false")])))]),
			..Deployment::default()
		};
		let report = preflight(&[probe()], &IndexMap::new(), &raw, &deployment);
		assert!(report.errors.iter().any(|error| error.contains("exactly one env_file")), "{:?}", report.errors);
	}

	#[test]
	fn listing_and_sentences() {
		let stated = |reader: &str, name: &str| match (reader, name) {
			("probe", "DB") => super::Stated::Set,
			("probe", "ON") => super::Stated::Unset,
			_ => super::Stated::Absent,
		};
		let listing = render_list(&[probe()], &compose(), &stated);
		assert!(
			listing.contains("set     DB  [probe]")
				&& listing.contains("unset   ON  [probe]")
				&& listing.contains("MISSING TOKEN")
				&& listing.contains("RP_ID  [probe]  required while AUTH is true"),
			"{listing}"
		);
		assert!(listing.contains("MISSING STORAGE  [docker compose]") && !listing.contains("ENV_DIR"), "{listing}");
		assert_eq!(first_line("One. Two.\nThree"), "One.");
		assert_eq!(first_line(&"x".repeat(120)).len(), 100);
	}
}
