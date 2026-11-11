use super::{current, failure_class, id_at, prune, read, request, template_of, Asked, Folders, State, LIFETIME, TEMPLATES};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn folders(dir: &Path) -> Folders {
	let folders = Folders {
		requests: dir.join("preview-requests"),
		renders: dir.join("renders/preview"),
		status: dir.join("renders/preview-worker"),
		work: dir.join("renders/preview-work"),
		lane: dir.join("renders/call-worker"),
	};
	for folder in [&folders.requests, &folders.renders, &folders.status, &folders.work, &folders.lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	folders
}

/// 2026-11-08T07:45:00Z.
fn at(offset_secs: u64) -> SystemTime {
	UNIX_EPOCH + Duration::from_secs(1_794_123_900 + offset_secs)
}

fn set_mtime(path: &Path, when: SystemTime) {
	std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

/// What the worker leaves once `id` is made, its video's time `made`.
fn render(folders: &Folders, id: &str, made: SystemTime) {
	let folder = folders.renders.join(id);
	std::fs::create_dir_all(&folder).unwrap();
	std::fs::write(folder.join("preview.mp4"), b"0123456789").unwrap();
	let template = template_of(id).unwrap();
	// The sample as recorded: the call's week, Playoff Delta's fill.
	let (file, sample) = if template == "conference-call" {
		("week.json", serde_json::json!({ "template": { "id": template, "version": 2 } }))
	} else {
		("narration.json", serde_json::json!({ "format": { "id": template, "version": 2 } }))
	};
	std::fs::write(folder.join(file), sample.to_string()).unwrap();
	set_mtime(&folder.join("preview.mp4"), made);
}

fn fail(folders: &Folders, id: &str, why: &str, when: SystemTime) {
	let log = folders.status.join(String::from(id) + ".failed.log");
	std::fs::write(&log, why).unwrap();
	set_mtime(&log, when);
}

fn job(folders: &Folders, id: &str, state: &str, yields: u32) {
	std::fs::write(
		folders.lane.join("job.state"),
		String::from("id=1\nkind=preview\nkey=") + id + "\nsince=1\nstate=" + state + "\nstate_since=1\nyields=" + &yields.to_string() + "\nran=0",
	)
	.unwrap();
}

fn requests(folders: &Folders) -> Vec<String> {
	let mut names: Vec<String> = std::fs::read_dir(&folders.requests)
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.collect();
	names.sort_unstable();
	names
}

#[test]
fn an_id_is_a_known_template_and_a_utc_second() {
	assert_eq!(id_at("conference-call", at(0)), "conference-call-20261108T074500Z");
	assert_eq!(template_of("conference-call-20261108T074500Z"), Some("conference-call"));
	assert_eq!(template_of("playoff-delta-legend-20261108T074500Z"), Some("playoff-delta-legend"));
	for not in [
		"podcast-20261108T074500Z",
		"conference-call-20261108T0745Z",
		"conference-call-2026110XT074500Z",
		"conference-call",
		"../conference-call-20261108T074500Z",
		"conference-call-20261108T074500Z/x",
	] {
		assert_eq!(template_of(not), None, "{not}");
	}
	assert_eq!(template_of("rust-essay-20261110T050000Z"), Some("rust-essay"), "the essay's template (RR16)");
	assert_eq!(TEMPLATES.len(), 4, "some-ui's TEMPLATE_IDS, and the worker's PREVIEW_ID");
}

#[tokio::test]
async fn a_request_writes_one_and_another_while_it_is_pending_answers_that_one() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let Asked::New(first) = request(&folders, "conference-call", at(0)).await.unwrap() else {
		panic!("a first request is new")
	};
	assert_eq!((first.id.as_str(), first.state), ("conference-call-20261108T074500Z", State::Waiting));
	assert_eq!(requests(&folders), ["conference-call-20261108T074500Z.json"], "written whole: no partial file is left");
	let body: serde_json::Value = serde_json::from_slice(&std::fs::read(folders.requests.join("conference-call-20261108T074500Z.json")).unwrap()).unwrap();
	assert_eq!(body["template"], "conference-call");

	for (state, yields, expected) in [
		("waiting", 0, State::Waiting),
		("rendering", 0, State::Rendering),
		("paused", 0, State::Paused),
		("waiting", 1, State::GivenWay),
	] {
		job(&folders, &first.id, state, yields);
		let again = request(&folders, "conference-call", at(60)).await.unwrap();
		assert_eq!(again, Asked::Pending(super::Preview { state: expected, ..first.clone() }), "{state} {yields}");
	}
	assert_eq!(requests(&folders).len(), 1, "never a second while one is to be made");

	let Asked::New(other) = request(&folders, "playoff-delta-week", at(60)).await.unwrap() else {
		panic!("each template's own")
	};
	assert_eq!(other.state, State::Waiting, "another preview's job is not this one's");
}

#[tokio::test]
async fn a_ready_preview_says_when_it_was_made_its_size_version_and_when_it_goes() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	request(&folders, "conference-call", at(0)).await.unwrap();
	render(&folders, "conference-call-20261108T074500Z", at(300));
	let preview = current(&folders, "conference-call", at(600)).await.unwrap().unwrap();
	assert_eq!(
		serde_json::to_value(&preview).unwrap(),
		serde_json::json!({
			"id": "conference-call-20261108T074500Z",
			"version": 2,
			"state": "ready",
			"made_at": 1_794_124_200_u64,
			"expires_at": 1_794_124_200_u64 + LIFETIME.as_secs(),
			"bytes": 10,
		})
	);
	assert_eq!(
		current(&folders, "conference-call", at(300) + LIFETIME).await.unwrap(),
		None,
		"a day after it was made it is gone"
	);
	assert_eq!(
		current(&folders, "playoff-delta-legend", at(600)).await.unwrap(),
		None,
		"a template never asked for has none"
	);

	let Asked::New(next) = request(&folders, "conference-call", at(900)).await.unwrap() else {
		panic!("a ready one is no pending one")
	};
	assert_eq!(next.id, "conference-call-20261108T080000Z");
}

