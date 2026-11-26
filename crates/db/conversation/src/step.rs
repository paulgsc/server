//! The record's step, as home replays it (HOST16-S, paulgsc/server#1243).
//!
//! A conversation's state is its log replayed (paulgsc/some-ui
//! `packages/video-render/src/conversation.ts`, `stepConversation`, a pure
//! function), and an event the step refuses is never kept. Home replays the
//! log it keeps with the rules here before it keeps one more.
//!
//! **What home holds, and what it leaves to the writer.** The step's rules
//! fall in two kinds:
//!
//! - **The log's own structure,** held here exactly as `stepConversation`
//!   holds it, under its names: an event written against the log as it
//!   stands (`basis`, else `stale`), by its kind's author (`EVENT_AUTHORS`,
//!   else `author`), which events each phase accepts
//!   (`CONVERSATION_ACCEPTS`), a point once and at most [`POINTS_MOST`], each
//!   version numbered next and answering every kind of every open flag once
//!   (`answer-kinds`, `answer-twice`, `unanswered`), a flag once, on the
//!   version the owner heard (`flag-version`) and on a turn or speaker it
//!   has, a withdrawal of an open flag only, the measure and the green light
//!   of the newest version only, under [`MOST_MS`], notes only once
//!   green-lit, on a turn of the newest version whose words the record holds,
//!   each word once and inside the turn, following at least one rule and
//!   only rules a standing approval set, each offer made once (`offer-again`)
//!   and only once green-lit, and each pick naming an offer made
//!   (`preview-unknown`) and one of its variants (`preview-variant`),
//!   changing a standing pick only by moving it (`moves-unknown`,
//!   `preview-again`), and setting a rule another preview's pick set only by
//!   moving that one (`rule-again`) (C1582).
//! - **The content rules that rest on some-ui's closed vocabularies** (the
//!   cast and its roles, each character's emotions, the flag kinds' voices,
//!   scopes and answers, Kokoro's voices, how one version's turns differ from
//!   the last's, and a /1 record's lift, which home never keeps) are not ported: they would be a second copy of those lists
//!   that drifts from the first. Every writer runs `stepConversation` before
//!   it sends (the phone, Studio and the agent all do), and home trusts it for
//!   those. `docs/conversations.md` names the split and its invariant.
//!
//! So an event home refuses is one `stepConversation` refuses too, with the
//! same `why` and fields, and when an event breaks more than one rule home
//! names the first of those it holds.
//!
//! **Order is the log's, never a clock** (paulgsc/some-ui C1580;
//! docs/conveyor.md invariant 2). An event's `at` is its writer's clock,
//! kept and never compared. Its place is its index in the log, and a writer
//! that names `basis`, the events the log held when it wrote, is refused as
//! `stale` when the log has moved since.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The record's schema.
pub const SCHEMA: &str = "some-ui.conversation/2";

/// `CONVERSATION_POLICY.pointsMost`: the most points one conversation has.
pub const POINTS_MOST: usize = 40;

/// `CONVERSATION_POLICY.mostMs`: the longest podcast a version may be
/// green-lit at, as home measured it.
pub const MOST_MS: u64 = 20 * 60_000;

/// The bounds of an offer (`PreviewOfferSchema`, C1582): its heading, a
/// variant's title, how many callouts and each one's length.
const OFFER_ABOUT_MOST: usize = 80;
const OFFER_TITLE_MOST: usize = 60;
const OFFER_CALLOUTS_MOST: usize = 12;
const OFFER_CALLOUT_MOST: usize = 80;

/// `CONVERSATION_POLICY.noteMost`: the owner's own words on a preview.
const SAID_MOST: usize = 4000;

/// `AUTHORS`: who writes a record's events (C1581).
pub const AUTHORS: [&str; 3] = ["owner", "agent", "home"];

/// `PREVIEW_VARIANTS_MOST`: the most variants one preview offers.
pub const PREVIEW_VARIANTS_MOST: usize = 3;

/// The most characters of a flag's, a preview's, a variant's or a rule's id
/// (`FlagIdSchema`, trimmed).
const NAME_MOST: usize = 64;

/// Where a conversation stands (`ConversationPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
	New,
	Drafting,
	Podcast,
	Flagged,
	GreenLit,
}

