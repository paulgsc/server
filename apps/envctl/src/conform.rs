//! Conformance, for CI (ENV3, #616): names only, never a value, and no key.
//!
//! Every deployment file is one sops wrote; every name in it is a variable
//! its binary or compose reads, and every variable of its binary has a name
//! in it (an empty value is "decided: unset"); a role that runs compose has a file for every
//! labelled service; every binary of ours that reads its environment through
//! `app-config` has a checked-in schema; and no tracked file is a plain-text
//! env file. Each problem is named.

use crate::{
	compose::{self, ComposeVariables},
	preflight::{DEPENDENCY, FROM_MAKE},
	schema::Schema,
	sops,
};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeSet, sync::LazyLock};

static COMPOSE_OWN: LazyLock<Regex> = LazyLock::new(|| crate::dotenv::regex(r"^(COMPOSE_\w+|DOCKER_\w+)$"));
static RECIPIENT: LazyLock<Regex> = LazyLock::new(|| crate::dotenv::regex(r"^sops_(age|pgp|kms|gcp_kms|azure_kv|hc_vault)__list_\d+__map_"));

/// What sops' own metadata in `encrypted` lacks, and each name whose value
/// is not encrypted: sops writes a MAC, its version and at least one
/// recipient, and wraps every value but an empty one (which it leaves as it
/// is) in `ENC[...]`. A MAC is not verified here: that needs the key. A line
/// is reported by its number, never its text.
pub(crate) fn sops_problems(encrypted: &str) -> Vec<String> {
	let mut problems = Vec::new();
	let mut seen = BTreeSet::new();
	for (number, line) in encrypted.lines().enumerate() {
		if line.trim().is_empty() {
			continue;
		}
		if line.trim_start().starts_with('#') {
			if !line.starts_with("#ENC[") {
				problems.push(format!("a comment in the clear on line {} (sops encrypts comments)", number + 1));
			}
			continue;
		}
		let Some((name, value)) = line.split_once('=') else {
			problems.push(format!("line {} is no NAME=value", number + 1));
			continue;
		};
		if name == "sops_mac" {
			if !value.starts_with("ENC[") {
				problems.push("sops_mac is not one sops wrote".to_owned());
			}
			seen.insert("mac");
		} else if name == "sops_version" {
			seen.insert("version");
		} else if RECIPIENT.is_match(name) {
			seen.insert("recipient");
		} else if !sops::is_name(name) {
			// Not a name, so it may hold a value: reported by its number only.
			problems.push(format!("line {} is no NAME=value", number + 1));
		} else if !name.starts_with("sops_") && !value.is_empty() && !value.starts_with("ENC[") {
			problems.push(format!("{name} in the clear"));
		}
	}
	for (part, missing) in [("mac", "no sops_mac"), ("version", "no sops_version"), ("recipient", "no recipient")] {
		if !seen.contains(part) {
			problems.push(missing.to_owned());
		}
	}
	problems
}

/// Whether a tracked path is a plain-text env file: `.env`, `.env.<x>`, or
/// `<x>.env` that is not `<x>.sops.env`.
pub(crate) fn plain_env_file(path: &str) -> bool {
	let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
	let env_extension = std::path::Path::new(&name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("env"));
	name == ".env" || name.starts_with(".env.") || (env_extension && !name.ends_with(sops::SUFFIX))
}

/// One role's deployment directory: its files by app (compose's own as
/// [`sops::COMPOSE`]), each with its text.
pub(crate) struct Role {
	pub(crate) name: String,
	pub(crate) files: Vec<(String, String)>,
}

