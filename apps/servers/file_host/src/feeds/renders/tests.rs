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
fn a_worker_gone_mid_render_reads_silent_within_alive_stale() {
	let feeds = [(Feed::Call, FeedSeen::default())];
	let run = decided("run", "clear", NOW, false);
	let busy = lane("busy", NOW);
	let held = |state: &str, alive: Option<i64>, lane: &Lane| {
		let mut job = job(state, NOW - 600, 0);
		job.alive = alive;
		board(&feeds, Some(&job), Some(lane), NOW).now.unwrap().held_by
	};
	assert_eq!(held("rendering", Some(NOW - ALIVE_STALE), &run), None, "a stamp at the bound is alive");
	assert_eq!(
		held("rendering", Some(NOW - ALIVE_STALE - 1), &run),
		Some(HeldBy::Silent),
		"the lane's run says nothing of a worker gone"
	);
	assert_eq!(held("paused", Some(NOW - 5), &busy), Some(HeldBy::Busy));
	assert_eq!(held("paused", Some(NOW - 600), &busy), Some(HeldBy::Silent));
	assert_eq!(
		held("rendering", None, &run),
		None,
		"a worker from before #739 stamps none, and is judged by the lane alone"
	);
	assert_eq!(
		held("waiting", Some(NOW - 600), &run),
		Some(HeldBy::Clear),
		"a job waiting is the lane's; its worker stamps last-pass"
	);
	assert_eq!(held("done", Some(NOW - 600), &run), None);
}

#[test]
fn a_sign_of_life_is_read_whole_or_not_at_all() {
	assert_eq!(parse_alive("id=17\nat=1794139200\n"), Some((String::from("17"), 1_794_139_200)));
	for broken in ["id=17\n", "at=1\n", "id=17\nat=soon\n", ""] {
		assert_eq!(parse_alive(broken), None, "{broken}");
	}
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
			at: NOW - 15 * 86_400,
			template: None,
			part: None,
			expires_at: None,
			bytes: None,
			duration_ms: None,
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

/// A week of Rust Rounds held, its essay's Monday (paulgsc/server#890).
fn round(week: &str, monday: (i32, u32, u32)) -> (String, NaiveDate) {
	(String::from(week), NaiveDate::from_ymd_opt(monday.0, monday.1, monday.2).unwrap())
}

#[test]
fn a_week_s_essay_is_written_the_monday_after_its_due_at_home() {
	let window = |due: &str| rust_round_repo::week::Window {
		opens: String::from("2026-11-09T00:00:00-08:00"),
		due: String::from(due),
	};
	let monday = NaiveDate::from_ymd_opt(2026, 11, 16);
	assert_eq!(essay_monday(&window("2026-11-15T23:59:00-08:00")), monday);
	// The same instant in UTC is still Sunday at home.
	assert_eq!(essay_monday(&window("2026-11-16T07:59:00Z")), monday);
	assert_eq!(essay_monday(&window("Sunday")), None, "a due that is no instant promises nothing");
}

#[test]
fn the_rust_essay_comes_the_monday_after_its_round_even_before_any_essay() {
	// NOW is Sunday Nov 8 at home: 2026-w45 closes tonight, its essay Monday.
	let seen = FeedSeen {
		rounds: vec![round("2026-w45", (2026, 11, 9))],
		..FeedSeen::default()
	};
	assert_eq!(seen.newest_taken, None, "no essay was ever taken: the round alone promises it");
	let read = board(&[(Feed::RustEssay, seen)], None, None, NOW);
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("rust-essay"),
			on: String::from("2026-11-09")
		}]
	);
}

#[test]
fn the_rust_essay_taken_promises_the_next_round_s_monday_and_a_past_one_none() {
	let mut seen = FeedSeen {
		held: vec![(String::from("2026-w45"), NOW - 3_600)],
		newest_taken: Some(NOW - 3_600),
		rounds: vec![round("2026-w46", (2026, 11, 16)), round("2026-w45", (2026, 11, 9))],
		..FeedSeen::default()
	};
	assert_eq!(board(&[(Feed::RustEssay, seen.clone())], None, None, NOW).coming[0].on, "2026-11-16");
	// Let go is taken too.
	seen.held.clear();
	seen.let_go.insert(String::from("2026-w45"));
	assert_eq!(board(&[(Feed::RustEssay, seen.clone())], None, None, NOW).coming[0].on, "2026-11-16");
	// A Monday gone by with no newer round open: nothing is owed on a day.
	seen.rounds = vec![round("2026-w44", (2026, 11, 2))];
	assert_eq!(board(&[(Feed::RustEssay, seen)], None, None, NOW).coming, []);
}

#[test]
fn the_interview_essay_comes_the_monday_after_its_week_even_before_any_essay() {
	// NOW is Sunday Nov 8 at home: the interview week closes tonight, its essay Monday.
	let seen = FeedSeen {
		rounds: vec![round("2026-w45", (2026, 11, 9))],
		..FeedSeen::default()
	};
	let read = board(&[(Feed::InterviewEssay, seen.clone())], None, None, NOW);
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("interview-essay"),
			on: String::from("2026-11-09")
		}]
	);
	// Once its essay is taken, nothing newer is owed on a day.
	let taken = FeedSeen {
		held: vec![(String::from("2026-w45"), NOW - 3_600)],
		newest_taken: Some(NOW - 3_600),
		..seen
	};
	assert_eq!(board(&[(Feed::InterviewEssay, taken)], None, None, NOW).coming, []);
}

#[test]
fn a_cards_essay_is_expected_on_no_day() {
	let seen = FeedSeen {
		held: vec![(String::from("zfs-2026-11-02"), NOW - 3_600)],
		newest_taken: Some(NOW - 3_600),
		..FeedSeen::default()
	};
	assert_eq!(board(&[(Feed::CardsEssay, seen)], None, None, NOW).coming, []);
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
		template: None,
		part: None,
		reason: None,
		over: None,
		blocking: None,
		scene_names: None,
	})
	.unwrap();
	assert_eq!(json, serde_json::json!({ "feed": "call", "key": "2026-11-06", "at": 1, "by": "recorder", "exit": 1 }));
	let json = serde_json::to_value(Failed {
		feed: String::from("call"),
		key: String::from("2026-11-06"),
		at: 1,
		failure: failure_in(home),
		template: None,
		part: None,
		reason: None,
		over: None,
		blocking: None,
		scene_names: None,
	})
	.unwrap();
	assert_eq!(json, serde_json::json!({ "feed": "call", "key": "2026-11-06", "at": 1, "by": "home", "exit": null }));
}

