use super::{parse, replay, Choice, Kind, Phase, Refusal, State, MOST_MS, POINTS_MOST};
use serde_json::{json, Value};

fn event(value: Value) -> super::Event {
	parse(value).unwrap()
}

fn topic() -> Value {
	json!({ "event": "topic", "by": "owner", "at": "2026-11-26T10:00:00.000Z", "topic": { "said": "why the record is a log" } })
}

fn point(take: &str) -> Value {
	json!({ "event": "point", "by": "owner", "at": "2026-11-26T10:01:00.000Z", "take": take })
}

fn version(n: u64, answers: &Value) -> Value {
	json!({
		"event": "version",
		"by": "agent",
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

/// A flag on version 1.
fn flag(id: &str, turn: &str) -> Value {
	flag_on(1, id, turn)
}

fn flag_on(version: u64, id: &str, turn: &str) -> Value {
	json!({ "event": "flag", "by": "owner", "at": "2026-11-26T10:03:00.000Z", "version": version, "flag": { "id": id, "on": { "scope": "turn", "turn": turn }, "kinds": ["too-fast"] } })
}

fn heard(version: u64, ms: u64) -> Value {
	json!({ "event": "heard", "by": "home", "at": "2026-11-26T10:04:00.000Z", "version": version, "ms": ms })
}

fn green_light(version: u64) -> Value {
	json!({ "event": "green-light", "by": "owner", "at": "2026-11-26T10:05:00.000Z", "version": version })
}

fn approve(preview: &str, chose: &Value) -> Value {
	let mut chose = chose.clone();
	// A pick names the variant; its rule is the offer's (C1582).
	if let Some(chose) = chose.as_object_mut() {
		chose.remove("rule");
	}
	json!({ "event": "approve", "by": "owner", "at": "2026-11-26T10:06:00.000Z", "preview": preview, "chose": chose })
}

fn offer(preview: &str, variants: &[(&str, &str)]) -> Value {
	let variants: Vec<Value> = variants
		.iter()
		.map(|(variant, rule)| json!({ "variant": variant, "title": variant.to_uppercase(), "callouts": ["what is in it"], "rule": rule }))
		.collect();
	json!({ "event": "offer", "by": "agent", "at": "2026-11-26T10:05:30.000Z", "offer": { "preview": preview, "about": "How one scene gives way to the next", "recommended": variants[0]["variant"], "variants": variants } })
}

/// Green-lit, with the agent's offers made: `p-close` (a, b, c), `p` (a, b)
/// and `p-close-2` (a, b), whose b sets `calm-close` as `p-close`'s a does.
fn offered() -> Vec<Value> {
	let mut values = green_lit();
	values.push(offer("p-close", &[("a", "calm-close"), ("b", "lively-close"), ("c", "playful-close")]));
	values.push(offer("p", &[("a", "r"), ("b", "r2")]));
	values.push(offer("p-close-2", &[("a", "quiet-close"), ("b", "calm-close")]));
	values
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
fn an_approval_is_kept_with_the_rule_its_offer_set() {
	let mut values = offered();
	values.push(approve("p-close", &json!({ "kind": "variant", "variant": "b" })));
	let state = state_of(&values);
	let kept = state.standing("p-close").unwrap();
	assert_eq!(
		kept.chose,
		Choice::Variant {
			variant: "b".to_owned(),
			rule: "lively-close".to_owned()
		}
	);
	assert_eq!(state.rules().collect::<Vec<_>>(), ["lively-close"]);
	assert_eq!(state.offers().len(), 3);
}

#[test]
fn a_pick_names_an_offer_made_and_one_of_its_variants() {
	assert_eq!(
		refused(&offered(), approve("p-close", &json!({ "kind": "variant", "variant": "d" }))),
		Refusal::PreviewVariant {
			preview: "p-close".to_owned(),
			variant: "d".to_owned()
		}
	);
	assert_eq!(
		refused(&offered(), approve("p-open", &json!({ "kind": "variant", "variant": "a" }))),
		Refusal::PreviewUnknown { preview: "p-open".to_owned() }
	);
	assert_eq!(
		refused(&green_lit(), approve("p-close", &json!({ "kind": "else", "said": "neither, slower" }))),
		Refusal::PreviewUnknown { preview: "p-close".to_owned() },
		"no offer, no pick"
	);
}

#[test]
fn a_pick_changes_only_by_moving_the_standing_one() {
	let mut values = offered();
	values.push(approve("p-close", &json!({ "kind": "else", "said": "neither, slower" })));
	let again = approve("p-close", &json!({ "kind": "variant", "variant": "a" }));
	assert_eq!(refused(&values, again.clone()), Refusal::PreviewAgain { preview: "p-close".to_owned() });
	let mut moves_nothing = again.clone();
	moves_nothing["moves"] = json!("p");
	assert_eq!(refused(&values, moves_nothing), Refusal::MovesUnknown { preview: "p".to_owned() });
	let mut changed = again;
	changed["moves"] = json!("p-close");
	values.push(changed);
	let state = state_of(&values);
	assert_eq!(
		state.standing("p-close").unwrap().chose,
		Choice::Variant {
			variant: "a".to_owned(),
			rule: "calm-close".to_owned()
		},
		"the latest pick stands; the first stays kept in the log"
	);
	assert_eq!(state.rules().collect::<Vec<_>>(), ["calm-close"]);
}

#[test]
fn an_offer_is_made_once_and_only_once_green_lit() {
	assert_eq!(refused(&offered(), offer("p", &[("a", "x"), ("b", "y")])), Refusal::OfferAgain { preview: "p".to_owned() });
	assert_eq!(
		refused(&[topic(), version(1, &json!([]))], offer("p", &[("a", "x"), ("b", "y")])),
		Refusal::OutOfPhase {
			phase: Phase::Podcast,
			event: Kind::Offer
		}
	);
	let mut by_owner = offer("q", &[("a", "x"), ("b", "y")]);
	by_owner["by"] = json!("owner");
	assert_eq!(
		refused(&green_lit(), by_owner),
		Refusal::Author {
			event: Kind::Offer,
			by: "owner".to_owned()
		}
	);
}

#[test]
fn an_offer_is_what_the_cards_schema_takes() {
	let good = offer("q", &[("a", "x"), ("b", "y")]);
	assert!(parse(good.clone()).is_ok());
	for broken in [
		offer("q", &[("a", "x")]),
		offer("q", &[("a", "x"), ("b", "y"), ("c", "z"), ("d", "w")]),
		offer("q", &[("a", "x"), ("a", "y")]),
		offer("q", &[("a", "x"), ("b", "x")]),
		offer("Q Q", &[("a", "x"), ("b", "y")]),
	] {
		assert!(parse(broken).is_err());
	}
	let mut unrecommended = good.clone();
	unrecommended["offer"]["recommended"] = json!("z");
	assert!(parse(unrecommended).is_err());
	let mut long = good.clone();
	long["offer"]["about"] = json!("x".repeat(81));
	assert!(parse(long).is_err());
	let mut twice = good.clone();
	twice["offer"]["variants"][0]["callouts"] = json!(["same", "same "]);
	assert!(parse(twice).is_err(), "callouts are each once, trimmed");
	let mut padded = good;
	padded["offer"]["about"] = json!("  a heading  ");
	assert!(parse(padded).is_ok(), "trimmed first, as zod's trim reads it");
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
fn order_is_the_logs_and_a_writers_clock_is_never_compared() {
	let mut early = point("take-2");
	early["at"] = json!("2026-11-26T09:00:00.000Z");
	assert!(state_of(&[topic()]).step(&event(early)).is_ok(), "an event dated before the last is kept in its place");
}

#[test]
fn an_event_written_against_an_older_log_is_stale() {
	let mut on_one = point("take-1");
	on_one["basis"] = json!(1);
	let after_topic = state_of(&[topic()]);
	assert!(after_topic.step(&event(on_one.clone())).is_ok(), "written against the log as it stands");
	let lit = state_of(&green_lit());
	assert_eq!(lit.step(&event(on_one)).unwrap_err(), Refusal::Stale { basis: 1, seq: 5 });
	// The basis is checked before the phase: a stale approval is stale, not out of phase.
	let mut first = approve("p", &json!({ "kind": "else", "said": "neither" }));
	first["basis"] = json!(3);
	assert_eq!(state_of(&[]).step(&event(first)).unwrap_err(), Refusal::Stale { basis: 3, seq: 0 });
	let mut topic_first = topic();
	topic_first["basis"] = json!(0);
	assert!(State::new().step(&event(topic_first)).is_ok(), "the first event, a topic, is written against 0");
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
	let withdraw = |flag: &str| json!({ "event": "withdraw", "by": "owner", "at": "2026-11-26T10:03:30.000Z", "flag": flag });
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
	let notes =
		|turn: &str, words: &Value, rules: &Value| json!({ "event": "notes", "by": "agent", "at": "2026-11-26T10:07:00.000Z", "turn": turn, "words": words, "rules": rules });
	let mut lit = offered();
	lit.push(approve("p-close", &json!({ "kind": "variant", "variant": "a" })));
	let calm = json!(["calm-close"]);
	assert_eq!(
		refused(&lit, notes("t9", &json!([{ "at": 0, "stress": 1 }]), &calm)),
		Refusal::NotesTurn { turn: "t9".to_owned() }
	);
	// t1's transcript has three words.
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 3, "stress": 1 }]), &calm)),
		Refusal::NotesWord { turn: "t1".to_owned(), at: 3 }
	);
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 1, "stress": 1 }, { "at": 1, "stress": 0 }]), &calm)),
		Refusal::NotesWord { turn: "t1".to_owned(), at: 1 }
	);
	assert_eq!(
		refused(&lit, notes("t1", &json!([{ "at": 1, "stress": 1 }]), &json!(["lively-close"]))),
		Refusal::NotesRule { rule: "lively-close".to_owned() }
	);
	assert!(
		parse(notes("t1", &json!([{ "at": 1, "stress": 1 }]), &json!([]))).is_err(),
		"notes follow at least one rule"
	);
	let mut kept = lit.clone();
	kept.push(notes("t1", &json!([{ "at": 2, "stress": 1 }]), &calm));
	assert_eq!(state_of(&kept).phase(), Phase::GreenLit);
	// Notes come only once the conversation is green-lit.
	let podcast = vec![topic(), version(1, &json!([]))];
	assert_eq!(
		refused(&podcast, notes("t1", &json!([{ "at": 0, "stress": 1 }]), &calm)),
		Refusal::OutOfPhase {
			phase: Phase::Podcast,
			event: Kind::Notes
		}
	);
}

