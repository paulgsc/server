//! A cards essay's scenes and takes, by the names its own cut list gives.
//!
//! For a failure that says which (Studio › Renders › Couldn't finish: the
//! owner's rule that a failure names the scene or part, never an id).
//!
//! Every name is read from the essay home holds, never made up:
//!
//! - **A scene** that overflows is a spoken card's turn
//!   (`<card>-ask-<k>` / `<card>-answer-<k>`, the recorder's ids):
//!   `<chapter>, exchange <k>, your answer` (or `your question`, `the
//!   question`, `the answer` for Kokoro's turns). Any other is its shot:
//!   `<chapter>, shot <n>`, its number among the essay's shots (the cut
//!   list's own count, not the video's scenes). The chapter is the nearest
//!   `spoken.chapter` of a `scene` shot at or before it, the essay's own
//!   words; with none before it, the name is the rest alone.
//! - **A take** is where it is played, said to the owner: its spoken card's
//!   chapter, the exchange, and whether it is their question or their
//!   answer. A spoken card has no title of its own.
//!
//! A name is one line of at most [`MOST_CHARS`] characters. A list is named
//! whole or not at all: one id the essay does not hold leaves the list
//! unnamed (`None`), so a name never sits beside the wrong id.

use serde_json::Value;

/// The longest name, in characters; a longer chapter is cut with `…`.
pub const MOST_CHARS: usize = 80;

/// The shots of `essay`, in order.
fn shots(essay: &Value) -> &[Value] {
	essay.get("shots").and_then(Value::as_array).map_or(&[][..], Vec::as_slice)
}

/// One line of the essay's own words: its whitespace runs made one space,
/// at most `most` characters, cut with `…`.
fn line(text: &str, most: usize) -> String {
	let words: Vec<&str> = text.split_whitespace().collect();
	let joined = words.join(" ");
	if joined.chars().count() <= most {
		return joined;
	}
	let mut cut: String = joined.chars().take(most.saturating_sub(1)).collect();
	cut.push('…');
	cut
}

/// Each shot's chapter: the nearest `spoken.chapter` of a `scene` shot at or
/// before it (the essay's own words); before the first, `Introduction` for
/// the opening introduction card, and none for any other shot.
fn chapters(essay: &Value) -> Vec<Option<String>> {
	let mut chapter: Option<String> = None;
	let mut chapters = Vec::new();
	for shot in shots(essay) {
		let spoken = shot.get("spoken");
		if spoken.and_then(|spoken| spoken.get("role")).and_then(Value::as_str) == Some("scene") {
			if let Some(said) = spoken
				.and_then(|spoken| spoken.get("chapter"))
				.and_then(Value::as_str)
				.filter(|said| !said.trim().is_empty())
			{
				chapter = Some(said.to_owned());
			}
		}
		// The opening introduction card plays before any chapter, under the
		// video's own `Introduction` mark (some-ui's SPOKEN_CATEGORY_TITLES).
		let opening = chapter.is_none() && spoken.and_then(|spoken| spoken.get("category")).and_then(Value::as_str) == Some("introduction");
		chapters.push(if opening { Some(String::from("Introduction")) } else { chapter.clone() });
	}
	chapters
}

/// `<chapter>, <rest>`, or `<rest>` with no chapter, in at most
/// [`MOST_CHARS`] characters (the chapter is what is cut).
fn named(chapter: Option<&str>, rest: &str) -> String {
	let room = MOST_CHARS.saturating_sub(rest.chars().count() + 2);
	match chapter {
		Some(chapter) if room > 0 => line(chapter, room) + ", " + rest,
		_ => line(rest, MOST_CHARS),
	}
}

/// A spoken card's turn as the owner reads it: exchange `k` (from 1), and
/// whose words. `owner` says the turn is theirs (their take), else Kokoro's.
fn turn_name(k: usize, ask: bool, owner: bool) -> String {
	let whose = match (ask, owner) {
		(true, true) => "your question",
		(false, true) => "your answer",
		(true, false) => "the question",
		(false, false) => "the answer",
	};
	String::from("exchange ") + &k.to_string() + ", " + whose
}

