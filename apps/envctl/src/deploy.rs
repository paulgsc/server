//! `deploy/<role>/<binary>.env`: every "ours" value of a binary, decided per
//! host in git (CFG9, #604). [`check`] holds each file to its schema and
//! each labelled compose service to reading one; [`Deployment`] edits one in
//! place for `import` and `resolve`.

use crate::{
	compose::{self, LABEL},
	dotenv::{self, shell_quote},
	schema::{Class, Schema, Variable},
	values::{bound_refusal, holds, refusal},
};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::LazyLock};

static UNSET: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^#unset\s+([A-Za-z_][A-Za-z0-9_]*)\s*$"));

/// The roles docker compose runs (`DEPLOY` in the box's `.env`). Every other
/// role is run from source (`make dev`), where nothing but its file sets the
/// wiring compose's own `environment:` gives a container.
pub(crate) const COMPOSE_ROLES: [&str; 1] = ["home"];

/// What a deployment file states for a variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stated {
	Value(String),
	/// `#unset NAME`: leaving it unset is the decision.
	Unset,
}

/// A deployment file's statements, and why any other line is not one, a
/// comment or blank.
pub(crate) struct Lines {
	pub(crate) stated: IndexMap<String, Stated>,
	pub(crate) problems: Vec<String>,
}

pub(crate) fn lines(text: &str) -> Lines {
	let mut stated = IndexMap::new();
	let mut problems = Vec::new();
	for (number, line) in text.lines().enumerate() {
		let stripped = line.trim();
		let (name, statement) = if let Some(captures) = UNSET.captures(stripped) {
			(captures[1].to_owned(), Stated::Unset)
		} else if stripped.is_empty() || stripped.starts_with('#') {
			continue;
		} else if let Some((name, raw)) = dotenv::assignment(line) {
			if raw.contains('$') {
				problems.push(format!(
					"{name} holds a `$`: compose would interpolate it from the box's .env, so the value would not be the one reviewed here"
				));
			}
			let value = dotenv::parse(line).shift_remove(&name).unwrap_or_default();
			(name, Stated::Value(value))
		} else {
			problems.push(format!("line {} is not NAME=value, `#unset NAME`, a comment or blank", number + 1));
			continue;
		};
		if stated.contains_key(&name) {
			problems.push(format!("{name} is stated twice"));
		}
		stated.insert(name, statement);
	}
	Lines { stated, problems }
}

/// Why `deploy/<role>/<binary>.env` is not every "ours" value of its binary,
/// decided: each stated once (a value, or `#unset` where unset is a choice),
/// nothing else, and each one its binary takes at start. `service` is the
/// compose service that reads it, for a role compose runs.
pub(crate) fn check_deployment(role: &str, schema: &Schema, text: &str, service: Option<&Value>) -> Vec<String> {
	let binary = &schema.binary;
	let Lines { stated, mut problems } = lines(text);
	let fixed = service.map(compose::sets).unwrap_or_default();
	let mut names: Vec<&String> = stated.iter().filter(|(_, statement)| matches!(statement, Stated::Value(_))).map(|(name, _)| name).collect();
	let mut unset: Vec<&String> = stated.iter().filter(|(_, statement)| **statement == Stated::Unset).map(|(name, _)| name).collect();
	unset.sort();
	names.extend(unset);
	let mut seen = IndexSet::new();
	for name in names {
		if !seen.insert(name) {
			continue;
		}
		let Some(variable) = schema.variable(name) else {
			problems.push(format!("{name} is not a variable of {binary} (infra/env/{binary}.json)"));
			continue;
		};
		match variable.class {
			Class::Secret => problems.push(format!("{name} is a secret: its value stays off git, in the box's .env")),
			Class::Private => problems.push(format!(
				"{name} is private: it names an account or a person, so it stays out of this public repository, in the box's .env until CFG11 (#607)"
			)),
			Class::Neutral => problems.push(format!("{name} is neutral: an override goes in the box's .env, not here")),
			Class::Ours if fixed.contains_key(name) => problems.push(format!(
				"{name} is set by compose's own `environment:` ({}), so a line here never reaches the container",
				fixed[name]
			)),
			Class::Ours if stated[name] == Stated::Unset && (variable.required || variable.default.is_some()) => problems.push(format!(
				"{name} cannot be left unset: {}",
				if variable.required {
					"it is required".to_owned()
				} else {
					format!("it defaults to {}; state the value", variable.default.as_deref().unwrap_or_default())
				}
			)),
			Class::Ours => {}
		}
	}
	let mut effective: BTreeMap<String, String> = schema.defaults().map(|(name, default)| (name.to_owned(), default.to_owned())).collect();
	for (name, statement) in &stated {
		if let Stated::Value(value) = statement {
			effective.insert(name.clone(), value.clone());
		}
	}
	effective.extend(fixed.iter().map(|(name, value)| (name.clone(), value.clone())));
	let current = |name: &str| effective.get(name).cloned();
	for variable in schema
		.variables
		.iter()
		.filter(|variable| variable.class == Class::Ours && !fixed.contains_key(&variable.name))
	{
		problems.extend(decided(variable, stated.get(&variable.name), &current));
	}
	problems.into_iter().map(|problem| format!("deploy/{role}/{binary}.env: {problem}")).collect()
}

