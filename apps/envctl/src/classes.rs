//! ENV-CLASS (#617): the environment holds deployment constants only, and
//! every variable says which it is.
//!
//! Each binary's schema carries every variable's class, which its `Config`
//! declares (`app_config::schema::Class`; `cargo test` refuses one with
//! none), and [`FILE`]'s `compose` carries those of the `${...}` variables
//! compose reads, but `make`'s own ([`FROM_MAKE`]) and the release's images
//! (`deploy/<role>/release.json`, docs/release.md). A variable of any class but
//! [`Class::Deployment`] is runtime policy or a source constant read from the
//! environment: it fails here, unless [`FILE`] lists it as moving, with the
//! issue that moves it. A `bool` that stays a deployment constant is where
//! policy has hidden before (`RUST_ROUNDS_ENABLED`, `GAME_FEED_ENABLED`), so
//! it fails too, unless [`FILE`] lists it as a deployment switch.
//!
//! [`FILE`] only shrinks: an entry whose variable has moved fails until it is
//! dropped, and against the base branch's copy (`--base-classes`, which CI
//! passes) an entry that names a variable the base did not list fails. So a
//! new policy value cannot land as env, and an old one cannot stay once its
//! lever lands, both at the PR.

use crate::{compose::ComposeVariables, preflight::FROM_MAKE, schema::Class, schema::Kind, schema::Schema};
use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path, sync::LazyLock};

/// The exceptions, relative to the checkout.
pub(crate) const FILE: &str = "infra/env-classes.json";

/// The app name compose's own `${...}` variables are listed under.
pub(crate) const COMPOSE_APP: &str = "compose";

static ISSUE: LazyLock<Regex> = LazyLock::new(|| crate::dotenv::regex(r"^[\w.-]+/[\w.-]+#\d+$"));

/// [`FILE`], as checked in.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Exceptions {
	/// What the file is, for whoever opens it; read by no one here.
	#[serde(rename = "$comment", default)]
	_comment: Vec<String>,
	/// The class of each variable compose reads from its role's
	/// `compose.sops.env`, which no `Config` declares.
	pub(crate) compose: Vec<ComposeClass>,
	/// A variable of another class still read from the environment, on its
	/// way out.
	pub(crate) moving: Vec<Moving>,
	/// A `bool` that is a deployment constant.
	pub(crate) switches: Vec<Switch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComposeClass {
	pub(crate) variable: String,
	pub(crate) class: Class,
	pub(crate) why: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Moving {
	pub(crate) app: String,
	pub(crate) variable: String,
	pub(crate) class: Class,
	/// The issue whose change removes it from the environment, as
	/// `owner/repo#number`.
	pub(crate) issue: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Switch {
	pub(crate) app: String,
	pub(crate) variable: String,
	/// Where the owner confirmed it a deployment constant, or will.
	pub(crate) issue: String,
}

impl Exceptions {
	pub(crate) fn parse(text: &str) -> Result<Self> {
		serde_json::from_str(text).with_context(|| format!("{FILE} is not {{\"compose\": [...], \"moving\": [...], \"switches\": [...]}}"))
	}

	pub(crate) fn load(repo: &Path) -> Result<Self> {
		let path = repo.join(FILE);
		Self::parse(&std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?)
	}

	/// Every `(app, variable)` either list names.
	fn names(&self) -> BTreeSet<(&str, &str)> {
		self
			.moving
			.iter()
			.map(|entry| (entry.app.as_str(), entry.variable.as_str()))
			.chain(self.switches.iter().map(|entry| (entry.app.as_str(), entry.variable.as_str())))
			.collect()
	}
}

/// One variable the stack reads, with its class.
struct Read<'a> {
	app: &'a str,
	variable: &'a str,
	class: Class,
	switch: bool,
}

impl Read<'_> {
	fn is(&self, app: &str, variable: &str) -> bool {
		self.app == app && self.variable == variable
	}
}

/// Every variable that is not a deployment constant and is not moving, every
/// deployment `bool` not listed as a switch, every compose variable with no
/// class, every exception that no longer holds, and, given the base branch's
/// copy, every exception it did not have; each by app and name.
///
/// `images` are the release's variables, one per compose service
/// (`release::variable`), which `release.json` sets and nothing classes.
pub(crate) fn problems(
	schemas: &[Schema],
	compose_variables: &ComposeVariables,
	images: &BTreeSet<String>,
	exceptions: &Exceptions,
	base: Option<&Exceptions>,
) -> Vec<String> {
	let mut problems = Vec::new();
	let read = read(schemas, compose_variables, images, exceptions, &mut problems);
	problems.extend(read.iter().filter_map(|variable| class_problem(variable, exceptions)));
	problems.extend(entry_problems(&read, exceptions, base));
	problems
}

