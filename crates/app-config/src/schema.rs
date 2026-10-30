//! Each binary's configuration, readable without starting it (CFG7, #503).
//!
//! [`variables`] walks a clap `Command` and describes every argument that has
//! an environment variable, so there is no second declaration: the `Config`
//! is the schema. Each binary checks a copy into `infra/env/<binary>.json`
//! with [`assert_checked_in`], and `scripts/env_schema.py` generates
//! `example.env`, lists the stack's variables and runs the preflight from
//! those copies, so none of that needs Rust on the host.
//!
//! A requirement that depends on another variable (`WEBAUTHN_RP_ID` while
//! `AUTH_ENABLED` is on) is checked by the code that refuses to start without
//! it, and clap cannot see it; the binary declares it beside its `Config` as
//! a [`RequiredWhen`], so the schema and the preflight know it too.
//!
//! A variable is a secret when its `Config` hides its value
//! (`hide_env_values = true`): `--help` shows no value for it, and nothing
//! generated from the schema does either.

use clap::{builder::ValueParser, Arg, ArgAction, Command};
use serde::Serialize;
use std::{
	any::TypeId,
	net::SocketAddr,
	path::{Path, PathBuf},
	time::Duration,
};

/// The variable that makes [`assert_checked_in`] write the schema instead of
/// comparing it (`make env-schema`).
pub const UPDATE_VARIABLE: &str = "UPDATE_ENV_SCHEMA";

/// One binary's schema, as checked in.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Schema {
	pub binary: String,
	pub about: String,
	pub variables: Vec<Variable>,
}

/// One environment variable a binary reads.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Variable {
	pub name: String,
	/// The `--help` heading it is listed under, if any.
	pub group: Option<String>,
	/// Whether the binary refuses to start without it. One that is required
	/// only when another is set (`WEBAUTHN_RP_ID` with `AUTH_ENABLED`) is
	/// checked by the binary at start and says so in its help.
	pub required: bool,
	/// The value it takes when unset. Never given for a secret.
	pub default: Option<String>,
	pub kind: Kind,
	/// The values an `enum` takes.
	pub values: Vec<String>,
	/// The separator of a variable that holds a list.
	pub delimiter: Option<char>,
	pub secret: bool,
	pub help: String,
	/// Required, set and not empty, while another variable has a value.
	pub required_when: Option<Condition>,
}

/// One variable a binary requires only while another has a value.
#[derive(Debug, Clone, Copy)]
pub struct RequiredWhen {
	pub variable: &'static str,
	pub when: &'static str,
	pub is: &'static str,
}

/// [`RequiredWhen`]'s condition, as checked in.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Condition {
	pub variable: String,
	pub value: String,
}

/// What a value must parse as.
///
/// A custom parser is named by what it returns, which is all clap exposes,
/// so its own limits (a ratio from 0 to 1, an interval of at least 1) are in
/// its help and checked by the binary.
#[derive(Debug, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
	Bool,
	U16,
	U32,
	U64,
	Usize,
	Number,
	/// Whole seconds.
	Seconds,
	SocketAddr,
	Enum,
	Path,
	String,
}

/// The schema of the binary `cmd` describes, with the requirements it
/// declares beside its `Config`.
///
/// # Panics
///
/// When an argument's parser returns a type [`Kind`] does not name (map it
/// here deliberately rather than describe it wrongly), or a requirement names
/// a variable `cmd` does not declare.
#[must_use]
pub fn schema(binary: &str, cmd: &Command, required_when: &[RequiredWhen]) -> Schema {
	let mut variables = variables(cmd);
	for rule in required_when {
		assert!(
			variables.iter().any(|variable| variable.name == rule.when),
			"{} is required when {}, which {binary} does not declare",
			rule.variable,
			rule.when
		);
		let Some(variable) = variables.iter_mut().find(|variable| variable.name == rule.variable) else {
			panic!("{binary} declares no {} to require when {}", rule.variable, rule.when)
		};
		variable.required_when = Some(Condition {
			variable: rule.when.to_owned(),
			value: rule.is.to_owned(),
		});
	}
	Schema {
		binary: binary.to_owned(),
		about: cmd.get_about().map(ToString::to_string).unwrap_or_default(),
		variables,
	}
}

/// Every argument of `cmd` that has an environment variable, in declaration
/// order.
///
/// # Panics
///
/// As [`schema`].
#[must_use]
pub fn variables(cmd: &Command) -> Vec<Variable> {
	cmd
		.get_arguments()
		.filter_map(|arg| arg.get_env().and_then(std::ffi::OsStr::to_str).map(|name| variable(name, arg)))
		.collect()
}