/// #929: a refusal by the render policy reaches the phone as a closed
/// reason with the recorder's numbers, never its words; anything not of the
/// worker's shape is no refusal.
#[test]
fn a_policy_refusal_is_its_reason_and_numbers() {
	let long = Over::parse("reason=too-long\nminutes=24.4\ncap=30.0\nheld=ab12\n").unwrap();
	assert_eq!(long, Over::Long { minutes: 244, cap_minutes: 300 });
	assert_eq!(long.reason(), "too-long");
	let large = Over::parse("reason=too-large\nmb=512\ncap=500\nheld=ab12\n").unwrap();
	assert_eq!(large, Over::Large { mb: 512, cap_mb: 500 });
	for text in [
		"",
		"reason=too-long\nminutes=24.4\n",
		"reason=too-long\nminutes=24.45\ncap=30.0\n",
		"reason=too-long\nminutes=-1\ncap=30\n",
		"reason=too-large\nmb=5.5\ncap=500\n",
		"reason=too-large\nmb=99999999\ncap=500\n",
		"reason=the video would run 24.4 min\nminutes=24.4\ncap=30\n",
		"reason=overflow\nscenes=\ncount=0\n",
		"reason=overflow\nscenes=dive-2,beside-2\ncount=1\n",
		"reason=overflow\nscenes=dive-2\n",
		"reason=overflow\nscenes=a scene, of words\ncount=2\n",
		"reason=overflow\nscenes=-dive\ncount=1\n",
		"reason=overflow\nscenes=a,b,c,d,e,f,g,h,i,j,k,l,m\ncount=13\n",
		"reason=overflow\nscenes=a,b,c\ncount=14\n",
	] {
		assert_eq!(Over::parse(text), None, "{text:?}");
	}
	// #975: scenes past the frame, by id, of how many; past twelve, the first
	// twelve.
	let twelve = (1..=12).map(|i| ["s", &i.to_string()].concat()).collect::<Vec<_>>().join(",");
	assert!(matches!(
		Over::parse(&["reason=overflow\nscenes=", &twelve, "\ncount=14\n"].concat()),
		Some(Over::Overflow { scene_count: 14, .. })
	));
	let overflow = Over::parse("reason=overflow\nscenes=dive-2,beside-2,dive-3\ncount=3\nheld=ab12\n").unwrap();
	assert_eq!(
		overflow,
		Over::Overflow {
			scenes: vec![String::from("dive-2"), String::from("beside-2"), String::from("dive-3")],
			scene_count: 3,
		}
	);
	assert_eq!(overflow.reason(), "overflow");
	let failed = |over: Over| Failed {
		feed: String::from("rust-essay"),
		key: String::from("2026-w46"),
		at: 1,
		failure: Failure {
			by: FailedBy::Recorder,
			exit: Some(1),
		},
		template: None,
		part: None,
		reason: Some(over.reason()),
		over: Some(over),
		blocking: None,
		scene_names: None,
	};
	assert_eq!(
		serde_json::to_value(failed(long)).unwrap(),
		serde_json::json!({ "feed": "rust-essay", "key": "2026-w46", "at": 1, "by": "recorder", "exit": 1, "reason": "too-long", "minutes": 24.4, "capMinutes": 30.0 })
	);
	assert_eq!(
		serde_json::to_value(failed(large)).unwrap(),
		serde_json::json!({ "feed": "rust-essay", "key": "2026-w46", "at": 1, "by": "recorder", "exit": 1, "reason": "too-large", "mb": 512, "capMb": 500 })
	);
	assert_eq!(
		serde_json::to_value(failed(overflow)).unwrap(),
		serde_json::json!({ "feed": "rust-essay", "key": "2026-w46", "at": 1, "by": "recorder", "exit": 1, "reason": "overflow", "scenes": ["dive-2", "beside-2", "dive-3"], "sceneCount": 3 })
	);
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
		feeds: vec![(Feed::Call, renders.clone(), status.clone()), (Feed::PlayoffDelta, pd_renders, pd_status)],
		clip_walks: None,
		previews: None,
		cards_essays: None,
		lane: status.clone(),
	};
	// Another job's stale sign of life is not this one's.
	std::fs::write(status.join("job.alive"), b"id=17\nat=1794000000\n").unwrap();
	let read_now = || async { read(&db, &folders, "", NOW).await.unwrap() };
	assert_eq!(read_now().await.now.unwrap().held_by, Some(HeldBy::Busy));
	std::fs::write(status.join("job.alive"), b"id=1794000000000000000\nat=1794000000\n").unwrap();
	assert_eq!(read_now().await.now.unwrap().held_by, Some(HeldBy::Silent), "its own, gone stale");
	std::fs::write(status.join("job.alive"), b"id=1794000000000000000\nat=1794139195\n").unwrap();
	let read = read_now().await;
	let now = read.now.unwrap();
	assert_eq!((now.key.as_str(), now.state, now.held_by), ("2026-11-06", Phase::Paused, Some(HeldBy::Busy)));
	assert!((now.estimate.unwrap().share - 0.5).abs() < 1e-9);
	assert_eq!(read.done.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["2026-10-30"]);
	assert_eq!(read.done[0].bytes, Some(5), "a done call's video says its size");
	assert_eq!(read.done[0].duration_ms, None, "a file that isn't an mp4 says no length");
	let mp4 = crate::feeds::mp4::tests::mp4(0, 1_000, 61_400, true);
	std::fs::write(renders.join("2026-10-30").join("call.mp4"), &mp4).unwrap();
	let made = read_now().await;
	assert_eq!((made.done[0].bytes, made.done[0].duration_ms), (Some(u64::try_from(mp4.len()).unwrap()), Some(61_400)));
	assert_eq!(
		serde_json::to_value(&made.done[0]).unwrap()["durationMs"],
		serde_json::json!(61_400),
		"the phone reads it as `durationMs`"
	);
	// A feed's download follows a link to a plain file (`feeds::videos`), so
	// its size is the file the link reaches.
	let video = renders.join("2026-10-30").join("call.mp4");
	let elsewhere = dir.path().join("elsewhere.mp4");
	std::fs::write(&elsewhere, b"the linked video").unwrap();
	std::fs::remove_file(&video).unwrap();
	std::os::unix::fs::symlink(&elsewhere, &video).unwrap();
	assert_eq!(read_now().await.done[0].bytes, Some(16), "a linked call video's size is what the download serves");
	assert_eq!(read.waiting, []);
	assert_eq!(
		read.coming,
		[Coming {
			feed: String::from("call"),
			on: String::from("2026-11-13")
		}]
	);
}