/// Every problem in the deployment files and compose's wiring, by name.
pub(crate) fn problems(schemas: &[Schema], compose_variables: &ComposeVariables, raw: &Value, roles: &[Role]) -> Vec<String> {
	let mut problems = Vec::new();
	let mut labelled: Vec<(String, &Value)> = Vec::new();
	for (name, service) in compose::services(raw) {
		if let Some(binary) = compose::labels(service).get(compose::LABEL) {
			problems.extend(compose::wired(service, binary).map(|problem| format!("service {name} {problem}")));
			if !schemas.iter().any(|schema| &schema.binary == binary) {
				problems.push(format!("service {name} names env-schema {binary}, and infra/env/ has no {binary}.json"));
			}
			labelled.push((binary.clone(), service));
		} else if !compose::env_file_paths(service).is_empty() {
			problems.push(format!(
				"service {name} reads an env_file and carries no {} label, so nothing holds it to a schema",
				compose::LABEL
			));
		}
	}
	for role in roles {
		let shown = |app: &str| sops::relative(&role.name, app).display().to_string();
		let has = |app: &str| role.files.iter().any(|(file, _)| file == app);
		let runs_compose = has(sops::COMPOSE);
		if runs_compose {
			for (binary, _) in &labelled {
				if !has(binary) {
					problems.push(format!("{} does not exist: {} runs compose, which starts {binary}", shown(binary), role.name));
				}
			}
		}
		for (app, text) in &role.files {
			let file = shown(app);
			problems.extend(sops_problems(text).into_iter().map(|problem| format!("{file} is not sops-encrypted: {problem}")));
			let names = sops::names(text);
			if app == sops::COMPOSE {
				for name in names {
					if !compose_variables.contains_key(&name) && !COMPOSE_OWN.is_match(&name) && !FROM_MAKE.contains(&name.as_str()) {
						problems.push(format!("{file} sets {name}, which no compose file reads"));
					}
					if FROM_MAKE.contains(&name.as_str()) {
						problems.push(format!("{file} sets {name}, which make sets itself"));
					}
				}
				continue;
			}
			let Some(schema) = schemas.iter().find(|schema| &schema.binary == app) else {
				problems.push(format!("{file} is for {app}, and infra/env/ has no {app}.json"));
				continue;
			};
			// In a role that runs compose, compose's own environment: wires these
			// for the app, and wins; a role that runs the binary itself (dev)
			// states them like any other.
			let wired: BTreeSet<String> = if runs_compose {
				labelled
					.iter()
					.filter(|(binary, _)| binary == app)
					.flat_map(|(_, service)| compose::own_environment(service).into_keys())
					.collect()
			} else {
				BTreeSet::new()
			};
			for name in &names {
				if wired.contains(name) {
					problems.push(format!("{file} sets {name}, which compose's own environment: sets for {app} and wins"));
				} else if schema.variable(name).is_none() && !DEPENDENCY.is_match(name) {
					problems.push(format!("{file} sets {name}, which {app} does not declare (renamed or retired?)"));
				}
			}
			for variable in &schema.variables {
				if !wired.contains(&variable.name) && !names.contains(&variable.name) {
					problems.push(format!(
						"{file} does not state {}: every variable of {app} is decided per role (its value, or empty for unset)",
						variable.name
					));
				}
			}
		}
	}
	problems
}

/// From `cargo metadata --no-deps`: each binary named after its package
/// that reads its environment through `app-config`, which therefore has a
/// schema to check in.
pub(crate) fn configured_binaries(metadata: &Value) -> Vec<String> {
	let mut binaries: Vec<String> = metadata
		.get("packages")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter(|package| {
			package
				.get("dependencies")
				.and_then(Value::as_array)
				.is_some_and(|dependencies| dependencies.iter().any(|dependency| dependency.get("name").and_then(Value::as_str) == Some("app-config")))
		})
		.filter_map(|package| {
			let name = package.get("name")?.as_str()?;
			let is_bin = package.get("targets")?.as_array()?.iter().any(|target| {
				target.get("name").and_then(Value::as_str) == Some(name) && target.get("kind").and_then(Value::as_array).is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
			});
			is_bin.then(|| name.to_owned())
		})
		.collect();
	binaries.sort();
	binaries
}