/// Every variable the schemas and compose read, but `make`'s and the
/// release's; a compose variable with no class, or a class for one no
/// compose file reads, goes to `problems`.
fn read<'a>(
	schemas: &'a [Schema],
	compose_variables: &'a ComposeVariables,
	images: &BTreeSet<String>,
	exceptions: &'a Exceptions,
	problems: &mut Vec<String>,
) -> Vec<Read<'a>> {
	let mut read: Vec<Read> = schemas
		.iter()
		.flat_map(|schema| {
			schema.variables.iter().map(|variable| Read {
				app: &schema.binary,
				variable: &variable.name,
				class: variable.class,
				switch: variable.kind == Kind::Bool,
			})
		})
		.collect();
	for name in compose_variables.keys() {
		if FROM_MAKE.contains(&name.as_str()) || images.contains(name) {
			continue;
		}
		match exceptions.compose.iter().find(|known| &known.variable == name) {
			Some(known) => read.push(Read {
				app: COMPOSE_APP,
				variable: name,
				class: known.class,
				// Compose's variables have no kind: a name that reads as a
				// switch is held as one.
				switch: name.ends_with("_ENABLED"),
			}),
			None => problems.push(format!("compose reads {name}, which has no class: add it to {FILE}'s compose, with why (ENV-CLASS)")),
		}
	}
	for known in &exceptions.compose {
		if !compose_variables.contains_key(&known.variable) {
			problems.push(format!("{FILE} classes compose's {}, which no compose file reads: drop it", known.variable));
		} else if known.why.trim().is_empty() {
			problems.push(format!("{FILE} gives compose's {} no reason for its class", known.variable));
		}
	}
	read
}

/// What is wrong with `variable` being read from the environment, if
/// anything.
fn class_problem(variable: &Read, exceptions: &Exceptions) -> Option<String> {
	let moving = exceptions.moving.iter().find(|entry| variable.is(&entry.app, &entry.variable));
	let switch = exceptions.switches.iter().any(|entry| variable.is(&entry.app, &entry.variable));
	match (variable.class, moving) {
		(Class::Deployment, _) if variable.switch && !switch => Some(format!(
			"{} {} is a bool classed deployment: a switch is runtime policy unless the owner confirms it a fact of the deployment ({FILE}'s switches)",
			variable.app, variable.variable
		)),
		(class, None) if class != Class::Deployment => Some(format!(
			"{} {} is classed {}, and the environment holds deployment constants only: make it {} (ENV-CLASS, README.md, \"Configuration\")",
			variable.app,
			variable.variable,
			class.name(),
			if class == Class::Policy { "a lever (file_host's settings)" } else { "a const" }
		)),
		(class, Some(entry)) if class != Class::Deployment && entry.class != class => Some(format!(
			"{FILE} lists {} {} as moving {}, and it is classed {}",
			variable.app,
			variable.variable,
			entry.class.name(),
			class.name()
		)),
		_ => None,
	}
}

/// Every exception that no longer holds, is listed twice or names no issue,
/// and, given the base branch's copy, every one it did not have.
fn entry_problems(read: &[Read], exceptions: &Exceptions, base: Option<&Exceptions>) -> Vec<String> {
	let mut problems = Vec::new();
	let held = |app: &str, name: &str, wanted: &dyn Fn(&Read) -> bool| read.iter().any(|variable| variable.is(app, name) && wanted(variable));
	for entry in &exceptions.moving {
		if !held(&entry.app, &entry.variable, &|variable| variable.class != Class::Deployment) {
			problems.push(format!(
				"{FILE} lists {} {} as moving, and it is no longer read or no longer classed {}: drop the entry",
				entry.app,
				entry.variable,
				entry.class.name()
			));
		}
	}
	for entry in &exceptions.switches {
		if !held(&entry.app, &entry.variable, &|variable| variable.switch && variable.class == Class::Deployment) {
			problems.push(format!(
				"{FILE} lists {} {} as a deployment switch, and it is no longer a bool classed deployment: drop the entry",
				entry.app, entry.variable
			));
		}
	}
	let mut seen = BTreeSet::new();
	for (app, variable, issue) in exceptions
		.moving
		.iter()
		.map(|entry| (&entry.app, &entry.variable, &entry.issue))
		.chain(exceptions.switches.iter().map(|entry| (&entry.app, &entry.variable, &entry.issue)))
	{
		if !seen.insert((app, variable)) {
			problems.push(format!("{FILE} lists {app} {variable} twice"));
		}
		if !ISSUE.is_match(issue) {
			problems.push(format!("{FILE}'s {app} {variable} names no issue as owner/repo#number"));
		}
	}
	if let Some(base) = base {
		let before = base.names();
		for (app, variable) in exceptions.names() {
			if !before.contains(&(app, variable)) {
				problems.push(format!(
					"{FILE} lists {app} {variable}, which the base branch's does not: the exceptions only shrink, so a new variable is a deployment constant or is not env"
				));
			}
		}
	}
	problems
}

