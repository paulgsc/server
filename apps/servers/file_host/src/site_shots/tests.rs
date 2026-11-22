use super::*;
use mailbox_kit::{Method, Request, Response, Token};
use std::sync::{Arc, Mutex};

/// Answers each put with the next canned status, and keeps every request;
/// a clone sees the same.
#[derive(Default, Clone)]
struct Fake(Arc<Inner>);

#[derive(Default)]
struct Inner {
	answers: Mutex<Vec<(u16, &'static str)>>,
	sent: Mutex<Vec<Request>>,
}

impl MailboxTransport for Fake {
	type Error = &'static str;

	async fn send(&self, request: Request) -> Result<Response, Self::Error> {
		self.0.sent.lock().unwrap().push(request);
		let next = {
			let mut answers = self.0.answers.lock().unwrap();
			(!answers.is_empty()).then(|| answers.remove(0))
		};
		let Some((status, body)) = next else {
			return Err("connection refused");
		};
		Ok(Response {
			status,
			body: body.as_bytes().to_vec(),
		})
	}
}

fn mailbox(answers: Vec<(u16, &'static str)>) -> (Mailbox<Fake>, Fake) {
	let fake = Fake(Arc::new(Inner {
		answers: Mutex::new(answers),
		sent: Mutex::new(Vec::new()),
	}));
	(Mailbox::new("https://mailbox.example.test", Token::new("t".to_owned()), fake.clone()).unwrap(), fake)
}

const ADDED: (u16, &str) = (201, r#"{"change":"added","seq":1}"#);

fn folders(root: &Path) -> Folders {
	let folders = Folders {
		shots: root.join("shots"),
		renders: root.join("renders"),
		work: root.join("work"),
		status: root.join("status"),
	};
	for folder in [&folders.shots, &folders.renders, &folders.work, &folders.status] {
		std::fs::create_dir_all(folder).unwrap();
	}
	folders
}

/// Monday 2026-11-23, ten hours after the last whole week ended.
fn wednesday() -> DateTime<Utc> {
	"2026-11-23T10:00:00Z".parse().unwrap()
}

fn png() -> Vec<u8> {
	let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
	bytes.extend([0; 32]);
	bytes
}

fn mp4() -> Vec<u8> {
	let mut bytes = b"\0\0\0\x20ftypisom".to_vec();
	bytes.extend([0; 32]);
	bytes
}

/// The render worker's finished folder for the week.
fn finish(folders: &Folders, clip: &[u8], still: &[u8]) {
	let done = folders.renders.join("2026-11-23");
	std::fs::create_dir_all(&done).unwrap();
	std::fs::write(done.join(CLIP), clip).unwrap();
	std::fs::write(done.join(STILL), still).unwrap();
}

#[test]
fn a_week_ends_at_the_last_monday_midnight_utc() {
	let at = |text: &str| window_end(text.parse().unwrap()).to_string();
	assert_eq!(at("2026-11-23T00:00:00Z"), "2026-11-23");
	assert_eq!(at("2026-11-25T10:00:00Z"), "2026-11-23");
	assert_eq!(at("2026-11-29T23:59:59Z"), "2026-11-23");
	assert_eq!(at("2026-11-22T23:59:59Z"), "2026-11-16");
}

#[test]
fn the_sample_is_what_the_walk_reads() {
	let end = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
	let body: serde_json::Value = serde_json::from_slice(&sample(end, "1.2.3")).unwrap();
	assert_eq!(
		body,
		serde_json::json!({
			"schema": "some-ui.walk-sample/home-dashboards/1",
			"version": "1.2.3",
			"origin": "http://grafana:3000",
			"to": "2026-11-23T00:00:00Z",
			"days": 7,
		})
	);
}

#[tokio::test]
async fn a_week_is_asked_once_then_sent_and_home_keeps_nothing_of_it() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, fake) = mailbox(vec![ADDED, ADDED]);

	let first = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	assert_eq!(first, Done { asked: true, ..Done::default() });
	assert!(folders.shots.join("2026-11-23.json").exists());
	// Asked once: a second pass before the render writes nothing new.
	assert_eq!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap(), Done::default());

	finish(&folders, &mp4(), &png());
	std::fs::create_dir_all(folders.work.join("2026-11-23")).unwrap();
	let sent = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	assert_eq!(sent, Done { sent: true, ..Done::default() });
	let requests = mailbox_requests(&fake);
	assert_eq!(
		requests,
		[
			(Method::Put, "/v1/site-assets/items/cores-20261123T000000000Z.png".to_owned(), "image/png"),
			(Method::Put, "/v1/site-assets/items/server-20261123T000000000Z.mp4".to_owned(), "video/mp4"),
		]
	);
	assert!(folders.shots.join("2026-11-23.sent").exists());
	for gone in [folders.shots.join("2026-11-23.json"), folders.renders.join("2026-11-23"), folders.work.join("2026-11-23")] {
		assert!(!gone.exists(), "{}", gone.display());
	}
	// Sent: the week is never asked again.
	assert_eq!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap(), Done::default());
	assert!(!folders.shots.join("2026-11-23.json").exists());
}

fn mailbox_requests(fake: &Fake) -> Vec<(Method, String, &'static str)> {
	fake
		.0
		.sent
		.lock()
		.unwrap()
		.iter()
		.map(|request| {
			(
				request.method,
				request.url.trim_start_matches("https://mailbox.example.test").to_owned(),
				request.body_type.as_str(),
			)
		})
		.collect()
}

