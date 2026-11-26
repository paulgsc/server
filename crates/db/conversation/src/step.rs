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
//!   stands (`basis`, else `stale`), which events each phase accepts
//!   (`CONVERSATION_ACCEPTS`), a point once and at most
//!   [`POINTS_MOST`], each version numbered next and answering every open
//!   flag once, a flag once and on a turn or speaker the newest version has,
//!   a withdrawal of an open flag only, the measure and the green light of
//!   the newest version only, under [`MOST_MS`], a notes event on a turn of
//!   the newest version, each word once and inside the turn, following only
//!   rules an approval set, and each preview approved once, naming only the
//!   variants it offered.
//! - **The content rules that rest on some-ui's closed vocabularies** (the
//!   cast and its roles, each character's emotions, the flag kinds' voices,
//!   scopes and answers, Kokoro's voices, how one version's turns differ from
//!   the last's) are not ported: they would be a second copy of those lists
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
			Self::Podcast => &[Kind::Point, Kind::Flag, Kind::Heard, Kind::GreenLight, Kind::Notes],
			Self::Flagged => &[Kind::Point, Kind::Flag, Kind::Withdraw, Kind::Version, Kind::Heard, Kind::Notes],
			Self::GreenLit => &[Kind::Notes, Kind::Approve],
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
	Approve,
}