/// Why one ours variable, stated as `statement` (None: not at all), is not a
/// decision its binary takes, `current` giving every value in effect.
pub(crate) fn decided(variable: &Variable, statement: Option<&Stated>, current: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
	let name = &variable.name;
	let Some(statement) = statement else {
		let unset = if variable.required || variable.default.is_some() {
			String::new()
		} else {
			format!(", or `#unset {name}`")
		};
		return vec![format!("{name} is not stated: decide its value here{unset}")];
	};
	let value = match statement {
		Stated::Value(value) => Some(value.as_str()),
		Stated::Unset => None,
	};
	let mut problems = Vec::new();
	if let Some(condition) = &variable.required_when {
		if holds(condition, current) && value.is_none_or(|value| value.trim().is_empty()) {
			problems.push(format!("{name} is required while {} is {}", condition.variable, condition.value));
		}
	}
	let Some(value) = value else { return problems };
	if variable.required && value.trim().is_empty() {
		problems.push(format!("{name} is required, and empty"));
	} else if let Some(problem) = refusal(variable, value).or_else(|| bound_refusal(variable, value, current)) {
		problems.push(format!("{name}={value} {problem}"));
	}
	problems
}

/// Why a labelled compose service does not take its binary's "ours" values
/// from its deployment file and only its secrets, private values and neutral
/// overrides from the box's `.env`.
pub(crate) fn check_service(name: &str, schema: &Schema, service: &Value) -> Vec<String> {
	let binary = &schema.binary;
	let mut problems = Vec::new();
	if compose::reads_env_file(service) {
		problems.push("reads the whole .env (`env_file`), so every line of it reaches the container".to_owned());
	}
	let suffix = format!("}}/{binary}.env");
	if !compose::env_file_paths(service)
		.iter()
		.any(|(path, required)| *required && path.contains("/deploy/${DEPLOY") && path.ends_with(&suffix))
	{
		problems.push(format!("does not read deploy/${{DEPLOY:?...}}/{binary}.env (`env_file`, required)"));
	}
	let own = compose::own_environment(service);
	for variable in &schema.variables {
		let variable_name = &variable.name;
		match (variable.class, own.get(variable_name)) {
			(Class::Ours, Some(value)) if value.as_deref().is_none_or(|value| value.contains('$')) => {
				problems.push(format!(
					"passes {variable_name} from .env (`environment:`), and it is ours: its value is the deployment file's"
				));
			}
			(Class::Ours, _) | (_, Some(_)) => {}
			(class, None) => problems.push(format!(
				"does not pass {variable_name} ({}) from .env: add `- {variable_name}` to its `environment:`",
				class.name()
			)),
		}
	}
	problems.into_iter().map(|problem| format!("service {name}: {problem}")).collect()
}

/// The labelled compose services of `raw`, by binary, and the problems with
/// any label.
pub(crate) fn labelled<'a>(schemas: &[Schema], raw: &'a Value) -> (BTreeMap<String, (&'a str, &'a Value)>, Vec<String>) {
	let mut services = BTreeMap::new();
	let mut problems = Vec::new();
	for (name, service) in compose::services(raw) {
		let Some(binary) = compose::labels(service).get(LABEL).cloned() else { continue };
		if schemas.iter().any(|schema| schema.binary == binary) {
			services.insert(binary, (name, service));
		} else {
			problems.push(format!("service {name} names env-schema {binary}, and infra/env/ has no {binary}.json"));
		}
	}
	(services, problems)
}