#[cfg(test)]
// `${...}` is compose's syntax under test here, not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
	use super::{configured_binaries, plain_env_file, problems, sops_problems, Role};
	use crate::{
		compose::{variables, LABEL},
		schema::{
			fixture::{schema, var},
			Kind,
		},
	};
	use serde_json::json;

	const SOPS_TAIL: &str = "sops_age__list_0__map_recipient=age1x\nsops_mac=ENC[AES256_GCM,data:m,type:str]\nsops_version=3.11.0\n";

	fn encrypted(names: &[&str]) -> String {
		let mut text = String::from("#ENC[AES256_GCM,data:c,type:comment]\n");
		for name in names {
			text.push_str(name);
			text.push_str("=ENC[AES256_GCM,data:v,type:str]\n");
		}
		text + SOPS_TAIL
	}

	#[test]
	fn a_file_sops_did_not_write_is_named() {
		assert!(sops_problems(&encrypted(&["A"])).is_empty());
		assert_eq!(sops_problems("A=ENC[x]\n"), ["no sops_mac", "no sops_version", "no recipient"]);
		let plain = format!("A=plain\n# a note\n{SOPS_TAIL}");
		assert_eq!(sops_problems(&plain), ["A in the clear", "a comment in the clear on line 2 (sops encrypts comments)"]);
		assert!(sops_problems(&format!("EMPTY=\n{SOPS_TAIL}")).is_empty(), "sops leaves an empty value as it is");
		assert_eq!(
			sops_problems(&format!("-----BEGIN KEY-----\n{SOPS_TAIL}")),
			["line 1 is no NAME=value"],
			"never the line's text"
		);
		assert_eq!(sops_problems(&format!("{SOPS_TAIL}sops_mac=forged\n")), ["sops_mac is not one sops wrote"]);
	}

	#[test]
	fn a_plain_env_file_is_one_by_its_name() {
		for plain in [".env", "apps/x/.env", ".env.local", "deploy/home/file_host.env", "x/prod.env", "X/PROD.ENV"] {
			assert!(plain_env_file(plain), "{plain}");
		}
		for fine in ["deploy/home/file_host.sops.env", "apps/envctl/src/dotenv.rs", "envrc", ".envrc"] {
			assert!(!plain_env_file(fine), "{fine}");
		}
	}

	fn raw() -> serde_json::Value {
		json!({"services": {
			"app": {
				"labels": {LABEL: "probe"},
				"env_file": [{"path": "/r/${ENV_DIR:?x}/probe.env", "required": true}],
				"environment": ["WIRE=w://wire"],
			},
			"other": {"labels": {LABEL: "ghost"}, "env_file": [{"path": "/r/${ENV_DIR:?x}/ghost.env", "required": true}]},
			"stray": {"env_file": [{"path": "/r/.env"}]},
		}})
	}

	#[test]
	fn every_problem_is_named_and_none_needs_a_key() {
		let probe = schema("probe", vec![var("ON", Kind::Bool), var("WIRE", Kind::String)]);
		let compose_variables = variables(&[("a.yml".to_owned(), "      - ${STORAGE:?s}:/data\n".to_owned())]);
		let roles = [
			Role {
				name: "home".into(),
				files: vec![
					("compose".into(), encrypted(&["STORAGE", "OLD_PATH", "REPO_ROOT", "COMPOSE_PROJECT_NAME"])),
					("probe".into(), encrypted(&["ON", "WIRE", "RETIRED", "OTEL_BSP_MAX_QUEUE_SIZE"])),
				],
			},
			Role {
				name: "dev".into(),
				files: vec![("probe".into(), "ON=plain\n".to_owned()), ("unknown".into(), encrypted(&[]))],
			},
		];
		let found = problems(&[probe], &compose_variables, &raw(), &roles);
		let expected = [
			"service other names env-schema ghost, and infra/env/ has no ghost.json",
			"service stray reads an env_file and carries no env-schema label, so nothing holds it to a schema",
			"deploy/home/ghost.sops.env does not exist: home runs compose, which starts ghost",
			"deploy/home/compose.sops.env sets OLD_PATH, which no compose file reads",
			"deploy/home/compose.sops.env sets REPO_ROOT, which make sets itself",
			"deploy/home/probe.sops.env sets WIRE, which compose's own environment: sets for probe and wins",
			"deploy/home/probe.sops.env sets RETIRED, which probe does not declare (renamed or retired?)",
			"deploy/dev/probe.sops.env is not sops-encrypted: ON in the clear",
			"deploy/dev/probe.sops.env is not sops-encrypted: no sops_mac",
			"deploy/dev/probe.sops.env is not sops-encrypted: no sops_version",
			"deploy/dev/probe.sops.env is not sops-encrypted: no recipient",
			"deploy/dev/probe.sops.env does not state WIRE: every variable of probe is decided per role (its value, or empty for unset)",
			"deploy/dev/unknown.sops.env is for unknown, and infra/env/ has no unknown.json",
		];
		assert_eq!(
			found, expected,
			"dev runs no compose: it needs no ghost file, and states WIRE itself; OTEL_BSP_* is the SDK's own"
		);
	}

	#[test]
	fn every_variable_is_stated_in_every_role_but_compose_wiring() {
		let probe = schema("probe", vec![var("ON", Kind::Bool), var("URL", Kind::String), var("WIRE", Kind::String)]);
		let raw = json!({"services": {"app": {
			"labels": {LABEL: "probe"},
			"env_file": [{"path": "/r/${ENV_DIR:?x}/probe.env", "required": true}],
			"environment": ["WIRE=w://wire"],
		}}});
		let role = |files: Vec<(&str, String)>| Role {
			name: "home".into(),
			files: files.into_iter().map(|(app, text)| (app.to_owned(), text)).collect(),
		};
		let unset_url = encrypted(&["ON"]).replace("#ENC", "URL=\n#ENC");
		let stated = [role(vec![("compose", encrypted(&[])), ("probe", unset_url)])];
		assert!(
			problems(std::slice::from_ref(&probe), &variables(&[]), &raw, &stated).is_empty(),
			"an empty value states it unset"
		);
		let forgot = [role(vec![("compose", encrypted(&[])), ("probe", encrypted(&["ON"]))])];
		assert_eq!(
			problems(&[probe], &variables(&[]), &raw, &forgot),
			["deploy/home/probe.sops.env does not state URL: every variable of probe is decided per role (its value, or empty for unset)"]
		);
	}

	#[test]
	fn a_binary_reading_app_config_needs_a_schema() {
		let metadata = json!({"packages": [
			{"name": "file_host", "dependencies": [{"name": "app-config"}], "targets": [{"name": "file_host", "kind": ["bin"]}, {"name": "dump-routes", "kind": ["bin"]}]},
			{"name": "app-config", "dependencies": [], "targets": [{"name": "app_config", "kind": ["lib"]}]},
			{"name": "repo", "dependencies": [{"name": "app-config"}], "targets": [{"name": "repo", "kind": ["lib"]}]},
			{"name": "new_app", "dependencies": [{"name": "app-config"}], "targets": [{"name": "new_app", "kind": ["bin"]}]},
		]});
		assert_eq!(configured_binaries(&metadata), ["file_host", "new_app"]);
	}
}
