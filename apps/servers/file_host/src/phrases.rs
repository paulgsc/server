//! The phrase shelf's intents in home's meaning index, and the shop's lookup
//! (paulgsc/server#1261, KPLAY's KP4; paulgsc/some-ui#2853).
//!
//! The owner says, in English, what they want to say (a hunch), and home
//! finds the Korean phrases that do it, deterministically: a model writes a
//! phrase only when nothing fits (KP5), from the hunches kept here.
//!
//! - **The intents are a synced file** ([`INTENTS`], `phrases/intents.json`,
//!   call C1605): some-ui's phrase shelf
//!   (`packages/ui/topik/src/lib/topik/phrases/shelf.ts`), each phrase's id
//!   and its English intents, copied here with the commit it was copied from
//!   and compiled in. A phrase reaches home with a release; a copy that does
//!   not index takes down the lookup alone (`meaning::phrase_index`). The phone and
//!   home may hold different shelves for a while, and each side is safe: the
//!   phone drops a phrase id its build does not hold, and home names only
//!   phrases in its copy.
//! - **One vector per intent, keyed by phrase id** ([`Phrases`]), made by the
//!   meaning index's model (`meaning::StaticModel`) once, at start, in memory:
//!   the shelf is the build's, not the owner's, so there is no table.
//! - **Home decides** ([`Phrases::look_up`]). A phrase's score is its nearest
//!   intent's cosine with the hunch's words. Every phrase at or above
//!   [`THRESHOLD`] and within [`NEAR`] of the best is a hit, closest first.
//!   None is a miss, and the route keeps the hunch in the request that
//!   answers it (`korean_repo::hunch`), so a miss is never lost between two
//!   calls. Both numbers are policy, moved by a call.
//! - **Nothing it reads reaches a log line**: neither the words nor a vector
//!   nor a score.

use serde::{Deserialize, Serialize};
use voice_repo::meaning::{cosine, Embed};

/// The shelf's intents, as some-ui's shelf holds them (call C1605).
pub const INTENTS: &str = include_str!("phrases/intents.json");

/// The schema [`INTENTS`] is written in.
pub const SCHEMA: &str = "some-ui.phrase-intents/1";

/// The least cosine a phrase's nearest intent may have with a hunch's words
/// for the phrase to do it.
///
/// Policy, set by call C1606 from `potion-multilingual-128M`'s scores over the
/// seed shelf (docs/phrases.md, "The threshold"): a hunch said in other words
/// than an intent's mostly scores above it ("take a seat", 0.81 to
/// `ph.please-sit-here`), and an unrelated sentence that shares an intent's
/// words below ("see you tomorrow", 0.746 to `ph.will-it-rain-tomorrow`). A
/// static model is coarse, so it errs toward a miss: a miss is kept for the
/// routine that writes phrases, and a wrong phrase would be shown.
pub const THRESHOLD: f32 = 0.75;

/// How far below the best phrase's score a runner-up may be and still come.
///
/// Policy (C1606): close phrases come together, and a phrase that only
/// clears [`THRESHOLD`] beside a much nearer one does not.
pub const NEAR: f32 = 0.1;

const _: () = assert!(THRESHOLD > 0.0 && THRESHOLD < 1.0 && NEAR >= 0.0);

/// A phrase of the shelf, as [`INTENTS`] holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Phrase {
	id: String,
	intents: Vec<String>,
}

/// [`INTENTS`]' shape.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Shelf {
	schema: String,
	/// Which shelf the copy is of, and at which commit: read by people.
	#[serde(rename = "source")]
	_source: String,
	phrases: Vec<Phrase>,
}

/// Whether `id` reads as a phrase's id: `ph.` and a lowercase slug.
fn is_phrase_id(id: &str) -> bool {
	id.strip_prefix("ph.").is_some_and(|slug| {
		!slug.is_empty() && !slug.starts_with('-') && !slug.ends_with('-') && slug.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
	})
}

