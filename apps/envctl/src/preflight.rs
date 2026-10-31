//! `make preflight`, before `docker compose up` (CFG8 #504): every required
//! variable missing, every value its binary would refuse and every "ours"
//! value `.env` would override, all in one pass.

use crate::{
	compose::{self, ComposeVariables},
	dotenv::{self, Entries},
	schema::{Class, Schema},
	values::{bound_refusal, refusal},
};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;
use std::{
	collections::BTreeSet,
	fmt::Write as _,
	path::{Path, PathBuf},
	sync::LazyLock,
};

/// Read by a dependency, not declared by any `Config` (README.md,
/// "Configuration", ENV1's Tolerance): a line setting one is not stale.
static DEPENDENCY: LazyLock<Regex> = LazyLock::new(|| {
	dotenv::regex(r"^(COMPOSE_\w+|DOCKER_\w+|TOKIO_WORKER_THREADS|POLARS_MAX_THREADS|RUST_BACKTRACE|OTEL_RESOURCE_ATTRIBUTES|OTEL_EXPORTER_OTLP_HEADERS|OTEL_BSP_\w+)$")
});
/// Read by docker compose itself, never passed to a container.
static COMPOSE_OWN: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^(COMPOSE_\w+|DOCKER_\w+)$"));

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
	/// "ours" variables the env file sets, where nothing may override the
	/// deployment file.
	pub(crate) misplaced: Vec<Gap>,
	pub(crate) unread: Vec<String>,
	pub(crate) errors: Vec<String>,
	pub(crate) warnings: Vec<String>,
}

