use super::{offered, previews, Approved, Folder, Unoffered, MANIFEST};
use conversation_repo::step::{Approval, Choice};
use serde_json::{json, Value};
use std::path::Path;

/// Whose previews the tests make.
pub(crate) const A: &str = "subject-a";

/// What the renderer leaves once `preview` of subject A's conversation `c`
/// is made.
pub(crate) fn render(root: &Path, preview: &str, variants: &[(&str, &str)], recommended: &str) {
	let folder = root.join(A).join("c").join(preview);
	std::fs::create_dir_all(&folder).unwrap();
	for (variant, _) in variants {
		std::fs::write(folder.join(String::from(*variant) + ".mp4"), b"0123456789").unwrap();
	}
	let manifest = json!({
		"schema": "some-ui.conversation-preview/1",
		"preview": preview,
		"about": "How one scene gives way to the next",
		"recommended": recommended,
		"variants": variants.iter().map(|(variant, rule)| json!({ "variant": variant, "title": variant.to_uppercase(), "rule": rule, "callouts": ["Mochi leans in", "the close is slower"] })).collect::<Vec<Value>>()
	});
	std::fs::write(folder.join(MANIFEST), manifest.to_string()).unwrap();
}

fn approval(preview: &str, offered: &[&str], recommended: &str, chose: Choice) -> Approval {
	Approval {
		preview: preview.to_owned(),
		offered: offered.iter().map(|v| (*v).to_owned()).collect(),
		recommended: recommended.to_owned(),
		chose,
		moves: None,
	}
}

fn variant(variant: &str, rule: &str) -> Choice {
	Choice::Variant {
		variant: variant.to_owned(),
		rule: rule.to_owned(),
	}
}

#[tokio::test]
async fn a_conversation_lists_its_ready_previews_by_id_with_the_records_approval() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-open", &[("a", "warm-open"), ("b", "cool-open")], "b");
	render(dir.path(), "p-close", &[("a", "calm-close"), ("b", "lively-close"), ("c", "playful-close")], "a");
	let picked = approval("p-close", &["a", "b", "c"], "a", variant("c", "playful-close"));
	let listed = previews(&folder, A, "c", &[picked]).await.unwrap();
	let ids: Vec<&str> = listed.previews.iter().map(|p| p.preview.as_str()).collect();
	assert_eq!(ids, ["p-close", "p-open"]);
	let close = &listed.previews[0];
	assert_eq!(close.recommended, "a");
	assert_eq!(close.about, "How one scene gives way to the next");
	assert_eq!(close.variants[1].title, "B");
	assert_eq!(close.variants.len(), 3);
	assert_eq!(close.variants[2].rule, "playful-close");
	assert_eq!(close.variants[2].video, "/api/v1/conversations/c/previews/p-close/c/video");
	assert_eq!(close.variants[0].callouts, ["Mochi leans in", "the close is slower"]);
	assert_eq!(
		close.approved,
		Some(Approved {
			chose: variant("c", "playful-close")
		})
	);
	assert_eq!(listed.previews[1].approved, None);
	assert!(listed.unreadable.is_empty());
}

#[tokio::test]
async fn a_preview_still_being_made_is_not_offered_and_a_broken_one_is_named() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-making", &[("a", "r1"), ("b", "r2")], "a");
	std::fs::remove_file(dir.path().join(A).join("c/p-making/b.mp4")).unwrap();
	std::fs::create_dir_all(dir.path().join(A).join("c/p-unwritten")).unwrap();
	render(dir.path(), "p-broken", &[("a", "r1"), ("b", "r2")], "z");
	render(dir.path(), "p-alone", &[("a", "r1")], "a");
	let listed = previews(&folder, A, "c", &[]).await.unwrap();
	assert!(listed.previews.is_empty());
	assert_eq!(listed.unreadable, ["p-alone", "p-broken"]);
	// A line over PreviewOfferSchema's bound is unreadable too.
	render(dir.path(), "p-long", &[("a", "r1"), ("b", "r2")], "a");
	let manifest = dir.path().join(A).join("c/p-long").join(MANIFEST);
	let mut long: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
	long["about"] = json!("x".repeat(81));
	std::fs::write(&manifest, long.to_string()).unwrap();
	assert!(previews(&folder, A, "c", &[]).await.unwrap().unreadable.contains(&"p-long".to_owned()));
	assert!(previews(&folder, A, "none-yet", &[]).await.unwrap().previews.is_empty());
	assert!(previews(&folder, A, "../c", &[]).await.unwrap().previews.is_empty());
}

#[tokio::test]
async fn an_approval_names_a_preview_exactly_as_home_offers_it() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &[("a", "calm-close"), ("b", "lively-close")], "a");
	let offer = |a: Approval| {
		let folder = folder.clone();
		async move { offered(&folder, A, "c", &a).await.unwrap() }
	};
	assert_eq!(offer(approval("p-close", &["a", "b"], "a", variant("b", "lively-close"))).await, Ok(()));
	let neither = Choice::Else {
		said: "neither, slower".to_owned(),
	};
	assert_eq!(offer(approval("p-close", &["a", "b"], "a", neither)).await, Ok(()));
	let wrong = Err(Unoffered::PreviewOffer { preview: "p-close".to_owned() });
	assert_eq!(
		offer(approval("p-close", &["a", "b"], "a", variant("b", "calm-close"))).await,
		wrong,
		"the rule is the variant's"
	);
	assert_eq!(
		offer(approval("p-close", &["b", "a"], "a", variant("b", "lively-close"))).await,
		wrong,
		"offered in home's order"
	);
	assert_eq!(
		offer(approval("p-close", &["a", "b"], "b", variant("b", "lively-close"))).await,
		wrong,
		"home's recommendation"
	);
	assert_eq!(
		offer(approval("p-other", &["a", "b"], "a", variant("b", "lively-close"))).await,
		Err(Unoffered::PreviewUnknown { preview: "p-other".to_owned() })
	);
	assert_eq!(
		offer(approval("Not An Id", &["a", "b"], "a", variant("b", "lively-close"))).await,
		Err(Unoffered::PreviewUnknown { preview: "Not An Id".to_owned() })
	);
}

#[tokio::test]
async fn another_subjects_previews_are_none_of_the_callers() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &[("a", "calm-close"), ("b", "lively-close")], "a");
	assert_eq!(previews(&folder, A, "c", &[]).await.unwrap().previews.len(), 1);
	let theirs = previews(&folder, "subject-b", "c", &[]).await.unwrap();
	assert!(
		theirs.previews.is_empty() && theirs.unreadable.is_empty(),
		"the same conversation id under another subject is another conversation"
	);
	let pick = approval("p-close", &["a", "b"], "a", variant("b", "lively-close"));
	assert_eq!(
		offered(&folder, "subject-b", "c", &pick).await.unwrap(),
		Err(Unoffered::PreviewUnknown { preview: "p-close".to_owned() })
	);
	assert!(!folder.video("subject-b", "c", "p-close", "a").unwrap().exists());
}

#[test]
fn a_video_is_only_ever_one_plain_segment_per_name() {
	let folder = Folder("/r".into());
	assert_eq!(folder.video(A, "c", "p", "a").unwrap(), Path::new("/r/subject-a/c/p/a.mp4"));
	assert!(folder.video(A, "c", "..", "a").is_none());
	assert!(folder.video(A, "c", "p", "a/b").is_none());
	assert!(folder.video(A, "C", "p", "a").is_none());
	assert!(folder.video("../subject-a", "c", "p", "a").is_none());
}