#[cfg(test)]
// `${...}` is compose's syntax under test here, not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
mod tests {
	use super::{problems, Exceptions};
	use crate::{
		compose::variables,
		schema::{
			fixture::{schema, var},
			Class, Kind, Variable,
		},
	};
	use std::{collections::BTreeSet, path::Path};

	fn images() -> BTreeSet<String> {
		BTreeSet::from(["IMAGE_X".to_owned()])
	}

	fn classed(name: &str, kind: Kind, class: Class) -> Variable {
		Variable { class, ..var(name, kind) }
	}

	fn exceptions(text: &str) -> Exceptions {
		Exceptions::parse(text).unwrap()
	}

	const NONE: &str = r#"{"compose": [], "moving": [], "switches": []}"#;

	#[test]
	fn only_a_deployment_constant_is_env() {
		let probe = schema(
			"probe",
			vec![
				classed("URL", Kind::String, Class::Deployment),
				classed("LIMIT", Kind::U32, Class::Policy),
				classed("TICK", Kind::U64, Class::Source),
			],
		);
		assert_eq!(
			problems(&[probe], &variables(&[]), &BTreeSet::new(), &exceptions(NONE), None),
			[
				"probe LIMIT is classed policy, and the environment holds deployment constants only: make it a lever (file_host's settings) (ENV-CLASS, README.md, \"Configuration\")",
				"probe TICK is classed source, and the environment holds deployment constants only: make it a const (ENV-CLASS, README.md, \"Configuration\")",
			]
		);
	}

