use super::*;
use call_week_repo::Week;
use sqlx::sqlite::SqlitePoolOptions;

/// 2026-11-08 12:00 UTC, a Sunday (04:00 at home).
const NOW: i64 = 1_794_139_200;

/// What `render-calls.sh`'s `job_state` writes.
fn worker_text(state: &str, state_since: i64, ran: u64) -> String {
	let mut text = String::from("id=1794000000000000000\nkind=call\nkey=2026-11-06\nsince=1794130000\nstate=");
	text.push_str(state);
	text.push_str("\nstate_since=");
	text.push_str(&state_since.to_string());
	text.push_str("\nyields=1\nran=");
	text.push_str(&ran.to_string());
	text.push('\n');
	text
}

fn job(state: &str, state_since: i64, ran: u64) -> Job {
	Job::parse(&worker_text(state, state_since, ran)).unwrap()
}

fn lane(why: &str, at: i64) -> Lane {
	let mut text = String::from("verb=pause\nwhy=");
	text.push_str(why);
	text.push_str("\nstops=0\nstuck=false\nat=");
	text.push_str(&at.to_string());
	Lane::parse(&text).unwrap()
}

#[test]
fn a_job_reads_as_the_worker_writes_it() {
	let read = job("paused", NOW - 30, 120);
	assert_eq!(read.kind, "call");
	assert_eq!(read.key, "2026-11-06");
	assert_eq!(read.state, Phase::Paused);
	assert_eq!((read.since, read.state_since, read.yields, read.ran), (1_794_130_000, NOW - 30, 1, 120));
}

#[test]
fn a_job_from_before_ran_was_written_has_rendered_nothing() {
	let text = worker_text("waiting", NOW, 0).replace("ran=0\n", "");
	assert_eq!(Job::parse(&text).unwrap().ran, 0);
}

#[test]
fn a_job_not_whole_is_none() {
	for broken in [
		worker_text("frozen", NOW, 0),
		worker_text("waiting", NOW, 0).replace("key=2026-11-06", "key=../x"),
		worker_text("waiting", NOW, 0).replace("yields=1\n", ""),
		worker_text("waiting", NOW, 0).replace("ran=0", "ran=x"),
		String::new(),
	] {
		assert_eq!(Job::parse(&broken), None, "{broken}");
	}
}

#[test]
fn the_estimate_is_the_share_of_the_last_render_s_time() {
	// 300 s before, and 60 since it resumed, of a render that took 600.
	let estimate = estimate(&job("rendering", NOW - 60, 300), Some(600), NOW).unwrap();
	assert!((estimate.share - 0.6).abs() < 1e-9);
	assert_eq!(estimate.seconds_left, Some(240));
}

#[test]
fn a_paused_render_s_bar_holds_still() {
	let paused = job("paused", NOW - 500, 300);
	assert_eq!(estimate(&paused, Some(600), NOW), estimate(&paused, Some(600), NOW + 1000));
}

#[test]
fn the_estimate_never_says_done() {
	let estimate = estimate(&job("rendering", NOW - 900, 0), Some(600), NOW).unwrap();
	assert!((estimate.share - MOST_SHOWN).abs() < 1e-9);
	assert_eq!(estimate.seconds_left, None);
}

#[test]
fn with_no_render_measured_there_is_no_estimate() {
	assert_eq!(estimate(&job("rendering", NOW, 0), None, NOW), None);
	assert_eq!(estimate(&job("rendering", NOW, 0), Some(0), NOW), None);
}

#[test]
fn a_job_held_back_says_why_by_the_lane() {
	let feeds = [(Feed::Call, FeedSeen::default())];
	let paused = job("paused", NOW, 0);
	let board_of = |lane: Option<&Lane>| board(&feeds, Some(&paused), lane, NOW).now.unwrap().held_by;
	assert_eq!(board_of(Some(&lane("busy", NOW - 3))), Some(String::from("busy")));
	assert_eq!(board_of(Some(&lane("busy", NOW - 300))), Some(String::from("silent")));
	assert_eq!(board_of(None), Some(String::from("silent")));
	let rendering = job("rendering", NOW, 0);
	assert_eq!(board(&feeds, Some(&rendering), Some(&lane("clear", NOW)), NOW).now.unwrap().held_by, None);
}