#[tokio::test]
async fn a_clips_walks_are_on_the_board_as_episodes_are() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (given, renders, status, lane) = (
		dir.path().join("clip-walks"),
		dir.path().join("clip-walk"),
		dir.path().join("clip-walk-worker"),
		dir.path().join("call-worker"),
	);
	for folder in [&renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	// Three walks given, and a folder that isn't one; one walked, one failed.
	for key in ["ledger-wake-after", "ledger-wake-before", "other-after", "not-a-walk"] {
		std::fs::create_dir_all(given.join(key)).unwrap();
	}
	std::fs::create_dir_all(renders.join("ledger-wake-after")).unwrap();
	std::fs::write(renders.join("ledger-wake-after/walk.json"), b"{}").unwrap();
	std::fs::write(status.join("ledger-wake-before.failed.log"), b"the recorder exited 1\n").unwrap();
	let job = worker_text("rendering", NOW - 10, 0)
		.replace("kind=call", "kind=clip-walk")
		.replace("key=2026-11-06", "key=other-after");
	std::fs::write(lane.join("job.state"), job).unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: Some((given, renders, status)),
		previews: None,
		cards_essays: None,
		lane,
	};
	let read = read(&db, &folders, "", NOW).await.unwrap();
	let now = read.now.unwrap();
	assert_eq!((now.feed.as_str(), now.key.as_str()), ("clip-walk", "other-after"));
	assert_eq!(
		read.done.iter().map(|e| (e.feed.as_str(), e.key.as_str())).collect::<Vec<_>>(),
		[("clip-walk", "ledger-wake-after")]
	);
	assert_eq!(read.done[0].bytes, None, "a walk's walk.json is no video: no size");
	assert_eq!(read.failed.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["ledger-wake-before"]);
	assert_eq!(read.waiting, [], "the walk in progress is not also waiting, and what isn't a walk is not read");
	assert_eq!(read.coming, [], "a walk is expected on no day");
}

/// Each cards essay home holds is on the board as an episode is: done once
/// its `cards.mp4` is made, failed with its log, or waiting (CARDS7, #854).
#[tokio::test]
async fn the_cards_essays_home_holds_are_on_the_board_as_episodes_are() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (renders, status, lane) = (dir.path().join("cards-essay"), dir.path().join("cards-essay-worker"), dir.path().join("call-worker"));
	for folder in [&renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	{
		let mut conn = db.acquire().await.unwrap();
		for key in ["zfs-2026-11-02", "nixos-upgrade-2026-11-07", "rust-scratchpad-2026-11-08"] {
			let essay = cards_essay_repo::Essay {
				key,
				day: &key[key.len() - 10..],
				body: b"{}",
				taken_at: "2026-11-08T10:00:00.000Z",
			};
			cards_essay_repo::keep(&mut conn, &essay).await.unwrap();
		}
	}
	std::fs::create_dir_all(renders.join("zfs-2026-11-02")).unwrap();
	std::fs::write(renders.join("zfs-2026-11-02").join(crate::cards_essays::VIDEO), b"video").unwrap();
	std::fs::write(status.join("nixos-upgrade-2026-11-07.failed.log"), b"C1: its picture is a capture, not a card\n").unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: None,
		previews: None,
		cards_essays: Some((renders.clone(), status)),
		lane,
	};
	let read = read(&db, &folders, "", NOW).await.unwrap();
	let shelf = |episodes: &[Episode]| episodes.iter().map(|e| (e.feed.clone(), e.key.clone())).collect::<Vec<_>>();
	assert_eq!(shelf(&read.done), [(String::from("cards-essay"), String::from("zfs-2026-11-02"))]);
	// Its size, from the stat that dates it: the total the phone's ranged
	// fetch counts against.
	assert_eq!(read.done[0].bytes, Some(5));
	assert_eq!(serde_json::to_value(&read.done[0]).unwrap()["bytes"], serde_json::json!(5), "the phone reads it as `bytes`");
	assert_eq!(read.failed.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["nixos-upgrade-2026-11-07"]);
	assert_eq!(
		(read.failed[0].reason, read.failed[0].over.clone()),
		(None, None),
		"a failure not the policy's says no reason"
	);
	assert_eq!(shelf(&read.waiting), [(String::from("cards-essay"), String::from("rust-scratchpad-2026-11-08"))]);
	assert_eq!(read.waiting[0].bytes, None, "a waiting essay has no video yet");
	assert_eq!(read.waiting[0].template, None, "an essay names no template");
	assert_eq!(read.coming, [], "a cards essay is expected on no day: a session writes one on request");
	// A link where the video goes is not one the download serves: done, as
	// before, with no size.
	let video = renders.join("zfs-2026-11-02").join(crate::cards_essays::VIDEO);
	let elsewhere = dir.path().join("elsewhere.mp4");
	std::fs::write(&elsewhere, b"not the video").unwrap();
	std::fs::remove_file(&video).unwrap();
	std::os::unix::fs::symlink(&elsewhere, &video).unwrap();
	let read = super::read(&db, &folders, "", NOW).await.unwrap();
	assert_eq!(shelf(&read.done), [(String::from("cards-essay"), String::from("zfs-2026-11-02"))]);
	assert_eq!(read.done[0].bytes, None, "a link's size is not the video's");
	// #929: one the policy refused says so, on any shelf, from the worker's
	// `.refused`; one past the file's ceiling is read as no refusal.
	let status = dir.path().join("cards-essay-worker");
	let refused = status.join("nixos-upgrade-2026-11-07.refused");
	std::fs::write(&refused, b"reason=too-long\nminutes=24.4\ncap=15.0\nheld=ab12\n").unwrap();
	let read = super::read(&db, &folders, "", NOW).await.unwrap();
	assert_eq!(
		(read.failed[0].reason, read.failed[0].over.clone()),
		(Some("too-long"), Some(Over::Long { minutes: 244, cap_minutes: 150 }))
	);
	// #975: as does one whose scenes don't fit the frame.
	std::fs::write(&refused, b"reason=overflow\nscenes=dive-2,beside-2\ncount=2\nheld=ab12\n").unwrap();
	let read = super::read(&db, &folders, "", NOW).await.unwrap();
	assert_eq!(
		(read.failed[0].reason, read.failed[0].over.clone()),
		(
			Some("overflow"),
			Some(Over::Overflow {
				scenes: vec![String::from("dive-2"), String::from("beside-2")],
				scene_count: 2
			})
		)
	);
	std::fs::write(&refused, "reason=too-long\nminutes=24.4\ncap=15.0\n".repeat(1000)).unwrap();
	let read = super::read(&db, &folders, "", NOW).await.unwrap();
	assert_eq!(read.failed[0].over, None, "a file past its ceiling is no refusal");
}