/// Every deployment file, `files[(role, binary)]`, held to its binary's
/// schema, and every labelled service of `raw` to reading one: the CI check
/// that a PR adding an "ours" variable decides it in every deployment file.
pub(crate) fn check(schemas: &[Schema], raw: &Value, files: &BTreeMap<(String, String), String>) -> Vec<String> {
	let (services, mut problems) = labelled(schemas, raw);
	for (binary, (name, service)) in &services {
		if let Some(schema) = schemas.iter().find(|schema| &schema.binary == binary) {
			problems.extend(check_service(name, schema, service));
		}
	}
	let mut roles: Vec<String> = files
		.keys()
		.map(|(role, _)| role.clone())
		.chain(COMPOSE_ROLES.iter().map(|role| (*role).to_owned()))
		.collect();
	roles.sort();
	roles.dedup();
	for role in &roles {
		let composed = COMPOSE_ROLES.contains(&role.as_str());
		if composed {
			problems.extend(
				services
					.keys()
					.filter(|binary| !files.contains_key(&(role.clone(), (*binary).clone())))
					.map(|binary| format!("deploy/{role}/{binary}.env is missing: compose runs {binary} there")),
			);
		}
		for ((file_role, binary), text) in files {
			if file_role != role {
				continue;
			}
			let Some(schema) = schemas.iter().find(|schema| &schema.binary == binary) else {
				problems.push(format!("deploy/{role}/{binary}.env names no binary in infra/env/"));
				continue;
			};
			let service = if composed { services.get(binary).map(|(_, service)| *service) } else { None };
			problems.extend(check_deployment(role, schema, text, service));
		}
	}
	problems
}

/// Every `deploy/<role>/<binary>.env` under `directory`.
pub(crate) fn read_all(directory: &Path) -> anyhow::Result<BTreeMap<(String, String), String>> {
	let mut files = BTreeMap::new();
	let Ok(roles) = std::fs::read_dir(directory) else { return Ok(files) };
	for role in roles {
		let role = role?.path();
		if !role.is_dir() {
			continue;
		}
		for file in std::fs::read_dir(&role)? {
			let path = file?.path();
			let (Some(role_name), Some(stem)) = (role.file_name(), path.file_stem()) else {
				continue;
			};
			if path.extension().is_some_and(|extension| extension == "env") && !stem.to_string_lossy().contains('.') {
				files.insert(
					(role_name.to_string_lossy().into_owned(), stem.to_string_lossy().into_owned()),
					std::fs::read_to_string(&path)?,
				);
			}
		}
	}
	Ok(files)
}

/// A deployment file as `export NAME='value'` lines for a shell to `eval`,
/// skipping every variable `environment` already sets (`make dev`).
pub(crate) fn render_exports(text: &str, environment: &dyn Fn(&str) -> bool) -> String {
	lines(text)
		.stated
		.into_iter()
		.filter_map(|(name, statement)| match statement {
			Stated::Value(value) if !environment(&name) => Some(format!("export {name}={}\n", shell_quote(&value))),
			_ => None,
		})
		.collect()
}

/// A deployment file, edited line by line so its comments stay where they
/// are.
pub(crate) struct Deployment {
	lines: Vec<String>,
}

impl Deployment {
	pub(crate) fn parse(text: &str) -> Self {
		Self {
			lines: text.lines().map(str::to_owned).collect(),
		}
	}

	pub(crate) fn render(&self) -> String {
		let mut out = self.lines.join("\n");
		out.push('\n');
		out
	}

	fn position(&self, name: &str) -> Option<usize> {
		self.lines.iter().position(|line| {
			let stripped = line.trim();
			UNSET.captures(stripped).is_some_and(|captures| &captures[1] == name) || dotenv::assignment(line).is_some_and(|(found, _)| found == name)
		})
	}

