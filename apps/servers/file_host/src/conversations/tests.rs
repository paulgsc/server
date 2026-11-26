use super::{offered, previews, Approved, Folder, Unoffered, MANIFEST};
use conversation_repo::step::{Approval, Choice};
use serde_json::{json, Value};
use std::path::Path;

/// What the renderer leaves once `preview` of conversation `c` is made.
pub(crate) fn render(root: &Path, preview: &str, variants: &[(&str, &str)], recommended: &str) {
	let folder = root.join("c").join(preview);
	std::fs::create_dir_all(&folder).unwrap();
	for (variant, _) in variants {
		std::fs::write(folder.join(String::from(*variant) + ".mp4"), b"0123456789").unwrap();
	}
	let manifest = json!({
		"schema": "some-ui.conversation-preview/1",
		"preview": preview,
		"recommended": recommended,
		"variants": variants.iter().map(|(variant, rule)| json!({ "variant": variant, "rule": rule, "callouts": ["Mochi leans in", "the close is slower"] })).collect::<Vec<Value>>()
	});
	std::fs::write(folder.join(MANIFEST), manifest.to_string()).unwrap();
}

fn approval(preview: &str, offered: &[&str], recommended: &str, chose: Choice) -> Approval {
	Approval {
		preview: preview.to_owned(),
		offered: offered.iter().map(|v| (*v).to_owned()).collect(),
		recommended: recommended.to_owned(),
		chose,
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
	let listed = previews(&folder, "c", &[picked]).await.unwrap();
	let ids: Vec<&str> = listed.previews.iter().map(|p| p.preview.as_str()).collect();
	assert_eq!(ids, ["p-close", "p-open"]);
	let close = &listed.previews[0];
	assert_eq!(close.recommended, "a");
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
	std::fs::remove_file(dir.path().join("c/p-making/b.mp4")).unwrap();
	std::fs::create_dir_all(dir.path().join("c/p-unwritten")).unwrap();
	render(dir.path(), "p-broken", &[("a", "r1"), ("b", "r2")], "z");
	render(dir.path(), "p-alone", &[("a", "r1")], "a");
	let listed = previews(&folder, "c", &[]).await.unwrap();
	assert!(listed.previews.is_empty());
	assert_eq!(listed.unreadable, ["p-alone", "p-broken"]);
	assert!(previews(&folder, "none-yet", &[]).await.unwrap().previews.is_empty());
	assert!(previews(&folder, "../c", &[]).await.unwrap().previews.is_empty());
}

#[tokio::test]
async fn an_approval_names_a_preview_exactly_as_home_offers_it() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &[("a", "calm-close"), ("b", "lively-close")], "a");
	let offer = |a: Approval| {
		let folder = folder.clone();
		async move { offered(&folder, "c", &a).await.unwrap() }
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

#[test]
fn a_video_is_only_ever_one_plain_segment_per_name() {
	let folder = Folder("/r".into());
	assert_eq!(folder.video("c", "p", "a").unwrap(), Path::new("/r/c/p/a.mp4"));
	assert!(folder.video("c", "..", "a").is_none());
	assert!(folder.video("c", "p", "a/b").is_none());
	assert!(folder.video("C", "p", "a").is_none());
}