/// The read reads the weeks of Rust Rounds the caller holds for the essay's
/// day: only theirs, only a week that reads as its name (paulgsc/server#890).
#[tokio::test]
async fn the_read_promises_the_rust_essay_from_the_caller_s_open_round() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (renders, status, lane) = (dir.path().join("rust-essay"), dir.path().join("rust-essay-worker"), dir.path().join("call-worker"));
	for folder in [&renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	// A week under `named`'s name whose body says `id`, due at `due` when it has one.
	let hold_as = |subject: &'static str, seq: i64, named: &'static str, id: &'static str, due: Option<&'static str>| {
		let db = db.clone();
		async move {
			let mut week = serde_json::json!({
				"schema": "some-ui.rust-week/1", "id": id,
				"tickets": [{ "id": "QM-311", "program": "fn main() {}\n" }],
			});
			if let Some(due) = due {
				week["opens"] = serde_json::json!("2026-11-02T00:00:00-08:00");
				week["due"] = serde_json::json!(due);
			}
			let body = week.to_string();
			let sha256 = rust_round_repo::content_hash(body.as_bytes());
			let name = rust_round_repo::week::mailbox_name(named);
			let item = mailbox_repo::ItemIn {
				folder: "brief",
				name: &name,
				seq,
				sha256: &sha256,
				body: body.as_bytes(),
			};
			mailbox_repo::keep_item(&mut db.acquire().await.unwrap(), subject, item, "2026-11-02T08:00:00.000Z")
				.await
				.unwrap();
		}
	};
	let folders = Folders {
		feeds: vec![(Feed::RustEssay, renders, status)],
		clip_walks: None,
		previews: None,
		cards_essays: None,
		lane,
	};
	for subject in ["me", "someone-else"] {
		sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES (?1, ?1)")
			.bind(subject)
			.execute(&db)
			.await
			.unwrap();
	}
	assert_eq!(read(&db, &folders, "me", NOW).await.unwrap().coming, [], "no round held: nothing promised");
	let due = Some("2026-11-08T23:59:00-08:00");
	hold_as("someone-else", 1, "2026-w45", "2026-w45", due).await;
	assert_eq!(read(&db, &folders, "me", NOW).await.unwrap().coming, [], "another subject's round is not the caller's");
	hold_as("me", 2, "2026-w45", "2026-w46", due).await;
	assert_eq!(
		read(&db, &folders, "me", NOW).await.unwrap().coming,
		[],
		"a file that is not the week its name says is no round"
	);
	hold_as("me", 3, "2026-w44", "2026-w44", None).await;
	assert_eq!(read(&db, &folders, "me", NOW).await.unwrap().coming, [], "a week with no window owes no essay");
	hold_as("me", 4, "2026-w46", "2026-w46", Some("2026-11-15T23:59:00-08:00")).await;
	assert_eq!(
		read(&db, &folders, "me", NOW).await.unwrap().coming,
		[Coming {
			feed: String::from("rust-essay"),
			on: String::from("2026-11-16")
		}]
	);
}

#[tokio::test]
async fn the_read_promises_the_interview_essay_from_the_caller_s_open_week() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (renders, status, lane) = (
		dir.path().join("interview-essay"),
		dir.path().join("interview-essay-worker"),
		dir.path().join("call-worker"),
	);
	for folder in [&renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	sqlx::query("INSERT INTO account (subject_id, user_handle) VALUES ('me', 'me')").execute(&db).await.unwrap();
	let folders = Folders {
		feeds: vec![(Feed::InterviewEssay, renders, status)],
		clip_walks: None,
		previews: None,
		cards_essays: None,
		lane,
	};
	assert_eq!(read(&db, &folders, "me", NOW).await.unwrap().coming, [], "no week held: nothing promised");
	let body = serde_json::json!({
		"schema": "some-ui.interview-week/1", "id": "2026-w45",
		"opens": "2026-11-02T00:00:00-08:00", "due": "2026-11-08T23:59:00-08:00",
		"company": { "name": "Fernbrook Labs" }, "role": "Platform engineer",
		"questions": [{ "id": "about-you", "family": "about-you", "prompt": "Tell me about yourself.", "targetSeconds": 90 }],
		"cards": [{ "id": "home-first", "script": "I built home first because.", "targetSeconds": 90 }],
	})
	.to_string();
	let name = rust_round_repo::interview::mailbox_name("2026-w45");
	let sha256 = rust_round_repo::content_hash(body.as_bytes());
	let item = mailbox_repo::ItemIn {
		folder: "brief",
		name: &name,
		seq: 1,
		sha256: &sha256,
		body: body.as_bytes(),
	};
	mailbox_repo::keep_item(&mut db.acquire().await.unwrap(), "me", item, "2026-11-02T08:00:00.000Z")
		.await
		.unwrap();
	assert_eq!(
		read(&db, &folders, "me", NOW).await.unwrap().coming,
		[Coming {
			feed: String::from("interview-essay"),
			on: String::from("2026-11-09")
		}]
	);
}