	/// `name` stated as `statement`: its line replaced, or added at the end of
	/// its `# ── group ──` section (a new one at the end when there is none).
	/// A `# WAITS ON THE OWNER: NAME` note above it goes, since it is decided.
	pub(crate) fn set(&mut self, name: &str, statement: &Stated, group: Option<&str>) {
		self.drop_waiting(name);
		let line = match statement {
			Stated::Value(value) => format!("{name}={}", quote(value)),
			Stated::Unset => format!("#unset {name}"),
		};
		if let Some(at) = self.position(name) {
			self.lines[at] = line;
			return;
		}
		let heading = format!("# ── {} ──", group.unwrap_or("Other"));
		if let Some(start) = self.lines.iter().position(|existing| existing.trim() == heading) {
			let end = self.lines[start + 1..]
				.iter()
				.position(|existing| existing.starts_with("# ── "))
				.map_or(self.lines.len(), |offset| start + 1 + offset);
			let mut at = end;
			while at > start + 1 && self.lines[at - 1].trim().is_empty() {
				at -= 1;
			}
			self.lines.insert(at, line);
		} else {
			while self.lines.last().is_some_and(|last| last.trim().is_empty()) {
				self.lines.pop();
			}
			self.lines.extend([String::new(), heading, String::new(), line]);
		}
	}

	fn drop_waiting(&mut self, name: &str) {
		let marker = format!("# WAITS ON THE OWNER: {name}");
		let Some(start) = self.lines.iter().position(|line| line.starts_with(&marker)) else {
			return;
		};
		let mut end = start + 1;
		while end < self.lines.len() && self.lines[end].starts_with('#') && !self.lines[end].starts_with("# ── ") && !self.lines[end].starts_with("# WAITS ON") {
			end += 1;
		}
		if end < self.lines.len() && self.lines[end].trim().is_empty() && start > 0 && self.lines[start - 1].trim().is_empty() {
			end += 1;
		}
		self.lines.drain(start..end);
	}
}

/// A value as a dotenv line compose, dotenvy and a shell all read back whole:
/// bare when it is plain, double-quoted otherwise.
pub(crate) fn quote(value: &str) -> String {
	let plain = !value.is_empty() && value.chars().all(|character| character.is_ascii_alphanumeric() || "-_.,:/@+=%".contains(character));
	if plain {
		value.to_owned()
	} else {
		format!("\"{}\"", value.replace('\\', r"\\").replace('"', "\\\""))
	}
}

#[cfg(test)]
mod tests {
	use super::{check, check_service, lines, quote, render_exports, Deployment, Stated};
	use crate::{compose::LABEL, schema::fixture::ours};
	use serde_json::{json, Value};
	use std::collections::BTreeMap;

	const GOOD: &str = "# a comment\nON=true\nURL=https://x\n#unset WHO\nDAYS=30\n";

	fn raw() -> Value {
		json!({"services": {"app": {
			"labels": {LABEL: "probe"},
			"env_file": [{"path": "/repo/infra/compose/${ROOT:?x}/deploy/${DEPLOY:?set DEPLOY}/probe.env", "required": true}],
			"environment": ["WIRE=w://wire", "PORT", "KEY", "ACCOUNT"],
		}}})
	}

	fn files(pairs: &[(&str, &str, &str)]) -> BTreeMap<(String, String), String> {
		pairs
			.iter()
			.map(|(role, binary, text)| (((*role).to_owned(), (*binary).to_owned()), (*text).to_owned()))
			.collect()
	}

	#[test]
	fn a_file_that_decides_every_ours_value_passes() {
		let dev = format!("{GOOD}WIRE=sqlite://dev.db\n");
		assert_eq!(check(&[ours()], &raw(), &files(&[("home", "probe", GOOD), ("dev", "probe", &dev)])), Vec::<String>::new());
	}