fn call_seen() -> FeedSeen {
	FeedSeen {
		held: vec![
			(String::from("2026-10-23"), NOW - 16 * 86_400),
			(String::from("2026-10-30"), NOW - 9 * 86_400),
			(String::from("2026-11-06"), NOW - 2 * 86_400),
		],
		videos: BTreeMap::from([(String::from("2026-10-23"), NOW - 15 * 86_400)]),
		failed: BTreeMap::from([(String::from("2026-10-30"), (NOW - 86_400, String::from("the stage did not answer")))]),
		took: Some(1200),
		newest_week: NaiveDate::from_ymd_opt(2026, 11, 6),
	}
}

fn pd_seen(taken: i64) -> FeedSeen {
	FeedSeen {
		held: vec![(String::from("2026-SF-09"), taken), (String::from("2026-SF-10"), taken + 60)],
		..FeedSeen::default()
	}
}

#[test]
fn each_episode_is_in_one_place() {
	let feeds = [(Feed::Call, call_seen()), (Feed::PlayoffDelta, pd_seen(NOW - 86_400))];
	let read = board(&feeds, None, None, NOW);
	assert_eq!(read.now, None);
	assert_eq!(read.done, [Episode { feed: String::from("call"), key: String::from("2026-10-23"), at: NOW - 15 * 86_400 }]);
	assert_eq!(read.failed.len(), 1);
	assert_eq!(read.failed[0].said, "the stage did not answer");
	let waiting: Vec<(&str, &str)> = read.waiting.iter().map(|e| (e.feed.as_str(), e.key.as_str())).collect();
	// The worker's order: the call's first, each feed's oldest first.
	assert_eq!(waiting, [("call", "2026-11-06"), ("playoff-delta", "2026-SF-09"), ("playoff-delta", "2026-SF-10")]);
}

#[test]
fn the_job_in_progress_is_not_also_waiting_or_failed() {
	let mut seen = call_seen();
	seen.failed.insert(String::from("2026-11-06"), (NOW, String::from("old")));
	let feeds = [(Feed::Call, seen)];
	let running = job("rendering", NOW - 60, 0);
	let read = board(&feeds, Some(&running), None, NOW);
	assert!(read.waiting.iter().all(|e| e.key != "2026-11-06"));
	assert!(read.failed.iter().all(|f| f.key != "2026-11-06"));
	let now = read.now.unwrap();
	assert_eq!((now.feed.as_str(), now.key.as_str(), now.gave_way), ("call", "2026-11-06", 1));
	assert!(now.estimate.is_some(), "the call's took measures it");
}

#[test]
fn a_job_of_a_kind_this_read_does_not_know_still_shows() {
	let text = worker_text("waiting", NOW, 0).replace("kind=call", "kind=preview").replace("key=2026-11-06", "key=p-7");
	let preview = Job::parse(&text).unwrap();
	let read = board(&[(Feed::Call, call_seen())], Some(&preview), None, NOW);
	let now = read.now.unwrap();
	assert_eq!((now.feed.as_str(), now.key.as_str(), now.estimate), ("preview", "p-7", None));
}

#[test]
fn only_the_last_few_finished_are_shown_newest_first() {
	let mut seen = FeedSeen::default();
	for day in 0..8_i64 {
		let key = (NaiveDate::from_ymd_opt(2026, 9, 4).unwrap() + Duration::days(day * 7)).to_string();
		seen.held.push((key.clone(), NOW - 100 * 86_400 + day));
		seen.videos.insert(key, NOW - 90 * 86_400 + day);
	}
	let read = board(&[(Feed::Call, seen)], None, None, NOW);
	assert_eq!(read.done.len(), DONE_SHOWN);
	assert!(read.done.windows(2).all(|pair| pair[0].at > pair[1].at));
}

#[test]
fn the_call_s_next_week_is_a_week_after_its_newest() {
	let read = board(&[(Feed::Call, call_seen())], None, None, NOW);
	assert_eq!(read.coming, [Coming { feed: String::from("call"), on: String::from("2026-11-13") }]);
}

