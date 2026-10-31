//! The compose files: the `${...}` variables they declare, and what
//! `docker compose config --no-interpolate` says each service is given.

use crate::dotenv::{self, interpolate, Entries, Token};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;
use std::{
	path::{Path, PathBuf},
	process::Command,
	sync::LazyLock,
};

static INCLUDE: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^\s*-\s*path:\s*(\S+)\s*$"));
static OTHER_NAME: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r"^[A-Z][A-Z0-9_]*: "));

/// The label naming the binary a service's container runs (#504).
pub(crate) const LABEL: &str = "env-schema";

/// One `${NAME...}` the compose files interpolate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ComposeVariable {
	pub(crate) name: String,
	pub(crate) required: bool,
	pub(crate) default: Option<String>,
	pub(crate) message: String,
	pub(crate) help: String,
	pub(crate) files: Vec<String>,
	/// `${NAME:?}` somewhere, which refuses an empty value too; `${NAME?}`
	/// refuses only an unset one.
	pub(crate) nonempty: bool,
	/// Where `help` came from, so a better source replaces a worse one.
	help_from: HelpFrom,
}

/// A variable's help, from worst to best source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum HelpFrom {
	#[default]
	Nothing,
	/// The comment above a use with a default or a `?`.
	Declared,
	/// The comment that names it.
	Named,
}

pub(crate) type ComposeVariables = IndexMap<String, ComposeVariable>;

/// `docker-compose.yml` and each file it includes, in that order.
pub(crate) fn files(compose: &Path) -> Result<Vec<PathBuf>> {
	let text = std::fs::read_to_string(compose).with_context(|| format!("reading {}", compose.display()))?;
	let parent = compose.parent().unwrap_or_else(|| Path::new("."));
	let mut files = vec![compose.to_path_buf()];
	files.extend(text.lines().filter_map(|line| INCLUDE.captures(line)).map(|captures| parent.join(&captures[1])));
	Ok(files)
}

/// Every `${NAME...}` in the compose files under `repo`.
pub(crate) fn read(repo: &Path) -> Result<ComposeVariables> {
	let mut texts = Vec::new();
	for path in files(&repo.join("docker-compose.yml"))? {
		let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
		let shown = path.strip_prefix(repo).unwrap_or(&path).to_string_lossy().replace("/./", "/");
		texts.push((shown.trim_start_matches("./").to_owned(), text));
	}
	Ok(variables(&texts))
}

/// Every `${NAME...}` in `files`, as (display name, text) pairs.
///
/// A variable's help is the comment block directly above a use: the first
/// that names it, else the first with a default or a `?`, else the first,
/// else the `?` message. One used with `?` anywhere is required everywhere:
/// compose interpolates the whole project before it starts any service
/// (#407), so it blocks them all.
pub(crate) fn variables(files: &[(String, String)]) -> ComposeVariables {
	let mut found = ComposeVariables::new();
	for (file, text) in files {
		let lines: Vec<&str> = text.lines().collect();
		for (number, line) in lines.iter().enumerate() {
			if line.trim_start().starts_with('#') {
				continue;
			}
			for token in dotenv::tokens(line) {
				let Token::Braced { name, colon, operator, argument } = token else { continue };
				let entry = found.entry(name.to_owned()).or_insert_with(|| ComposeVariable {
					name: name.to_owned(),
					..ComposeVariable::default()
				});
				if !entry.files.contains(file) {
					entry.files.push(file.clone());
				}
				let above = comment_above(&lines, number);
				let names_it = dotenv::regex(&format!(r"\b{}\b", regex::escape(name))).is_match(&above);
				if entry.help_from < HelpFrom::Named && names_it {
					entry.help_from = HelpFrom::Named;
					entry.help = own_paragraph(&above, name);
				} else if entry.help_from < HelpFrom::Declared && operator.is_some() {
					entry.help_from = HelpFrom::Declared;
					entry.help = above;
				} else if entry.help_from == HelpFrom::Nothing && entry.help.is_empty() {
					entry.help = above;
				}
				match operator {
					Some('?') => {
						entry.required = true;
						entry.nonempty = entry.nonempty || colon;
						if entry.message.is_empty() {
							argument.clone_into(&mut entry.message);
						}
					}
					Some('-') if entry.default.is_none() => entry.default = Some(argument.to_owned()),
					_ => {}
				}
			}
		}
	}
	found
}

