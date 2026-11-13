//! Whether home can hold every service at its memory cap (L3 #709, part 3):
//! `make up` reads `MemTotal` on the box it runs on and sums the compose
//! services' `mem_limit`s against it (`docs/home-compute.md`, "The caps
//! check").
//!
//! It warns and never refuses, until home's facts are published (#709,
//! part 1) and the owner decides whether `ollama` counts (#705, D3).

use crate::compose;
use serde_json::Value;
use std::fmt::Write as _;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Who the kernel kills first, read from the `oom_score_adj` each class sets
/// (`docs/home-compute.md`, "Who is killed first").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
	Protected,
	Interactive,
	Tooling,
	Batch,
}

impl Class {
	const fn of(oom_score_adj: i64) -> Self {
		match oom_score_adj {
			i64::MIN..0 => Self::Protected,
			0..500 => Self::Interactive,
			500..800 => Self::Tooling,
			_ => Self::Batch,
		}
	}

	const fn name(self) -> &'static str {
		match self {
			Self::Protected => "protected",
			Self::Interactive => "interactive",
			Self::Tooling => "tooling",
			Self::Batch => "batch",
		}
	}
}

/// One service's memory cap, all its replicas together; None where it
/// declares none or none envctl can read. Its class, None where its
/// `oom_score_adj` can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cap {
	pub(crate) service: String,
	pub(crate) bytes: Option<u64>,
	pub(crate) class: Option<Class>,
}

/// Compose's byte size, as compose itself reads one (go-units'
/// `RAMInBytes`): a number, in any form a float takes (`.5`, `1.`, `1e3`,
/// `+1`), then an optional space and unit `b`, or `k`, `m`, `g`, `t` or `p`
/// (binary, any case) with an optional `b` or `ib`. None for anything else,
/// as an uninterpolated `${…}` is, and for 0, which Docker takes as no cap.
fn bytes(text: &str) -> Option<u64> {
	let text = text.to_ascii_lowercase();
	let (number, shift) = [("kib", 10), ("mib", 20), ("gib", 30), ("tib", 40), ("pib", 50)]
		.into_iter()
		.chain([("kb", 10), ("mb", 20), ("gb", 30), ("tb", 40), ("pb", 50)])
		.chain([("k", 10), ("m", 20), ("g", 30), ("t", 40), ("p", 50), ("b", 0)])
		.find_map(|(unit, shift)| text.strip_suffix(unit).map(|number| (number, shift)))
		.unwrap_or((&text, 0));
	let number = number.strip_suffix(' ').unwrap_or(number).replace('_', "");
	// What a float parses and compose would not: a name, or a bare sign.
	if !number.chars().any(|c| c.is_ascii_digit()) || number.contains(|c: char| c.is_ascii_alphabetic() && c != 'e') {
		return None;
	}
	let sized = if let Ok(whole) = number.parse::<u64>() {
		whole.checked_mul(1 << shift)?
	} else {
		let decimal = number.parse::<f64>().ok().filter(|decimal| decimal.is_finite() && *decimal >= 0.0)?;
		#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		// a size with a fraction, truncated to bytes as compose truncates it
		let sized = (decimal * (1u64 << shift) as f64) as u64;
		sized
	};
	Some(sized).filter(|&sized| sized > 0)
}

/// A whole number compose may give as a number or as text.
fn whole(value: &Value) -> Option<i64> {
	match value {
		Value::Number(number) => number.as_i64(),
		Value::String(text) => text.trim().parse().ok(),
		_ => None,
	}
}

/// A size compose may give as a number of bytes or as text.
fn size(value: &Value) -> Option<u64> {
	match value {
		Value::Number(number) => number.as_u64().filter(|&sized| sized > 0),
		Value::String(text) => bytes(text),
		_ => None,
	}
}

/// One replica's cap: `mem_limit`, or else `deploy.resources.limits.memory`.
/// None when the one it has can't be read, or it has neither.
fn replica_cap(service: &Value) -> Option<u64> {
	service
		.get("mem_limit")
		.filter(|value| !value.is_null())
		.map_or_else(|| service.pointer("/deploy/resources/limits/memory").filter(|value| !value.is_null()).and_then(size), size)
}

