//! `infra/env/<binary>.json`: each binary's clap `Config`, as its tests check
//! it in (`app_config::schema`, CFG7 #503). This is the contract every
//! subcommand reads (#613): names, kinds and rules, never a deployment's
//! values. No list of variables is written anywhere else.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// The order binaries are reported in; any other follows, by name.
const ORDER: [&str; 3] = ["file_host", "orchestrator", "some-obs"];

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Schema {
	pub(crate) binary: String,
	pub(crate) about: String,
	pub(crate) variables: Vec<Variable>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Variable {
	pub(crate) name: String,
	pub(crate) group: Option<String>,
	pub(crate) required: bool,
	pub(crate) default: Option<String>,
	pub(crate) kind: Kind,
	pub(crate) values: Vec<String>,
	pub(crate) delimiter: Option<char>,
	/// `hide_env_values`: `--help` shows no value. Every value is a secret
	/// here (#613), so only the NixOS options read this.
	pub(crate) secret: bool,
	pub(crate) help: String,
	pub(crate) required_when: Option<Condition>,
	pub(crate) bounds: Option<Bounds>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
	Bool,
	U16,
	U32,
	U64,
	Usize,
	Number,
	Seconds,
	SocketAddr,
	Enum,
	Path,
	String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Condition {
	pub(crate) variable: String,
	pub(crate) value: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bounds {
	pub(crate) min: Option<f64>,
	pub(crate) max: Option<f64>,
	pub(crate) when: Vec<Condition>,
}

/// `When::is` for "set to anything but nothing".
pub(crate) const SET: &str = "*";

impl Schema {
	pub(crate) fn variable(&self, name: &str) -> Option<&Variable> {
		self.variables.iter().find(|variable| variable.name == name)
	}
}

/// Every schema in `directory`, in [`ORDER`].
pub(crate) fn load(directory: &Path) -> Result<Vec<Schema>> {
	let mut schemas = Vec::new();
	for entry in std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))? {
		let path = entry?.path();
		if path.extension().is_some_and(|extension| extension == "json") {
			let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
			schemas.push(serde_json::from_str::<Schema>(&text).with_context(|| format!("{} is not a schema app_config::schema writes", path.display()))?);
		}
	}
	let rank = |binary: &str| ORDER.iter().position(|known| *known == binary).unwrap_or(ORDER.len());
	schemas.sort_by(|a, b| (rank(&a.binary), &a.binary).cmp(&(rank(&b.binary), &b.binary)));
	Ok(schemas)
}

#[cfg(test)]
pub(crate) mod fixture {
	//! Schemas for the other modules' tests.
	use super::{Bounds, Condition, Kind, Schema, Variable};

	pub(crate) fn var(name: &str, kind: Kind) -> Variable {
		Variable {
			name: name.to_owned(),
			group: None,
			required: false,
			default: None,
			kind,
			values: Vec::new(),
			delimiter: None,
			secret: false,
			help: format!("{name}'s help."),
			required_when: None,
			bounds: None,
		}
	}

	pub(crate) fn when(variable: &str, value: &str) -> Condition {
		Condition {
			variable: variable.to_owned(),
			value: value.to_owned(),
		}
	}

	pub(crate) fn bounds(min: Option<f64>, max: Option<f64>, conditions: &[(&str, &str)]) -> Bounds {
		Bounds {
			min,
			max,
			when: conditions.iter().map(|(variable, value)| when(variable, value)).collect(),
		}
	}

	pub(crate) fn schema(binary: &str, variables: Vec<Variable>) -> Schema {
		Schema {
			binary: binary.to_owned(),
			about: String::new(),
			variables,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::load;
	use std::path::Path;

	/// Every checked-in schema reads, field for field: a field
	/// `app_config::schema` adds fails here until envctl knows it.
	#[test]
	fn every_checked_in_schema_reads() {
		let schemas = load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../infra/env")).unwrap();
		assert_eq!(
			schemas.iter().map(|schema| schema.binary.as_str()).collect::<Vec<_>>(),
			["file_host", "orchestrator", "some-obs"]
		);
	}
}
