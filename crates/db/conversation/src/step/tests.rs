use super::{parse, replay, Choice, Kind, Phase, Refusal, State, MOST_MS, POINTS_MOST};
use serde_json::{json, Value};

fn event(value: Value) -> super::Event {
	parse(value).unwrap()
}

fn topic() -> Value {
	json!({ "event": "topic", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "why the record is a log" } })
}

fn point(take: &str) -> Value {
	json!({ "event": "point", "at": "2026-11-26T10:01:00.000Z", "take": take })
}

fn version(n: u64, answers: &Value) -> Value {
	json!({
		"event": "version",
		"at": "2026-11-26T10:02:00.000Z",
		"version": {
			"n": n,
			"speakers": [
				{ "id": "owner", "character": "mochi", "voice": { "kind": "owner" } },
				{ "id": "tin", "character": "tin-owlet", "voice": { "kind": "kokoro" } }
			],
			"turns": [
				{ "id": "t1", "speaker": "owner", "voice": "owner", "take": "take-1", "transcript": "one two three" },
				{ "id": "t2", "speaker": "tin", "voice": "kokoro", "said": "a retort in five words" }
			],
			"answers": answers
		}
	})
}

fn flag(id: &str, turn: &str) -> Value {
	json!({ "event": "flag", "at": "2026-11-26T10:03:00.000Z", "flag": { "id": id, "on": { "scope": "turn", "turn": turn }, "kinds": ["too-fast"] } })
}

fn heard(version: u64, ms: u64) -> Value {
	json!({ "event": "heard", "at": "2026-11-26T10:04:00.000Z", "version": version, "ms": ms })
}

fn green_light(version: u64) -> Value {
	json!({ "event": "green-light", "at": "2026-11-26T10:05:00.000Z", "version": version })
}

fn approve(preview: &str, chose: &Value) -> Value {
	json!({
		"event": "approve",
		"at": "2026-11-26T10:06:00.000Z",
		"preview": preview,
		"offered": ["a", "b"],
		"recommended": "a",
		"chose": chose
	})
}

/// A conversation green-lit at version 1.
fn green_lit() -> Vec<Value> {
	vec![topic(), point("take-1"), version(1, &json!([])), heard(1, 60_000), green_light(1)]
}

/// `values`, each a minute after the one before, so time order holds.
fn in_order(values: &[Value]) -> Vec<super::Event> {
	values
		.iter()
		.enumerate()
		.map(|(minute, value)| {
			let mut value = value.clone();
			let pad = if minute < 10 { "0" } else { "" };
			value["at"] = json!(String::from("2026-11-26T10:") + pad + &minute.to_string() + ":00.000Z");
			event(value)
		})
		.collect()
}

fn state_of(values: &[Value]) -> State {
	replay(&in_order(values)).unwrap()
}

/// Why `next`, the minute after `values`, is refused.
fn refused(values: &[Value], next: Value) -> Refusal {
	let mut all = values.to_vec();
	all.push(next);
	let mut events = in_order(&all);
	let last = events.pop().unwrap();
	replay(&events).unwrap().step(&last).unwrap_err()
}

#[test]
fn a_log_steps_through_every_phase_to_the_green_light() {
	let mut values = vec![topic()];
	assert_eq!(state_of(&values).phase(), Phase::Drafting);
	values.push(version(1, &json!([])));
	assert_eq!(state_of(&values).phase(), Phase::Podcast);
	values.push(flag("too fast", "t1"));
	assert_eq!(state_of(&values).phase(), Phase::Flagged);
	values.push(version(2, &json!([{ "flag": "too fast", "how": "conditioned" }])));
	assert_eq!(state_of(&values).phase(), Phase::Podcast);
	values.push(heard(2, 60_000));
	values.push(green_light(2));
	assert_eq!(state_of(&values).phase(), Phase::GreenLit);
}

#[test]
fn an_approval_is_kept_with_the_rule_it_set() {
	let mut values = green_lit();
	values.push(approve("p-close", &json!({ "kind": "variant", "variant": "b", "rule": "calm-close" })));
	let state = state_of(&values);
	assert_eq!(state.approvals().len(), 1);
	assert_eq!(
		state.approvals()[0].chose,
		Choice::Variant {
			variant: "b".to_owned(),
			rule: "calm-close".to_owned()
		}
	);
	assert_eq!(state.rules().collect::<Vec<_>>(), ["calm-close"]);
}