#[tokio::test]
async fn a_failed_preview_says_why_in_a_class_never_its_log() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	for (template, log, class) in [
		("playoff-delta-week", "playoff-delta-week has no sample fill yet, so it can't be previewed", "no-sample"),
		("playoff-delta-legend", "the video would run 16 min, past the policy's 15 min", "too-long"),
		("conference-call", "Error: page.goto: Timeout 30000ms exceeded at /opt/some-ui", "render"),
	] {
		let Asked::New(asked) = request(&folders, template, at(0)).await.unwrap() else {
			panic!()
		};
		fail(&folders, &asked.id, log, at(60));
		let preview = read(&folders, &asked.id).await.unwrap();
		assert_eq!((preview.state, preview.failed), (State::Failed, Some(class)), "{log}");
		assert!(!serde_json::to_value(&preview).unwrap().to_string().contains("/opt"), "nothing of the log leaves home");
		let Asked::New(again) = request(&folders, template, at(120)).await.unwrap() else {
			panic!("a failed one is no pending one")
		};
		assert_ne!(again.id, asked.id);
	}
}

#[test]
fn each_refusal_the_recorder_words_has_its_class() {
	// The wording is paulgsc/some-ui `packages/video-render/src/policy.ts`'s
	// and `tests/record/preview.ts`'s at the commit the render's image is
	// published from (its pin in infra/compose/call-render.yml).
	for (log, class) in [
		("playoff-delta-week has no sample fill yet, so it can't be previewed", "no-sample"),
		("the video would run 16 min, past the policy's 15 min", "too-long"),
		("the video is 512 MB, past the policy's 500 MB", "too-large"),
		("cache warning: step 3 grew the cache from 600 MB past the policy's 500 MB; kept", "render"),
		("Error: page.goto: Timeout 30000ms exceeded", "render"),
	] {
		assert_eq!(failure_class(log), class, "{log}");
	}
}