/// The part of a comment block that is `name`'s: from its line that starts
/// `NAME:` to the next that starts another `OTHER:`, else the whole block, so
/// one block can declare two variables used on one line.
pub(crate) fn own_paragraph(block: &str, name: &str) -> String {
	let lines: Vec<&str> = block.lines().collect();
	let Some(start) = lines.iter().position(|line| line.starts_with(&format!("{name}:"))) else {
		return block.to_owned();
	};
	let mut own = vec![lines[start]];
	own.extend(lines[start + 1..].iter().take_while(|line| !OTHER_NAME.is_match(line)));
	own.join("\n").trim().to_owned()
}

fn comment_above(lines: &[&str], number: usize) -> String {
	let mut block: Vec<&str> = lines[..number]
		.iter()
		.rev()
		.map(|line| line.trim())
		.take_while(|line| line.starts_with('#'))
		.map(|line| line.trim_start_matches('#').trim())
		.collect();
	block.reverse();
	block.join("\n").trim().to_owned()
}

/// `docker compose config` as JSON, not interpolated.
///
/// Not interpolated, so it reads even with a required variable unset (which
/// stops the interpolated form outright, naming only that one). It needs
/// compose v5: an older one refuses a short-syntax volume holding `${X:?}`.
pub(crate) fn config(repo: &Path) -> Result<Value> {
	let output = Command::new("docker")
		.args(["compose", "config", "--no-interpolate", "--format", "json"])
		.current_dir(repo)
		.output()
		.context("running docker compose config")?;
	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		bail!("{}", if stderr.trim().is_empty() { "docker compose config failed" } else { stderr.trim() });
	}
	serde_json::from_slice(&output.stdout).context("docker compose config printed no JSON")
}

/// The services of a `config`, by name, in order.
pub(crate) fn services(raw: &Value) -> Vec<(&str, &Value)> {
	let mut services: Vec<(&str, &Value)> = raw
		.get("services")
		.and_then(Value::as_object)
		.map(|services| services.iter().map(|(name, service)| (name.as_str(), service)).collect())
		.unwrap_or_default();
	services.sort_by_key(|(name, _)| *name);
	services
}

pub(crate) fn labels(service: &Value) -> IndexMap<String, String> {
	match service.get("labels") {
		Some(Value::Object(labels)) => labels.iter().map(|(key, value)| (key.clone(), text(value))).collect(),
		Some(Value::Array(items)) => items
			.iter()
			.filter_map(Value::as_str)
			.filter_map(|item| item.split_once('='))
			.map(|(key, value)| (key.to_owned(), value.to_owned()))
			.collect(),
		_ => IndexMap::new(),
	}
}

fn text(value: &Value) -> String {
	value.as_str().map_or_else(|| value.to_string(), str::to_owned)
}

/// Each `env_file` of a service, its path as written and whether it is
/// required, in the order compose reads them.
pub(crate) fn env_file_paths(service: &Value) -> Vec<(String, bool)> {
	let Some(files) = service.get("env_file").and_then(Value::as_array) else {
		return Vec::new();
	};
	files
		.iter()
		.map(|entry| match entry {
			Value::Object(entry) => (
				entry.get("path").map(text).unwrap_or_default(),
				entry.get("required").and_then(Value::as_bool).unwrap_or(true),
			),
			other => (text(other), true),
		})
		.collect()
}

pub(crate) fn reads_env_file(service: &Value) -> bool {
	env_file_paths(service).iter().any(|(path, _)| path.ends_with("/.env"))
}

/// An `env_file` path as compose opens it. Uninterpolated, compose has
/// already joined a path that starts with `${...}` to the compose file's
/// directory, so that prefix goes first.
pub(crate) fn resolve_path(raw: &str, given: &dyn Fn(&str) -> Option<String>) -> PathBuf {
	let start = raw.find("${").unwrap_or(0);
	PathBuf::from(interpolate(&raw[start..], given))
}

/// A service's own `environment:`, a bare name (passed from the shell or
/// `.env`) as None.
pub(crate) fn own_environment(service: &Value) -> IndexMap<String, Option<String>> {
	match service.get("environment") {
		Some(Value::Object(own)) => own.iter().map(|(name, value)| (name.clone(), (!value.is_null()).then(|| text(value)))).collect(),
		Some(Value::Array(items)) => items
			.iter()
			.filter_map(Value::as_str)
			.map(|item| {
				item
					.split_once('=')
					.map_or_else(|| (item.to_owned(), None), |(name, value)| (name.to_owned(), Some(value.to_owned())))
			})
			.collect(),
		_ => IndexMap::new(),
	}
}