/// Why a shelf is refused, naming the phrase when there is one.
fn refused(why: &str, id: &str) -> anyhow::Error {
	anyhow::Error::msg(String::from(why) + id)
}

/// The shelf's phrases, their ids each once and each with an intent.
fn shelf(text: &str) -> anyhow::Result<Vec<Phrase>> {
	let shelf: Shelf = serde_json::from_str(text).map_err(|err| refused("the phrase intents are not the shelf's shape: ", &err.to_string()))?;
	if shelf.schema != SCHEMA {
		return Err(refused("the phrase intents are not ", SCHEMA));
	}
	if shelf.phrases.is_empty() {
		return Err(refused("the phrase intents name no phrase", ""));
	}
	let mut seen = std::collections::HashSet::new();
	for phrase in &shelf.phrases {
		if !is_phrase_id(&phrase.id) {
			return Err(refused("a phrase's id is not ph.<slug>: ", &phrase.id));
		}
		if !seen.insert(phrase.id.as_str()) {
			return Err(refused("a phrase is named twice: ", &phrase.id));
		}
		if phrase.intents.is_empty() || phrase.intents.iter().any(|intent| intent.trim().is_empty()) {
			return Err(refused("a phrase has no intent, or an empty one: ", &phrase.id));
		}
	}
	Ok(shelf.phrases)
}

/// Every intent of the shelf, embedded: one vector per intent, keyed by its
/// phrase's id, in the shelf's order.
pub struct Phrases {
	intents: Vec<(String, Vec<f32>)>,
}

impl std::fmt::Debug for Phrases {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Phrases").field("intents", &self.intents.len()).finish_non_exhaustive()
	}
}

impl Phrases {
	/// [`INTENTS`] embedded by `embed`.
	///
	/// # Errors
	/// As [`Self::from_text`].
	pub fn index(embed: &dyn Embed) -> anyhow::Result<Self> {
		Self::from_text(INTENTS, embed)
	}

	/// The intents `text` holds, embedded by `embed`.
	///
	/// # Errors
	/// `text` not in [`SCHEMA`]'s shape, a phrase named twice or with no
	/// intent, or an intent the model knows none of the words of, whose vector
	/// would point nowhere and never match.
	pub fn from_text(text: &str, embed: &dyn Embed) -> anyhow::Result<Self> {
		let mut intents = Vec::new();
		for phrase in shelf(text)? {
			for intent in &phrase.intents {
				let vector = embed.embed(intent);
				if vector.iter().all(|x| *x == 0.0) {
					return Err(refused("the model knows none of an intent's words, of ", &phrase.id));
				}
				intents.push((phrase.id.clone(), vector));
			}
		}
		Ok(Self { intents })
	}

	/// The phrases a hunch whose words embed to `words` finds: each phrase
	/// whose nearest intent's cosine is at least [`THRESHOLD`] and within
	/// [`NEAR`] of the best phrase's, closest first (as near as each other, in
	/// the shelf's order, as far as floats tell a tie). Empty is a miss.
	#[must_use]
	pub fn look_up(&self, words: &[f32]) -> Vec<&str> {
		let mut scores: Vec<(&str, f32)> = Vec::new();
		for (id, intent) in &self.intents {
			let score = cosine(words, intent);
			match scores.iter_mut().find(|(seen, _)| seen == id) {
				Some((_, best)) => *best = best.max(score),
				None => scores.push((id, score)),
			}
		}
		let best = scores.iter().map(|(_, score)| *score).fold(f32::NEG_INFINITY, f32::max);
		let mut hits: Vec<(&str, f32)> = scores.into_iter().filter(|(_, score)| *score >= THRESHOLD && *score >= best - NEAR).collect();
		// Stable, so phrases as near as each other keep the shelf's order.
		hits.sort_by(|a, b| b.1.total_cmp(&a.1));
		hits.into_iter().map(|(id, _)| id).collect()
	}
}