#[test]
fn a_pick_of_a_variant_the_preview_never_offered_is_refused() {
	let refusal = refused(&green_lit(), approve("p-close", &json!({ "kind": "variant", "variant": "c", "rule": "loud-close" })));
	assert_eq!(
		refusal,
		Refusal::PreviewVariant {
			preview: "p-close".to_owned(),
			variant: "c".to_owned()
		}
	);
	let mut unoffered = approve("p-close", &json!({ "kind": "else", "said": "neither, slower" }));
	unoffered["recommended"] = json!("z");
	assert_eq!(
		refused(&green_lit(), unoffered),
		Refusal::PreviewVariant {
			preview: "p-close".to_owned(),
			variant: "z".to_owned()
		}
	);
}

#[test]
fn a_preview_is_approved_once() {
	let mut values = green_lit();
	values.push(approve("p-close", &json!({ "kind": "else", "said": "neither, slower" })));
	let again = approve("p-close", &json!({ "kind": "variant", "variant": "a", "rule": "calm-close" }));
	assert_eq!(refused(&values, again), Refusal::PreviewAgain { preview: "p-close".to_owned() });
}

#[test]
fn an_approval_before_the_green_light_is_out_of_phase() {
	let values = vec![topic(), version(1, &json!([]))];
	let refusal = refused(&values, approve("p", &json!({ "kind": "else", "said": "neither" })));
	assert_eq!(
		refusal,
		Refusal::OutOfPhase {
			phase: Phase::Podcast,
			event: Kind::Approve
		}
	);
}

#[test]
fn the_refusals_serialize_as_the_records_step_names_them() {
	let refusal = Refusal::PreviewVariant {
		preview: "p".to_owned(),
		variant: "c".to_owned(),
	};
	assert_eq!(serde_json::to_value(refusal).unwrap(), json!({ "why": "preview-variant", "preview": "p", "variant": "c" }));
	let out = Refusal::OutOfPhase {
		phase: Phase::GreenLit,
		event: Kind::GreenLight,
	};
	assert_eq!(
		serde_json::to_value(out).unwrap(),
		json!({ "why": "out-of-phase", "phase": "green-lit", "event": "green-light" })
	);
	assert_eq!(serde_json::to_value(Refusal::TooManyPoints).unwrap(), json!({ "why": "too-many-points" }));
}

#[test]
fn an_event_dated_before_the_last_is_out_of_time() {
	let mut early = point("take-2");
	early["at"] = json!("2026-11-26T09:00:00.000Z");
	assert_eq!(
		state_of(&[topic()]).step(&event(early)).unwrap_err(),
		Refusal::OutOfTime {
			since: "2026-11-26T10:00:00.000Z".to_owned(),
			at: "2026-11-26T09:00:00.000Z".to_owned()
		}
	);
}

#[test]
fn points_are_each_once_and_at_most_forty() {
	assert_eq!(refused(&[topic(), point("take-1")], point("take-1")), Refusal::PointAgain { take: "take-1".to_owned() });
	let mut values = vec![topic()];
	values.extend((0..POINTS_MOST).map(|n| point(&(String::from("take-") + &n.to_string()))));
	assert_eq!(refused(&values, point("one-more")), Refusal::TooManyPoints);
}

#[test]
fn a_version_is_the_next_and_answers_each_open_flag_once() {
	let flagged = vec![topic(), version(1, &json!([])), flag("f", "t1")];
	assert_eq!(refused(&flagged, version(3, &json!([]))), Refusal::VersionNumber { expected: 2, got: 3 });
	assert_eq!(refused(&flagged, version(2, &json!([]))), Refusal::Unanswered { flag: "f".to_owned() });
	assert_eq!(
		refused(&flagged, version(2, &json!([{ "flag": "g", "how": "kept", "note": "why" }]))),
		Refusal::AnswerUnknown { flag: "g".to_owned() }
	);
	let twice = json!([{ "flag": "f", "how": "conditioned" }, { "flag": "f", "how": "conditioned" }]);
	assert_eq!(refused(&flagged, version(2, &twice)), Refusal::AnswerTwice { flag: "f".to_owned() });
}

#[test]
fn a_flag_is_raised_once_on_what_the_newest_version_has() {
	let podcast = vec![topic(), version(1, &json!([]))];
	assert_eq!(
		refused(&podcast, flag("f", "t9")),
		Refusal::TurnUnknown {
			flag: "f".to_owned(),
			turn: "t9".to_owned()
		}
	);
	let mut on_speaker = flag("f", "t1");
	on_speaker["flag"]["on"] = json!({ "scope": "speaker", "speaker": "nobody" });
	assert_eq!(
		refused(&podcast, on_speaker),
		Refusal::FlagSpeakerUnknown {
			flag: "f".to_owned(),
			speaker: "nobody".to_owned()
		}
	);
	let mut flagged = podcast;
	flagged.push(flag("f", "t1"));
	assert_eq!(refused(&flagged, flag("f", "t2")), Refusal::FlagAgain { flag: "f".to_owned() });
}

