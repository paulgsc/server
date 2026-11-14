//! What each image in a release provides and requires of the others, and
//! the one rule that holds them together (RELEASE R3, #790; docs/release.md,
//! "Constraints").
//!
//! An image says it with two OCI labels its Dockerfile writes,
//! [`PROVIDES`] and [`REQUIRES`], each comma-separated `name=value` pairs. A
//! value is an epoch (an integer: a requirement of n is met by a provision
//! of at least n) or a 40-hex commit (met by the same commit only). A
//! release fits together when every image's every requirement is met by
//! another image's provision. An image without the labels requires nothing.
//!
//! CI checks a release with `scripts/check_release.py`, which reads the
//! labels from the registry; the deploy checks it again here, from the
//! pulled images, before `up` (R4, #793). The two sides are run on the same
//! cases: `scripts/check_release.py`'s `test_the_rule_is_envctl_s` reads
//! this file's `case(…)` lines.
#![cfg_attr(not(test), expect(dead_code, reason = "envctl deploy checks the release with it before up (R4, #793)"))]

use std::collections::BTreeMap;

/// The label an image names what it provides with.
pub(crate) const PROVIDES: &str = "io.paulgsc.provides";
/// The label an image names what it requires of the others with.
pub(crate) const REQUIRES: &str = "io.paulgsc.requires";

/// What one image provides and requires.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Constraints {
	pub(crate) provides: BTreeMap<String, String>,
	pub(crate) requires: BTreeMap<String, String>,
}

fn is_name(name: &str) -> bool {
	name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_epoch(value: &str) -> bool {
	!value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn is_commit(value: &str) -> bool {
	value.len() == 40 && value.chars().all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
}

/// One label's `name=value` pairs. Empty text says nothing.
pub(crate) fn pairs(text: &str) -> Result<BTreeMap<String, String>, String> {
	let mut found = BTreeMap::new();
	for part in text.split(',').map(str::trim).filter(|part| !part.is_empty()) {
		let Some((name, value)) = part.split_once('=').filter(|(name, value)| is_name(name) && (is_epoch(value) || is_commit(value))) else {
			return Err(format!("`{part}` is not name=epoch or name=<40-hex commit>"));
		};
		if found.insert(name.to_owned(), value.to_owned()).is_some() {
			return Err(format!("`{name}` is given twice"));
		}
	}
	Ok(found)
}

impl Constraints {
	/// An image's constraints from its labels, as `docker image inspect`
	/// gives them.
	pub(crate) fn from_labels(labels: &BTreeMap<String, String>) -> Result<Self, String> {
		let read = |label: &str| pairs(labels.get(label).map_or("", String::as_str));
		Ok(Self {
			provides: read(PROVIDES)?,
			requires: read(REQUIRES)?,
		})
	}
}

/// An epoch is met by one at least as high; a commit by itself.
fn meets(provided: &str, required: &str) -> bool {
	if is_epoch(required) {
		return is_epoch(provided) && provided.parse::<u128>().ok() >= required.parse::<u128>().ok();
	}
	provided == required
}

/// Each requirement no other image in the set meets, as one line, worded as
/// `scripts/check_release.py` words it.
pub(crate) fn unmet(images: &BTreeMap<String, Constraints>) -> Vec<String> {
	let mut problems = Vec::new();
	for (service, image) in images {
		for (name, required) in &image.requires {
			let offered: Vec<(&String, &String)> = images
				.iter()
				.filter(|(other, _)| *other != service)
				.filter_map(|(other, constraints)| Some((other, constraints.provides.get(name)?)))
				.collect();
			if offered.iter().any(|(_, value)| meets(value, required)) {
				continue;
			}
			if offered.is_empty() {
				problems.push(format!("{service} requires {name}={required}, which no other image in the release provides"));
			} else {
				let given: Vec<String> = offered.iter().map(|(other, value)| format!("{other} provides {name}={value}")).collect();
				problems.push(format!("{service} requires {name}={required}, but {}", given.join(", ")));
			}
		}
	}
	problems
}

#[cfg(test)]
mod tests {
	use super::*;

	fn labels(provides: &str, requires: &str) -> BTreeMap<String, String> {
		[(PROVIDES, provides), (REQUIRES, requires)]
			.map(|(label, value)| (label.to_owned(), value.to_owned()))
			.into()
	}

	/// One image providing `provides` and another requiring `requires`: the
	/// set fits together exactly when `met`. `scripts/check_release.py`
	/// reads each `case(…)` call here and runs it too, so keep its
	/// arguments string literals.
	fn case(provides: &str, requires: &str, why: &str, met: bool) {
		let images: BTreeMap<String, Constraints> = [("p", labels(provides, "")), ("r", labels("", requires))]
			.map(|(service, labels)| (service.to_owned(), Constraints::from_labels(&labels).unwrap()))
			.into();
		assert_eq!(unmet(&images).is_empty(), met, "{why}: {:?}", unmet(&images));
	}

	#[test]
	fn the_rule_check_release_shares() {
		case(
			"contract=6,render-folders=3",
			"render-folders=4",
			"render reads a newer layout than file_host writes (#790)",
			false,
		);
		case("render-folders=4", "render-folders=4", "the same epoch", true);
		case("render-folders=5", "render-folders=4", "a higher epoch", true);
		case("nats-subjects=1", "rust-runner-protocol=1", "a requirement nothing provides", false);
		case(
			"ui=d495258000000000000000000000000000000000",
			"ui=d495258000000000000000000000000000000000",
			"the same commit",
			true,
		);
		case(
			"ui=d495258000000000000000000000000000000000",
			"ui=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
			"another commit",
			false,
		);
		case("ui=10", "ui=d495258000000000000000000000000000000000", "an epoch never meets a commit", false);
		case("", "", "nothing either way", true);
	}

	#[test]
	fn render_reading_a_newer_layout_than_file_host_writes_is_refused() {
		let images: BTreeMap<String, Constraints> = [("file-host", labels("contract=6,render-folders=3", "")), ("render", labels("", "render-folders=4"))]
			.map(|(service, labels)| (service.to_owned(), Constraints::from_labels(&labels).unwrap()))
			.into();
		assert_eq!(unmet(&images), vec!["render requires render-folders=4, but file-host provides render-folders=3".to_owned()]);
	}

	#[test]
	fn an_image_does_not_meet_its_own_requirement() {
		let images: BTreeMap<String, Constraints> = [("a".to_owned(), Constraints::from_labels(&labels("x=2", "x=1")).unwrap())].into();
		assert_eq!(unmet(&images), vec!["a requires x=1, which no other image in the release provides".to_owned()]);
	}

	#[test]
	fn an_image_without_labels_requires_nothing() {
		assert_eq!(Constraints::from_labels(&BTreeMap::new()).unwrap(), Constraints::default());
	}

	#[test]
	fn a_malformed_label_is_refused_not_skipped() {
		for bad in ["render-folders", "render-folders=x", "Render=1", "a=1,a=2", "c=abc123"] {
			assert!(Constraints::from_labels(&labels("", bad)).is_err(), "{bad}");
		}
	}
}