#[tokio::test]
async fn a_template_s_previews_are_on_the_board_while_home_keeps_them() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (requests, renders, status, lane) = (
		dir.path().join("preview-requests"),
		dir.path().join("preview"),
		dir.path().join("preview-worker"),
		dir.path().join("call-worker"),
	);
	for folder in [&requests, &renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	let at = |secs: i64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(secs).unwrap());
	let touched = |path: &std::path::Path, secs: i64| std::fs::File::options().write(true).open(path).unwrap().set_modified(at(secs)).unwrap();
	// NOW is 2026-11-08T12:00:00Z.
	let (ready, expired, broken, overdue, queued, running) = (
		"conference-call-20261108T060000Z",
		"rust-essay-20261106T060000Z",
		"playoff-delta-week-20261108T061000Z",
		"playoff-delta-legend-20261107T050000Z",
		"rust-essay-20261108T070000Z",
		"conference-call-20261108T074500Z",
	);
	for id in [ready, expired, broken, queued, running] {
		std::fs::write(requests.join(String::from(id) + ".json"), b"{}").unwrap();
	}
	std::fs::write(requests.join(String::from(overdue) + ".overdue"), b"{}").unwrap();
	touched(&requests.join(String::from(overdue) + ".overdue"), NOW - 600);
	// Neither a preview's id nor a whole request: never read.
	std::fs::write(requests.join("podcast-20261108T060000Z.json"), b"{}").unwrap();
	std::fs::write(requests.join(String::from(".") + queued + ".json"), b"{}").unwrap();
	for (id, when) in [(ready, NOW - 3_600), (expired, NOW - 86_400 - 1)] {
		std::fs::create_dir_all(renders.join(id)).unwrap();
		std::fs::write(renders.join(id).join("preview.mp4"), b"mp4").unwrap();
		touched(&renders.join(id).join("preview.mp4"), when);
	}
	std::fs::write(status.join(String::from(broken) + ".failed.log"), b"the recorder exited 3\n").unwrap();
	touched(&status.join(String::from(broken) + ".failed.log"), NOW - 1_200);
	std::fs::write(status.join("took"), b"400\n").unwrap();
	let job = worker_text("rendering", NOW - 100, 100)
		.replace("kind=call", "kind=preview")
		.replace("key=2026-11-06", &(String::from("key=") + running));
	std::fs::write(lane.join("job.state"), job).unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: None,
		previews: Some((requests, renders, status)),
		cards_essays: None,
		lane,
	};
	let read = read(&db, &folders, "", NOW).await.unwrap();
	let now = read.now.unwrap();
	assert_eq!((now.feed.as_str(), now.key.as_str()), ("preview", running));
	assert!((now.estimate.unwrap().share - 0.5).abs() < 1e-9, "its bar is the last preview's time");
	let rows = |rows: &[Episode]| rows.iter().map(|e| (e.feed.clone(), e.key.clone())).collect::<Vec<_>>();
	let preview = |key: &str| (String::from("preview"), String::from(key));
	assert_eq!(rows(&read.done), [preview(ready)], "one past its day is off the board");
	assert!(
		read.failed.iter().all(|f| f.key != expired) && read.waiting.iter().all(|e| e.key != expired),
		"one made and past its day is gone, never failed: only one never made is"
	);
	assert_eq!(rows(&read.waiting), [preview(queued)]);
	assert_eq!(
		read.failed.iter().map(|f| (f.key.as_str(), f.at, f.failure)).collect::<Vec<_>>(),
		[
			(overdue, NOW - 600, Failure { by: FailedBy::Home, exit: None }),
			(
				broken,
				NOW - 1_200,
				Failure {
					by: FailedBy::Recorder,
					exit: Some(3)
				}
			),
		]
	);
	assert_eq!(read.coming, [], "a preview is expected on no day");
	// What the phone links and says by: the template, a done one's day, a
	// failed one's class.
	assert_eq!(now.template.as_deref(), Some("conference-call"));
	assert_eq!(
		serde_json::to_value(&read.done[0]).unwrap(),
		serde_json::json!({ "feed": "preview", "key": ready, "at": NOW - 3_600, "template": "conference-call", "expiresAt": NOW - 3_600 + 86_400, "bytes": 3 })
	);
	assert_eq!(
		serde_json::to_value(&read.waiting[0]).unwrap(),
		serde_json::json!({ "feed": "preview", "key": queued, "at": NOW - 5 * 3_600, "template": "rust-essay" })
	);
	assert_eq!(
		read.failed.iter().map(|f| (f.template.as_deref(), f.reason)).collect::<Vec<_>>(),
		[(Some("playoff-delta-legend"), Some("overdue")), (Some("playoff-delta-week"), Some("render"))]
	);
}

#[tokio::test]
async fn a_preview_past_its_deadline_is_failed_whatever_the_worker_last_wrote() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (requests, renders, status, lane) = (
		dir.path().join("preview-requests"),
		dir.path().join("preview"),
		dir.path().join("preview-worker"),
		dir.path().join("call-worker"),
	);
	for folder in [&requests, &renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	let overdue = "conference-call-20261107T050000Z";
	std::fs::write(requests.join(String::from(overdue) + ".overdue"), b"{}").unwrap();
	// One refused by the policy past its deadline, with no video: overdue
	// says why, alone (#929).
	let refused = "conference-call-20261107T060000Z";
	std::fs::write(requests.join(String::from(refused) + ".overdue"), b"{}").unwrap();
	std::fs::write(status.join(String::from(refused) + ".failed.log"), b"the recorder exited 1\n").unwrap();
	std::fs::write(status.join(String::from(refused) + ".refused"), b"reason=too-large\nmb=512\ncap=500\nheld=ab12\n").unwrap();
	// A render that landed after the deadline: `previews::read` calls it
	// failed, and so does the board.
	std::fs::create_dir_all(renders.join(overdue)).unwrap();
	std::fs::write(renders.join(overdue).join("preview.mp4"), b"mp4").unwrap();
	let job = worker_text("rendering", NOW - 100, 100)
		.replace("kind=call", "kind=preview")
		.replace("key=2026-11-06", &(String::from("key=") + overdue));
	std::fs::write(lane.join("job.state"), job).unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: None,
		previews: Some((requests, renders, status)),
		cards_essays: None,
		lane,
	};
	let read = read(&db, &folders, "", NOW).await.unwrap();
	assert_eq!(read.now, None, "the worker lets an overdue preview go");
	// By key: each row's time is its marker's, which may fall either side
	// of a second.
	let mut rows = read.failed.iter().map(|f| (f.key.as_str(), f.reason, f.over.clone())).collect::<Vec<_>>();
	rows.sort_by_key(|row| row.0);
	assert_eq!(
		rows,
		[(overdue, Some("overdue"), None), (refused, Some("overdue"), None)],
		"an overdue preview carries no refusal's numbers"
	);
	assert_eq!(read.done, [], "its late video is not done");
}