/// How many of it `up` starts: `scale`, or else `deploy.replicas`, or one.
/// None when the one it has can't be read.
fn replicas(service: &Value) -> Option<u64> {
	service
		.get("scale")
		.filter(|value| !value.is_null())
		.or_else(|| service.pointer("/deploy/replicas").filter(|value| !value.is_null()))
		.map_or(Some(1), |value| whole(value).and_then(|count| u64::try_from(count).ok()))
}

/// Each service `up` starts, with its cap and class, from compose's
/// uninterpolated config. A service under `profiles:` is left out unless
/// one of them is in `active` (`COMPOSE_PROFILES`; `*` is every profile),
/// as `up` leaves it.
pub(crate) fn caps(raw: &Value, active: &[&str]) -> Vec<Cap> {
	compose::services(raw)
		.into_iter()
		.filter(|(_, service)| {
			service
				.get("profiles")
				.and_then(Value::as_array)
				.filter(|profiles| !profiles.is_empty())
				.is_none_or(|profiles| profiles.iter().filter_map(Value::as_str).any(|profile| active.contains(&"*") || active.contains(&profile)))
		})
		.map(|(name, service)| Cap {
			service: name.to_owned(),
			bytes: replica_cap(service).zip(replicas(service)).and_then(|(cap, count)| cap.checked_mul(count)),
			class: service
				.get("oom_score_adj")
				.filter(|value| !value.is_null())
				.map_or(Some(Class::of(0)), |value| whole(value).map(Class::of)),
		})
		.collect()
}

/// The profiles `COMPOSE_PROFILES` names, comma-separated.
pub(crate) fn profiles(compose_profiles: Option<&str>) -> Vec<&str> {
	compose_profiles
		.unwrap_or_default()
		.split(',')
		.map(str::trim)
		.filter(|profile| !profile.is_empty())
		.collect()
}

/// `MemTotal` from `/proc/meminfo`'s text, in bytes.
pub(crate) fn mem_total(meminfo: &str) -> Option<u64> {
	let line = meminfo.lines().find_map(|line| line.strip_prefix("MemTotal:"))?;
	let kib = line.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
	kib.checked_mul(1024)
}

fn gib(bytes: u64) -> String {
	#[allow(clippy::cast_precision_loss)] // a memory size, shown to a hundredth of a GiB
	let gib = bytes as f64 / GIB;
	format!("{gib:.2} GiB")
}

fn class_name(class: Option<Class>) -> &'static str {
	class.map_or("class unread: its oom_score_adj is not a number", Class::name)
}