impl Phase {
	/// `CONVERSATION_ACCEPTS`: the events this phase accepts.
	#[must_use]
	pub const fn accepts(self) -> &'static [Kind] {
		match self {
			Self::New => &[Kind::Topic],
			Self::Drafting => &[Kind::Point, Kind::Version],
			Self::Podcast => &[Kind::Point, Kind::Flag, Kind::Heard, Kind::GreenLight],
			Self::Flagged => &[Kind::Point, Kind::Flag, Kind::Withdraw, Kind::Version, Kind::Heard],
			Self::GreenLit => &[Kind::Notes, Kind::Offer, Kind::Approve],
		}
	}
}

/// An event's kind (`ConversationEventKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
	Topic,
	Point,
	Version,
	Flag,
	Withdraw,
	Heard,
	GreenLight,
	Notes,
	Offer,
	Approve,
}

impl Kind {
	/// Every kind, in the schema's order.
	pub const ALL: [Self; 10] = [
		Self::Topic,
		Self::Point,
		Self::Version,
		Self::Flag,
		Self::Withdraw,
		Self::Heard,
		Self::GreenLight,
		Self::Notes,
		Self::Offer,
		Self::Approve,
	];

	/// The wire name, as the event's `event` field and the table's `event`
	/// column write it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Topic => "topic",
			Self::Point => "point",
			Self::Version => "version",
			Self::Flag => "flag",
			Self::Withdraw => "withdraw",
			Self::Heard => "heard",
			Self::GreenLight => "green-light",
			Self::Notes => "notes",
			Self::Offer => "offer",
			Self::Approve => "approve",
		}
	}

	/// `EVENT_AUTHORS`: who may write an event of this kind (C1581). The
	/// owner never sets a value: they give the topic and points, flag,
	/// withdraw, green-light and approve; the agent writes versions and
	/// notes; home measures.
	#[must_use]
	pub const fn authors(self) -> &'static [&'static str] {
		match self {
			Self::Topic | Self::Point | Self::Flag | Self::Withdraw | Self::GreenLight | Self::Approve => &["owner"],
			Self::Version | Self::Notes | Self::Offer => &["agent"],
			Self::Heard => &["home"],
		}
	}

	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|kind| kind.as_str() == name)
	}

	/// The fields an event of this kind has, beside `event`, `at` and `by`:
	/// each required.
	const fn fields(self) -> &'static [&'static str] {
		match self {
			Self::Topic => &["topic"],
			Self::Point => &["take"],
			Self::Version | Self::GreenLight => &["version"],
			Self::Flag => &["version", "flag"],
			Self::Withdraw => &["flag"],
			Self::Heard => &["version", "ms"],
			Self::Notes => &["turn", "words", "rules"],
			Self::Offer => &["offer"],
			Self::Approve => &["preview", "chose"],
		}
	}

	const fn optional(self) -> &'static [&'static str] {
		match self {
			Self::Approve => &["moves"],
			_ => &[],
		}
	}
}

/// Why an event was refused (`ConversationRefusal`, the cases home holds):
/// the same `why` and fields, so the phone reads it with `refusalLine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "why", rename_all = "kebab-case")]
pub enum Refusal {
	Stale { basis: u64, seq: u64 },
	Author { event: Kind, by: String },
	OutOfPhase { phase: Phase, event: Kind },
	TooManyPoints,
	PointAgain { take: String },
	VersionNumber { expected: u64, got: u64 },
	Unanswered { flag: String },
	AnswerUnknown { flag: String },
	AnswerTwice { flag: String },
	AnswerKinds { flag: String, kind: String },
	FlagAgain { flag: String },
	FlagVersion { flag: String, expected: u64, got: u64 },
	TurnUnknown { flag: String, turn: String },
	FlagSpeakerUnknown { flag: String, speaker: String },
	FlagNotOpen { flag: String },
	HeardVersion { expected: u64, got: u64 },
	HeardAgain { version: u64 },
	Unheard { version: u64 },
	TooLong { ms: u64 },
	GreenLightVersion { expected: u64, got: u64 },
	NotesTurn { turn: String },
	NotesWord { turn: String, at: u64 },
	NotesRule { rule: String },
	OfferAgain { preview: String },
	PreviewUnknown { preview: String },
	MovesUnknown { preview: String },
	PreviewAgain { preview: String },
	RuleAgain { rule: String },
	PreviewVariant { preview: String, variant: String },
}

/// One event, read: its kind, when, and the event as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
	pub kind: Kind,
	/// `at`, as sent: the writer's clock, never compared.
	pub at: String,
	/// How many events the log held when the writer wrote it, if it says.
	basis: Option<u64>,
	/// Who wrote it (`AUTHORS`): `owner`, `agent` or `home`.
	pub by: String,
	body: Body,
	/// The whole event, as sent.
	pub value: Value,
}