#[test]
fn progress_reads_as_the_worker_keeps_it() {
	let text = "id=1794000000000000000\nstation=recording\nscene=5\nscenes=22\nsince=1794139000\n";
	assert_eq!(
		Progress::parse(text),
		Some((
			String::from("1794000000000000000"),
			Progress {
				station: Station::Recording,
				scene: Some(5),
				scenes: Some(22),
				since: 1_794_139_000,
			}
		))
	);
	let untold = "id=7\nstation=assembling\nscene=3\nscenes=\nsince=1\n";
	assert_eq!(
		Progress::parse(untold).map(|(_, p)| (p.station, p.scenes)),
		Some((Station::Assembling, None)),
		"a recorder that never said how many"
	);
	for broken in [
		"id=7\nstation=encoding\nscene=3\nscenes=\nsince=1\n",
		"id=7\nstation=recording\nscene=x\nscenes=\nsince=1\n",
		"station=recording\nscene=3\nscenes=\nsince=1\n",
	] {
		assert_eq!(Progress::parse(broken), None, "{broken}");
	}
}

#[test]
fn progress_is_shown_only_while_the_job_renders_or_is_paused() {
	let progress = Progress {
		station: Station::Recording,
		scene: Some(2),
		scenes: Some(6),
		since: NOW - 30,
	};
	for (state, shown) in [("rendering", true), ("paused", true), ("waiting", false), ("done", false)] {
		let mut job = job(state, NOW - 60, 0);
		job.progress = Some(progress);
		let read = board(&[], Some(&job), Some(&decided("run", "clear", NOW, false)), NOW);
		assert_eq!(read.now.unwrap().progress, shown.then_some(progress), "{state}");
	}
	let json = serde_json::to_value(progress).unwrap();
	assert_eq!(json, serde_json::json!({ "station": "recording", "scene": 2, "scenes": 6, "since": NOW - 30 }));
}

#[tokio::test]
async fn a_run_s_progress_is_its_job_s_only() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let lane = dir.path().join("call-worker");
	std::fs::create_dir_all(&lane).unwrap();
	std::fs::write(lane.join("job.state"), worker_text("rendering", NOW - 10, 0)).unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: None,
		previews: None,
		cards_essays: None,
		lane: lane.clone(),
	};
	assert_eq!(read(&db, &folders, "", NOW).await.unwrap().now.unwrap().progress, None, "the recorder has said nothing yet");
	std::fs::write(
		lane.join("job.progress"),
		"id=1794000000000000000\nstation=assembling\nscene=6\nscenes=6\nsince=1794139100\n",
	)
	.unwrap();
	let progress = read(&db, &folders, "", NOW).await.unwrap().now.unwrap().progress.unwrap();
	assert_eq!((progress.station, progress.scene, progress.scenes), (Station::Assembling, Some(6), Some(6)));
	std::fs::write(
		lane.join("job.progress"),
		"id=1793000000000000000\nstation=recording\nscene=6\nscenes=6\nsince=1794139100\n",
	)
	.unwrap();
	assert_eq!(read(&db, &folders, "", NOW).await.unwrap().now.unwrap().progress, None, "another job's");
}

/// A part of a call's week of more than one (paulgsc/some-ui#2918) carries
/// which part and of how many, now, waiting, failed or done: the phone's
/// "2026-11-06 · part 2 of 3". A week of one video's row carries neither.
#[test]
fn a_part_of_a_week_says_which_part_of_how_many() {
	let mut seen = FeedSeen {
		held: vec![
			(String::from("2026-11-06"), NOW - 86_400),
			(String::from("2026-11-06-p2"), NOW - 86_400),
			(String::from("2026-11-06-p3"), NOW - 86_400),
			(String::from("2026-10-30"), NOW - 8 * 86_400),
		],
		..FeedSeen::default()
	};
	seen.videos.insert(String::from("2026-11-06"), NOW - 3_600);
	for part in 1..=3 {
		let key = call_week_repo::video::part_key("2026-11-06", part);
		seen.parts.insert(key, (part, 3));
	}
	let job = Job::parse("id=7\nkind=call\nkey=2026-11-06-p2\nsince=1\nstate=rendering\nstate_since=1\nyields=0\nran=0\n").unwrap();
	let read = board(&[(Feed::Call, seen)], Some(&job), None, NOW);
	let now = read.now.unwrap();
	assert_eq!((now.key.as_str(), now.part), ("2026-11-06-p2", Some(PartOf { part: 2, of: 3 })));
	let json = serde_json::to_value(&now).unwrap();
	assert_eq!((json["part"].clone(), json["of"].clone()), (serde_json::json!(2), serde_json::json!(3)));
	assert_eq!(read.done[0].part, Some(PartOf { part: 1, of: 3 }));
	assert_eq!(
		read.waiting.iter().map(|e| (e.key.as_str(), e.part)).collect::<Vec<_>>(),
		[("2026-10-30", None), ("2026-11-06-p3", Some(PartOf { part: 3, of: 3 }))]
	);
	let json = serde_json::to_value(&read.waiting[0]).unwrap();
	assert!(json.get("part").is_none() && json.get("of").is_none(), "{json}");
}

/// A cards essay home refused before the render (paulgsc/server#1052) reads
/// as its class alone, and no other text does.
#[test]
fn a_home_refusal_reads_as_its_class() {
	assert_eq!(super::home_refusal("reason=take-missing\n"), Some("take-missing"));
	assert_eq!(super::home_refusal("reason=take-unheard\n"), Some("take-unheard"));
	assert_eq!(super::home_refusal("reason=snip-missing\n"), Some("snip-missing"));
	assert_eq!(super::home_refusal("reason=clip-missing\n"), Some("clip-missing"));
	assert_eq!(super::home_refusal("reason=snip-gone\n"), Some("snip-gone"));
	assert_eq!(super::home_refusal("reason=page-missing\n"), Some("page-missing"));
	assert_eq!(super::home_refusal("reason=page-gone\n"), Some("page-gone"));
	assert_eq!(super::home_refusal("reason=too-long\n"), None);
	assert_eq!(super::home_refusal("take-missing\n"), None);
	assert_eq!(Over::parse("reason=take-missing\n"), None, "never the policy's");
}

