use super::{previews, unrendered, Folder, Unrendered};
use conversation_repo::step::{self, replay, State};
use serde_json::{json, Value};
use std::path::Path;

/// Whose clips the tests make.
pub(crate) const A: &str = "subject-a";

/// What the renderer leaves once `preview`'s clips of subject A's
/// conversation `c` are made.
pub(crate) fn render(root: &Path, preview: &str, variants: &[&str]) {
	let folder = root.join(A).join("c").join(preview);
	std::fs::create_dir_all(&folder).unwrap();
	for variant in variants {
		std::fs::write(folder.join(String::from(*variant) + ".mp4"), b"0123456789").unwrap();
	}
}

/// The agent's offer of `preview`, variants `a` (`calm-close`) and `b`
/// (`lively-close`), `a` recommended.
pub(crate) fn offer(preview: &str) -> Value {
	json!({ "event": "offer", "by": "agent", "at": "2026-11-26T10:05:30.000Z", "offer": {
		"preview": preview,
		"about": "How the essay closes",
		"recommended": "a",
		"variants": [
			{ "variant": "a", "title": "Calm", "callouts": ["Mochi leans in", "the close is slower"], "rule": String::from(preview) + "-calm" },
			{ "variant": "b", "title": "Lively", "callouts": ["the close is quicker"], "rule": String::from(preview) + "-lively" }
		]
	} })
}

fn state(values: Vec<Value>) -> State {
	let mut log = vec![
		json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "a topic" } }),
		json!({ "event": "version", "by": "agent", "at": "2026-11-26T10:02:00.000Z", "version": {
			"n": 1, "speakers": [{ "id": "tin", "character": "tin-owlet", "voice": { "kind": "kokoro" } }],
			"turns": [{ "id": "t1", "speaker": "tin", "voice": "kokoro", "said": "a line" }], "answers": []
		} }),
		json!({ "event": "heard", "by": "home", "at": "2026-11-26T10:04:00.000Z", "version": 1, "ms": 60_000 }),
		json!({ "event": "green-light", "by": "owner", "at": "2026-11-26T10:05:00.000Z", "version": 1 }),
	];
	log.extend(values);
	let events: Vec<_> = log.into_iter().map(|v| step::parse(v).unwrap()).collect();
	replay(&events).unwrap()
}

#[tokio::test]
async fn a_conversation_lists_its_offers_in_order_with_their_clips_and_standing_pick() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &["a", "b"]);
	render(dir.path(), "p-open", &["a", "b"]);
	let pick = json!({ "event": "approve", "by": "owner", "at": "2026-11-26T10:06:00.000Z", "preview": "p-close", "chose": { "kind": "variant", "variant": "b" } });
	let listed = previews(&folder, A, "c", &state(vec![offer("p-open"), offer("p-close"), pick])).await.unwrap();
	let ids: Vec<&str> = listed.previews.iter().map(|p| p.preview.as_str()).collect();
	assert_eq!(ids, ["p-open", "p-close"], "in the order offered");
	let close = &listed.previews[1];
	assert_eq!(close.about, "How the essay closes");
	assert_eq!(close.recommended, "a");
	assert_eq!(close.variants[1].title, "Lively");
	assert_eq!(close.variants[1].rule, "p-close-lively");
	assert_eq!(close.variants[0].callouts, ["Mochi leans in", "the close is slower"]);
	assert_eq!(close.variants[1].video, "/api/v1/conversations/c/previews/p-close/b/video");
	assert_eq!(
		serde_json::to_value(&close.approved).unwrap(),
		json!({ "chose": { "kind": "variant", "variant": "b", "rule": "p-close-lively" } })
	);
	assert_eq!(listed.previews[0].approved, None);
	assert!(listed.unreadable.is_empty());
}

#[tokio::test]
async fn an_offer_needs_every_clip_on_home_and_one_gone_is_named() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &["a"]);
	let made = step::parse(offer("p-close")).unwrap();
	assert_eq!(
		unrendered(&folder, A, "c", made.offer().unwrap()).await.unwrap(),
		Some(Unrendered::OfferUnrendered {
			offer: "p-close".to_owned(),
			variant: "b".to_owned()
		})
	);
	render(dir.path(), "p-close", &["a", "b"]);
	assert_eq!(unrendered(&folder, A, "c", made.offer().unwrap()).await.unwrap(), None);
	std::fs::remove_file(dir.path().join(A).join("c/p-close/b.mp4")).unwrap();
	let listed = previews(&folder, A, "c", &state(vec![offer("p-close")])).await.unwrap();
	assert!(listed.previews.is_empty());
	assert_eq!(listed.unreadable, ["p-close"], "a clip gone since is named, never shown broken");
}

#[tokio::test]
async fn another_subjects_clips_are_none_of_the_callers() {
	let dir = tempfile::tempdir().unwrap();
	let folder = Folder(dir.path().to_owned());
	render(dir.path(), "p-close", &["a", "b"]);
	let made = step::parse(offer("p-close")).unwrap();
	assert_eq!(unrendered(&folder, A, "c", made.offer().unwrap()).await.unwrap(), None);
	assert!(
		unrendered(&folder, "subject-b", "c", made.offer().unwrap()).await.unwrap().is_some(),
		"the same conversation id under another subject is another conversation"
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