impl Event {
	/// The offer it makes, if it is one.
	#[must_use]
	pub const fn offer(&self) -> Option<&Offer> {
		match &self.body {
			Body::Offer(offer) => Some(offer),
			_ => None,
		}
	}
}

/// What home reads of each kind.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Body {
	Topic,
	Point { take: String },
	Version(Version),
	Flag { version: u64, id: String, on: Scope, kinds: Vec<String> },
	Withdraw { flag: String },
	Heard { version: u64, ms: u64 },
	GreenLight { version: u64 },
	Notes { turn: String, words: Vec<u64>, rules: Vec<String> },
	Offer(Offer),
	Approve(Pick),
}

/// What a flag covers (`FlagScopeSchema`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
	Turn(String),
	Speaker(String),
	Conversation,
}

/// What home keeps of a version: its number, its speakers' ids, its turns,
/// and the flags it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
	n: u64,
	speakers: Vec<String>,
	turns: Vec<Turn>,
	/// Each answer's flag and the kinds it names (else all the flag's), in
	/// order; an `unasked` change names none.
	answers: Vec<(String, Option<Vec<String>>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Turn {
	id: String,
	speaker: String,
	/// Its words, as the notes count them, where the record knows them
	/// (`wordsOf`).
	words: Option<usize>,
}

/// A preview the agent offers the owner (`offer`, C1582): what it bundles,
/// its two or three variants, and the one recommended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
	pub preview: String,
	pub about: String,
	pub recommended: String,
	pub variants: Vec<OfferVariant>,
}

/// One variant of an offer: its name, what its clip calls out, and the rule
/// picking it sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferVariant {
	pub variant: String,
	pub title: String,
	pub callouts: Vec<String>,
	pub rule: String,
}

/// The owner's pick on an offer, as sent (`approve`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pick {
	preview: String,
	chose: Picked,
	moves: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Picked {
	Variant { variant: String },
	Else { said: String },
}

/// An approval as kept (`KeptApproval`): the preview, what the owner chose
/// with the rule it set, and the approval it moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approval {
	pub preview: String,
	pub chose: Choice,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub moves: Option<String>,
}

/// What the owner chose on a preview: a variant and the rule it set, or
/// something else in their own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Choice {
	Variant { variant: String, rule: String },
	Else { said: String },
}

/// A flag as the state keeps it (`KeptFlag`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeptFlag {
	id: String,
	kinds: Vec<String>,
	version: u64,
	withdrawn: bool,
}

/// Where a conversation stands, as home keeps it: what the step reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
	phase: Phase,
	/// How many events the log holds: the next event's place.
	seq: u64,
	points: Vec<String>,
	latest: Option<Version>,
	flags: Vec<KeptFlag>,
	/// Each measured version and how long it runs.
	heard: Vec<(u64, u64)>,
	offers: Vec<Offer>,
	/// Each preview's standing approval (`standingApprovals`), in the order
	/// first set: a later approval that moves one takes its place.
	standing: Vec<Approval>,
}

impl Default for State {
	fn default() -> Self {
		Self::new()
	}
}

impl State {
	/// A conversation before its topic (`NEW_CONVERSATION`).
	#[must_use]
	pub const fn new() -> Self {
		Self {
			phase: Phase::New,
			seq: 0,
			points: Vec::new(),
			latest: None,
			flags: Vec::new(),
			heard: Vec::new(),
			offers: Vec::new(),
			standing: Vec::new(),
		}
	}

	#[must_use]
	pub const fn phase(&self) -> Phase {
		self.phase
	}

	/// Each offer, in the order made.
	#[must_use]
	pub fn offers(&self) -> &[Offer] {
		&self.offers
	}

	/// `preview`'s standing approval, if the owner has picked on it.
	#[must_use]
	pub fn standing(&self, preview: &str) -> Option<&Approval> {
		self.standing.iter().find(|approval| approval.preview == preview)
	}

	/// The rules the standing approvals set, in their order
	/// (`approvedRules`).
	fn rules(&self) -> impl Iterator<Item = &str> {
		self.standing.iter().filter_map(|approval| match &approval.chose {
			Choice::Variant { rule, .. } => Some(rule.as_str()),
			Choice::Else { .. } => None,
		})
	}

	fn latest_n(&self) -> u64 {
		self.latest.as_ref().map_or(0, |version| version.n)
	}

	fn open_flags(&self) -> impl Iterator<Item = &KeptFlag> {
		let latest = self.latest.as_ref().map(|version| version.n);
		self.flags.iter().filter(move |flag| Some(flag.version) == latest && !flag.withdrawn)
	}

