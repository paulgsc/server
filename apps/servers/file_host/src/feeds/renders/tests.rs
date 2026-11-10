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
	decided("pause", why, at, false)
}

fn decided(verb: &str, why: &str, at: i64, stuck: bool) -> Lane {
	let mut text = String::from("verb=");
	text.push_str(verb);
	text.push_str("\nwhy=");
	text.push_str(why);
	text.push_str("\nstops=0\nstuck=");
	text.push_str(if stuck { "true" } else { "false" });
	text.push_str("\nat=");
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
	assert_eq!(board_of(Some(&lane("busy", NOW - 3))), Some(HeldBy::Busy));
	assert_eq!(board_of(Some(&lane("busy", NOW - 300))), Some(HeldBy::Silent));
	assert_eq!(board_of(None), Some(HeldBy::Silent));
	assert_eq!(board_of(Some(&lane("a-reason-from-later", NOW))), Some(HeldBy::Unknown), "a closed set");
	let waiting = job("waiting", NOW, 0);
	let about_to = board(&feeds, Some(&waiting), Some(&decided("run", "clear", NOW, false)), NOW).now.unwrap();
	assert_eq!(about_to.held_by, Some(HeldBy::Clear));
	let rendering = job("rendering", NOW, 0);
	assert_eq!(board(&feeds, Some(&rendering), Some(&decided("run", "clear", NOW, false)), NOW).now.unwrap().held_by, None);
}

#[test]
fn a_job_the_worker_left_rendering_is_not_rendering() {
	let feeds = [(Feed::Call, FeedSeen::default())];
	let left = job("rendering", NOW - 3600, 0);
	assert_eq!(board(&feeds, Some(&left), None, NOW).now.unwrap().held_by, Some(HeldBy::Silent));
	let stale_run = decided("run", "clear", NOW - 300, false);
	assert_eq!(board(&feeds, Some(&left), Some(&stale_run), NOW).now.unwrap().held_by, Some(HeldBy::Silent));
}

#[test]
fn stuck_is_only_a_fresh_decision_s() {
	let feeds = [(Feed::Call, FeedSeen::default())];
	let waiting = job("waiting", NOW, 0);
	let stuck = |at| board(&feeds, Some(&waiting), Some(&decided("stop", "memory", at, true)), NOW).now.unwrap().stuck;
	assert!(stuck(NOW));
	assert!(!stuck(NOW - 300));
}

#[test]
fn a_job_whose_episode_was_let_go_or_rendered_is_over() {
	let mut seen = call_seen();
	seen.let_go.insert(String::from("2026-11-06"));
	let left = job("waiting", NOW, 0);
	assert_eq!(board(&[(Feed::Call, seen)], Some(&left), None, NOW).now, None);
	let mut seen = call_seen();
	seen.videos.insert(String::from("2026-11-06"), NOW);
	let read = board(&[(Feed::Call, seen)], Some(&left), None, NOW);
	assert_eq!(read.now, None);
	assert!(read.done.iter().any(|e| e.key == "2026-11-06"), "it shows once, as done");
}

#[test]
fn a_job_moving_its_video_into_place_is_finishing() {
	let finishing = job("done", NOW, 600);
	assert_eq!(finishing.state, Phase::Finishing);
	let read = board(&[(Feed::Call, call_seen())], Some(&finishing), None, NOW);
	let now = read.now.unwrap();
	assert_eq!((now.state, now.held_by), (Phase::Finishing, None));
	assert!(read.waiting.iter().all(|e| e.key != "2026-11-06"));
}

fn call_seen() -> FeedSeen {
	FeedSeen {
		held: vec![
			(String::from("2026-10-23"), NOW - 16 * 86_400),
			(String::from("2026-10-30"), NOW - 9 * 86_400),
			(String::from("2026-11-06"), NOW - 2 * 86_400),
		],
		videos: BTreeMap::from([(String::from("2026-10-23"), NOW - 15 * 86_400)]),
		failed: BTreeMap::from([(
			String::from("2026-10-30"),
			(
				NOW - 86_400,
				Failure {
					by: FailedBy::Recorder,
					exit: Some(1),
				},
			),
		)]),
		took: Some(1200),
		newest_week: NaiveDate::from_ymd_opt(2026, 11, 6),
		newest_taken: Some(NOW - 2 * 86_400),
		..FeedSeen::default()
	}
}

fn pd_seen(taken: i64) -> FeedSeen {
	FeedSeen {
		held: vec![(String::from("2026-SF-09"), taken), (String::from("2026-SF-10"), taken + 60)],
		newest_taken: Some(taken + 60),
		..FeedSeen::default()
	}
}

#[test]
fn each_episode_is_in_one_place() {
	let feeds = [(Feed::Call, call_seen()), (Feed::PlayoffDelta, pd_seen(NOW - 86_400))];
	let read = board(&feeds, None, None, NOW);
	assert_eq!(read.now, None);
	assert_eq!(
		read.done,
		[Episode {
			feed: String::from("call"),
			key: String::from("2026-10-23"),
			at: NOW - 15 * 86_400
		}]
	);
	assert_eq!(read.failed.len(), 1);
	assert_eq!(
		read.failed[0].failure,
		Failure {
			by: FailedBy::Recorder,
			exit: Some(1)
		}
	);
	let waiting: Vec<(&str, &str)> = read.waiting.iter().map(|e| (e.feed.as_str(), e.key.as_str())).collect();
	// The worker's order: the call's first, each feed's oldest first.
	assert_eq!(waiting, [("call", "2026-11-06"), ("playoff-delta", "2026-SF-09"), ("playoff-delta", "2026-SF-10")]);
}