fn variable(name: &str, arg: &Arg) -> Variable {
	let secret = arg.is_hide_env_values_set();
	let delimiter = arg.get_value_delimiter();
	let defaults: Vec<String> = arg.get_default_values().iter().map(|value| value.to_string_lossy().into_owned()).collect();
	let separator = delimiter.map(String::from).unwrap_or_default();
	let default = (!secret && !arg.is_hide_default_value_set() && !defaults.is_empty()).then(|| defaults.join(separator.as_str()));
	let kind = kind(name, arg);
	let values = if kind == Kind::Enum {
		arg.get_possible_values().iter().map(|value| value.get_name().to_owned()).collect()
	} else {
		Vec::new()
	};
	let help = arg.get_long_help().or_else(|| arg.get_help()).map(ToString::to_string).unwrap_or_default();
	Variable {
		name: name.to_owned(),
		group: arg.get_help_heading().map(str::to_owned),
		required: arg.is_required_set(),
		default,
		kind,
		values,
		delimiter,
		secret,
		help,
		required_when: None,
	}
}

fn kind(name: &str, arg: &Arg) -> Kind {
	let parser: &ValueParser = arg.get_value_parser();
	let id = parser.type_id();
	if matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) || id == TypeId::of::<bool>() {
		return Kind::Bool;
	}
	if !arg.get_possible_values().is_empty() {
		return Kind::Enum;
	}
	let kinds = [
		(TypeId::of::<u16>(), Kind::U16),
		(TypeId::of::<u32>(), Kind::U32),
		(TypeId::of::<u64>(), Kind::U64),
		(TypeId::of::<usize>(), Kind::Usize),
		(TypeId::of::<f64>(), Kind::Number),
		(TypeId::of::<Duration>(), Kind::Seconds),
		(TypeId::of::<SocketAddr>(), Kind::SocketAddr),
		(TypeId::of::<PathBuf>(), Kind::Path),
		(TypeId::of::<String>(), Kind::String),
	];
	let Some((_, kind)) = kinds.into_iter().find(|(type_id, _)| id == *type_id) else {
		panic!("{name}'s parser returns a type app_config::schema::Kind does not name; add it there")
	};
	kind
}

/// Renders `schema` as the JSON checked in, ending in a newline.
///
/// # Panics
///
/// Never: the schema is plain data.
#[must_use]
pub fn render(schema: &Schema) -> Vec<u8> {
	let mut out = Vec::new();
	let Ok(()) = serde_json::to_writer_pretty(&mut out, schema) else {
		unreachable!("a Schema always serializes")
	};
	out.push(b'\n');
	out
}

/// Holds the copy of `binary`'s schema checked in at `path` to its `Config`,
/// `cmd`, and its `required_when`. With [`UPDATE_VARIABLE`] set, writes it
/// instead.
///
/// # Panics
///
/// When the copy differs from the `Config`, or cannot be read or written; it
/// is meant for a test.
#[allow(clippy::disallowed_methods)] // A test's own switch, as INSTA_UPDATE is, not configuration (#484).
pub fn assert_checked_in(binary: &str, cmd: &Command, required_when: &[RequiredWhen], path: &Path) {
	let rendered = render(&schema(binary, cmd, required_when));
	if std::env::var_os(UPDATE_VARIABLE).is_some() {
		if let Err(err) = std::fs::write(path, &rendered) {
			panic!("writing {}: {err}", path.display());
		}
		return;
	}
	let checked_in = std::fs::read(path).unwrap_or_default();
	assert!(
		checked_in == rendered,
		"{} is not {binary}'s Config; run `make env-schema` and commit what it writes\n--- {binary}'s Config:\n{}",
		path.display(),
		String::from_utf8_lossy(&rendered)
	);
}

/// Holds each shared group's `--help` heading to its own variables.
///
/// clap's derive never restores the heading after a `#[command(flatten)]`,
/// so a field declared after a group, without a `help_heading` of its own,
/// silently lands under that group's heading in `--help`, in the schema and
/// in `example.env`.
///
/// # Panics
///
/// When a variable outside a group carries that group's heading; it is meant
/// for a test.
pub fn assert_groups_hold_only_their_own(cmd: &Command) {
	use clap::Args;
	let groups = [
		crate::NatsArgs::augment_args(Command::new("nats")),
		crate::LogArgs::augment_args(Command::new("log")),
		crate::OtelArgs::augment_args(Command::new("otel")),
	];
	for group in groups {
		let own: Vec<&str> = group.get_arguments().filter_map(|arg| arg.get_env().and_then(std::ffi::OsStr::to_str)).collect();
		let Some(heading) = group.get_arguments().find_map(Arg::get_help_heading) else {
			continue;
		};
		for arg in cmd.get_arguments() {
			let Some(name) = arg.get_env().and_then(std::ffi::OsStr::to_str) else { continue };
			assert!(
				arg.get_help_heading() != Some(heading) || own.contains(&name),
				"{name} is listed under {heading} because it follows that group without a help_heading of its own"
			);
		}
	}
}