	/// `reviewing`: `flagged` while the newest version has an open flag.
	fn review(&mut self) {
		self.phase = if self.open_flags().next().is_some() { Phase::Flagged } else { Phase::Podcast };
	}

	/// The state after `event`, or why it is refused (`stepConversation`).
	/// A refused event leaves the state as it was.
	///
	/// # Errors
	/// The first rule `event` breaks, of those home holds.
	pub fn step(&self, event: &Event) -> Result<Self, Refusal> {
		let mut next = self.clone();
		next.apply(event)?;
		Ok(next)
	}

	fn apply(&mut self, event: &Event) -> Result<(), Refusal> {
		if let Some(basis) = event.basis {
			if basis != self.seq {
				return Err(Refusal::Stale { basis, seq: self.seq });
			}
		}
		if !event.kind.authors().contains(&event.by.as_str()) {
			return Err(Refusal::Author {
				event: event.kind,
				by: event.by.clone(),
			});
		}
		let out_of_phase = Refusal::OutOfPhase {
			phase: self.phase,
			event: event.kind,
		};
		if !self.phase.accepts().contains(&event.kind) {
			return Err(out_of_phase);
		}
		match &event.body {
			Body::Topic => {
				self.phase = Phase::Drafting;
			}
			Body::Point { take } => {
				if self.points.contains(take) {
					return Err(Refusal::PointAgain { take: take.clone() });
				}
				if self.points.len() >= POINTS_MOST {
					return Err(Refusal::TooManyPoints);
				}
				self.points.push(take.clone());
			}
			Body::Version(version) => {
				let expected = self.latest_n() + 1;
				if version.n != expected {
					return Err(Refusal::VersionNumber { expected, got: version.n });
				}
				self.answers_hold(version)?;
				self.latest = Some(version.clone());
				self.phase = Phase::Podcast;
			}
			Body::Flag { version, id, on, kinds } => self.flag(*version, id, on, kinds, out_of_phase)?,
			Body::Withdraw { flag } => {
				if !self.open_flags().any(|open| &open.id == flag) {
					return Err(Refusal::FlagNotOpen { flag: flag.clone() });
				}
				let latest = self.latest_n();
				for kept in &mut self.flags {
					if &kept.id == flag && kept.version == latest {
						kept.withdrawn = true;
					}
				}
				self.review();
			}
			Body::Heard { version, ms } => {
				let expected = self.latest_n();
				if *version != expected {
					return Err(Refusal::HeardVersion { expected, got: *version });
				}
				if self.heard.iter().any(|(heard, _)| heard == version) {
					return Err(Refusal::HeardAgain { version: *version });
				}
				self.heard.push((*version, *ms));
				self.review();
			}
			Body::GreenLight { version } => {
				let expected = self.latest_n();
				if *version != expected {
					return Err(Refusal::GreenLightVersion { expected, got: *version });
				}
				let Some(ms) = self.heard.iter().find(|(heard, _)| *heard == expected).map(|(_, ms)| *ms) else {
					return Err(Refusal::Unheard { version: expected });
				};
				if ms > MOST_MS {
					return Err(Refusal::TooLong { ms });
				}
				self.phase = Phase::GreenLit;
			}
			Body::Notes { turn, words, rules } => self.notes(turn, words, rules, out_of_phase)?,
			Body::Offer(offer) => {
				if self.offers.iter().any(|kept| kept.preview == offer.preview) {
					return Err(Refusal::OfferAgain { preview: offer.preview.clone() });
				}
				self.offers.push(offer.clone());
			}
			Body::Approve(pick) => self.approve(pick)?,
		}
		self.seq += 1;
		Ok(())
	}

	/// A flag on the newest version: once, on a turn or speaker it has.
	fn flag(&mut self, version: u64, id: &str, on: &Scope, kinds: &[String], out_of_phase: Refusal) -> Result<(), Refusal> {
		if self.flags.iter().any(|flag| flag.id == id) {
			return Err(Refusal::FlagAgain { flag: id.to_owned() });
		}
		let Some(latest) = &self.latest else {
			return Err(out_of_phase);
		};
		// A flag lands on the version the owner heard, or nowhere.
		if version != latest.n {
			return Err(Refusal::FlagVersion {
				flag: id.to_owned(),
				expected: latest.n,
				got: version,
			});
		}
		match on {
			Scope::Turn(turn) if !latest.turns.iter().any(|t| &t.id == turn) => {
				return Err(Refusal::TurnUnknown {
					flag: id.to_owned(),
					turn: turn.clone(),
				});
			}
			Scope::Speaker(speaker) if !latest.speakers.contains(speaker) => {
				return Err(Refusal::FlagSpeakerUnknown {
					flag: id.to_owned(),
					speaker: speaker.clone(),
				});
			}
			Scope::Turn(_) | Scope::Speaker(_) | Scope::Conversation => {}
		}
		self.flags.push(KeptFlag {
			id: id.to_owned(),
			kinds: kinds.to_vec(),
			version,
			withdrawn: false,
		});
		self.review();
		Ok(())
	}

