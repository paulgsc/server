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

/// One service's memory cap, None where it declares none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cap {
	pub(crate) service: String,
	pub(crate) bytes: Option<u64>,
	pub(crate) class: Class,
}

/// Compose's byte size: a number of bytes, or one with a `b`, `k`, `m` or
/// `g` unit (binary, any case, `kb`/`mb`/`gb` alike). None when it is
/// neither, as an uninterpolated `${…}` is.
fn bytes(text: &str) -> Option<u64> {
	let text = text.trim().to_ascii_lowercase();
	let text = text.strip_suffix('b').filter(|rest| rest.ends_with(['k', 'm', 'g'])).unwrap_or(&text);
	let (number, unit) = match text.char_indices().last()? {
		(at, 'k') => (&text[..at], 1u64 << 10),
		(at, 'm') => (&text[..at], 1 << 20),
		(at, 'g') => (&text[..at], 1 << 30),
		(at, 'b') => (&text[..at], 1),
		_ => (text, 1),
	};
	number.trim().parse::<u64>().ok()?.checked_mul(unit)
}

/// Each service's cap and class, from compose's uninterpolated config.
pub(crate) fn caps(raw: &Value) -> Vec<Cap> {
	compose::services(raw)
		.into_iter()
		.map(|(name, service)| Cap {
			service: name.to_owned(),
			bytes: match service.get("mem_limit") {
				Some(Value::Number(number)) => number.as_u64(),
				Some(Value::String(text)) => bytes(text),
				_ => None,
			},
			class: Class::of(service.get("oom_score_adj").and_then(Value::as_i64).unwrap_or(0)),
		})
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

/// What `make up` warns of: a service with no cap, and every cap together
/// above `mem_total`, naming the largest caps, which bind first.
///
/// Every service at its cap, the batch class's too: its caps are its peak
/// (the render's 4 GiB and the speaker's 3 GiB are the lane's declared peak
/// and what the speaker keeps between renders), and the interactive room the
/// lane's floor holds is already inside the interactive caps.
pub(crate) fn warnings(caps: &[Cap], mem_total: Option<u64>) -> Vec<String> {
	let mut warnings: Vec<String> = caps
		.iter()
		.filter(|cap| cap.bytes.is_none())
		.map(|cap| format!("service {} ({}) declares no mem_limit, so home's caps can't be summed whole", cap.service, cap.class.name()))
		.collect();
	let Some(mem_total) = mem_total else {
		warnings.push("this machine's MemTotal could not be read from /proc/meminfo, so home's caps were not checked".to_owned());
		return warnings;
	};
	let total: u64 = caps.iter().filter_map(|cap| cap.bytes).sum();
	if total <= mem_total {
		return warnings;
	}
	let mut largest: Vec<&Cap> = caps.iter().filter(|cap| cap.bytes.is_some()).collect();
	largest.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.service.cmp(&b.service)));
	let mut said = format!(
		"the services' memory caps total {}, above this machine's MemTotal of {} by {}: home can run out before any service reaches its cap. The largest:",
		gib(total),
		gib(mem_total),
		gib(total - mem_total)
	);
	for cap in largest.iter().take(3) {
		let _ = write!(said, " {} {} ({}),", cap.service, gib(cap.bytes.unwrap_or(0)), cap.class.name());
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
			class,
		}
	}

	#[test]
	fn compose_sizes_read_as_binary_bytes() {
		assert_eq!(bytes("4G"), Some(4 * G));
		assert_eq!(bytes("512m"), Some(512 << 20));
		assert_eq!(bytes("128MB"), Some(128 << 20));
		assert_eq!(bytes("1kb"), Some(1024));
		assert_eq!(bytes("100b"), Some(100));
		assert_eq!(bytes("2048"), Some(2048));
		assert_eq!(bytes("${CAP}"), None);
		assert_eq!(bytes(""), None);
		assert_eq!(bytes("g"), None);
	}

	#[test]
	fn a_class_is_read_from_its_oom_score_adj() {
		let raw = json!({"services": {
			"file-host": {"mem_limit": "4G", "oom_score_adj": -500},
			"rust-runner": {"mem_limit": "1G", "oom_score_adj": 0},
			"metabase": {"mem_limit": "2G", "oom_score_adj": 500},
			"render": {"mem_limit": "4G", "oom_score_adj": 800},
			"ollama": {"mem_limit": "8G", "oom_score_adj": 900},
			"bare": {}
		}});
		let caps = caps(&raw);
		let class_of = |name: &str| caps.iter().find(|cap| cap.service == name).unwrap().class;
		assert_eq!(class_of("file-host"), Class::Protected);
		assert_eq!(class_of("rust-runner"), Class::Interactive);
		assert_eq!(class_of("metabase"), Class::Tooling);
		assert_eq!(class_of("render"), Class::Batch);
		assert_eq!(class_of("ollama"), Class::Batch);
		assert_eq!(class_of("bare"), Class::Interactive);
		assert_eq!(caps.iter().find(|cap| cap.service == "bare").unwrap().bytes, None);
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
	fn an_uncapped_service_and_an_unread_mem_total_are_said() {
		let caps = [cap("bare", None, Class::Interactive)];
		let said = warnings(&caps, None);
		assert_eq!(said.len(), 2);
		assert!(said[0].starts_with("service bare (interactive) declares no mem_limit"));
		assert!(said[1].contains("MemTotal could not be read"));
	}
}
