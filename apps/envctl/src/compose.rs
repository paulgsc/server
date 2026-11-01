//! The compose files: the `${...}` variables they declare, and what
//! `docker compose config --no-interpolate` says each service is given.

use crate::dotenv::{self, interpolate, Token};
use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;
use std::{
	path::{Path, PathBuf},
	process::Command,
	sync::LazyLock,
};

static INCLUDE: LazyLock<Regex> = LazyLock::new(|| dotenv::regex(r#"^\s+-\s+(?:path:\s*)?['"]?([^'"\s#]+)['"]?\s*(?:#.*)?$"#));
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
	let mut files = Vec::new();
	let mut pending = vec![compose.to_path_buf()];
	while let Some(path) = pending.pop() {
		if files.contains(&path) {
			continue;
		}
		let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
		let parent = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
		let mut included: Vec<PathBuf> = included(&text).into_iter().map(|file| parent.join(file)).collect();
		included.reverse();
		files.push(path);
		pending.extend(included);
	}
	Ok(files)
}

/// The files a compose file's top-level `include:` names, long form
/// (`- path: x.yml`) or short (`- x.yml`), in order. A `path:` anywhere else
/// (an `env_file`'s) is not an include.
pub(crate) fn included(text: &str) -> Vec<String> {
	let mut inside = false;
	let mut found = Vec::new();
	for line in text.lines() {
		if line.trim().is_empty() || line.trim_start().starts_with('#') {
			continue;
		}
		if !line.starts_with(char::is_whitespace) {
			inside = line.trim_end() == "include:";
			continue;
		}
		if inside {
			if let Some(captures) = INCLUDE.captures(line) {
				found.push(captures[1].to_owned());
			}
		}
	}
	found
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
/// Not interpolated, so it reads with no variable set and no file
/// decrypted, and no value is in what it prints. It needs compose v5: an
/// older one refuses a short-syntax volume holding `${X:?}`.
pub(crate) fn config(repo: &Path) -> Result<Value> {
	let output = Command::new("docker")
		.args(["compose", "--env-file", "/dev/null", "config", "--no-interpolate", "--format", "json"])
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

/// Whether a labelled service reads only its own decrypted file, and
/// nothing else (#614): one `env_file`, `${ENV_DIR...}/<binary>.env`. The
/// problem, when it does not.
pub(crate) fn wired(service: &Value, binary: &str) -> Option<String> {
	let files = env_file_paths(service);
	let own = format!("/{binary}.env");
	match files.as_slice() {
		[(path, true)] if path.contains("${ENV_DIR") && path.ends_with(&own) && !path.contains("/.env") => None,
		[] => Some(format!("reads no env_file; it takes ${{ENV_DIR}}/{binary}.env, its own decrypted file")),
		_ => Some(format!(
			"reads {}; it takes exactly one env_file, required, ${{ENV_DIR}}/{binary}.env",
			files.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>().join(", ")
		)),
	}
}

/// A service's own `environment:`, a bare name (passed from compose's own
/// environment) as None.
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

/// Each published port of a service: what it publishes as written (it may
/// hold a `${...}`), and the container port with its protocol
/// (`11434/tcp`), as docker inspect keys a binding.
pub(crate) fn ports(service: &Value) -> Vec<(String, String)> {
	service
		.get("ports")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|port| match port {
			Value::String(short) => {
				// `[ip:]published:target[/protocol]`; the published part may be a
				// `${X:-d}` holding a colon itself, so the target is the last part.
				let (published, target) = short.rsplit_once(':')?;
				let target = if target.contains('/') { target.to_owned() } else { format!("{target}/tcp") };
				Some((published.to_owned(), target))
			}
			Value::Object(long) => {
				let published = text(long.get("published")?);
				let protocol = long.get("protocol").and_then(Value::as_str).unwrap_or("tcp");
				Some((published, format!("{}/{protocol}", text(long.get("target")?))))
			}
			_ => None,
		})
		.collect()
}

/// The `${NAME}`s a service uses where import cannot read a value back from
/// its container: anywhere but its `environment:`, its volumes' sources, its
/// ports and its `env_file`.
pub(crate) fn unread_uses(service: &Value) -> Vec<String> {
	let Some(fields) = service.as_object() else { return Vec::new() };
	let mut names = Vec::new();
	let mut walk = vec![];
	for (key, value) in fields {
		if !matches!(key.as_str(), "environment" | "volumes" | "ports" | "env_file") {
			walk.push(value);
		}
	}
	while let Some(value) = walk.pop() {
		match value {
			Value::String(text) => names.extend(dotenv::tokens(text).into_iter().filter_map(|token| match token {
				Token::Braced { name, .. } | Token::Bare(name) => Some(name.to_owned()),
				_ => None,
			})),
			Value::Array(items) => walk.extend(items),
			Value::Object(map) => walk.extend(map.values()),
			_ => {}
		}
	}
	names
}

/// Each bind mount of a service, its source as written and its target.
pub(crate) fn volumes(service: &Value) -> Vec<(String, String)> {
	service
		.get("volumes")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|volume| Some((text(volume.get("source")?), text(volume.get("target")?))))
		.collect()
}

/// What a container is given: its decrypted file's lines, then its own
/// `environment:` over them, interpolated from compose's own variables.
pub(crate) fn environment_of(service: &Value, file: &crate::sops::Plain, given: &dyn Fn(&str) -> Option<String>) -> IndexMap<String, String> {
	let mut environment = file.clone();
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
	use super::{included, own_paragraph, variables, wired};
	use serde_json::json;

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
	fn only_the_top_level_include_block_is_included() {
		let text = "include:\n  - path: ./a.yml\n  # - path: ./off.yml\n  - b.yml\n  - path: 'c.yml' # c\nservices:\n  x:\n    env_file:\n      - path: ${ENV_DIR:?x}/x.env\n";
		assert_eq!(included(text), ["./a.yml", "b.yml", "c.yml"]);
		assert!(included("services:\n  x:\n    env_file:\n      - path: ./y.env\n").is_empty());
	}

	#[test]
	fn ports_and_unread_uses_are_found_where_compose_puts_them() {
		let service = json!({
			"ports": ["127.0.0.1:${OLLAMA_HOST_PORT:-11434}:11434", "8080:80/udp", {"published": "${P:-9}", "target": 9000}],
			"build": {"context": "${REPO_ROOT:?x}/infra"},
			"command": ["--flag", "${LEVEL:-info}"],
			"environment": ["A=${B}"],
		});
		assert_eq!(
			super::ports(&service),
			[
				("127.0.0.1:${OLLAMA_HOST_PORT:-11434}".to_owned(), "11434/tcp".to_owned()),
				("8080".to_owned(), "80/udp".to_owned()),
				("${P:-9}".to_owned(), "9000/tcp".to_owned()),
			]
		);
		let mut unread = super::unread_uses(&service);
		unread.sort();
		assert_eq!(unread, ["LEVEL", "REPO_ROOT"], "environment, volumes and ports are read back; the rest is not");
	}

	#[test]
	fn a_labelled_service_reads_only_its_own_decrypted_file() {
		let own = json!({"env_file": [{"path": "/r/infra/compose/${ENV_DIR:?x}/probe.env", "required": true}]});
		assert_eq!(wired(&own, "probe"), None);
		let whole = json!({"env_file": [{"path": "/r/infra/compose/${REPO_ROOT:?x}/.env"}, {"path": "/r/infra/compose/${ENV_DIR:?x}/probe.env"}]});
		assert!(wired(&whole, "probe").unwrap().contains("exactly one"));
		let other = json!({"env_file": [{"path": "${ENV_DIR:?x}/other.env", "required": true}]});
		assert!(wired(&other, "probe").is_some());
		let optional = json!({"env_file": [{"path": "${ENV_DIR:?x}/probe.env", "required": false}]});
		assert!(wired(&optional, "probe").is_some(), "a missing file would start it on nothing");
		assert!(wired(&json!({}), "probe").unwrap().contains("reads no env_file"));
	}
}