#[test]
fn a_turn_whose_words_the_record_does_not_hold_takes_no_notes() {
	let mut untranscribed = version(1, &json!([]));
	untranscribed["version"]["turns"][0] = json!({ "id": "t1", "speaker": "owner", "voice": "owner", "take": "take-1" });
	let mut lit = vec![topic(), untranscribed, heard(1, 60_000), green_light(1), offer("p", &[("a", "r"), ("b", "r2")])];
	lit.push(approve("p", &json!({ "kind": "variant", "variant": "a" })));
	let notes = json!({ "event": "notes", "by": "agent", "at": "2026-11-26T10:07:00.000Z", "turn": "t1", "words": [{ "at": 0, "stress": 1 }], "rules": ["r"] });
	assert_eq!(refused(&lit, notes), Refusal::NotesWord { turn: "t1".to_owned(), at: 0 });
}

#[test]
fn each_kind_of_event_has_its_author() {
	let mut by_agent = topic();
	by_agent["by"] = json!("agent");
	assert_eq!(
		State::new().step(&event(by_agent)).unwrap_err(),
		Refusal::Author {
			event: Kind::Topic,
			by: "agent".to_owned()
		}
	);
	let mut owner_heard = heard(1, 5);
	owner_heard["by"] = json!("owner");
	assert_eq!(
		refused(&[topic(), version(1, &json!([]))], owner_heard),
		Refusal::Author {
			event: Kind::Heard,
			by: "owner".to_owned()
		}
	);
	let mut nobody = topic();
	nobody["by"] = json!("someone");
	assert!(parse(nobody).is_err());
	let mut unsaid = topic();
	unsaid.as_object_mut().unwrap().remove("by");
	assert!(parse(unsaid).is_err(), "every event names its author");
	// After `stale`, before the phase.
	let mut stale_agent = approve("p", &json!({ "kind": "else", "said": "neither" }));
	stale_agent["by"] = json!("agent");
	stale_agent["basis"] = json!(2);
	assert_eq!(State::new().step(&event(stale_agent.clone())).unwrap_err(), Refusal::Stale { basis: 2, seq: 0 });
	stale_agent["basis"] = json!(0);
	assert_eq!(
		State::new().step(&event(stale_agent)).unwrap_err(),
		Refusal::Author {
			event: Kind::Approve,
			by: "agent".to_owned()
		}
	);
}