/// Run again (`renders_again`): a cards essay home refused names the items
/// that still block it, by id, as its `.refused` says them; one that names
/// none whole (written before it named any) is its reason alone.
#[test]
fn a_home_refusal_names_what_blocks_it() {
	let text = "reason=take-unheard\nitems=note-a,note-b\ncount=2\n";
	assert_eq!(home_refusal(text), Some("take-unheard"));
	assert_eq!(
		Blocking::parse(text),
		Some(Blocking {
			items: vec![String::from("note-a"), String::from("note-b")],
			item_count: 2,
			item_names: None,
		})
	);
	let frames = Blocking::parse("reason=clip-missing\nitems=clip-1/before/f001.png\ncount=1\n").unwrap();
	assert_eq!(frames.items, vec![String::from("clip-1/before/f001.png")]);
	let snip = Blocking::parse("reason=snip-missing\nitems=0123abcd web:/settings:phone:1.2.3:0123456789ab\ncount=1\n").unwrap();
	assert_eq!(snip.items, vec![String::from("0123abcd web:/settings:phone:1.2.3:0123456789ab")]);
	let twelve = (1..=12).map(|i| ["n", &i.to_string()].concat()).collect::<Vec<_>>().join(",");
	assert_eq!(
		Blocking::parse(&["reason=take-missing\nitems=", &twelve, "\ncount=20\n"].concat()).map(|b| b.item_count),
		Some(20)
	);
	for text in [
		"reason=take-missing\n",
		"reason=take-missing\nitems=\ncount=0\n",
		"reason=take-missing\nitems=a,b\ncount=1\n",
		"reason=take-missing\nitems=a\n",
		"reason=take-missing\nitems= padded\ncount=1\n",
		"reason=take-missing\nitems=a\tb\ncount=1\n",
		"reason=take-missing\nitems=caf\u{e9}\ncount=1\n",
	] {
		assert_eq!(Blocking::parse(text), None, "{text:?}");
	}
	let row = Failed {
		feed: String::from("cards-essay"),
		key: String::from("walk-2026-11-20"),
		at: 1,
		failure: Failure { by: FailedBy::Home, exit: None },
		template: None,
		part: None,
		reason: Some("take-unheard"),
		over: None,
		blocking: Blocking::parse(text),
		scene_names: None,
	};
	assert_eq!(
		serde_json::to_value(row).unwrap(),
		serde_json::json!({ "feed": "cards-essay", "key": "walk-2026-11-20", "at": 1, "by": "home", "exit": null, "reason": "take-unheard", "items": ["note-a", "note-b"], "itemCount": 2 })
	);
}

/// The owner's rule that a failure names its scene or part (RETRY): a cards
/// essay's overflow says its scenes, and a take refusal its takes, by the
/// names the essay's own cut list gives their place, never an id; absent
/// where the essay can't name every one.
#[tokio::test]
async fn a_cards_essays_failure_names_its_scenes_and_takes() {
	let db = pool().await;
	let dir = tempfile::tempdir().unwrap();
	let (renders, status, lane) = (dir.path().join("cards-essay"), dir.path().join("cards-essay-worker"), dir.path().join("call-worker"));
	for folder in [&renders, &status, &lane] {
		std::fs::create_dir_all(folder).unwrap();
	}
	let body = serde_json::json!({
		"shots": [
			{ "id": "intro", "spoken": { "role": "intro", "said": "Hello." } },
			{ "id": "dive-1", "spoken": { "role": "scene", "chapter": "Snapshots.", "narration": "Free." } },
			{ "id": "me", "spoken": { "role": "spoken", "category": "questions", "exchanges": [
				{ "ask": { "voice": "kokoro", "said": "Why?" }, "answer": { "voice": "owner", "take": "note-a" } },
			] } },
		],
	})
	.to_string();
	{
		let mut conn = db.acquire().await.unwrap();
		for key in ["wide-2026-11-02", "mumbled-2026-11-03", "odd-2026-11-04"] {
			let essay = cards_essay_repo::Essay {
				key,
				day: &key[key.len() - 10..],
				body: body.as_bytes(),
				taken_at: "2026-11-08T10:00:00.000Z",
			};
			cards_essay_repo::keep(&mut conn, &essay).await.unwrap();
		}
	}
	for key in ["wide-2026-11-02", "mumbled-2026-11-03", "odd-2026-11-04"] {
		std::fs::write(status.join(String::from(key) + ".failed.log"), b"x\n").unwrap();
	}
	std::fs::write(status.join("wide-2026-11-02.refused"), b"reason=overflow\nscenes=dive-1,intro\ncount=2\nheld=ab\n").unwrap();
	std::fs::write(status.join("mumbled-2026-11-03.refused"), b"reason=take-unheard\nitems=note-a\ncount=1\n").unwrap();
	// A take no card plays: never named beside the wrong id.
	std::fs::write(status.join("odd-2026-11-04.refused"), b"reason=take-missing\nitems=note-z\ncount=1\n").unwrap();
	let folders = Folders {
		feeds: vec![],
		clip_walks: None,
		previews: None,
		cards_essays: Some((renders, status)),
		lane,
	};
	let read = read(&db, &folders, "", NOW).await.unwrap();
	let row = |key: &str| serde_json::to_value(read.failed.iter().find(|row| row.key == key).unwrap()).unwrap();
	assert_eq!(row("wide-2026-11-02")["sceneNames"], serde_json::json!(["Snapshots., shot 2", "shot 1"]));
	assert_eq!(row("wide-2026-11-02")["scenes"], serde_json::json!(["dive-1", "intro"]), "beside its ids, in their order");
	assert_eq!(row("mumbled-2026-11-03")["itemNames"], serde_json::json!(["Snapshots., exchange 1, your answer"]));
	assert_eq!(row("odd-2026-11-04").get("itemNames"), None);
	assert_eq!(row("odd-2026-11-04").get("sceneNames"), None);
}