#[test]
fn a_withdrawal_takes_back_an_open_flag_only() {
	let flagged = vec![topic(), version(1, &json!([])), flag("f", "t1")];
	let withdraw = |flag: &str| json!({ "event": "withdraw", "at": "2026-11-26T10:03:30.000Z", "flag": flag });
	assert_eq!(refused(&flagged, withdraw("g")), Refusal::FlagNotOpen { flag: "g".to_owned() });
	let mut values = flagged;
	values.push(withdraw("f"));
	assert_eq!(state_of(&values).phase(), Phase::Podcast);
	// Nothing is open now, so `podcast` takes no withdrawal at all.
	assert_eq!(
		refused(&values, withdraw("f")),
		Refusal::OutOfPhase {
			phase: Phase::Podcast,
			event: Kind::Withdraw
		}
	);
}

#[test]
fn the_green_light_needs_the_newest_version_measured_and_under_the_bound() {
	let podcast = vec![topic(), version(1, &json!([]))];
	assert_eq!(refused(&podcast, green_light(1)), Refusal::Unheard { version: 1 });
	assert_eq!(refused(&podcast, green_light(2)), Refusal::GreenLightVersion { expected: 1, got: 2 });
	assert_eq!(refused(&podcast, heard(2, 1)), Refusal::HeardVersion { expected: 1, got: 2 });
	let mut long = podcast;
	long.push(heard(1, MOST_MS + 1));
	assert_eq!(refused(&long, green_light(1)), Refusal::TooLong { ms: MOST_MS + 1 });
	assert_eq!(refused(&long, heard(1, 5)), Refusal::HeardAgain { version: 1 });
}

#[test]
fn notes_name_a_turn_of_the_newest_version_its_words_once_and_approved_rules() {
	let notes = |turn: &str, words: &Value, rules: &Value| json!({ "event": "notes", "at": "2026-11-26T10:07:00.000Z", "turn": turn, "words": words, "rules": rules });
	let lit = green_lit();
	assert_eq!(
		refused(&lit, notes("t9", &json!([{ "at": 0, "stress": 1 }]), &json!([]))),
		Refusal::NotesTurn { turn: "t9".to_owned() }
	);
	// t1's transcript has three words.
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 3, "stress": 1 }]), &json!([]))),
		Refusal::NotesWord { turn: "t1".to_owned(), at: 3 }
	);
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 1, "stress": 1 }, { "at": 1, "stress": 0 }]), &json!([]))),
		Refusal::NotesWord { turn: "t1".to_owned(), at: 1 }
	);
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 1, "stress": 1 }]), &json!(["calm-close"]))),
		Refusal::NotesRule { rule: "calm-close".to_owned() }
	);
	let mut approved = lit;
	approved.push(approve("p-close", &json!({ "kind": "variant", "variant": "a", "rule": "calm-close" })));
	approved.push(notes("t1", &json!([{ "at": 2, "stress": 1 }]), &json!(["calm-close"])));
	assert_eq!(state_of(&approved).phase(), Phase::GreenLit);
}

#[test]
fn an_event_is_exactly_its_kinds_fields() {
	let mut extra = point("take-1");
	extra["note"] = json!("not a field");
	assert!(parse(extra).is_err());
	let mut local = point("take-1");
	local["at"] = json!("2026-11-26T10:01:00+01:00");
	assert!(parse(local).is_err());
	assert!(parse(json!({ "event": "edit", "at": "2026-11-26T10:01:00Z" })).is_err());
	assert!(parse(json!({ "event": "point", "at": "2026-11-26T10:01:00Z" })).is_err());
	let mut thrice = approve("p", &json!({ "kind": "else", "said": "neither" }));
	thrice["offered"] = json!(["a", "a"]);
	assert!(parse(thrice).is_err());
	let mut set = approve("p", &json!({ "kind": "variant", "variant": "a" }));
	assert!(parse(set.clone()).is_err());
	set["chose"] = json!({ "kind": "variant", "variant": "a", "rule": "r", "value": 3 });
	assert!(parse(set).is_err(), "a pick sets a rule, never a value");
}

#[test]
fn a_refused_step_leaves_the_state_as_it_was() {
	let before = state_of(&green_lit());
	let after = before.clone();
	assert!(after.step(&event(approve("p", &json!({ "kind": "variant", "variant": "c", "rule": "r" })))).is_err());
	assert_eq!(before, after);
}