#[test]
fn a_flag_lands_on_the_version_the_owner_heard() {
	let two = vec![
		topic(),
		version(1, &json!([])),
		flag("f", "t1"),
		version(2, &json!([{ "flag": "f", "how": "conditioned" }])),
	];
	assert_eq!(
		refused(&two, flag_on(1, "g", "t1")),
		Refusal::FlagVersion {
			flag: "g".to_owned(),
			expected: 2,
			got: 1
		}
	);
	assert!(state_of(&two).step(&event(flag_on(2, "g", "t1"))).is_ok());
}

#[test]
fn a_rule_already_set_is_set_again_only_by_naming_the_approval_it_moves() {
	let mut set = offered();
	set.push(approve("p-close", &json!({ "kind": "variant", "variant": "a" })));
	// p-close-2's b sets calm-close too.
	let again = approve("p-close-2", &json!({ "kind": "variant", "variant": "b" }));
	assert_eq!(refused(&set, again.clone()), Refusal::RuleAgain { rule: "calm-close".to_owned() });
	let mut wrong = again.clone();
	wrong["moves"] = json!("p");
	assert_eq!(refused(&set, wrong), Refusal::MovesUnknown { preview: "p".to_owned() });
	let mut moving = again;
	moving["moves"] = json!("p-close");
	set.push(moving);
	let state = state_of(&set);
	assert!(state.standing("p-close").is_none(), "the moved approval no longer stands");
	assert_eq!(state.rules().collect::<Vec<_>>(), ["calm-close"]);
}

