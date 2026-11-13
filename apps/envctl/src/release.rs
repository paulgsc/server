//! A deployment's release (RELEASE R1, #788): every image compose starts,
//! named in one file, `deploy/<role>/release.json`.
//!
//! Each compose service that starts from an image reads it as
//! `${IMAGE_<SERVICE>:?…}`, and envctl gives compose those variables from
//! the release, beside the role's `compose.sops.env`. So `make up`,
//! `make compose` and the deploy all read the one file, and a service cannot
//! be moved on its own by editing its compose line. Image references are not
//! secret, so the release is the one plain file under `deploy/`.
//!
//! What it holds:
//!
//! ```json
//! {"serial": 1, "server_commit": "", "services": {"file-host": {"image": "pgathondu/server:latest"}}}
//! ```
//!
//! `serial` rises with every release. `server_commit` is the commit the
//! release was formed at (filled by CI once it forms releases, R2 #789).

use crate::{compose, sops};
use anyhow::{Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
	sync::LazyLock,
};

/// The release's file name, in `deploy/<role>/`.
pub(crate) const FILE: &str = "release.json";

/// What each service's image variable starts with. Not every compose
/// variable that starts with it is the release's: [`set_by_release`] is.
const PREFIX: &str = "IMAGE_";

/// One `${IMAGE_<SERVICE>:?message}`, the only shape an `image:` may take.
static IMAGE_LINE: LazyLock<Regex> = LazyLock::new(|| crate::dotenv::regex(r"^\$\{(IMAGE_[A-Z0-9_]+):\?[^}]*\}$"));

/// An image reference as the release states it: no space, no variable.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| crate::dotenv::regex(r"^[a-z0-9][a-z0-9._/:-]*(@sha256:[0-9a-f]{64})?$"));

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Release {
	pub(crate) serial: u64,
	#[serde(default)]
	pub(crate) server_commit: String,
	pub(crate) services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Service {
	pub(crate) image: String,
}

/// `deploy/<role>/release.json`, relative to the checkout.
pub(crate) fn relative(role: &str) -> PathBuf {
	Path::new(sops::DEPLOY).join(role).join(FILE)
}

/// The compose variable a service's image is read from: `file-host` reads
/// `IMAGE_FILE_HOST`.
pub(crate) fn variable(service: &str) -> String {
	let mut name = PREFIX.to_owned();
	name.extend(service.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }));
	name
}

/// The compose variables the release sets: the [`variable`] of each service
/// that starts from an image, and no other, as [`Release::variables`] gives
/// compose once [`role_problems`] holds. A bare `IMAGE_` prefix would also
/// take a variable such as `IMAGE_QUALITY` that no release sets, and so would
/// a build-only service's name; the class check would then ask a class for
/// it while every other rule refused the files that could set it (#837).
pub(crate) fn set_by_release(raw: &Value) -> BTreeSet<String> {
	imaged(raw).into_iter().map(|(service, _)| variable(service)).collect()
}

pub(crate) fn parse(text: &str) -> Result<Release> {
	serde_json::from_str(text).context("not a release (serial, server_commit, services: {<service>: {image}})")
}

/// The role's release, None where it has none.
pub(crate) fn read(repo: &Path, role: &str) -> Result<Option<Release>> {
	let path = repo.join(relative(role));
	if !path.exists() {
		return Ok(None);
	}
	let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
	parse(&text).with_context(|| format!("{} is not a release", relative(role).display())).map(Some)
}

impl Release {
	/// What compose is given: each service's `IMAGE_*` and its image.
	pub(crate) fn variables(&self) -> IndexMap<String, String> {
		self.services.iter().map(|(service, entry)| (variable(service), entry.image.clone())).collect()
	}
}

/// The services compose starts from an image, each with its `image:` as
/// `docker compose config --no-interpolate` prints it.
fn imaged(raw: &Value) -> Vec<(&str, &str)> {
	compose::services(raw)
		.into_iter()
		.filter_map(|(name, service)| Some((name, service.get("image")?.as_str()?)))
		.collect()
}

/// Without a key: every `image:` is its own service's `${IMAGE_*:?…}`.
pub(crate) fn line_problems(raw: &Value) -> Vec<String> {
	let mut problems = Vec::new();
	for (service, image) in imaged(raw) {
		let wanted = variable(service);
		let read = IMAGE_LINE.captures(image).and_then(|found| found.get(1)).map(|found| found.as_str());
		if read != Some(wanted.as_str()) {
			problems.push(format!(
				"service {service} starts from `{image}`; its image is `${{{wanted}:?…}}`, which its role's {FILE} decides"
			));
		}
	}
	problems
}