#[test]
fn the_job_in_progress_is_not_also_waiting_or_failed() {
	let mut seen = call_seen();
	seen.failed.insert(String::from("2026-11-06"), (NOW, Failure { by: FailedBy::Home, exit: None }));
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
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("call"),
			on: String::from("2026-11-13")
		}]
	);
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
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("playoff-delta"),
			on: String::from("2026-11-10")
		}]
	);
}

#[test]
fn a_feed_quiet_for_weeks_expects_nothing() {
	let read = board(&[(Feed::PlayoffDelta, pd_seen(NOW - (ACTIVE_DAYS + 1) * 86_400))], None, None, NOW);
	assert_eq!(read.coming, []);
	assert_eq!(board(&[(Feed::Call, FeedSeen::default())], None, None, NOW).coming, []);
}

#[test]
fn a_failed_attempt_is_a_code_never_its_words() {
	// As `fail()` writes it: the header, then each problem, then the worker's exit.
	let refused = "recorded 00-open\nThese scenes don't fit the frame:\nscene 07-said-by-someone\nthe recorder exited 1\n";
	assert_eq!(
		failure_in(refused),
		Failure {
			by: FailedBy::Recorder,
			exit: Some(1)
		}
	);
	let crashed = "TypeError: x\n    at y\nNode.js v22.23.4\nthe recorder exited 134\n";
	assert_eq!(
		failure_in(crashed),
		Failure {
			by: FailedBy::Recorder,
			exit: Some(134)
		}
	);
	let home = "the finished call of 2026-11-06 could not be moved into /renders/call\n";
	assert_eq!(failure_in(home), Failure { by: FailedBy::Home, exit: None });
	let paused = "the recorder exited 1 soon after a pause; it runs again from its kept steps, not counted as a failure\nthe recorder exited 2\n";
	assert_eq!(failure_in(paused).exit, Some(2));
	assert_eq!(
		failure_in(""),
		Failure {
			by: FailedBy::Recorder,
			exit: None
		}
	);
	let json = serde_json::to_value(Failed {
		feed: String::from("call"),
		key: String::from("2026-11-06"),
		at: 1,
		failure: failure_in(refused),
	})
	.unwrap();
	assert_eq!(json, serde_json::json!({ "feed": "call", "key": "2026-11-06", "at": 1, "by": "recorder", "exit": 1 }));
	let json = serde_json::to_value(Failed {
		feed: String::from("call"),
		key: String::from("2026-11-06"),
		at: 1,
		failure: failure_in(home),
	})
	.unwrap();
	assert_eq!(json, serde_json::json!({ "feed": "call", "key": "2026-11-06", "at": 1, "by": "home", "exit": null }));
}

#[test]
fn the_cadence_counts_episodes_let_go() {
	// The owner let every week go once its link was recorded: the feed is
	// still on its cadence.
	let seen = FeedSeen {
		let_go: BTreeSet::from([String::from("2026-10-30"), String::from("2026-11-06")]),
		newest_week: NaiveDate::from_ymd_opt(2026, 11, 6),
		newest_taken: Some(NOW - 2 * 86_400),
		..FeedSeen::default()
	};
	assert_eq!(board(&[(Feed::Call, seen)], None, None, NOW).coming[0].on, "2026-11-13");
}

#[test]
fn waiting_is_in_the_worker_s_order_by_key() {
	let pd = FeedSeen {
		// Taken in the other order from their keys'.
		held: vec![(String::from("2026-SF-10"), NOW - 100), (String::from("2026-KC-10"), NOW - 50)],
		newest_taken: Some(NOW - 50),
		..FeedSeen::default()
	};
	let read = board(&[(Feed::PlayoffDelta, pd)], None, None, NOW);
	let keys: Vec<&str> = read.waiting.iter().map(|e| e.key.as_str()).collect();
	assert_eq!(keys, ["2026-KC-10", "2026-SF-10"]);
}

#[test]
fn home_s_day_is_home_s_near_midnight() {
	// 2026-11-11 07:30 UTC is Tuesday Nov 10, 23:30 at home. A fill taken
	// Monday expects the next today, Tuesday Nov 10: by UTC's day it would
	// be a week later.
	let late = 1_794_382_200;
	let pd = FeedSeen {
		held: vec![(String::from("2026-SF-10"), 1_794_254_400)],
		newest_taken: Some(1_794_254_400),
		..FeedSeen::default()
	};
	let read = board(&[(Feed::PlayoffDelta, pd)], None, None, late);
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("playoff-delta"),
			on: String::from("2026-11-10")
		}]
	);
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
		for (from, to, taken_at) in [("2026-10-30", "2026-11-05", "2026-10-31T12:00:00Z"), ("2026-11-06", "2026-11-12", "2026-11-07T12:00:00Z")] {
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
	let read = read(&db, &folders, "", NOW).await.unwrap();
	let now = read.now.unwrap();
	assert_eq!((now.key.as_str(), now.state, now.held_by), ("2026-11-06", Phase::Paused, Some(HeldBy::Busy)));
	assert!((now.estimate.unwrap().share - 0.5).abs() < 1e-9);
	assert_eq!(read.done.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["2026-10-30"]);
	assert_eq!(read.waiting, []);
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("call"),
			on: String::from("2026-11-13")
		}]
	);
}