#[test]
fn a_call_week_long_past_comes_on_its_next_friday() {
	let mut seen = call_seen();
	seen.newest_week = NaiveDate::from_ymd_opt(2026, 10, 23);
	assert_eq!(board(&[(Feed::Call, seen)], None, None, NOW).coming[0].on, "2026-11-13");
}

#[test]
fn playoff_delta_comes_on_the_tuesday_after_its_newest_fill() {
	// Taken Saturday Nov 7 at home: next Tuesday is Nov 10.
	let read = board(&[(Feed::PlayoffDelta, pd_seen(NOW - 86_400))], None, None, NOW);
	assert_eq!(read.coming, [Coming { feed: String::from("playoff-delta"), on: String::from("2026-11-10") }]);
}

#[test]
fn a_feed_quiet_for_weeks_expects_nothing() {
	let read = board(&[(Feed::PlayoffDelta, pd_seen(NOW - (ACTIVE_DAYS + 1) * 86_400))], None, None, NOW);
	assert_eq!(read.coming, []);
	assert_eq!(board(&[(Feed::Call, FeedSeen::default())], None, None, NOW).coming, []);
}

#[test]
fn a_failed_attempt_shows_what_the_recorder_said_not_the_worker() {
	let log = "recorded 00-open\nThese scenes don't fit the frame, so the video would cut them off:\nthe recorder exited 1\n";
	assert_eq!(said_in(log), "These scenes don't fit the frame, so the video would cut them off:");
	let worker_only = "the call of 2026-11-06 waits for room on home (the lane: stop, memory)\nthe recorder exited 1\n";
	assert_eq!(said_in(worker_only), "the recorder exited 1");
	assert_eq!(said_in(&"x".repeat(500)).chars().count(), 200);
	assert_eq!(said_in(""), "");
}

async fn pool() -> SqlitePool {
	let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
	crate::schema::MIGRATOR.run(&pool).await.unwrap();
	pool
}

#[tokio::test]
async fn the_read_is_what_the_worker_and_the_lane_left() {
	let db = pool().await;
	{
		let mut conn = db.acquire().await.unwrap();
		for (from, to, taken_at) in [
			("2026-10-30", "2026-11-05", "2026-10-31T12:00:00Z"),
			("2026-11-06", "2026-11-12", "2026-11-07T12:00:00Z"),
		] {
			let week = Week { from, to, body: b"{}", taken_at };
			call_week_repo::keep(&mut conn, &week).await.unwrap();
		}
	}
	let dir = tempfile::tempdir().unwrap();
	let (renders, status) = (dir.path().join("call"), dir.path().join("call-worker"));
	let (pd_renders, pd_status) = (dir.path().join("playoff-delta"), dir.path().join("playoff-delta-worker"));
	for folder in [&renders, &status, &pd_renders, &pd_status] {
		std::fs::create_dir_all(folder).unwrap();
	}
	std::fs::create_dir_all(renders.join("2026-10-30")).unwrap();
	std::fs::write(renders.join("2026-10-30").join("call.mp4"), b"video").unwrap();
	std::fs::write(status.join("took"), b"1200\n").unwrap();
	std::fs::write(status.join("job.state"), worker_text("paused", NOW - 10, 600)).unwrap();
	std::fs::write(status.join("lane"), b"verb=pause\nwhy=busy\nstops=0\nstuck=false\nat=1794139198\n").unwrap();
	let folders = Folders {
		feeds: vec![(Feed::Call, renders, status.clone()), (Feed::PlayoffDelta, pd_renders, pd_status)],
		lane: status,
	};
	let read = read(&db, &folders, NOW).await.unwrap();
	let now = read.now.unwrap();
	assert_eq!((now.key.as_str(), now.state, now.held_by.as_deref()), ("2026-11-06", Phase::Paused, Some("busy")));
	assert!((now.estimate.unwrap().share - 0.5).abs() < 1e-9);
	assert_eq!(read.done.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["2026-10-30"]);
	assert_eq!(read.waiting, []);
	assert_eq!(read.coming, [Coming { feed: String::from("call"), on: String::from("2026-11-13") }]);
}