/// The recorder's own names for scenes past the frame (some-ui#3455), kept
/// by the worker as `name.<id>=` lines: said for every shelf, beside the ids;
/// none unless every kept scene is named.
#[test]
fn a_refusal_carries_the_recorders_names_for_its_scenes_when_it_names_each() {
	let text = "reason=overflow\nscenes=ask-1,chapter-2\ncount=5\nheld=ab\nname.ask-1=How a round opens, scene 3\nname.chapter-2=scene 7\n";
	let scenes = vec![String::from("ask-1"), String::from("chapter-2")];
	assert_eq!(
		recorder_names(text, &scenes),
		Some(vec![String::from("How a round opens, scene 3"), String::from("scene 7")])
	);
	assert_eq!(recorder_names("reason=overflow\nname.ask-1=x\n", &scenes), None, "one unnamed: none");
	assert_eq!(recorder_names("name.ask-1=a\u{7}b\nname.chapter-2=c\n", &scenes), None, "a control character is no name");
	assert_eq!(
		recorder_names("name.ask-1=a\u{2028}b\nname.chapter-2=c\n", &scenes),
		None,
		"nor a line break of another kind"
	);
	assert_eq!(
		recorder_names(&["name.ask-1=", &"y".repeat(200), "\nname.chapter-2=c\n"].concat(), &scenes),
		None,
		"nor one past its cap"
	);
	assert_eq!(
		recorder_names("name.ask-1-x=a\nname.ask-1=b\nname.chapter-2=c\n", &scenes).map(|names| names[0].clone()),
		Some(String::from("b")),
		"an id is matched whole"
	);
	assert_eq!(
		recorder_names("name.ask-1=a\u{202E}b\nname.chapter-2=c\n", &scenes),
		None,
		"a name that reorders text is none"
	);
	assert!(
		recorder_names("name.ask-1=\u{1F468}\u{200D}\u{1F373} cooks\nname.chapter-2=c\n", &scenes).is_some(),
		"a joiner is text"
	);
}

/// Names go with the ids the worker keeps (its first 12 of 14 here): every
/// kept scene named gives exactly those 12, in their order, whatever else was
/// named; one of the 12 unnamed gives none, though scenes past the cut are.
#[tokio::test]
async fn scene_names_are_the_kept_scenes_whole_or_none() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("k.refused");
	let ids: Vec<String> = (1..=14).map(|n| ["scene-", &n.to_string()].concat()).collect();
	let name = |id: &str| ["Chapter, ", id].concat();
	let refusal = |named: &dyn Fn(&str) -> bool| {
		let mut text = String::from("reason=overflow\nscenes=");
		text.push_str(&ids[..12].join(","));
		text.push_str("\ncount=14\nheld=ab\n");
		for id in ids.iter().filter(|id| named(id)) {
			text.push_str(&["name.", id, "=", &name(id), "\n"].concat());
		}
		text
	};
	std::fs::write(&path, refusal(&|_| true)).unwrap();
	match read_refused(&path).await {
		Some(Refused::Over(Over::Overflow { scenes, scene_count }, names)) => {
			assert_eq!((scenes.as_slice(), scene_count), (&ids[..12], 14));
			assert_eq!(names, Some(ids[..12].iter().map(|id| name(id)).collect()));
		}
		other => panic!("{other:?}"),
	}
	std::fs::write(&path, refusal(&|id| id != "scene-7")).unwrap();
	match read_refused(&path).await {
		Some(Refused::Over(Over::Overflow { scenes, .. }, names)) => {
			assert_eq!((scenes.as_slice(), names), (&ids[..12], None), "one kept scene unnamed: none");
		}
		other => panic!("{other:?}"),
	}
}

/// The longest overflow the worker writes, twelve ids of 64 bytes each named
/// in 480 four-byte characters, is read whole under the ceiling: its scenes
/// still reach the board, though names that long are none of the board's.
#[tokio::test]
async fn the_longest_named_overflow_reads_whole() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("k.refused");
	let ids: Vec<String> = (0..12u8).map(|n| String::from(char::from(b'a' + n)) + &"x".repeat(63)).collect();
	let mut text = String::from("reason=overflow\nscenes=");
	text.push_str(&ids.join(","));
	text.push_str("\ncount=99\nheld=");
	text.push_str(&"f".repeat(64));
	text.push('\n');
	for id in &ids {
		text.push_str(&["name.", id, "=", &"\u{1D11E}".repeat(480), "\n"].concat());
	}
	assert!(text.len() as u64 <= super::REFUSED_CEILING, "{} bytes", text.len());
	std::fs::write(&path, &text).unwrap();
	match read_refused(&path).await {
		Some(Refused::Over(Over::Overflow { scenes, scene_count }, names)) => {
			assert_eq!((scenes, scene_count, names), (ids, 99, None));
		}
		other => panic!("{other:?}"),
	}
}

/// A recorder's name that is no UTF-8 is a name refused, never the refusal:
/// its reason, scenes and count still reach the board.
#[tokio::test]
async fn a_name_of_bytes_that_are_no_utf8_loses_only_the_names() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("k.refused");
	let mut bytes = b"reason=overflow\nscenes=a,b\ncount=2\nheld=ab\nname.a=".to_vec();
	bytes.extend_from_slice(b"\xff\xfe\nname.b=fine\n");
	std::fs::write(&path, bytes).unwrap();
	match read_refused(&path).await {
		Some(Refused::Over(Over::Overflow { scenes, scene_count }, names)) => {
			assert_eq!((scenes, scene_count, names), (vec![String::from("a"), String::from("b")], 2, None));
		}
		other => panic!("{other:?}"),
	}
}

#[test]
fn the_longest_page_id_is_an_item_and_a_refusal_naming_twelve_reads_whole() {
	let longest = String::from("9").repeat(20) + " " + &"a".repeat(96) + "-0123456789ab.png";
	assert_eq!(longest.len(), super::MOST_ITEM_BYTES);
	assert!(super::item_id(&longest) && !super::item_id(&(longest.clone() + "x")));
	let items = vec![longest; super::MOST_ITEMS];
	let text = String::from("reason=page-gone\nitems=") + &items.join(",") + "\ncount=99\n";
	assert!(text.len() as u64 <= super::REFUSED_CEILING, "{} bytes", text.len());
	assert_eq!(super::Blocking::parse(&text).map(|blocking| blocking.items.len()), Some(super::MOST_ITEMS));
}