#[tokio::test]
async fn the_prune_takes_a_day_old_preview_and_any_older_than_the_newest_ready() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let ids = ["conference-call-20261108T074500Z", "conference-call-20261108T080000Z", "conference-call-20261108T081500Z"];
	for (n, id) in ids.iter().enumerate() {
		std::fs::write(folders.requests.join(String::from(*id) + ".json"), "{}").unwrap();
		std::fs::create_dir_all(folders.work.join(id)).unwrap();
		std::fs::write(folders.status.join(String::from(*id) + ".cache.log"), "").unwrap();
		let n = u64::try_from(n).unwrap();
		if n < 2 {
			render(&folders, id, at(n * 900 + 300));
		}
	}
	// The newest is still to be made: the two ready ones before it.
	let pruned = prune(&folders, at(1_000)).await.unwrap();
	assert_eq!(pruned.removed, [ids[0]], "an older one goes once a newer one is ready");
	for gone in [
		folders.requests.join(String::from(ids[0]) + ".json"),
		folders.renders.join(ids[0]),
		folders.work.join(ids[0]),
		folders.status.join(String::from(ids[0]) + ".cache.log"),
	] {
		assert!(!gone.exists(), "{}", gone.display());
	}
	assert!(
		folders.renders.join(ids[1]).join("preview.mp4").exists(),
		"the newest ready one stays while the next is made"
	);
	assert!(prune(&folders, at(2_000)).await.unwrap().removed.is_empty(), "nothing more is due");

	let pruned = prune(&folders, at(900 + 300) + LIFETIME).await.unwrap();
	assert_eq!(pruned.removed, [ids[1]], "a day after it was made it goes, the newest ready included");
	assert_eq!(
		requests(&folders),
		[String::from(ids[2]) + ".json"],
		"the one still to be made stays, however long it waits"
	);
}

#[tokio::test]
async fn the_prune_takes_a_failed_one_a_day_after_and_never_the_job_in_progress() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (failed, running) = ("playoff-delta-week-20261108T074500Z", "conference-call-20261108T074500Z");
	for id in [failed, running] {
		std::fs::write(folders.requests.join(String::from(id) + ".json"), "{}").unwrap();
	}
	fail(&folders, failed, "has no sample fill yet", at(60));
	// A ready folder the worker left for the job in progress, as if the
	// prune ran between the move and the job's end: it is still the job's.
	render(&folders, running, at(0));
	job(&folders, running, "rendering", 0);
	assert!(
		prune(&folders, at(120)).await.unwrap().removed.is_empty(),
		"a failure stays a day, for the phone to say why"
	);
	let pruned = prune(&folders, at(60) + LIFETIME + LIFETIME).await.unwrap();
	assert_eq!(pruned.removed, [failed]);
	assert!(folders.renders.join(running).exists(), "the job in progress is never pruned");
	assert!(!folders.status.join(String::from(failed) + ".failed.log").exists());
}

#[tokio::test]
async fn a_request_file_of_another_shape_is_never_a_preview() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	for name in [
		"podcast-20261108T074500Z.json",
		"conference-call.json",
		".conference-call-20261108T074500Z.json",
		"conference-call-20261108T074500Z.txt",
	] {
		std::fs::write(folders.requests.join(name), "{}").unwrap();
	}
	assert_eq!(current(&folders, "conference-call", at(0)).await.unwrap(), None);
	assert!(
		prune(&folders, at(0) + LIFETIME + LIFETIME).await.unwrap().removed.is_empty(),
		"nor pruned: what it doesn't know it leaves alone"
	);
}