/// The words a hunch heard in a take is looked up and kept by.
///
/// `text` trimmed, and cut to its first
/// [`korean_repo::hunch::MAX_WORDS_CHARS`] characters, as a hunch's words are
/// held, so a long take is kept by its start rather than not at all; `None`
/// for nothing heard.
#[must_use]
pub fn heard_words(text: &str) -> Option<String> {
	let words: String = text.trim().chars().take(korean_repo::hunch::MAX_WORDS_CHARS).collect();
	let words = words.trim_end();
	(!words.is_empty()).then(|| words.to_owned())
}

/// Home's answer to a hunch's words, as the shop's port reads it
/// (some-ui's `packages/ui/topik/src/lib/topik/shop`, `Lookup`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Lookup {
	/// The phrases that do it, closest first; never empty.
	Hit { phrases: Vec<String> },
	/// None did, and home kept the hunch.
	Kept,
}

#[cfg(test)]
pub(crate) mod tests {
	use super::{is_phrase_id, shelf, Lookup, Phrases, INTENTS, NEAR, THRESHOLD};
	use voice_repo::meaning::{Embed, DIMENSIONS};

	/// A model with one axis per word it knows, so a text's vector is the
	/// unit mean of its words' axes and two texts' cosine counts the words
	/// they share.
	pub(crate) struct Words(pub &'static [&'static str]);

	impl Embed for Words {
		fn model(&self) -> &'static str {
			"words"
		}

		fn embed(&self, text: &str) -> Vec<f32> {
			let mut v = vec![0.0_f32; DIMENSIONS];
			for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
				if let Some(axis) = self.0.iter().position(|known| known.eq_ignore_ascii_case(word)) {
					v[axis] += 1.0;
				}
			}
			let norm = voice_repo::meaning::norm(&v);
			if norm > 0.0 {
				for x in &mut v {
					*x /= norm;
				}
			}
			v
		}
	}

	pub(crate) const VOCABULARY: &[&str] = &[
		"where", "my", "phone", "put", "find", "eat", "together", "rain", "tomorrow", "lost", "with", "me", "will", "it",
	];

	pub(crate) const SHELF: &str = r#"{"schema": "some-ui.phrase-intents/1", "source": "test", "phrases": [
		{"id": "ph.where-is-my-phone", "intents": ["where my phone", "find my phone"]},
		{"id": "ph.lost-my-phone", "intents": ["lost my phone"]},
		{"id": "ph.shall-we-eat-together", "intents": ["eat together", "eat with me"]},
		{"id": "ph.will-it-rain-tomorrow", "intents": ["will it rain tomorrow"]}
	]}"#;

	fn phrases() -> Phrases {
		Phrases::from_text(SHELF, &Words(VOCABULARY)).unwrap()
	}

	fn look_up(words: &str) -> Vec<String> {
		phrases().look_up(&Words(VOCABULARY).embed(words)).into_iter().map(str::to_owned).collect()
	}

	/// An intent said as written is its phrase, alone when nothing else is
	/// near.
	#[test]
	fn an_intent_said_as_written_finds_its_phrase() {
		assert_eq!(look_up("where my phone"), ["ph.where-is-my-phone"]);
		assert_eq!(look_up("will it rain tomorrow"), ["ph.will-it-rain-tomorrow"]);
	}

	/// A phrase scores by its nearest intent, and a close runner-up comes
	/// with the best, closest first.
	#[test]
	fn the_nearest_intent_scores_and_close_runners_up_come_too() {
		// "find my phone" is 1.0 to the first phrase; "lost my phone" shares
		// two words of three, 0.67, below the threshold and NEAR.
		assert_eq!(look_up("find my phone"), ["ph.where-is-my-phone"]);
		// "my phone" is 0.82 to both "where my phone" and "lost my phone":
		// tied, so both, in either order, as floats may break a tie.
		let mut tied = look_up("my phone");
		tied.sort_unstable();
		assert_eq!(tied, ["ph.lost-my-phone", "ph.where-is-my-phone"]);
		// "eat with me" is the second phrase's own intent.
		assert_eq!(look_up("eat with me"), ["ph.shall-we-eat-together"]);
	}

	/// Words near nothing, or known to the model not at all, are a miss.
	#[test]
	fn a_hunch_near_nothing_is_a_miss() {
		assert!(look_up("rain").is_empty(), "0.5 to will-it-rain-tomorrow, below the threshold");
		assert!(look_up("good morning").is_empty(), "no word the model knows");
	}

	#[test]
	fn the_policy_reads_as_its_call_says() {
		assert!((THRESHOLD - 0.75).abs() < f32::EPSILON && (NEAR - 0.1).abs() < f32::EPSILON);
	}

	/// The compiled-in shelf reads, every phrase once with an intent, and the
	/// story's own case is on it.
	#[test]
	fn the_synced_shelf_reads() {
		let phrases = shelf(INTENTS).unwrap();
		assert_eq!(phrases.len(), 14);
		assert_eq!(phrases[0].id, "ph.where-is-my-phone");
		assert!(phrases[0].intents.iter().any(|intent| intent == "where's my phone"));
	}

	#[test]
	fn a_shelf_out_of_shape_is_refused_by_what_is_wrong() {
		let refused = |text: &str| Phrases::from_text(text, &Words(VOCABULARY)).unwrap_err().to_string();
		assert!(refused("{}").starts_with("the phrase intents are not the shelf's shape"));
		assert_eq!(
			refused(r#"{"schema": "x", "source": "", "phrases": []}"#),
			"the phrase intents are not some-ui.phrase-intents/1"
		);
		assert_eq!(
			refused(r#"{"schema": "some-ui.phrase-intents/1", "source": "", "phrases": []}"#),
			"the phrase intents name no phrase"
		);
		let one = |phrases: &str| String::from(r#"{"schema": "some-ui.phrase-intents/1", "source": "", "phrases": ["#) + phrases + "]}";
		assert_eq!(refused(&one(r#"{"id": "where", "intents": ["my phone"]}"#)), "a phrase's id is not ph.<slug>: where");
		assert_eq!(
			refused(&one(r#"{"id": "ph.a", "intents": ["my phone"]}, {"id": "ph.a", "intents": ["eat"]}"#)),
			"a phrase is named twice: ph.a"
		);
		assert_eq!(refused(&one(r#"{"id": "ph.a", "intents": []}"#)), "a phrase has no intent, or an empty one: ph.a");
		assert_eq!(
			refused(&one(r#"{"id": "ph.a", "intents": ["good morning"]}"#)),
			"the model knows none of an intent's words, of ph.a"
		);
	}

	#[test]
	fn words_heard_are_trimmed_and_cut_to_what_a_hunch_holds() {
		assert_eq!(super::heard_words("  where my phone \n").as_deref(), Some("where my phone"));
		assert_eq!(super::heard_words("   "), None);
		let long = String::from(" ") + &"가".repeat(250);
		let cut = super::heard_words(&long).unwrap();
		assert_eq!(cut.chars().count(), 200);
		assert!(korean_repo::hunch::is_words(&cut));
	}

	#[test]
	fn a_phrase_id_is_ph_and_a_slug() {
		assert!(is_phrase_id("ph.where-is-my-phone") && is_phrase_id("ph.a1"));
		assert!(!is_phrase_id("ph.") && !is_phrase_id("ph.-a") && !is_phrase_id("ph.a-") && !is_phrase_id("ph.A") && !is_phrase_id("where"));
	}

	#[test]
	fn the_answer_is_the_ports_shape() {
		let hit = Lookup::Hit {
			phrases: vec!["ph.where-is-my-phone".into()],
		};
		assert_eq!(serde_json::to_value(hit).unwrap(), serde_json::json!({"kind": "hit", "phrases": ["ph.where-is-my-phone"]}));
		assert_eq!(serde_json::to_value(Lookup::Kept).unwrap(), serde_json::json!({"kind": "kept"}));
	}
}