	#[test]
	fn a_moving_variable_waits_on_its_issue_and_its_entry_goes_when_it_moves() {
		let listed = exceptions(r#"{"compose": [], "moving": [{"app": "probe", "variable": "LIMIT", "class": "policy", "issue": "o/r#1"}], "switches": []}"#);
		let reading = schema("probe", vec![classed("LIMIT", Kind::U32, Class::Policy)]);
		assert!(problems(&[reading], &variables(&[]), &BTreeSet::new(), &listed, None).is_empty());
		let moved = schema("probe", vec![]);
		assert_eq!(
			problems(&[moved], &variables(&[]), &BTreeSet::new(), &listed, None),
			["infra/env-classes.json lists probe LIMIT as moving, and it is no longer read or no longer classed policy: drop the entry"]
		);
		let reclassed = schema("probe", vec![classed("LIMIT", Kind::U32, Class::Source)]);
		assert_eq!(
			problems(&[reclassed], &variables(&[]), &BTreeSet::new(), &listed, None),
			["infra/env-classes.json lists probe LIMIT as moving policy, and it is classed source"]
		);
	}

	#[test]
	fn a_deployment_bool_is_a_switch_the_owner_confirmed() {
		let probe = schema("probe", vec![classed("ON", Kind::Bool, Class::Deployment)]);
		assert_eq!(
			problems(std::slice::from_ref(&probe), &variables(&[]), &BTreeSet::new(), &exceptions(NONE), None),
			["probe ON is a bool classed deployment: a switch is runtime policy unless the owner confirms it a fact of the deployment (infra/env-classes.json's switches)"]
		);
		let confirmed = exceptions(r#"{"compose": [], "moving": [], "switches": [{"app": "probe", "variable": "ON", "issue": "o/r#2"}]}"#);
		assert!(problems(&[probe], &variables(&[]), &BTreeSet::new(), &confirmed, None).is_empty());
		let gone = schema("probe", vec![classed("ON", Kind::Bool, Class::Policy)]);
		let found = problems(&[gone], &variables(&[]), &BTreeSet::new(), &confirmed, None);
		assert!(found.iter().any(|problem| problem.contains("no longer a bool classed deployment")), "{found:?}");
	}

	#[test]
	fn the_exceptions_only_shrink_against_the_base_branch() {
		let probe = schema("probe", vec![classed("LIMIT", Kind::U32, Class::Policy), classed("ZONE", Kind::String, Class::Policy)]);
		let base = exceptions(r#"{"compose": [], "moving": [{"app": "probe", "variable": "LIMIT", "class": "source", "issue": "o/r#1"}], "switches": []}"#);
		let head = exceptions(
			r#"{"compose": [], "moving": [{"app": "probe", "variable": "LIMIT", "class": "policy", "issue": "o/r#1"}, {"app": "probe", "variable": "ZONE", "class": "policy", "issue": "o/r#3"}], "switches": []}"#,
		);
		assert_eq!(
			problems(&[probe], &variables(&[]), &BTreeSet::new(), &head, Some(&base)),
			["infra/env-classes.json lists probe ZONE, which the base branch's does not: the exceptions only shrink, so a new variable is a deployment constant or is not env"],
			"a reclassed entry may stay; a new one may not"
		);
	}

	#[test]
	fn an_entry_names_its_issue_once() {
		let probe = schema("probe", vec![classed("LIMIT", Kind::U32, Class::Policy)]);
		let listed = exceptions(
			r#"{"compose": [], "moving": [{"app": "probe", "variable": "LIMIT", "class": "policy", "issue": "soon"}, {"app": "probe", "variable": "LIMIT", "class": "policy", "issue": "o/r#1"}], "switches": []}"#,
		);
		assert_eq!(
			problems(&[probe], &variables(&[]), &BTreeSet::new(), &listed, None),
			[
				"infra/env-classes.json's probe LIMIT names no issue as owner/repo#number",
				"infra/env-classes.json lists probe LIMIT twice"
			]
		);
	}

	#[test]
	fn every_compose_variable_has_a_class_but_make_s_and_the_release_s() {
		let read = variables(&[(
			"a.yml".to_owned(),
			"      - ${STORAGE:?s}:/data\n      - ${REPO_ROOT:?r}:/r\n    image: ${IMAGE_X:?i}\n      - ${NEW_PATH:?n}:/n\n".to_owned(),
		)]);
		let classed = exceptions(
			r#"{"compose": [{"variable": "STORAGE", "class": "deployment", "why": "A path."}, {"variable": "GONE", "class": "deployment", "why": "Was read."}, {"variable": "NEW_PATH", "class": "policy", "why": "x"}], "moving": [], "switches": []}"#,
		);
		assert_eq!(
			problems(&[], &read, &images(), &classed, None),
			[
				"infra/env-classes.json classes compose's GONE, which no compose file reads: drop it",
				"compose NEW_PATH is classed policy, and the environment holds deployment constants only: make it a lever (file_host's settings) (ENV-CLASS, README.md, \"Configuration\")",
			],
			"make's REPO_ROOT and the release's IMAGE_X need none"
		);
		assert_eq!(
			problems(&[], &read, &images(), &exceptions(NONE), None),
			[
				"compose reads STORAGE, which has no class: add it to infra/env-classes.json's compose, with why (ENV-CLASS)",
				"compose reads NEW_PATH, which has no class: add it to infra/env-classes.json's compose, with why (ENV-CLASS)",
			]
		);
	}

	/// The review of #836: compose's variables have no kind, so a name that
	/// reads as a switch is held to the bool guard, and only the release's
	/// own `IMAGE_*` go unclassed.
	#[test]
	fn a_compose_switch_and_a_stray_image_variable_are_held() {
		let read = variables(&[(
			"a.yml".to_owned(),
			"    environment:\n      - X=${FEED_ENABLED:-false}\n      - Q=${IMAGE_QUALITY:-hi}\n".to_owned(),
		)]);
		let classed = exceptions(r#"{"compose": [{"variable": "FEED_ENABLED", "class": "deployment", "why": "x"}], "moving": [], "switches": []}"#);
		assert_eq!(
			problems(&[], &read, &images(), &classed, None),
			[
				"compose reads IMAGE_QUALITY, which has no class: add it to infra/env-classes.json's compose, with why (ENV-CLASS)",
				"compose FEED_ENABLED is a bool classed deployment: a switch is runtime policy unless the owner confirms it a fact of the deployment (infra/env-classes.json's switches)",
			]
		);
	}

	/// The checked-in file reads, and the checked-in schemas and compose
	/// files conform to it (CI's `envctl check` runs the same, with the base
	/// branch's copy).
	#[test]
	fn the_checkout_conforms() {
		let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let schemas = crate::schema::load(&repo.join("infra/env")).unwrap();
		let compose_variables = crate::compose::read(&repo).unwrap();
		// The release names every service that starts from an image.
		let release = crate::release::read(&repo, "home").unwrap().unwrap();
		let images = release.variables().into_keys().collect();
		let found = problems(&schemas, &compose_variables, &images, &Exceptions::load(&repo).unwrap(), None);
		assert!(found.is_empty(), "{found:#?}");
	}
}