	/// Notes on a turn of the newest version: each word once and inside it,
	/// following only rules an approval set.
	fn notes(&self, turn: &str, words: &[u64], rules: &[String], out_of_phase: Refusal) -> Result<(), Refusal> {
		let Some(latest) = &self.latest else {
			return Err(out_of_phase);
		};
		if let Some(rule) = rules.iter().find(|rule| !self.rules().any(|set| set == rule.as_str())) {
			return Err(Refusal::NotesRule { rule: rule.clone() });
		}
		let Some(found) = latest.turns.iter().find(|t| t.id == turn).filter(|t| latest.speakers.contains(&t.speaker)) else {
			return Err(Refusal::NotesTurn { turn: turn.to_owned() });
		};
		let mut seen = Vec::with_capacity(words.len());
		for &at in words {
			// A turn whose words the record does not hold takes no notes.
			let outside = found.words.is_none_or(|count| usize::try_from(at).map_or(true, |at| at >= count));
			if seen.contains(&at) || outside {
				return Err(Refusal::NotesWord { turn: turn.to_owned(), at });
			}
			seen.push(at);
		}
		Ok(())
	}

	/// An approval (C1582), in `stepConversation`'s order: of an offer made,
	/// naming one of its variants, moving only a standing approval, of a
	/// preview not yet standing unless it moves itself, and setting a rule
	/// another preview's standing approval set only by moving that one.
	fn approve(&mut self, pick: &Pick) -> Result<(), Refusal> {
		let preview = &pick.preview;
		let Some(offer) = self.offers.iter().find(|offer| &offer.preview == preview) else {
			return Err(Refusal::PreviewUnknown { preview: preview.clone() });
		};
		let chose = match &pick.chose {
			Picked::Variant { variant } => {
				let Some(offered) = offer.variants.iter().find(|v| &v.variant == variant) else {
					return Err(Refusal::PreviewVariant {
						preview: preview.clone(),
						variant: variant.clone(),
					});
				};
				Choice::Variant {
					variant: variant.clone(),
					rule: offered.rule.clone(),
				}
			}
			Picked::Else { said } => Choice::Else { said: said.clone() },
		};
		if let Some(moves) = &pick.moves {
			if self.standing(moves).is_none() {
				return Err(Refusal::MovesUnknown { preview: moves.clone() });
			}
		}
		if self.standing(preview).is_some() && pick.moves.as_ref() != Some(preview) {
			return Err(Refusal::PreviewAgain { preview: preview.clone() });
		}
		if let Choice::Variant { rule, .. } = &chose {
			let set_by = self
				.standing
				.iter()
				.find(|kept| &kept.preview != preview && matches!(&kept.chose, Choice::Variant { rule: set, .. } if set == rule));
			if set_by.is_some_and(|kept| pick.moves.as_ref() != Some(&kept.preview)) {
				return Err(Refusal::RuleAgain { rule: rule.clone() });
			}
		}
		let approval = Approval {
			preview: preview.clone(),
			chose,
			moves: pick.moves.clone(),
		};
		if let Some(moves) = pick.moves.as_ref().filter(|moves| *moves != preview) {
			self.standing.retain(|kept| &kept.preview != moves);
		}
		match self.standing.iter_mut().find(|kept| &kept.preview == preview) {
			Some(kept) => *kept = approval,
			None => self.standing.push(approval),
		}
		Ok(())
	}