/// The spoken card shot `card` (an index of `shots`) and its exchange `k`'s
/// turn, when `id` is one of its scenes as the recorder names it,
/// `<card>-ask-<k>` or `<card>-answer-<k>` (some-ui's `spokenCardScenes`).
fn card_turn(shots: &[Value], id: &str) -> Option<(usize, usize, bool)> {
	for (role, ask) in [("-ask-", true), ("-answer-", false)] {
		let Some(split) = id.rfind(role) else { continue };
		let (card, k) = (&id[..split], &id[split + role.len()..]);
		// A card's own id may hold `-ask-` (`q-ask-x-answer-1`): no number
		// here is the other split's to try, never an end.
		let Some(k) = k.parse::<usize>().ok().filter(|k| *k >= 1) else { continue };
		let at = shots.iter().position(|shot| {
			shot.get("id").and_then(Value::as_str) == Some(card) && shot.get("spoken").and_then(|spoken| spoken.get("role")).and_then(Value::as_str) == Some("spoken")
		})?;
		let exchanges = shots[at].get("spoken").and_then(|spoken| spoken.get("exchanges")).and_then(Value::as_array)?;
		return (k <= exchanges.len()).then_some((at, k, ask));
	}
	None
}

/// Whether exchange `k` (from 1) of the card at `shots[at]` has the owner
/// saying its question (`ask`) or its answer.
fn owners(shots: &[Value], at: usize, k: usize, ask: bool) -> bool {
	let turn = if ask { "ask" } else { "answer" };
	shots[at]
		.get("spoken")
		.and_then(|spoken| spoken.get("exchanges"))
		.and_then(Value::as_array)
		.and_then(|exchanges| exchanges.get(k - 1))
		.and_then(|exchange| exchange.get(turn))
		.and_then(|turn| turn.get("voice"))
		.and_then(Value::as_str)
		== Some("owner")
}

/// Each of `ids`, scenes as the recorder names one that overflows, by place.
///
/// In order: a spoken card's turn by its chapter and exchange; any
/// other shot by its chapter and its number among the essay's shots
/// (`shot <n>`, the cut list's own count, not the video's scenes). `None`
/// when any is no scene of `essay`.
#[must_use]
pub fn scene_names(essay: &Value, ids: &[String]) -> Option<Vec<String>> {
	let shots = shots(essay);
	let chapters = chapters(essay);
	ids
		.iter()
		.map(|id| {
			// A card's turn first: its id is the card's and the turn's, and a
			// shot named the same never takes its place.
			if let Some((at, k, ask)) = card_turn(shots, id) {
				return Some(named(chapters.get(at)?.as_deref(), &turn_name(k, ask, owners(shots, at, k, ask))));
			}
			let at = shots.iter().position(|shot| shot.get("id").and_then(Value::as_str) == Some(id.as_str()))?;
			Some(named(chapters.get(at)?.as_deref(), &(String::from("shot ") + &(at + 1).to_string())))
		})
		.collect()
}