/// PV1 (paulgsc/some-ui#2757): a request not ready a day after it was asked
/// ends as failed, `overdue`, even as the job in progress; the owner may ask
/// again at once, and it goes a day after it was found overdue.
#[tokio::test]
async fn a_request_not_ready_a_day_after_it_was_asked_fails_overdue() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let Asked::New(asked) = request(&folders, "conference-call", at(0)).await.unwrap() else {
		panic!()
	};
	std::fs::create_dir_all(folders.work.join(&asked.id)).unwrap();
	job(&folders, &asked.id, "rendering", 0);
	assert!(
		prune(&folders, at(0) + LIFETIME - Duration::from_secs(1)).await.unwrap().overdue.is_empty(),
		"within its day it waits"
	);

	let found = at(0) + LIFETIME;
	let pruned = prune(&folders, found).await.unwrap();
	assert_eq!(pruned.overdue, std::slice::from_ref(&asked.id));
	let overdue = folders.requests.join(String::from(&asked.id) + ".overdue");
	set_mtime(&overdue, found);
	assert!(!folders.requests.join(String::from(&asked.id) + ".json").exists(), "the worker lets go of it");
	let preview = current(&folders, "conference-call", found).await.unwrap().unwrap();
	assert_eq!(
		(preview.state, preview.failed),
		(State::Failed, Some("overdue")),
		"the job in progress included: it died, or never ran"
	);
	assert!(folders.work.join(&asked.id).exists(), "the job in progress keeps its work");

	// A late render lands; it is still overdue.
	std::fs::remove_file(folders.lane.join("job.state")).unwrap();
	render(&folders, &asked.id, found);
	assert_eq!(read(&folders, &asked.id).await.unwrap().failed, Some("overdue"));
	let Asked::New(again) = request(&folders, "conference-call", found + Duration::from_secs(60)).await.unwrap() else {
		panic!("an overdue one is no pending one")
	};
	assert_ne!(again.id, asked.id);

	let pruned = prune(&folders, found + Duration::from_secs(600)).await.unwrap();
	assert!(!folders.work.join(&asked.id).exists(), "once it is no job in progress, a failed one's work goes at once");
	assert!(!pruned.removed.contains(&asked.id), "the failure stays a day, for the phone to say why");
	let pruned = prune(&folders, found + LIFETIME).await.unwrap();
	assert!(pruned.removed.contains(&asked.id));
	assert!(!overdue.exists() && !folders.renders.join(&asked.id).exists());
}

/// What is left of a preview whose request is gone goes, and a half-written
/// request once it is a minute old; the job in progress and names that are
/// no preview's stay.
#[tokio::test]
async fn the_prune_sweeps_what_a_request_left_behind() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (orphan, running) = ("conference-call-20261108T074500Z", "playoff-delta-week-20261108T074500Z");
	render(&folders, orphan, at(0));
	std::fs::create_dir_all(folders.work.join(orphan)).unwrap();
	std::fs::write(folders.status.join(String::from(orphan) + ".failed.log"), "x").unwrap();
	std::fs::create_dir_all(folders.work.join(running)).unwrap();
	job(&folders, running, "rendering", 0);
	std::fs::create_dir_all(folders.renders.join("lost+found")).unwrap();
	let partial = folders.requests.join(String::from(".") + orphan + ".json");
	std::fs::write(&partial, "{").unwrap();
	set_mtime(&partial, at(0));

	let pruned = prune(&folders, at(30)).await.unwrap();
	assert!(!folders.renders.join(orphan).exists() && !folders.work.join(orphan).exists());
	assert!(!folders.status.join(String::from(orphan) + ".failed.log").exists());
	assert!(folders.work.join(running).exists(), "the job in progress is never swept");
	assert!(folders.renders.join("lost+found").exists(), "a name that is no preview's is left alone");
	assert!(partial.exists(), "a request being written is left a minute");
	assert_eq!(pruned.swept.len(), 3);
	prune(&folders, at(120)).await.unwrap();
	assert!(!partial.exists());
}

#[tokio::test]
async fn a_job_rendered_and_being_moved_into_place_reads_as_rendering() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let Asked::New(asked) = request(&folders, "conference-call", at(0)).await.unwrap() else {
		panic!()
	};
	job(&folders, &asked.id, "done", 2);
	assert_eq!(read(&folders, &asked.id).await.unwrap().state, State::Rendering);
}

#[tokio::test]
async fn a_playoff_delta_preview_says_its_version_from_the_fill_it_recorded() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	for template in ["playoff-delta-legend", "playoff-delta-week"] {
		request(&folders, template, at(0)).await.unwrap();
		let id = String::from(template) + "-20261108T074500Z";
		render(&folders, &id, at(300));
		assert_eq!(read(&folders, &id).await.unwrap().version, Some(2), "{template}");
	}
	// A fill of another format names no version of this one.
	let folder = folders.renders.join("playoff-delta-week-20261108T074500Z");
	std::fs::write(
		folder.join("narration.json"),
		serde_json::json!({ "format": { "id": "playoff-delta-legend", "version": 4 } }).to_string(),
	)
	.unwrap();
	assert_eq!(read(&folders, "playoff-delta-week-20261108T074500Z").await.unwrap().version, None);
}