	/// `answersRefusal`, the cases home holds: every answer names an open
	/// flag of the version before, each once, and every open flag is
	/// answered.
	fn answers_hold(&self, version: &Version) -> Result<(), Refusal> {
		let open: Vec<&KeptFlag> = self.open_flags().collect();
		// The kinds of each flag answered so far.
		let mut answered: Vec<(&str, Vec<&str>)> = Vec::new();
		for (id, named) in &version.answers {
			let Some(flag) = open.iter().find(|flag| &flag.id == id) else {
				return Err(Refusal::AnswerUnknown { flag: id.clone() });
			};
			// The kinds this answers: those it names, else all of the flag's.
			let kinds: Vec<&str> = named
				.as_ref()
				.map_or_else(|| flag.kinds.iter().map(String::as_str).collect(), |named| named.iter().map(String::as_str).collect());
			if let Some(foreign) = kinds.iter().find(|kind| !flag.kinds.iter().any(|own| own == *kind)) {
				return Err(Refusal::AnswerKinds {
					flag: flag.id.clone(),
					kind: (*foreign).to_owned(),
				});
			}
			let at = if let Some(at) = answered.iter().position(|(answered_id, _)| *answered_id == flag.id) {
				at
			} else {
				answered.push((flag.id.as_str(), Vec::new()));
				answered.len() - 1
			};
			let done = &mut answered[at].1;
			if kinds.iter().any(|kind| done.contains(kind)) {
				return Err(Refusal::AnswerTwice { flag: flag.id.clone() });
			}
			done.extend(kinds);
		}
		let whole = |flag: &&KeptFlag| {
			let done = answered.iter().find(|(id, _)| *id == flag.id).map_or(&[][..], |(_, kinds)| kinds.as_slice());
			flag.kinds.iter().all(|kind| done.contains(&kind.as_str()))
		};
		open
			.iter()
			.find(|flag| !whole(flag))
			.map_or(Ok(()), |flag| Err(Refusal::Unanswered { flag: flag.id.clone() }))
	}
}

/// The state `events` leave, or the first refused event's index and why
/// (`replayConversation`).
///
/// # Errors
/// The first event the step refuses, by its index in `events`.
pub fn replay<'a>(events: impl IntoIterator<Item = &'a Event>) -> Result<State, (usize, Refusal)> {
	let mut state = State::new();
	for (index, event) in events.into_iter().enumerate() {
		state.apply(event).map_err(|refusal| (index, refusal))?;
	}
	Ok(state)
}