	#[test]
	fn every_problem_is_named_once() {
		let problems = check(
			&[ours()],
			&raw(),
			&files(&[
				(
					"home",
					"probe",
					"ON=true\nDAYS=29\nWIRE=x\nPORT=1\nKEY=s\nACCOUNT=a\nGONE=1\n#unset WHO\n#unset WHO\nX=${Y}\nnot a line\n",
				),
				("dev", "probe", "ON=yes\n#unset DAYS\n"),
				("dev", "nobody", ""),
			]),
		);
		let expected = [
			"deploy/dev/nobody.env names no binary",
			"deploy/dev/probe.env: DAYS cannot be left unset: it defaults to 90",
			"deploy/dev/probe.env: URL is not stated",
			"deploy/dev/probe.env: WHO is not stated",
			"deploy/dev/probe.env: WIRE is not stated: decide its value here",
			"deploy/dev/probe.env: ON=yes must be true or false",
			"deploy/home/probe.env: WHO is stated twice",
			"deploy/home/probe.env: X holds a `$`",
			"deploy/home/probe.env: line 11 is not NAME=value",
			"deploy/home/probe.env: WIRE is set by compose's own `environment:` (w://wire)",
			"deploy/home/probe.env: PORT is neutral",
			"deploy/home/probe.env: KEY is a secret",
			"deploy/home/probe.env: ACCOUNT is private",
			"deploy/home/probe.env: GONE is not a variable of probe",
			"deploy/home/probe.env: X is not a variable of probe",
			"deploy/home/probe.env: URL is not stated: decide its value here, or `#unset URL`",
			"deploy/home/probe.env: DAYS=29 must be at least 30",
		];
		for want in expected {
			assert_eq!(problems.iter().filter(|problem| problem.starts_with(want)).count(), 1, "{want}: {problems:#?}");
		}
		assert_eq!(problems.len(), expected.len(), "{problems:#?}");
	}

	#[test]
	fn a_required_while_and_a_missing_file_are_named() {
		let unmet = check(&[ours()], &raw(), &files(&[("home", "probe", "ON=true\n#unset URL\n#unset WHO\nDAYS=30\n")]));
		assert_eq!(unmet, ["deploy/home/probe.env: URL is required while ON is true"]);
		assert_eq!(check(&[ours()], &raw(), &BTreeMap::new()), ["deploy/home/probe.env is missing: compose runs probe there"]);
	}

	#[test]
	fn a_service_takes_ours_from_its_file_and_the_rest_by_name() {
		let leaky = json!({"labels": {LABEL: "probe"}, "env_file": [{"path": "/x/${ROOT:?x}/.env"}], "environment": {"ON": null, "URL": "${URL:-u}", "PORT": null}});
		assert_eq!(
			check_service("app", &ours(), &leaky),
			[
				"service app: reads the whole .env (`env_file`), so every line of it reaches the container",
				"service app: does not read deploy/${DEPLOY:?...}/probe.env (`env_file`, required)",
				"service app: passes ON from .env (`environment:`), and it is ours: its value is the deployment file's",
				"service app: passes URL from .env (`environment:`), and it is ours: its value is the deployment file's",
				"service app: does not pass KEY (secret) from .env: add `- KEY` to its `environment:`",
				"service app: does not pass ACCOUNT (private) from .env: add `- ACCOUNT` to its `environment:`",
			]
		);
	}

	#[test]
	fn exports_keep_what_the_shell_already_sets() {
		let shell = |name: &str| name == "A";
		assert_eq!(
			render_exports("A=1\nB=\"x y; z\"\n#unset C\nD=\"it's\"\n", &shell),
			"export B='x y; z'\nexport D='it'\\''s'\n"
		);
	}

	#[test]
	fn an_edit_keeps_every_comment_and_reads_back() {
		let text = "# head\n\n# ── Switches ──\n\nON=false\n# WAITS ON THE OWNER: WHO, who it\n# is for.\n\n# ── Other ──\n\nX=1\n";
		let mut deployment = Deployment::parse(text);
		deployment.set("ON", &Stated::Value("true".into()), Some("Switches"));
		deployment.set("WHO", &Stated::Unset, Some("Switches"));
		deployment.set("URL", &Stated::Value("a b; c".into()), Some("Wiring"));
		let written = deployment.render();
		assert_eq!(
			written,
			"# head\n\n# ── Switches ──\n\nON=true\n#unset WHO\n\n# ── Other ──\n\nX=1\n\n# ── Wiring ──\n\nURL=\"a b; c\"\n"
		);
		let read = lines(&written);
		assert!(read.problems.is_empty());
		assert_eq!(read.stated["URL"], Stated::Value("a b; c".into()));
		assert_eq!((quote("https://x/"), quote("")), ("https://x/".to_owned(), "\"\"".to_owned()));
	}
}