#[tokio::test]
async fn a_picture_too_big_or_not_its_type_is_refused_and_never_sent() {
	for (clip, still) in [(vec![0; 786_433], png()), (mp4(), mp4()), (png(), png())] {
		let dir = tempfile::tempdir().unwrap();
		let folders = folders(dir.path());
		let (mailbox, fake) = mailbox(vec![ADDED, ADDED]);
		run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
		finish(&folders, &clip, &still);
		let done = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
		assert!(done.refused.is_some() && !done.sent, "{done:?}");
		assert!(folders.shots.join("2026-11-23.refused").exists());
		assert!(!folders.renders.join("2026-11-23").exists());
		// Nothing over the cap or of the wrong type reaches the mailbox.
		assert!(
			mailbox_requests(&fake).iter().all(|(_, path, _)| !path.contains("server-")),
			"{:?}",
			mailbox_requests(&fake)
		);
		assert_eq!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap(), Done::default());
	}
}

#[tokio::test]
async fn a_mailbox_that_does_not_answer_leaves_the_week_for_the_next_pass() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, _fake) = mailbox(vec![]);
	run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	finish(&folders, &mp4(), &png());
	assert!(matches!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await, Err(Failed::Mailbox(_))));
	assert!(folders.shots.join("2026-11-23.json").exists());
	assert!(folders.renders.join("2026-11-23").join(CLIP).exists());
	assert!(!folders.shots.join("2026-11-23.sent").exists());
}

#[tokio::test]
async fn a_conflict_or_a_newer_picture_held_counts_as_sent() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, _fake) = mailbox(vec![(409, r#"{"error":"conflict"}"#), (409, r#"{"error":"a newer one of this kind is held"}"#)]);
	run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	finish(&folders, &mp4(), &png());
	assert!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap().sent);
}

#[tokio::test]
async fn a_week_before_the_last_leaves_the_drive_and_only_the_last_is_asked() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, _fake) = mailbox(vec![]);
	let last_week: DateTime<Utc> = "2026-11-16T10:00:00Z".parse().unwrap();
	run_once(&mailbox, &folders, "1.2.3", last_week).await.unwrap();
	std::fs::create_dir_all(folders.renders.join("2026-11-16")).unwrap();
	std::fs::write(folders.shots.join("2026-11-09.sent"), b"").unwrap();
	std::fs::write(folders.shots.join("notes.txt"), b"not a week's").unwrap();
	run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	let mut left: Vec<String> = std::fs::read_dir(&folders.shots)
		.unwrap()
		.map(|entry| entry.unwrap().file_name().into_string().unwrap())
		.collect();
	left.sort();
	assert_eq!(left, ["2026-11-23.json", "notes.txt"]);
	assert!(!folders.renders.join("2026-11-16").exists());
}

#[tokio::test]
async fn a_week_ended_more_than_a_day_ago_is_missed_and_never_asked() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, _fake) = mailbox(vec![]);
	let thursday: DateTime<Utc> = "2026-11-26T10:00:00Z".parse().unwrap();
	assert_eq!(run_once(&mailbox, &folders, "1.2.3", thursday).await.unwrap(), Done { missed: true, ..Done::default() });
	assert!(!folders.shots.join("2026-11-23.json").exists());
}

#[tokio::test]
async fn a_render_drawn_after_prometheus_let_the_week_go_is_refused() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, fake) = mailbox(vec![ADDED, ADDED]);
	run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	finish(&folders, &mp4(), &png());
	let late = std::time::SystemTime::from("2026-11-24T09:00:00Z".parse::<DateTime<Utc>>().unwrap());
	std::fs::File::options()
		.write(true)
		.open(folders.renders.join("2026-11-23").join(CLIP))
		.unwrap()
		.set_modified(late)
		.unwrap();
	let done = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	assert!(done.refused.unwrap().contains("Prometheus"));
	assert!(mailbox_requests(&fake).is_empty());
}

#[tokio::test]
async fn a_failed_render_is_said_once_and_a_picture_over_the_cap_ends_the_week() {
	let dir = tempfile::tempdir().unwrap();
	let folders = folders(dir.path());
	let (mailbox, _fake) = mailbox(vec![]);
	run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	let log = folders.status.join("2026-11-23.failed.log");
	std::fs::write(&log, "walking\nGrafana's health answered 503\n\n").unwrap();
	let said = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	assert_eq!(said.render_failed.as_deref(), Some("Grafana's health answered 503"));
	// Said once: the same log is not said again, and the week is still asked.
	assert_eq!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap(), Done::default());
	assert!(folders.shots.join("2026-11-23.json").exists());

	// A later attempt's picture is over the cap: the same bytes would come again.
	std::thread::sleep(std::time::Duration::from_millis(20));
	std::fs::write(&log, "the server clip is 900000 bytes even at half size, over the mailbox's 786432\n").unwrap();
	let done = run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap();
	assert!(done.refused.unwrap().contains("over the mailbox's"));
	assert!(!folders.shots.join("2026-11-23.json").exists(), "the request is taken back, so the worker stops");
	assert!(folders.shots.join("2026-11-23.refused").exists());
	assert_eq!(run_once(&mailbox, &folders, "1.2.3", wednesday()).await.unwrap(), Done::default());
}
