//! What compose may mount from the tree a release is read from (RL10,
//! docs/release.md; C344).
//!
//! Home's deploy runs compose on a release's worktree (R4, #793), where
//! `REPO_ROOT` is that worktree: the files git holds at the release's
//! commit, and nothing else. So a `${REPO_ROOT}/…` bind mount is a tracked
//! path, mounted read-only. State that is not in git (`file_host`'s data, logs
//! and config, Grafana's built dashboards) is mounted from `${STATE_ROOT}`,
//! the owner's checkout, which envctl sets for `make up` and the deploy
//! alike. A mount of untracked or written state from `REPO_ROOT` would start
//! a service on an empty worktree directory, so `envctl check` refuses it.
//!
//! A `${REPO_ROOT}` mount must also lie inside a tree the release's key reads
//! ([`KEYED`]; `deploy.rs`, `RealHost::at`). The deploy takes a commit whose
//! key it already applied as that release, and restarts nothing, so a change
//! to a mounted file outside those trees would never reach home (#1132).

use crate::compose;
use serde_json::Value;

const REPO_ROOT: &str = "${REPO_ROOT";

/// The trees of a release's commit that its key reads, besides
/// `deploy/<role>/`, in the order the key writes them around it: `infra/`,
/// then `docker-compose.yml`. The deploy's key is built from these, so a
/// mount the check accepts is a mount whose change makes another release.
pub(crate) const KEYED: [&str; 2] = ["infra", "docker-compose.yml"];

/// Whether a path of the repository lies inside a tree the key reads.
fn keyed(path: &str) -> bool {
	!path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
		&& KEYED
			.iter()
			.any(|tree| path == *tree || path.strip_prefix(tree).is_some_and(|below| below.starts_with('/')))
}

/// Each `${REPO_ROOT}` bind mount that is written to or names no tracked
/// path. `tracked` is every path `git ls-files` lists.
pub(crate) fn problems(raw: &Value, tracked: &[&str]) -> Vec<String> {
	let mut problems = Vec::new();
	for (service, entry) in compose::services(raw) {
		for volume in entry.get("volumes").and_then(Value::as_array).into_iter().flatten() {
			let Some(source) = volume.get("source").and_then(Value::as_str) else {
				continue;
			};
			let Some(after) = source.strip_prefix(REPO_ROOT).and_then(|rest| rest.split_once('}')).map(|(_, path)| path) else {
				continue;
			};
			let rest = after.trim_start_matches('/');
			let target = volume.get("target").and_then(Value::as_str).unwrap_or("?");
			let held = tracked
				.iter()
				.any(|path| rest.is_empty() || *path == rest || path.strip_prefix(rest).is_some_and(|below| below.starts_with('/')));
			if !held {
				problems.push(format!(
					"service {service} mounts ${{REPO_ROOT}}/{rest} at {target}, which git does not hold, so a release's worktree has none: mount it from ${{STATE_ROOT}} (docs/release.md, RL10)"
				));
			}
			// `${REPO_ROOT}infra/…` names a sibling of the worktree, not a
			// path inside it, so only what follows a slash is the tree's.
			if !(after.starts_with('/') && keyed(rest)) {
				problems.push(format!(
					"service {service} mounts ${{REPO_ROOT}}/{rest} at {target}, outside the trees a release's key reads ({}): a change to it would be taken as the release already applied and never reach home; keep what compose mounts under infra/ (docs/release.md, RL10)",
					KEYED.join(", ")
				));
			}
			if volume.get("read_only").and_then(Value::as_bool) != Some(true) {
				problems.push(format!(
					"service {service} mounts ${{REPO_ROOT}}/{rest} at {target} writable; a release's tree is read-only: mount it :ro, or mount written state from ${{STATE_ROOT}} (docs/release.md, RL10)"
				));
			}
		}
	}
	problems
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	use std::path::Path;

	fn raw(source: &str, read_only: bool) -> Value {
		json!({"services": {"file-host": {"volumes": [{"type": "volume", "source": source, "target": "/t", "read_only": read_only}]}}})
	}

	const TRACKED: [&str; 2] = ["infra/nats/nats.conf", "infra/grafana/dashboards/call.jsonnet"];

	#[test]
	fn a_tracked_read_only_mount_is_the_release_s() {
		assert_eq!(problems(&raw("${REPO_ROOT:?x}/infra/nats/nats.conf", true), &TRACKED), Vec::<String>::new());
		assert_eq!(problems(&raw("${REPO_ROOT:?x}/infra/grafana/dashboards", true), &TRACKED), Vec::<String>::new());
	}

	#[test]
	fn untracked_or_written_state_is_refused_from_the_release_s_tree() {
		let found = problems(&raw("${REPO_ROOT:?x}/data", false), &TRACKED);
		assert_eq!(found.len(), 3, "{found:?}");
		assert!(found[0].contains("which git does not hold") && found[0].contains("STATE_ROOT"), "{found:?}");
		assert!(found[1].contains("outside the trees a release's key reads"), "{found:?}");
		assert!(found[2].contains("writable"), "{found:?}");
		assert_eq!(
			problems(&raw("${REPO_ROOT:?x}/infra/grafana/dash", true), &TRACKED).len(),
			1,
			"a prefix of a name is not its directory"
		);
	}

	/// #1132: a tracked, read-only mount outside the key's trees is refused,
	/// since a commit that changes only it is the release already applied.
	#[test]
	fn a_mount_outside_the_key_s_trees_is_refused() {
		let tracked = ["scripts/render.sh", "infra/nats/nats.conf", "docker-compose.yml", "deploy/home/release.json"];
		for source in [
			"${REPO_ROOT:?x}/scripts/render.sh",
			"${REPO_ROOT:?x}/deploy/home/release.json",
			"${REPO_ROOT:?x}",
			"${REPO_ROOT:?x}/",
			"${REPO_ROOT:?x}/infra/../scripts/render.sh",
			"${REPO_ROOT:?x}/infrastructure",
			"${REPO_ROOT:?x}infra/nats/nats.conf",
		] {
			let found = problems(&raw(source, true), &tracked);
			assert!(
				found.iter().any(|problem| problem.contains("outside the trees a release's key reads")),
				"{source}: {found:?}"
			);
		}
		for source in ["${REPO_ROOT:?x}/infra/nats/nats.conf", "${REPO_ROOT:?x}/infra", "${REPO_ROOT:?x}/docker-compose.yml"] {
			assert_eq!(problems(&raw(source, true), &tracked), Vec::<String>::new(), "{source}");
		}
	}

	#[test]
	fn state_root_and_other_sources_are_not_the_release_s() {
		assert_eq!(problems(&raw("${STATE_ROOT:?x}/data", false), &TRACKED), Vec::<String>::new());
		assert_eq!(problems(&raw("${STORAGE:?x}", false), &TRACKED), Vec::<String>::new());
	}

	/// The checkout's own compose files hold to it (C344).
	#[test]
	fn the_checkout_mounts_only_tracked_files_from_the_release() {
		let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
		let Ok(raw) = compose::config(&repo) else {
			return; // no docker compose here; `envctl check` runs it in CI
		};
		let listed = std::process::Command::new("git").args(["ls-files", "-z"]).current_dir(&repo).output().unwrap();
		let text = String::from_utf8_lossy(&listed.stdout);
		let tracked: Vec<&str> = text.split('\0').filter(|path| !path.is_empty()).collect();
		assert_eq!(problems(&raw, &tracked), Vec::<String>::new());
	}
}