#[test]
fn a_flag_of_several_kinds_is_answered_kind_by_kind() {
	let mut two_kinds = flag("f", "t1");
	two_kinds["flag"]["kinds"] = json!(["too-fast", "too-quiet"]);
	let flagged = vec![topic(), version(1, &json!([])), two_kinds];
	let half = json!([{ "flag": "f", "kinds": ["too-fast"], "how": "conditioned" }]);
	assert_eq!(refused(&flagged, version(2, &half)), Refusal::Unanswered { flag: "f".to_owned() });
	let both = json!([{ "flag": "f", "kinds": ["too-fast"], "how": "conditioned" }, { "flag": "f", "kinds": ["too-quiet"], "how": "conditioned" }]);
	assert!(state_of(&flagged).step(&event(version(2, &both))).is_ok());
	let foreign = json!([{ "flag": "f", "kinds": ["echo"], "how": "rewrote", "turns": ["t2"] }]);
	assert_eq!(
		refused(&flagged, version(2, &foreign)),
		Refusal::AnswerKinds {
			flag: "f".to_owned(),
			kind: "echo".to_owned()
		}
	);
	let twice = json!([{ "flag": "f", "how": "conditioned" }, { "flag": "f", "kinds": ["too-quiet"], "how": "conditioned" }]);
	assert_eq!(refused(&flagged, version(2, &twice)), Refusal::AnswerTwice { flag: "f".to_owned() });
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
	assert!(parse(json!({ "event": "point", "by": "owner", "at": "2026-11-26T10:01:00Z" })).is_err());
	let mut restated = approve("p", &json!({ "kind": "else", "said": "neither" }));
	restated["offered"] = json!(["a", "b"]);
	assert!(parse(restated).is_err(), "an approval names the offer, never restates it");
	let mut set = approve("p", &json!({ "kind": "variant", "variant": "a" }));
	assert!(parse(set.clone()).is_ok());
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

/// Every `why` home's step can answer: `Refusal`'s cases. A refusal of
/// `stepConversation` named here is one home must answer the same way; any
/// other is a rule home trusts the writers for (docs/conversations.md,
/// invariant 4).
const HELD: [&str; 28] = [
	"stale",
	"author",
	"out-of-phase",
	"too-many-points",
	"point-again",
	"version-number",
	"unanswered",
	"answer-unknown",
	"answer-twice",
	"answer-kinds",
	"flag-again",
	"flag-version",
	"turn-unknown",
	"flag-speaker-unknown",
	"flag-not-open",
	"heard-version",
	"heard-again",
	"unheard",
	"too-long",
	"green-light-version",
	"notes-turn",
	"notes-word",
	"notes-rule",
	"offer-again",
	"preview-unknown",
	"moves-unknown",
	"preview-again",
	"rule-again",
];

/// paulgsc/some-ui's `conversationCorpus` (`pnpm dump:conversation-refusals`),
/// kept here and held to that dump by `scripts/check_client_fixtures.py`.
const CORPUS: &str = include_str!("../../testdata/conversation_refusals.snapshot.json");

#[test]
fn home_answers_the_records_step_as_it_does_wherever_home_holds_the_rule() {
	let corpus: Value = serde_json::from_str(CORPUS).unwrap();
	assert_eq!(corpus["schema"], json!("some-ui.conversation-refusals/1"));
	let alphabet = corpus["alphabet"].as_object().unwrap();
	let cases = corpus["cases"].as_array().unwrap();
	assert!(cases.len() > 1000, "the corpus walks every state to its depth");
	let mut held_seen: Vec<String> = Vec::new();
	let mut problems: Vec<String> = Vec::new();
	for case in cases {
		let id = case["id"].as_str().unwrap();
		let mut state = State::new();
		for name in case["path"].as_array().unwrap() {
			let value = alphabet[name.as_str().unwrap()].clone();
			match parse(value).map(|event| state.step(&event)) {
				Ok(Ok(next)) => state = next,
				// The record's step took this event, so home must too.
				Ok(Err(refusal)) => {
					problems.push(String::from(id) + ": home refuses " + name.as_str().unwrap() + " on the path, as " + &serde_json::to_value(refusal).unwrap().to_string());
				}
				Err(problems_found) => {
					problems.push(String::from(id) + ": home cannot read " + name.as_str().unwrap() + ": " + &problems_found[0].0);
				}
			}
		}
		let next = alphabet[case["next"].as_str().unwrap()].clone();
		let home = match parse(next) {
			Ok(event) => match state.step(&event) {
				Ok(_) => None,
				Err(refusal) => Some(serde_json::to_value(refusal).unwrap()),
			},
			Err(found) => Some(json!({ "why": "unreadable", "field": found[0].0 })),
		};
		let theirs = &case["refusal"];
		let why = theirs["why"].as_str();
		match why {
			// The record's step takes it: so does home.
			None => {
				if let Some(home) = home {
					problems.push(String::from(id) + ": taken by the record's step, refused by home as " + &home.to_string());
				}
			}
			// A rule home holds: the same refusal, field for field.
			Some(why) if HELD.contains(&why) => {
				held_seen.push(why.to_owned());
				if home.as_ref() != Some(theirs) {
					problems.push(
						String::from(id)
							+ ": the record's step refuses it as "
							+ &theirs.to_string()
							+ ", home as "
							+ &home.map_or_else(|| String::from("taken"), |home| home.to_string()),
					);
				}
			}
			// A rule home trusts the writers for: home may take it.
			Some(_) => {}
		}
	}
	assert!(
		problems.is_empty(),
		"{} of {} cases differ; the first: {:#?}",
		problems.len(),
		cases.len(),
		&problems[..problems.len().min(10)]
	);
	let unreached: Vec<&str> = HELD.iter().copied().filter(|why| !held_seen.iter().any(|seen| seen == why)).collect();
	// Forty points, and a flag on a speaker no version has, are beyond the walk.
	assert_eq!(unreached, ["too-many-points", "flag-speaker-unknown"], "the corpus reaches every other rule home holds");
}

#[test]
fn home_reads_an_offer_as_the_cards_schema_does() {
	let corpus: Value = serde_json::from_str(CORPUS).unwrap();
	let probes = corpus["offers"].as_array().unwrap();
	assert!(probes.len() >= 20, "the corpus probes each of the offer's bounds");
	let differ: Vec<String> = probes
		.iter()
		.filter(|probe| parse(probe["event"].clone()).is_ok() != probe["takes"].as_bool().unwrap())
		.map(|probe| {
			String::from(probe["id"].as_str().unwrap())
				+ if probe["takes"].as_bool().unwrap() {
					": the schema takes it, home refuses it"
				} else {
					": the schema refuses it, home takes it"
				}
		})
		.collect();
	assert!(differ.is_empty(), "{differ:#?}");
}