/// A field that is not what the schema takes, and why.
pub type Problem = (String, &'static str);

fn problem(field: &str, why: &'static str) -> Vec<Problem> {
	vec![(field.to_owned(), why)]
}

/// A name (`FlagIdSchema`): trimmed, 1 to 64 characters.
fn is_name(text: &str) -> bool {
	let trimmed = text.trim();
	trimmed == text && !text.is_empty() && text.chars().count() <= NAME_MOST
}

/// An id (`showId`): `^[a-z0-9][a-z0-9-]{0,63}$`.
#[must_use]
pub fn is_id(text: &str) -> bool {
	let bytes = text.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= 64
		&& (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
		&& bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `z.iso.datetime()`: an RFC 3339 time in UTC, written with `Z`. Read only
/// to hold the field to its shape: no rule compares it.
fn instant(text: &str) -> Result<DateTime<FixedOffset>, Vec<Problem>> {
	let refused = || problem("at", "is an ISO-8601 time in UTC");
	if !text.ends_with('Z') {
		return Err(refused());
	}
	// Its parse error is the same refusal, of the field the caller names.
	let Ok(at) = DateTime::parse_from_rfc3339(text) else {
		return Err(refused());
	};
	Ok(at)
}

fn field<'a, T: Deserialize<'a>>(object: &'a Map<String, Value>, name: &str) -> Result<T, Vec<Problem>> {
	object.get(name).map_or_else(
		|| Err(problem(name, "is missing")),
		|value| T::deserialize(value).map_err(|err| vec![(name.to_owned(), serde_why(&err))]),
	)
}

fn serde_why(err: &serde_json::Error) -> &'static str {
	if err.is_data() {
		"is not what the schema takes"
	} else {
		"is not readable"
	}
}

#[derive(Deserialize)]
struct VersionWire {
	n: u64,
	speakers: Vec<SpeakerWire>,
	turns: Vec<TurnWire>,
	answers: Vec<AnswerWire>,
}

#[derive(Deserialize)]
struct SpeakerWire {
	id: String,
}

#[derive(Deserialize)]
struct TurnWire {
	id: String,
	speaker: String,
	voice: String,
	said: Option<String>,
	transcript: Option<String>,
	reads: Option<ReadsWire>,
}

#[derive(Deserialize)]
struct ReadsWire {
	said: String,
}

#[derive(Deserialize)]
struct AnswerWire {
	how: String,
	flag: Option<String>,
	kinds: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct FlagWire {
	id: String,
	on: ScopeWire,
	kinds: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "scope", rename_all = "kebab-case")]
enum ScopeWire {
	Turn { turn: String },
	Speaker { speaker: String },
	Conversation {},
}

#[derive(Deserialize)]
struct WordWire {
	at: u64,
}

fn words_in(said: &str) -> usize {
	said.split_whitespace().count()
}

impl TurnWire {
	fn turn(self) -> Turn {
		let said = if self.voice == "kokoro" {
			self.said
		} else {
			self.transcript.or_else(|| self.reads.map(|reads| reads.said))
		};
		Turn {
			id: self.id,
			speaker: self.speaker,
			words: said.as_deref().map(words_in),
		}
	}
}

fn version(value: VersionWire) -> Result<Version, Vec<Problem>> {
	if value.n < 1 {
		return Err(problem("version.n", "is a number from 1"));
	}
	let mut answers = Vec::with_capacity(value.answers.len());
	for answer in value.answers {
		if answer
			.kinds
			.as_ref()
			.is_some_and(|kinds| kinds.is_empty() || kinds.iter().enumerate().any(|(i, kind)| kinds[..i].contains(kind)))
		{
			return Err(problem("version.answers", "an answer's kinds are at least one, each once"));
		}
		match (answer.how.as_str(), answer.flag) {
			("unasked", None) if answer.kinds.is_none() => {}
			("unasked", _) => return Err(problem("version.answers", "an unasked change names no flag")),
			(_, Some(flag)) => answers.push((flag, answer.kinds)),
			(_, None) => return Err(problem("version.answers", "an answer names its flag")),
		}
	}
	Ok(Version {
		n: value.n,
		speakers: value.speakers.into_iter().map(|speaker| speaker.id).collect(),
		turns: value.turns.into_iter().map(TurnWire::turn).collect(),
		answers,
	})
}

fn body(kind: Kind, object: &Map<String, Value>) -> Result<Body, Vec<Problem>> {
	Ok(match kind {
		Kind::Topic => {
			let topic: Map<String, Value> = field(object, "topic")?;
			if topic.get("said").and_then(Value::as_str).is_none_or(|said| said.trim().is_empty()) {
				return Err(problem("topic.said", "is the owner's words"));
			}
			Body::Topic
		}
		Kind::Point => Body::Point { take: field(object, "take")? },
		Kind::Version => Body::Version(version(field(object, "version")?)?),
		Kind::Flag => {
			let flag: FlagWire = field(object, "flag")?;
			if !is_name(&flag.id) {
				return Err(problem("flag.id", "is a name of 1 to 64 characters"));
			}
			if flag.kinds.is_empty() || flag.kinds.iter().enumerate().any(|(i, kind)| flag.kinds[..i].contains(kind)) {
				return Err(problem("flag.kinds", "names at least one kind, each once"));
			}
			let version: u64 = field(object, "version")?;
			if version < 1 {
				return Err(problem("version", "is a number from 1"));
			}
			let on = match flag.on {
				ScopeWire::Turn { turn } => Scope::Turn(turn),
				ScopeWire::Speaker { speaker } => Scope::Speaker(speaker),
				ScopeWire::Conversation {} => Scope::Conversation,
			};
			Body::Flag {
				version,
				id: flag.id,
				on,
				kinds: flag.kinds,
			}
		}
		Kind::Withdraw => Body::Withdraw { flag: field(object, "flag")? },
		Kind::Heard => {
			let version: u64 = field(object, "version")?;
			let ms: u64 = field(object, "ms")?;
			if version < 1 || ms < 1 {
				return Err(problem("heard", "names a version from 1 and a length above 0"));
			}
			Body::Heard { version, ms }
		}
		Kind::GreenLight => {
			let version: u64 = field(object, "version")?;
			if version < 1 {
				return Err(problem("version", "is a number from 1"));
			}
			Body::GreenLight { version }
		}
		Kind::Notes => {
			let words: Vec<WordWire> = field(object, "words")?;
			if words.is_empty() {
				return Err(problem("words", "names at least one word"));
			}
			let rules: Vec<String> = field(object, "rules")?;
			if rules.is_empty() {
				return Err(problem("rules", "names at least one rule an approval set"));
			}
			Body::Notes {
				turn: field(object, "turn")?,
				words: words.into_iter().map(|word| word.at).collect(),
				rules,
			}
		}
		Kind::Offer => Body::Offer(offer(object)?),
		Kind::Approve => Body::Approve(pick(object)?),
	})
}

/// What home reads of an approval (`approve`, C1582).
fn pick(object: &Map<String, Value>) -> Result<Pick, Vec<Problem>> {
	let preview: String = field(object, "preview")?;
	let chose: Picked = field(object, "chose")?;
	let moves: Option<String> = if object.contains_key("moves") { Some(field(object, "moves")?) } else { None };
	if !is_id(&preview) || moves.as_deref().is_some_and(|moves| !is_id(moves)) {
		return Err(problem("preview", "is a preview's id"));
	}
	match &chose {
		Picked::Variant { variant } if !is_id(variant) => return Err(problem("chose.variant", "is a variant's id")),
		Picked::Else { said } if !fits(said, SAID_MOST) => return Err(problem("chose.said", "is the owner's words, at most 4,000 characters")),
		Picked::Variant { .. } | Picked::Else { .. } => {}
	}
	Ok(Pick { preview, chose, moves })
}

/// zod's `z.string().trim().min(1).max(most)`: trimmed, then 1 to `most`
/// characters.
fn fits(text: &str, most: usize) -> bool {
	let trimmed = text.trim();
	!trimmed.is_empty() && trimmed.chars().count() <= most
}

/// Each of `items` once.
fn distinct<T: PartialEq>(items: &[T]) -> bool {
	items.iter().enumerate().all(|(i, item)| !items[..i].contains(item))
}

/// What home reads of an offer (`offer`, C1582): `PreviewOfferSchema`'s
/// bounds, each line trimmed first.
fn offer(object: &Map<String, Value>) -> Result<Offer, Vec<Problem>> {
	let offer: Offer = field(object, "offer")?;
	let variants = &offer.variants;
	let ids: Vec<&str> = variants.iter().map(|v| v.variant.as_str()).collect();
	let rules: Vec<&str> = variants.iter().map(|v| v.rule.trim()).collect();
	let good = is_id(&offer.preview)
		&& fits(&offer.about, OFFER_ABOUT_MOST)
		&& (2..=PREVIEW_VARIANTS_MOST).contains(&variants.len())
		&& ids.iter().all(|id| is_id(id))
		&& distinct(&ids)
		&& distinct(&rules)
		&& ids.contains(&offer.recommended.as_str())
		&& variants.iter().all(|v| {
			let callouts: Vec<&str> = v.callouts.iter().map(|c| c.trim()).collect();
			fits(&v.title, OFFER_TITLE_MOST)
				&& fits(&v.rule, NAME_MOST)
				&& (1..=OFFER_CALLOUTS_MOST).contains(&callouts.len())
				&& v.callouts.iter().all(|c| fits(c, OFFER_CALLOUT_MOST))
				&& distinct(&callouts)
		});
	if !good {
		return Err(problem("offer", "is not an offer PreviewOfferSchema takes"));
	}
	Ok(Offer {
		about: offer.about.trim().to_owned(),
		variants: offer
			.variants
			.iter()
			.map(|v| OfferVariant {
				variant: v.variant.clone(),
				title: v.title.trim().to_owned(),
				callouts: v.callouts.iter().map(|c| c.trim().to_owned()).collect(),
				rule: v.rule.trim().to_owned(),
			})
			.collect(),
		preview: offer.preview,
		recommended: offer.recommended,
	})
}

/// Reads one event (`ConversationEventSchema`, what home reads of it): its
/// kind, its time, exactly its kind's fields, and what the step reads.
///
/// # Errors
/// Each field that is not what the schema takes.
pub fn parse(value: Value) -> Result<Event, Vec<Problem>> {
	let Value::Object(object) = &value else {
		return Err(problem("event", "is an object"));
	};
	let Some(kind) = object.get("event").and_then(Value::as_str).and_then(Kind::parse) else {
		return Err(problem("event", "is one of the schema's kinds"));
	};
	let Some(at) = object.get("at").and_then(Value::as_str) else {
		return Err(problem("at", "is missing"));
	};
	instant(at)?;
	let mut problems: Vec<Problem> = Vec::new();
	for name in object.keys() {
		if !["event", "at", "basis", "by"].contains(&name.as_str()) && !kind.fields().contains(&name.as_str()) && !kind.optional().contains(&name.as_str()) {
			problems.push((name.clone(), "is not a field of this kind of event"));
		}
	}
	for name in kind.fields() {
		if !object.contains_key(*name) {
			problems.push(((*name).to_owned(), "is missing"));
		}
	}
	if !problems.is_empty() {
		return Err(problems);
	}
	let basis = if object.contains_key("basis") { Some(field::<u64>(object, "basis")?) } else { None };
	let by: String = field(object, "by")?;
	if !AUTHORS.contains(&by.as_str()) {
		return Err(problem("by", "is the owner, the agent or home"));
	}
	let body = body(kind, object)?;
	let at = at.to_owned();
	Ok(Event { kind, at, basis, by, body, value })
}

#[cfg(test)]
mod tests;