/// What `make up` warns of: a service whose cap or class can't be read,
/// and every cap together above `mem_total`, naming the largest caps,
/// which bind first.
///
/// Every service at its cap, the batch class's too: its caps are its peak
/// (the render's 4 GiB and the speaker's 3 GiB are the lane's declared peak
/// and what the speaker keeps between renders), and the interactive room the
/// lane's floor holds is already inside the interactive caps.
pub(crate) fn warnings(caps: &[Cap], mem_total: Option<u64>) -> Vec<String> {
	let mut warnings: Vec<String> = Vec::new();
	for cap in caps {
		if cap.bytes.is_none() {
			warnings.push(format!(
				"service {} ({}) has no memory cap envctl can read, so home's caps can't be summed whole",
				cap.service,
				class_name(cap.class)
			));
		} else if cap.class.is_none() {
			warnings.push(format!("service {} has an oom_score_adj envctl can't read, so its class is unknown", cap.service));
		}
	}
	let Some(mem_total) = mem_total else {
		warnings.push("this machine's MemTotal could not be read from /proc/meminfo, so home's caps were not checked".to_owned());
		return warnings;
	};
	let total = caps.iter().filter_map(|cap| cap.bytes).fold(0u64, u64::saturating_add);
	if total <= mem_total {
		return warnings;
	}
	let mut largest: Vec<(&Cap, u64)> = caps.iter().filter_map(|cap| cap.bytes.map(|bytes| (cap, bytes))).collect();
	largest.sort_by(|(a, a_bytes), (b, b_bytes)| b_bytes.cmp(a_bytes).then_with(|| a.service.cmp(&b.service)));
	let mut said = format!(
		"the services' memory caps total {}, above this machine's MemTotal of {} by {}: home can run out before any service reaches its cap. The largest:",
		gib(total),
		gib(mem_total),
		gib(total - mem_total)
	);
	for (cap, bytes) in largest.iter().take(3) {
		let _ = write!(said, " {} {} ({}),", cap.service, gib(*bytes), class_name(cap.class));
	}
	said.pop();
	said.push_str(". Only a warning until home's facts are published and #705's D3 decides on ollama (#709)");
	warnings.push(said);
	warnings
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	const G: u64 = 1 << 30;

	fn cap(service: &str, bytes: Option<u64>, class: Class) -> Cap {
		Cap {
			service: service.to_owned(),
			bytes,
			class: Some(class),
		}
	}

	fn cap_of<'a>(caps: &'a [Cap], name: &str) -> &'a Cap {
		caps.iter().find(|cap| cap.service == name).unwrap()
	}

	#[test]
	fn compose_sizes_read_as_compose_reads_them() {
		assert_eq!(bytes("4G"), Some(4 * G));
		assert_eq!(bytes("512m"), Some(512 << 20));
		assert_eq!(bytes("128MB"), Some(128 << 20));
		assert_eq!(bytes("512MiB"), Some(512 << 20));
		assert_eq!(bytes("1kb"), Some(1024));
		assert_eq!(bytes("100b"), Some(100));
		assert_eq!(bytes("2048"), Some(2048));
		assert_eq!(bytes("1.5g"), Some(1_610_612_736));
		assert_eq!(bytes("0.5G"), Some(G / 2));
		assert_eq!(bytes("1t"), Some(1 << 40));
		assert_eq!(bytes("1p"), Some(1 << 50));
		assert_eq!(bytes("2 g"), Some(2 * G));
		assert_eq!(bytes("${CAP:-1G}"), None);
		assert_eq!(bytes(""), None);
		assert_eq!(bytes("g"), None);
		assert_eq!(bytes(".5g"), Some(G / 2));
		assert_eq!(bytes("1.g"), Some(G));
		assert_eq!(bytes("1e3m"), Some(1000 << 20));
		assert_eq!(bytes("+1g"), Some(G));
		assert_eq!(bytes("1_000"), Some(1000));
		assert_eq!(bytes("1.5 GiB"), Some(1_610_612_736));
		assert_eq!(bytes("1x"), None);
		assert_eq!(bytes("inf"), None);
		assert_eq!(bytes("nan"), None);
		assert_eq!(bytes("-1g"), None);
		assert_eq!(bytes("+"), None);
		assert_eq!(bytes("0"), None, "Docker takes 0 as no cap");
		assert_eq!(bytes("0g"), None);
	}

	#[test]
	fn a_class_is_read_from_its_oom_score_adj() {
		let raw = json!({"services": {
			"file-host": {"mem_limit": "4G", "oom_score_adj": -500},
			"rust-runner": {"mem_limit": "1G", "oom_score_adj": 0},
			"metabase": {"mem_limit": "2G", "oom_score_adj": 500},
			"render": {"mem_limit": "4G", "oom_score_adj": 800},
			"ollama": {"mem_limit": "8G", "oom_score_adj": "900"},
			"bare": {"mem_limit": "1G"},
			"unread": {"mem_limit": "1G", "oom_score_adj": "${ADJ:-900}"}
		}});
		let caps = caps(&raw, &[]);
		assert_eq!(cap_of(&caps, "file-host").class, Some(Class::Protected));
		assert_eq!(cap_of(&caps, "rust-runner").class, Some(Class::Interactive));
		assert_eq!(cap_of(&caps, "metabase").class, Some(Class::Tooling));
		assert_eq!(cap_of(&caps, "render").class, Some(Class::Batch));
		assert_eq!(cap_of(&caps, "ollama").class, Some(Class::Batch));
		assert_eq!(cap_of(&caps, "bare").class, Some(Class::Interactive));
		assert_eq!(cap_of(&caps, "unread").class, None);
	}

	#[test]
	fn a_cap_is_mem_limit_or_the_deploy_limit_times_its_replicas() {
		let raw = json!({"services": {
			"both": {"mem_limit": "1G", "deploy": {"resources": {"limits": {"memory": "1G"}}}},
			"deploy-only": {"deploy": {"resources": {"limits": {"memory": "2G"}}}},
			"scaled": {"mem_limit": "1G", "scale": 3},
			"replicated": {"mem_limit": "1G", "deploy": {"replicas": 2}},
			"uninterpolated": {"mem_limit": "${CAP:-1G}"},
			"unlimited": {"mem_limit": 0},
			"none": {}
		}});
		let caps = caps(&raw, &[]);
		assert_eq!(cap_of(&caps, "both").bytes, Some(G));
		assert_eq!(cap_of(&caps, "deploy-only").bytes, Some(2 * G));
		assert_eq!(cap_of(&caps, "scaled").bytes, Some(3 * G));
		assert_eq!(cap_of(&caps, "replicated").bytes, Some(2 * G));
		assert_eq!(cap_of(&caps, "uninterpolated").bytes, None);
		assert_eq!(cap_of(&caps, "unlimited").bytes, None);
		assert_eq!(cap_of(&caps, "none").bytes, None);
	}

	#[test]
	fn a_service_under_a_profile_counts_only_when_the_profile_is_on() {
		let raw = json!({"services": {
			"always": {"mem_limit": "1G"},
			"extra": {"mem_limit": "1G", "profiles": ["extra"]}
		}});
		let names = |active: &[&str]| caps(&raw, active).into_iter().map(|cap| cap.service).collect::<Vec<_>>();
		assert_eq!(names(&[]), ["always"]);
		assert_eq!(names(&profiles(Some("other, extra"))), ["always", "extra"]);
		assert_eq!(names(&profiles(Some("*"))), ["always", "extra"]);
		assert!(profiles(None).is_empty());
		assert!(profiles(Some(" , ")).is_empty());
	}

	#[test]
	fn mem_total_is_read_in_bytes() {
		assert_eq!(mem_total("MemTotal:       16303412 kB\nMemFree:  1 kB\n"), Some(16_303_412 * 1024));
		assert_eq!(mem_total("MemFree:  1 kB\n"), None);
		assert_eq!(mem_total("MemTotal: lots\n"), None);
	}

	#[test]
	fn caps_that_fit_say_nothing() {
		let caps = [cap("file-host", Some(4 * G), Class::Protected), cap("render", Some(4 * G), Class::Batch)];
		assert!(warnings(&caps, Some(8 * G)).is_empty());
	}

	#[test]
	fn caps_above_mem_total_name_the_largest_first() {
		let caps = [
			cap("nats", Some(G / 2), Class::Protected),
			cap("ollama", Some(8 * G), Class::Batch),
			cap("render", Some(4 * G), Class::Batch),
			cap("file-host", Some(4 * G), Class::Protected),
		];
		let said = warnings(&caps, Some(16 * G));
		assert_eq!(said.len(), 1);
		assert!(said[0].contains("total 16.50 GiB, above this machine's MemTotal of 16.00 GiB by 0.50 GiB"), "{}", said[0]);
		assert!(
			said[0].contains("The largest: ollama 8.00 GiB (batch), file-host 4.00 GiB (protected), render 4.00 GiB (batch)."),
			"{}",
			said[0]
		);
		assert!(!said[0].contains("nats"), "{}", said[0]);
	}

	#[test]
	fn an_unread_cap_or_class_and_an_unread_mem_total_are_said() {
		let caps = [
			cap("bare", None, Class::Interactive),
			Cap {
				service: "unread".to_owned(),
				bytes: Some(G),
				class: None,
			},
		];
		let said = warnings(&caps, None);
		assert_eq!(said.len(), 3, "{said:?}");
		assert!(said[0].starts_with("service bare (interactive) has no memory cap envctl can read"));
		assert!(said[1].starts_with("service unread has an oom_score_adj envctl can't read"));
		assert!(said[2].contains("MemTotal could not be read"));
	}
}