impl Kind {
	/// Every kind, in the schema's order.
	pub const ALL: [Self; 9] = [
		Self::Topic,
		Self::Point,
		Self::Version,
		Self::Flag,
		Self::Withdraw,
		Self::Heard,
		Self::GreenLight,
		Self::Notes,
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
			Self::Approve => "approve",
		}
	}

	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|kind| kind.as_str() == name)
	}

	/// The fields an event of this kind has, beside `event` and `at`: each
	/// required, but `notes`' `rules`.
	const fn fields(self) -> &'static [&'static str] {
		match self {
			Self::Topic => &["topic"],
			Self::Point => &["take"],
			Self::Version | Self::GreenLight => &["version"],
			Self::Flag | Self::Withdraw => &["flag"],
			Self::Heard => &["version", "ms"],
			Self::Notes => &["turn", "words"],
			Self::Approve => &["preview", "offered", "recommended", "chose"],
		}
	}

	const fn optional(self) -> &'static [&'static str] {
		match self {
			Self::Notes => &["rules"],
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
	OutOfPhase { phase: Phase, event: Kind },
	TooManyPoints,
	PointAgain { take: String },
	VersionNumber { expected: u64, got: u64 },
	Unanswered { flag: String },
	AnswerUnknown { flag: String },
	AnswerTwice { flag: String },
	FlagAgain { flag: String },
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
	PreviewAgain { preview: String },
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
	body: Body,
	/// The whole event, as sent.
	pub value: Value,
}

impl Event {
	/// The approval it is, if it is one.
	#[must_use]
	pub const fn approval(&self) -> Option<&Approval> {
		match &self.body {
			Body::Approve(approval) => Some(approval),
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
	Flag { id: String, on: Scope },
	Withdraw { flag: String },
	Heard { version: u64, ms: u64 },
	GreenLight { version: u64 },
	Notes { turn: String, words: Vec<u64>, rules: Vec<String> },
	Approve(Approval),
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
	/// Each answer's flag, in order; an `unasked` change names none.
	answers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Turn {
	id: String,
	speaker: String,
	/// Its words, as the notes count them, where the record knows them
	/// (`wordsOf`).
	words: Option<usize>,
}

/// An approval (`KeptApproval`), with when it was kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approval {
	pub preview: String,
	pub offered: Vec<String>,
	pub recommended: String,
	/// `PreviewChoice`, as sent.
	pub chose: Choice,
}

/// What the owner chose on a preview (`PreviewChoiceSchema`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Choice {
	Variant { variant: String, rule: String },
	Else { said: String },
}

/// A flag as the state keeps it (`KeptFlag`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeptFlag {
	id: String,
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
	approvals: Vec<Approval>,
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
			approvals: Vec::new(),
		}
	}

	#[must_use]
	pub const fn phase(&self) -> Phase {
		self.phase
	}

	/// Each approval, in the order kept.
	#[must_use]
	pub fn approvals(&self) -> &[Approval] {
		&self.approvals
	}

	/// The rules the owner's approvals set, in the order set
	/// (`approvedRules`).
	fn rules(&self) -> impl Iterator<Item = &str> {
		self.approvals.iter().filter_map(|approval| match &approval.chose {
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
			Body::Flag { id, on } => self.flag(id, on, out_of_phase)?,
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
			Body::Approve(approval) => self.approve(approval)?,
		}
		self.seq += 1;
		Ok(())
	}

	/// A flag on the newest version: once, on a turn or speaker it has.
	fn flag(&mut self, id: &str, on: &Scope, out_of_phase: Refusal) -> Result<(), Refusal> {
		if self.flags.iter().any(|flag| flag.id == id) {
			return Err(Refusal::FlagAgain { flag: id.to_owned() });
		}
		let Some(latest) = &self.latest else {
			return Err(out_of_phase);
		};
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
		let version = latest.n;
		self.flags.push(KeptFlag {
			id: id.to_owned(),
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
			let outside = found.words.is_some_and(|count| usize::try_from(at).map_or(true, |at| at >= count));
			if seen.contains(&at) || outside {
				return Err(Refusal::NotesWord { turn: turn.to_owned(), at });
			}
			seen.push(at);
		}
		Ok(())
	}

	/// An approval: of a preview once, naming only the variants it offered.
	fn approve(&mut self, approval: &Approval) -> Result<(), Refusal> {
		if self.approvals.iter().any(|kept| kept.preview == approval.preview) {
			return Err(Refusal::PreviewAgain {
				preview: approval.preview.clone(),
			});
		}
		let chosen = match &approval.chose {
			Choice::Variant { variant, .. } => Some(variant),
			Choice::Else { .. } => None,
		};
		if let Some(named) = std::iter::once(&approval.recommended).chain(chosen).find(|named| !approval.offered.contains(named)) {
			return Err(Refusal::PreviewVariant {
				preview: approval.preview.clone(),
				variant: named.clone(),
			});
		}
		self.approvals.push(approval.clone());
		Ok(())
	}

	/// `answersRefusal`, the cases home holds: every answer names an open
	/// flag of the version before, each once, and every open flag is
	/// answered.
	fn answers_hold(&self, version: &Version) -> Result<(), Refusal> {
		let open: Vec<&str> = self.open_flags().map(|flag| flag.id.as_str()).collect();
		let mut answered: Vec<&str> = Vec::new();
		for flag in &version.answers {
			if !open.contains(&flag.as_str()) {
				return Err(Refusal::AnswerUnknown { flag: flag.clone() });
			}
			if answered.contains(&flag.as_str()) {
				return Err(Refusal::AnswerTwice { flag: flag.clone() });
			}
			answered.push(flag);
		}
		open
			.into_iter()
			.find(|flag| !answered.contains(flag))
			.map_or(Ok(()), |flag| Err(Refusal::Unanswered { flag: flag.to_owned() }))
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
}

#[derive(Deserialize)]
struct FlagWire {
	id: String,
	on: ScopeWire,
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
		match (answer.how.as_str(), answer.flag) {
			("unasked", None) => {}
			("unasked", Some(_)) => return Err(problem("version.answers", "an unasked change names no flag")),
			(_, Some(flag)) => answers.push(flag),
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
			let on = match flag.on {
				ScopeWire::Turn { turn } => Scope::Turn(turn),
				ScopeWire::Speaker { speaker } => Scope::Speaker(speaker),
				ScopeWire::Conversation {} => Scope::Conversation,
			};
			Body::Flag { id: flag.id, on }
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
			let rules: Vec<String> = if object.contains_key("rules") { field(object, "rules")? } else { Vec::new() };
			Body::Notes {
				turn: field(object, "turn")?,
				words: words.into_iter().map(|word| word.at).collect(),
				rules,
			}
		}
		Kind::Approve => {
			let preview: String = field(object, "preview")?;
			let offered: Vec<String> = field(object, "offered")?;
			let recommended: String = field(object, "recommended")?;
			let chose: Choice = field(object, "chose")?;
			if !(2..=PREVIEW_VARIANTS_MOST).contains(&offered.len()) {
				return Err(problem("offered", "is two or three variants"));
			}
			if offered.iter().enumerate().any(|(i, variant)| offered[..i].contains(variant)) {
				return Err(problem("offered", "a variant is offered twice"));
			}
			let names = std::iter::once(&preview).chain(&offered).chain(std::iter::once(&recommended));
			let chosen: Vec<&String> = match &chose {
				Choice::Variant { variant, rule } => vec![variant, rule],
				Choice::Else { said } => {
					if said.trim().is_empty() {
						return Err(problem("chose.said", "is the owner's words"));
					}
					Vec::new()
				}
			};
			if !names.chain(chosen).all(|name| is_name(name)) {
				return Err(problem("approve", "names each preview, variant and rule in 1 to 64 characters"));
			}
			Body::Approve(Approval {
				preview,
				offered,
				recommended,
				chose,
			})
		}
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
		if !["event", "at", "basis"].contains(&name.as_str()) && !kind.fields().contains(&name.as_str()) && !kind.optional().contains(&name.as_str()) {
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
	let body = body(kind, object)?;
	let at = at.to_owned();
	Ok(Event { kind, at, basis, body, value })
}

#[cfg(test)]
mod tests;