/// One role's release against compose: it names exactly the services that
/// start from an image, each by a plain reference, and `compose.sops.env`
/// sets none of its variables.
pub(crate) fn role_problems(role: &str, raw: &Value, release: Option<&str>, compose_file: Option<&str>) -> Vec<String> {
	let shown = relative(role).display().to_string();
	let mut problems = Vec::new();
	if let Some(text) = compose_file {
		let file = sops::relative(role, sops::COMPOSE).display().to_string();
		let released = set_by_release(raw);
		problems.extend(
			sops::names(text)
				.into_iter()
				.filter(|name| released.contains(name))
				.map(|name| format!("{file} sets {name}, which {shown} sets")),
		);
	}
	let imaged = imaged(raw);
	let release = match release.map(parse) {
		None if compose_file.is_none() || imaged.is_empty() => return problems,
		None => {
			problems.push(format!("{shown} does not exist: {role} runs compose, which starts every image from it"));
			return problems;
		}
		Some(Err(err)) => {
			problems.push(format!("{shown}: {err:#}"));
			return problems;
		}
		Some(Ok(release)) => release,
	};
	for (service, _) in &imaged {
		if !release.services.contains_key(*service) {
			problems.push(format!("{shown} names no image for service {service}"));
		}
	}
	for (service, entry) in &release.services {
		if !imaged.iter().any(|(name, _)| name == service) {
			problems.push(format!("{shown} names {service}, which no compose service starts from an image"));
		}
		if !REFERENCE.is_match(&entry.image) {
			problems.push(format!("{shown} gives {service} `{}`, which is not an image reference", entry.image));
		}
	}
	problems
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn raw() -> Value {
		json!({"services": {
			"file-host": {"image": "${IMAGE_FILE_HOST:?run compose through make}"},
			"node_exporter": {"image": "${IMAGE_NODE_EXPORTER:?x}"},
			"builder": {"build": {"context": "."}}
		}})
	}

	const RELEASE: &str = r#"{"serial": 3, "server_commit": "", "services": {
		"file-host": {"image": "pgathondu/server:latest"},
		"node_exporter": {"image": "prom/node-exporter:v1.8.2@sha256:0000000000000000000000000000000000000000000000000000000000000000"}
	}}"#;

	#[test]
	fn a_service_reads_its_own_variable() {
		assert_eq!(variable("file-host"), "IMAGE_FILE_HOST");
		assert_eq!(variable("node_exporter"), "IMAGE_NODE_EXPORTER");
	}

	#[test]
	fn the_release_sets_each_service_s_variable_and_no_other() {
		let released = set_by_release(&raw());
		assert_eq!(
			released,
			["IMAGE_FILE_HOST", "IMAGE_NODE_EXPORTER"].map(String::from).into(),
			"not the build-only service's"
		);
		assert!(!released.contains("IMAGE_QUALITY") && !released.contains("STORAGE"));
		let release: BTreeSet<String> = parse(RELEASE).unwrap().variables().into_keys().collect();
		assert_eq!(released, release, "what a matching release gives compose");
	}

	#[test]
	fn the_release_gives_compose_each_image() {
		let release = parse(RELEASE).unwrap();
		assert_eq!(release.serial, 3);
		assert_eq!(release.variables().get("IMAGE_FILE_HOST").map(String::as_str), Some("pgathondu/server:latest"));
		assert_eq!(release.variables().len(), 2);
	}

	#[test]
	fn a_matching_release_has_no_problem() {
		assert_eq!(line_problems(&raw()), Vec::<String>::new());
		assert_eq!(role_problems("home", &raw(), Some(RELEASE), Some("")), Vec::<String>::new());
	}

	#[test]
	fn an_image_named_in_compose_is_refused() {
		let raw = json!({"services": {
			"redis": {"image": "redis:latest"},
			"render": {"image": "${IMAGE_KOKORO:?x}"}
		}});
		let found = line_problems(&raw);
		assert_eq!(found.len(), 2, "{found:?}");
		assert!(
			found[0].contains("service redis starts from `redis:latest`") && found[0].contains("IMAGE_REDIS"),
			"{found:?}"
		);
		assert!(found[1].contains("service render") && found[1].contains("IMAGE_RENDER"), "{found:?}");
	}

	#[test]
	fn a_release_that_misses_or_adds_a_service_is_refused() {
		let release = r#"{"serial": 1, "services": {"file-host": {"image": "pgathondu/server:latest"}, "ghost": {"image": "ghost:1"}}}"#;
		let found = role_problems("home", &raw(), Some(release), Some(""));
		assert_eq!(
			found,
			vec![
				"deploy/home/release.json names no image for service node_exporter".to_owned(),
				"deploy/home/release.json names ghost, which no compose service starts from an image".to_owned(),
			]
		);
	}

	#[test]
	fn a_reference_with_a_space_or_a_variable_is_refused() {
		for image in ["pgathondu/server :latest", "${OTHER}", "", "repo@sha256:short"] {
			let release = format!(r#"{{"serial": 1, "services": {{"file-host": {{"image": "{image}"}}, "node_exporter": {{"image": "a:1"}}}}}}"#);
			let found = role_problems("home", &raw(), Some(&release), Some(""));
			assert_eq!(found.len(), 1, "{image}: {found:?}");
			assert!(found[0].contains("not an image reference"), "{found:?}");
		}
	}

	#[test]
	fn a_role_running_compose_needs_its_release() {
		let found = role_problems("home", &raw(), None, Some(""));
		assert_eq!(
			found,
			vec!["deploy/home/release.json does not exist: home runs compose, which starts every image from it".to_owned()]
		);
		assert_eq!(role_problems("dev", &raw(), None, None), Vec::<String>::new(), "a role that runs no compose needs none");
	}

	#[test]
	fn an_unknown_field_or_bad_json_is_named() {
		let found = role_problems("home", &raw(), Some(r#"{"serial": 1, "services": {}, "extra": 1}"#), Some(""));
		assert_eq!(found.len(), 1);
		assert!(found[0].starts_with("deploy/home/release.json: not a release"), "{found:?}");
	}

	#[test]
	fn the_deployment_file_never_sets_an_image() {
		let compose = "IMAGE_FILE_HOST=ENC[AES256_GCM,data:x,type:str]\nSTORAGE=ENC[AES256_GCM,data:y,type:str]\nsops_version=3.9.0\n";
		let found = role_problems("home", &raw(), Some(RELEASE), Some(compose));
		assert_eq!(
			found,
			vec!["deploy/home/compose.sops.env sets IMAGE_FILE_HOST, which deploy/home/release.json sets".to_owned()]
		);
	}

	/// An `IMAGE_` name no service's image is read from is the deployment
	/// file's to set, like any other compose variable (#837).
	#[test]
	fn the_deployment_file_may_set_a_stray_image_name() {
		let compose = "IMAGE_QUALITY=ENC[AES256_GCM,data:x,type:str]\nsops_version=3.9.0\n";
		assert_eq!(role_problems("home", &raw(), Some(RELEASE), Some(compose)), Vec::<String>::new());
	}

	/// The checkout's own: every compose image is the release's, and the
	/// release names each one (RELEASE R1, #788).
	#[test]
	fn the_checkout_reads_every_image_from_home_s_release() {
		let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let release = read(&repo, "home").unwrap().unwrap();
		let mut lines = Vec::new();
		for file in compose::files(&repo.join("docker-compose.yml")).unwrap() {
			let text = std::fs::read_to_string(&file).unwrap();
			let mut service = None;
			let mut in_services = false;
			for line in text.lines() {
				if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() && !line.starts_with('#') {
					in_services = line.trim_end() == "services:";
					continue;
				}
				if !in_services {
					continue;
				}
				if let Some(name) = line.strip_prefix("  ").and_then(|rest| rest.strip_suffix(':')).filter(|name| !name.starts_with([' ', '#'])) {
					service = Some(name.to_owned());
				} else if let (Some(image), Some(service)) = (line.strip_prefix("    image: "), &service) {
					lines.push((service.clone(), image.trim().to_owned()));
				}
			}
		}
		assert!(lines.len() > 10, "the compose files were read: {lines:?}");
		let raw = json!({"services": lines.iter().map(|(service, image)| (service.clone(), json!({"image": image}))).collect::<serde_json::Map<_, _>>()});
		assert_eq!(line_problems(&raw), Vec::<String>::new());
		let text = std::fs::read_to_string(repo.join(relative("home"))).unwrap();
		assert_eq!(role_problems("home", &raw, Some(&text), Some("")), Vec::<String>::new());
		assert_eq!(release.services.len(), lines.len());
	}
}