impl Report {
	pub(crate) const fn failed(&self) -> bool {
		!(self.missing.is_empty() && self.invalid.is_empty() && self.misplaced.is_empty() && self.errors.is_empty())
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

/// What the stack would start on, given `.env`'s text, the shell's variables
/// and compose's uninterpolated config; `read` opens a deployment file.
pub(crate) fn preflight(
	schemas: &[Schema],
	compose_variables: &ComposeVariables,
	env_text: &str,
	shell: &IndexMap<String, String>,
	raw: &Value,
	read: &dyn Fn(&Path) -> Option<String>,
) -> Report {
	let mut report = Report::default();
	let entries: Entries = dotenv::resolve(&dotenv::parse_entries(env_text), shell);
	let env: IndexMap<String, String> = entries.iter().map(|(name, entry)| (name.clone(), entry.value.clone())).collect();
	let given = |name: &str| shell.get(name).or_else(|| env.get(name)).cloned();
	for variable in compose_variables.values() {
		let value = given(&variable.name);
		if variable.required && value.as_deref().is_none_or(|value| variable.nonempty && value.is_empty()) {
			let help = if variable.message.is_empty() { &variable.help } else { &variable.message };
			report
				.missing
				.push(gap(&variable.name, &format!("docker compose ({})", variable.files.join(", ")), help, "missing"));
		}
	}
	for (name, service) in compose::services(raw) {
		let Some(binary) = compose::labels(service).get(compose::LABEL).cloned() else {
			continue;
		};
		let Some(schema) = schemas.iter().find(|schema| schema.binary == binary) else {
			report.errors.push(format!("service {name} names env-schema {binary}, and infra/env/ has no {binary}.json"));
			continue;
		};
		let (deployed, missing_files) = compose::deploy_entries(service, &given, read);
		report.errors.extend(missing_files.into_iter().map(|error| format!("service {name}: {error}")));
		let environment = compose::environment_of(service, &entries, &given, &deployed);
		let reader = format!("{binary} (service {name})");
		for variable in &schema.variables {
			if variable.class == Class::Ours && env.contains_key(&variable.name) && !report.misplaced.iter().any(|gap| gap.name == variable.name) {
				report.misplaced.push(gap(
					&variable.name,
					&reader,
					&variable.help,
					&format!("decided in deploy/<role>/{binary}.env, which nothing may override; remove it from .env"),
				));
			}
		}
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
	}
	let read_names: BTreeSet<String> = schemas
		.iter()
		.flat_map(|schema| schema.variables.iter().map(|variable| variable.name.clone()))
		.chain(compose_variables.keys().cloned())
		.chain(dotenv::referenced_inside(env_text))
		.collect();
	let mut sorted: Vec<&String> = env.keys().collect();
	sorted.sort();
	report.unread = sorted
		.iter()
		.filter(|name| !read_names.contains(**name) && !DEPENDENCY.is_match(name))
		.map(|name| (*name).clone())
		.collect();
	// A dependency's own variable reaches a container only by name now that no
	// service takes .env whole (CFG9): one no service passes is dropped without
	// a word, as a stale line would be.
	let services = compose::services(raw);
	let passed: BTreeSet<String> = services.iter().flat_map(|(_, service)| compose::own_environment(service).into_keys()).collect();
	for name in sorted {
		if !services.is_empty() && DEPENDENCY.is_match(name) && !COMPOSE_OWN.is_match(name) && !compose_variables.contains_key(name) && !passed.contains(name) {
			report.warnings.push(format!(
				".env sets {name}, which no container is given: add `- {name}` to the `environment:` of the service whose dependency reads it"
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

pub(crate) fn render(report: &Report, env_file: &Path) -> String {
	let mut out = String::new();
	for error in &report.errors {
		let _ = writeln!(out, "error: {error}");
	}
	if !report.missing.is_empty() {
		let _ = writeln!(
			out,
			"{} required variable(s) missing from {} and the environment:",
			report.missing.len(),
			env_file.display()
		);
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
	if !report.misplaced.is_empty() {
		let _ = writeln!(
			out,
			"{} \"ours\" variable(s) {} sets, whose value the deployment file decides (CFG9):",
			report.misplaced.len(),
			env_file.display()
		);
		for gap in &report.misplaced {
			let _ = writeln!(out, "  {}  [{}]  {}", gap.name, gap.reader, gap.problem);
		}
	}
	for name in &report.unread {
		let _ = writeln!(out, "warning: {} sets {name}, which nothing reads (renamed or retired?)", env_file.display());
	}
	for warning in &report.warnings {
		let _ = writeln!(out, "warning: {warning}");
	}
	if !report.failed() {
		let _ = writeln!(
			out,
			"preflight: every required variable is set, every value parses, and {} overrides no deployment file.",
			env_file.display()
		);
	}
	out
}

/// A warning when `env_file` is not the one compose reads: the repo's `.env`,
/// and with it `$REPO_ROOT/.env` (#580).
pub(crate) fn other_env_file(repo: &Path, env_file: &Path, repo_root: Option<&str>) -> Option<String> {
	let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
	let mut read: Vec<PathBuf> = vec![canonical(&repo.join(".env"))];
	read.extend(repo_root.map(|root| canonical(&Path::new(root).join(".env"))));
	(!read.contains(&canonical(env_file))).then(|| {
		format!(
			"{} is not the .env docker compose reads ({}): this checks {}, and compose starts on the other",
			env_file.display(),
			repo.join(".env").display(),
			env_file.display()
		)
	})
}

/// The preflight against the real compose files and `env_file`.
pub(crate) fn run(repo: &Path, schemas: &[Schema], compose_variables: &ComposeVariables, env_file: &Path, shell: &IndexMap<String, String>) -> Report {
	let env_text = std::fs::read_to_string(env_file).unwrap_or_default();
	let repo_root = shell.get("REPO_ROOT").cloned().or_else(|| dotenv::parse(&env_text).shift_remove("REPO_ROOT"));
	let elsewhere = other_env_file(repo, env_file, repo_root.as_deref());
	let read = |path: &Path| std::fs::read_to_string(path).ok();
	let mut report = match compose::config(repo) {
		Ok(raw) => preflight(schemas, compose_variables, &env_text, shell, &raw, &read),
		Err(err) => {
			let mut report = preflight(schemas, compose_variables, &env_text, shell, &Value::Null, &read);
			report.errors.push(format!("docker compose config failed, so no service was checked: {err:#}"));
			report
		}
	};
	report.warnings.extend(elsewhere);
	report
}

/// Every variable the stack reads, who reads it, its default, and whether
/// `given` sets it (`make env`).
pub(crate) fn render_list(schemas: &[Schema], compose_variables: &ComposeVariables, given: &dyn Fn(&str) -> bool) -> String {
	let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
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
			rows.push((variable.name.clone(), readers.join(", "), default, when, variable.help.clone()));
		}
	}
	let mut rest: Vec<_> = compose_variables.values().filter(|variable| !seen.contains(&variable.name)).collect();
	rest.sort_by(|a, b| a.name.cmp(&b.name));
	for variable in rest {
		let default = if variable.required {
			"required".to_owned()
		} else {
			variable.default.clone().unwrap_or_default()
		};
		rows.push((variable.name.clone(), "docker compose".to_owned(), default, String::new(), variable.help.clone()));
	}
	let mut out = String::new();
	for (name, readers, default, when, help) in rows {
		let state = if given(&name) {
			"set  "
		} else if default == "required" {
			"UNSET"
		} else {
			"     "
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
	use super::{first_line, other_env_file, preflight, render_list};
	use crate::{
		compose::{variables, LABEL},
		schema::{
			fixture::{ours, schema, var, when},
			Class, Kind,
		},
	};
	use indexmap::IndexMap;
	use serde_json::json;
	use std::path::Path;

	fn shell(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
		pairs.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
	}

	fn probe() -> crate::schema::Schema {
		let mut db = var("DB", Kind::String);
		db.required = true;
		db.secret = true;
		db.class = Class::Secret;
		let mut token = var("TOKEN", Kind::String);
		token.required = true;
		let mut key = var("KEY", Kind::String);
		key.required = true;
		let mut on = var("ON", Kind::Bool);
		on.default = Some("false".into());
		let mut port = var("PORT", Kind::U16);
		port.default = Some("3000".into());
		let mut mode = var("MODE", Kind::Enum);
		mode.default = Some("a".into());
		mode.values = vec!["a".into(), "b".into()];
		let mut addr = var("ADDR", Kind::SocketAddr);
		addr.default = Some("0.0.0.0:1".into());
		let mut log = var("LOG", Kind::String);
		log.default = Some("info".into());
		let mut rp = var("RP_ID", Kind::String);
		rp.required_when = Some(when("AUTH", "true"));
		let mut auth = var("AUTH", Kind::Bool);
		auth.default = Some("true".into());
		let mut pwd = var("PWD", Kind::String);
		pwd.required = true;
		pwd.secret = true;
		pwd.class = Class::Secret;
		schema("probe", vec![db, token, key, on, port, mode, addr, log, rp, auth, pwd])
	}

	const ENV: &str = "DB=\"sqlite://x\" # the dev one\nPORT=70000\nPORT=70001\nMODE=c\nADDR=localhost:1\nON=\nPWD=''\nWHISPER_MODEL=old\nSTORAGE=/s\nDBX=\"${STORAGE}/y\"\nLIT='$STORAGE'\nTOKIO_WORKER_THREADS=4\nCOMPOSE_PROJECT_NAME=x\n# KEY=commented\n";

	fn compose() -> crate::compose::ComposeVariables {
		variables(&[
			("a.yml".to_owned(), "      - path: ${ROOT:?set ROOT}/.env\n      - LOG=${LOG:-info}\n".to_owned()),
			("b.yml".to_owned(), "      - ${ROOT:?set ROOT}/data\n      - ${BARE?set BARE}\n".to_owned()),
		])
	}

	#[test]
	fn every_gap_is_named_at_once() {
		let raw = json!({"services": {
			"app": {"labels": {LABEL: "probe"}, "env_file": [{"path": "/repo/infra/compose/${ROOT:?x}/.env"}], "environment": ["TOKEN=${TOKEN_FROM_SHELL}", "LOG=${LOG:-info}", "KEY", "MODE=b"]},
			"db": {"environment": {}},
		}});
		let none = |_: &Path| None;
		let report = preflight(&[probe()], &compose(), ENV, &IndexMap::new(), &raw, &none);
		let names = |gaps: &[super::Gap]| gaps.iter().map(|gap| gap.name.clone()).collect::<Vec<_>>();
		assert_eq!(names(&report.missing), ["ROOT", "BARE", "KEY", "RP_ID"], "a condition's default counts");
		assert_eq!(names(&report.invalid), ["ON", "PORT", "ADDR"], "an empty bool is refused, and the last of two lines counts");
		assert_eq!(report.unread, ["DBX", "LIT", "WHISPER_MODEL"]);
		let shelled = preflight(
			&[probe()],
			&compose(),
			&format!("{ENV}AUTH=false\n"),
			&shell(&[("ROOT", "/r"), ("BARE", ""), ("KEY", "k")]),
			&raw,
			&none,
		);
		assert!(
			shelled.missing.is_empty(),
			"the shell counts, `${{X?}}` allows empty, and an off condition asks nothing: {:?}",
			shelled.missing
		);
	}

	#[test]
	fn the_deployment_file_reaches_its_service_and_ours_in_env_is_refused() {
		let good = "# c\nON=true\nURL=https://x\n#unset WHO\nDAYS=30\n";
		let raw = json!({"services": {"app": {
			"labels": {LABEL: "probe"},
			"env_file": [{"path": "/repo/infra/compose/${ROOT:?x}/deploy/${DEPLOY:?set DEPLOY}/probe.env", "required": true}],
			"environment": ["WIRE=w://wire", "PORT", "KEY", "ACCOUNT"],
		}}});
		let read = |path: &Path| (path == Path::new("/r/deploy/home/probe.env")).then(|| good.to_owned());
		let home = shell(&[("ROOT", "/r"), ("DEPLOY", "home")]);
		let clean = preflight(&[ours()], &IndexMap::new(), "PORT=4000\nKEY=k\n", &home, &raw, &read);
		assert!(!clean.failed(), "{clean:?}");
		let misplaced = preflight(&[ours()], &IndexMap::new(), "ON=false\nPORT=1\n", &home, &raw, &read);
		assert_eq!(misplaced.misplaced.iter().map(|gap| gap.name.as_str()).collect::<Vec<_>>(), ["ON"]);
		let lost = preflight(&[ours()], &IndexMap::new(), "", &shell(&[("ROOT", "/r"), ("DEPLOY", "nowhere")]), &raw, &read);
		assert!(
			lost.errors.iter().any(|error| error.contains("/r/deploy/nowhere/probe.env does not exist")),
			"{:?}",
			lost.errors
		);
		let dropped = preflight(
			&[ours()],
			&IndexMap::new(),
			"OTEL_EXPORTER_OTLP_HEADERS=a=b\nKEY=k\nCOMPOSE_PROJECT_NAME=x\n",
			&home,
			&raw,
			&read,
		);
		assert_eq!(dropped.warnings.len(), 1, "{:?}", dropped.warnings);
		assert!(dropped.warnings[0].starts_with(".env sets OTEL_EXPORTER_OTLP_HEADERS"));
	}

	#[test]
	fn listing_and_sentences() {
		let listing = render_list(&[probe()], &compose(), &|name| name == "DB");
		assert!(
			listing.contains("set   DB  [probe]") && listing.contains("UNSET TOKEN") && listing.contains("RP_ID  [probe]  required while AUTH is true"),
			"{listing}"
		);
		assert_eq!(first_line("One. Two.\nThree"), "One.");
		assert_eq!(first_line(&"x".repeat(120)).len(), 100);
		assert!(other_env_file(Path::new("/srv/box"), Path::new("/srv/box/.env"), None).is_none());
		assert!(other_env_file(Path::new("/srv/box"), Path::new("/elsewhere/.env"), None).is_some());
	}
}