/// What a service's own `environment:` sets to a literal value, with no
/// `${...}`: compose's own network, which wins over the deployment file.
pub(crate) fn sets(service: &Value) -> IndexMap<String, String> {
	own_environment(service)
		.into_iter()
		.filter_map(|(name, value)| value.filter(|value| !value.contains('$')).map(|value| (name, value)))
		.collect()
}

/// The lines of every `env_file` of a service but `.env`, read with `read`,
/// a later file's over an earlier one's, and the errors for a required one
/// that is not there.
pub(crate) fn deploy_entries(service: &Value, given: &dyn Fn(&str) -> Option<String>, read: &dyn Fn(&Path) -> Option<String>) -> (Entries, Vec<String>) {
	let mut entries = Entries::new();
	let mut errors = Vec::new();
	for (raw, required) in env_file_paths(service) {
		if raw.ends_with("/.env") {
			continue;
		}
		let path = resolve_path(&raw, given);
		match read(&path) {
			Some(text) => entries.extend(dotenv::parse_entries(&text)),
			None if required => errors.push(format!("{} does not exist, and compose requires it (is DEPLOY right?)", path.display())),
			None => {}
		}
	}
	(entries, errors)
}

/// What the container is given: its deployment file's lines, the env file's
/// when it reads that whole, and its own `environment:` over them.
pub(crate) fn environment_of(service: &Value, entries: &Entries, given: &dyn Fn(&str) -> Option<String>, deployed: &Entries) -> IndexMap<String, String> {
	let value = |entry: &dotenv::Entry| if entry.literal { entry.value.clone() } else { interpolate(&entry.value, given) };
	let mut environment: IndexMap<String, String> = deployed.iter().map(|(name, entry)| (name.clone(), value(entry))).collect();
	if reads_env_file(service) {
		environment.extend(entries.iter().map(|(name, entry)| (name.clone(), value(entry))));
	}
	for (name, own) in own_environment(service) {
		match own {
			None => {
				if let Some(current) = given(&name) {
					environment.insert(name, current);
				}
			}
			Some(own) => {
				environment.insert(name, interpolate(&own, given));
			}
		}
	}
	environment
}

#[cfg(test)]
mod tests {
	use super::{own_paragraph, resolve_path, variables};
	use std::path::PathBuf;

	fn texts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
		pairs.iter().map(|(name, text)| ((*name).to_owned(), (*text).to_owned())).collect()
	}

	#[test]
	fn compose_variables_take_their_help_from_the_comment_that_names_them() {
		let found = variables(&texts(&[
			("a.yml", "# Not about it.\n      - ${ROOT}/x\n      - path: ${ROOT:?set ROOT}/.env\n      - LOG=${LOG:-info}\n      # x\n      - $${NOT_A_VARIABLE}\n      - ${MODELS:-/m}:/models\n"),
			("b.yml", "      # ROOT: the checkout's root.\n      - ${ROOT:?set ROOT}/data\n      - ${BARE?set BARE}\n# - ${COMMENTED_OUT:?no}\n"),
		]));
		assert_eq!(found.keys().collect::<Vec<_>>(), ["ROOT", "LOG", "MODELS", "BARE"]);
		assert!(found["BARE"].help.is_empty() && found["BARE"].message == "set BARE" && !found["BARE"].nonempty);
		assert!(found["ROOT"].required && found["ROOT"].nonempty && found["ROOT"].files == ["a.yml", "b.yml"]);
		assert_eq!(found["ROOT"].help, "ROOT: the checkout's root.");
		assert!(found["MODELS"].default.as_deref() == Some("/m") && !found["MODELS"].required);
		let pair = variables(&texts(&[(
			"c.yml",
			"      # Why.\n      # A: the first.\n      # More of A.\n      # B: the second.\n      - ${A:?a}/${B:?b}\n",
		)]));
		assert_eq!((pair["A"].help.as_str(), pair["B"].help.as_str()), ("A: the first.\nMore of A.", "B: the second."));
		assert_eq!(own_paragraph("no names here", "A"), "no names here");
	}

	#[test]
	fn an_env_file_path_is_opened_where_compose_opens_it() {
		let given = |name: &str| match name {
			"ROOT" => Some("/r".to_owned()),
			"DEPLOY" => Some("dev".to_owned()),
			_ => None,
		};
		assert_eq!(
			resolve_path("/repo/infra/compose/${ROOT:?x}/deploy/${DEPLOY:?y}/probe.env", &given),
			PathBuf::from("/r/deploy/dev/probe.env")
		);
	}
}
