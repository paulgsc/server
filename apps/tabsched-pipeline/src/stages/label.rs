///
/// Direct port of deriveLabel() + slugify() from pipeline/embed.ts.
use ws_events::tabsched::TabCapture;

/// Derive a short slug label from a capture URL.
///
/// Examples:
///   leetcode.com/problems/two-sum/  → lc-two-sum
///   github.com/tokio-rs/tokio       → tokio
///   doc.rust-lang.org/book/ch04     → rust-ch04
///   claude.ai/chat/abc123           → claude-chat
pub fn derive_label(capture: &TabCapture) -> String {
	let url = match url::Url::parse(&capture.url) {
		Ok(u) => u,
		Err(_) => return slugify(&capture.tab_title).chars().take(28).collect(),
	};

	let host = url.host_str().unwrap_or("").trim_start_matches("www.").to_string();

	let parts: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();

	// Known host → prefix mapping.  Empty string = use path tail directly.
	let prefix_map: &[(&str, &str)] = &[
		("leetcode.com", "lc"),
		("github.com", ""),
		("doc.rust-lang.org", "rust"),
		("docs.rs", "crate"),
		("neetcode.io", "neetcode"),
		("typst.app", "typst"),
	];

	let prefix_opt = prefix_map.iter().find(|(h, _)| *h == host).map(|(_, p)| *p);

	let slug = match prefix_opt {
		None => {
			// Unknown host: first segment of hostname + last two path parts
			let host_slug = host.split('.').next().unwrap_or(&host);
			let tail = parts.iter().rev().take(2).rev().cloned().collect::<Vec<_>>().join("-");
			slugify(&format!("{}-{}", host_slug, tail))
		}
		Some("") => {
			// e.g. github.com → owner/repo → last two parts
			slugify(&parts.iter().rev().take(2).rev().cloned().collect::<Vec<_>>().join("-"))
		}
		Some(prefix) => {
			let skip = ["problems", "en", "book"];
			let tail = parts.iter().filter(|p| !skip.contains(p)).last().copied().unwrap_or("");
			if tail.is_empty() {
				slugify(prefix)
			} else {
				slugify(&format!("{}-{}", prefix, tail))
			}
		}
	};

	slug.chars().take(28).collect()
}

fn slugify(s: &str) -> String {
	let lower = s.to_lowercase();
	let slug: String = lower.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
	// collapse and trim dashes
	let collapsed = slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
	collapsed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::{ContentKind, ExtractedContent, TabCapture};

	fn make_cap(url: &str) -> TabCapture {
		TabCapture {
			tab_id: 1,
			url: url.into(),
			tab_title: "title".into(),
			captured_at: "2025-01-01T00:00:00Z".into(),
			extractor: "generic".into(),
			domain: "test".into(),
			content: ExtractedContent {
				kind: ContentKind("article".into()),
				title: "t".into(),
				summary: "s".into(),
				headings: vec![],
				keywords: vec![],
				raw_length: 0,
				meta: Default::default(),
			},
			extraction_ok: true,
			extraction_error: None,
		}
	}

	#[test]
	fn test_leetcode() {
		let c = make_cap("https://leetcode.com/problems/two-sum/");
		assert_eq!(derive_label(&c), "lc-two-sum");
	}

	#[test]
	fn test_github() {
		let c = make_cap("https://github.com/tokio-rs/tokio");
		assert_eq!(derive_label(&c), "tokio-rs-tokio");
	}

	#[test]
	fn test_rust_doc() {
		let c = make_cap("https://doc.rust-lang.org/book/ch04-ownership.html");
		assert_eq!(derive_label(&c), "rust-ch04-ownership-html");
	}
}