/// Each of `takes`, note ids a spoken card plays, by where it is played.
///
/// In order: its chapter, its exchange, and whether it is the owner's question
/// or answer. `None` when any is played by no spoken card of `essay`.
#[must_use]
pub fn take_names(essay: &Value, takes: &[String]) -> Option<Vec<String>> {
	let shots = shots(essay);
	let chapters = chapters(essay);
	takes
		.iter()
		.map(|take| {
			for (at, shot) in shots.iter().enumerate() {
				let Some(spoken) = shot.get("spoken").filter(|spoken| spoken.get("role").and_then(Value::as_str) == Some("spoken")) else {
					continue;
				};
				let exchanges = spoken.get("exchanges").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
				for (k, exchange) in exchanges.iter().enumerate() {
					for (turn, ask) in [("ask", true), ("answer", false)] {
						let owner = exchange.get(turn).filter(|turn| turn.get("voice").and_then(Value::as_str) == Some("owner"));
						if owner.and_then(|turn| turn.get("take")).and_then(Value::as_str) == Some(take.as_str()) {
							return Some(named(chapters.get(at)?.as_deref(), &turn_name(k + 1, ask, true)));
						}
					}
				}
			}
			None
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn essay() -> Value {
		json!({
			"shots": [
				{ "id": "intro", "spoken": { "role": "intro", "said": "Hello." } },
				{ "id": "dive-1", "spoken": { "role": "scene", "chapter": "Snapshots,   not backups.", "narration": "A snapshot is free." } },
				{ "id": "dive-2", "spoken": { "role": "scene", "narration": "It costs nothing to take." } },
				{ "id": "me", "spoken": { "role": "spoken", "category": "questions", "exchanges": [
					{ "ask": { "voice": "kokoro", "said": "Why snapshots?" }, "answer": { "voice": "owner", "take": "note-a" } },
					{ "ask": { "voice": "owner", "take": "note-b" }, "answer": { "voice": "kokoro", "said": "Yes." } },
				] } },
				// A shot named as a card's turn would be: never taken for it.
				{ "id": "me-ask-1", "spoken": { "role": "scene", "narration": "Odd name." } },
			],
		})
	}

	fn ids(names: &[&str]) -> Vec<String> {
		names.iter().map(|name| (*name).to_owned()).collect()
	}

	#[test]
	fn a_scene_is_named_by_its_chapter_and_place_never_its_id() {
		assert_eq!(
			scene_names(&essay(), &ids(&["dive-2", "intro", "me-answer-1", "me-ask-1", "me-ask-2"])),
			Some(ids(&[
				"Snapshots, not backups., shot 3",
				"shot 1",
				"Snapshots, not backups., exchange 1, your answer",
				"Snapshots, not backups., exchange 1, the question",
				"Snapshots, not backups., exchange 2, your question",
			]))
		);
		assert_eq!(scene_names(&essay(), &ids(&["dive-2", "nope"])), None, "named whole or not at all");
		assert_eq!(scene_names(&essay(), &ids(&["me-ask-3"])), None, "an exchange the card doesn't have");
		assert_eq!(scene_names(&essay(), &ids(&["dive-1-ask-1"])), None, "a turn of a shot that is no spoken card");
		assert_eq!(scene_names(&json!({}), &ids(&["dive-1"])), None);
		// A card's own id may hold `-ask-`: each of its turns is still its.
		let odd = json!({ "shots": [
			{ "id": "q-ask-x", "spoken": { "role": "spoken", "category": "introduction", "exchanges": [
				{ "ask": { "voice": "owner", "take": "note-c" }, "answer": { "voice": "kokoro", "said": "Hi." } },
			] } },
		] });
		assert_eq!(
			scene_names(&odd, &ids(&["q-ask-x-answer-1", "q-ask-x-ask-1"])),
			Some(ids(&["Introduction, exchange 1, the answer", "Introduction, exchange 1, your question"])),
			"the opening introduction card under its own mark"
		);
		assert_eq!(scene_names(&essay(), &[]), Some(Vec::new()));
	}

	#[test]
	fn a_take_is_named_by_where_its_card_plays_it() {
		assert_eq!(
			take_names(&essay(), &ids(&["note-b", "note-a"])),
			Some(ids(&[
				"Snapshots, not backups., exchange 2, your question",
				"Snapshots, not backups., exchange 1, your answer"
			]))
		);
		assert_eq!(take_names(&essay(), &ids(&["note-z"])), None, "a take no card plays is never named");
		let early = json!({ "shots": [{ "id": "me", "spoken": { "role": "spoken", "category": "questions", "exchanges": [
			{ "ask": { "voice": "kokoro", "said": "Who?" }, "answer": { "voice": "owner", "take": "note-a" } },
		] } }] });
		assert_eq!(take_names(&early, &ids(&["note-a"])), Some(ids(&["exchange 1, your answer"])), "no chapter before it");
	}

	#[test]
	fn a_name_is_one_short_line() {
		let long = "word ".repeat(60);
		let essay = json!({ "shots": [{ "id": "a", "spoken": { "role": "scene", "chapter": long, "narration": "n" } }] });
		let names = scene_names(&essay, &ids(&["a"])).unwrap();
		assert!(names[0].chars().count() <= MOST_CHARS, "{}", names[0]);
		assert!(names[0].ends_with("…, shot 1"), "{}", names[0]);
		assert!(!names[0].contains('\n'));
	}
}
