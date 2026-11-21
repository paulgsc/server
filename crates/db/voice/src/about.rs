//! What a take is about, by its kind (paulgsc/some-ui#3219, TAKES20).
//!
//! The take library ([`crate::takes`]) lists every kind of voice note in one
//! shape, and this is the one per-kind part of it: what the take was said
//! for, read out of its `ref` by [`about`], one arm per [`Of`] and no
//! wildcard, so a new kind does not compile until it says what its takes are
//! about. Each [`About`] carries a `key`, `<kind or family>:<what>`
//! (`game:401772`, `interview:2026-w47`, `studio:ep04`), which the library
//! filters on.
//!
//! [`about`] is pure: the key and every field it fills come from the kind and
//! the `ref` alone (and, for a Studio take, the idea answer that names it), so
//! the library can filter its whole list by key before it reads anything
//! more. What only another table knows (a game bite's scenario and the play
//! it was matched to, an interview answer's rung) is read for the page the
//! library answers ([`with_game`], [`with_rung`]).

use crate::{bite_match::Method, is_interview_ref, is_opening_question, is_point, How, Mode, Of, VoiceError, OPENING};
use rust_round_repo::{
	interview::{Rung, Sent},
	week::{is_ticket_key, is_week_id},
};
use serde::Serialize;
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// What a take was said for: its key, and what its kind says of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct About {
	/// `<kind or family>:<what>`, the value the library's `about` filter
	/// matches.
	pub key: String,
	/// What the take's kind says of it.
	#[serde(flatten)]
	pub detail: Detail,
}

/// What a take's kind says of it, one variant per [`Of`] (a Studio take's
/// three forms apart).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Detail {
	/// A study session's reflection: the session.
	Reflection { session: String },
	/// A Jobs pick or scout: its id.
	Jobs { pick: String },
	/// A drama prompt's next scene: the prompt's stamp.
	Scene { prompt: String },
	/// A game bite: the game, what the screen showed, and the play home
	/// matched it to.
	Game(Box<GameAbout>),
	/// A Studio take of a scene.
	StudioScene { form: StudioForm, project: String, shot: String, take: String },
	/// An idea's spoken answer, a Studio note too.
	StudioIdea { form: StudioForm, idea: String, question: String },
	/// A Studio take with no scene, its `ref` as the phone named it.
	StudioLoose {
		form: StudioForm,
		#[serde(rename = "ref")]
		of_ref: String,
	},
	/// A soundbite: how it came to be recorded.
	Soundbite { how: String },
	/// A grammar-lens line read aloud: the line.
	Lens { line: String },
	/// A `LeetType` answer: the session file and its screen.
	Leetype { file: String, screen: Option<String> },
	/// A note said on a Rust Rounds card, a week's ticket or a pasted error.
	#[serde(rename_all = "camelCase")]
	RustRound {
		week: Option<String>,
		ticket: Option<String>,
		/// The speak-to point a ticket's take answers, from 1 (RR24); none
		/// for a take of the whole ticket, a card or a paste.
		point: Option<u8>,
		round: Option<String>,
		card: Option<String>,
		pasted_at: Option<String>,
	},
	/// A take of a week of mock interview: a question's answer or a card read.
	Interview {
		week: String,
		question: Option<String>,
		card: Option<String>,
		/// How the answer was given, when the week's send names this take.
		rung: Option<Rung>,
	},
}

/// Which of a Studio note's three forms it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StudioForm {
	/// A scene's take, `<project>:<shot>:<take>`.
	Scene,
	/// An idea's spoken answer.
	Idea,
	/// A take with no scene.
	Loose,
}

/// A game bite's take: the game, the bite as the screen showed it, and the
/// play home matched it to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameAbout {
	/// The game's id (ESPN's event id), the note's `ref`.
	pub game: String,
	/// The game as ESPN's scoreboard names it (`NE @ CHI`), from the newest
	/// summary home fetched of it; null while home has fetched none that
	/// names both teams.
	pub label: Option<String>,
	/// The bite as the phone described it; null for a game note with no
	/// bite row.
	pub bite: Option<BiteAbout>,
	/// The reconciler's newest match; null until it has matched the bite.
	#[serde(rename = "match")]
	pub matched: Option<MatchAbout>,
}

/// What a bite was said about and what the screen showed then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiteAbout {
	/// The scenario: the bite's own, or the one home mapped to a bite said
	/// first; null for a bite said first and never mapped.
	pub scenario: Option<String>,
	/// How the scenario came to be said.
	pub how: How,
	/// The live strip as the screen showed it: the moment and the clock.
	pub strip: String,
	/// The line of context the screen showed, if any.
	pub context: Option<String>,
	/// Whether the owner said they were watching.
	pub watching: bool,
	/// When the owner started saying it, on the phone's clock.
	pub started: String,
	/// How much the screen offered.
	pub mode: Mode,
}

/// The play a bite was matched to, or why none was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchAbout {
	/// `nearest` when a play was matched; else why not: `unmatched` (no play
	/// the scenario's rule accepts), `not-a-play` (the scenario is about a
	/// stretch of the game) or `no-scenario` (said first and never mapped).
	pub method: Method,
	/// The scenario it was matched under.
	pub scenario: Option<String>,
	/// The bite's start minus the play's time, in seconds; null without a play.
	pub gap_seconds: Option<i64>,
	/// The play, as home last saw it; null without one.
	pub play: Option<Play>,
}

/// A play of the game's feed, its latest version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Play {
	/// ESPN's play id.
	pub id: String,
	pub period: Option<i64>,
	pub clock: Option<String>,
	#[serde(rename = "type")]
	pub play_type: Option<String>,
	pub text: Option<String>,
}

/// The second part of a pasted error's Rust Rounds `ref`.
const PASTE: &str = "PASTE";
/// The second part of a teleprompter read's interview `ref`.
const READ: &str = "read";
/// What an idea's spoken answer's `ref` starts with.
const IDEA: &str = "idea:";

/// `<family>:<what>`.
fn key(family: &str, what: &str) -> String {
	String::from(family) + ":" + what
}

/// What a take said for `of` under `of_ref` is about; `idea` is the idea and
/// question an `idea_answer` names it for, if one does.
///
/// One arm per [`Of`], and no wildcard: a kind added to [`Of`] does not
/// compile until it has one, so the library can never list a kind it cannot
/// say anything of.
#[must_use]
pub fn about(of: Of, of_ref: &str, idea: Option<(String, String)>) -> About {
	let plain = |family: &str, detail: Detail| About { key: key(family, of_ref), detail };
	match of {
		Of::Reflection => plain(of.as_str(), Detail::Reflection { session: of_ref.to_owned() }),
		Of::Jobs => plain(of.as_str(), Detail::Jobs { pick: of_ref.to_owned() }),
		Of::Scene => plain(of.as_str(), Detail::Scene { prompt: of_ref.to_owned() }),
		Of::Soundbite => plain(of.as_str(), Detail::Soundbite { how: of_ref.to_owned() }),
		Of::Lens => plain(of.as_str(), Detail::Lens { line: of_ref.to_owned() }),
		Of::Game => About {
			key: key(of.as_str(), of_ref),
			detail: Detail::Game(Box::new(GameAbout {
				game: of_ref.to_owned(),
				label: None,
				bite: None,
				matched: None,
			})),
		},
		Of::Leetype => {
			let (file, screen) = of_ref.rsplit_once(':').map_or((of_ref, None), |(file, screen)| (file, Some(screen.to_owned())));
			About {
				key: key(of.as_str(), file),
				detail: Detail::Leetype { file: file.to_owned(), screen },
			}
		}
		Of::Studio => studio(of_ref, idea),
		Of::RustRound => rust_round(of_ref),
		Of::Interview => interview(of_ref),
	}
}

/// What a spoken card's part's take is named for: `spoken:<card>.<part>`
/// (paulgsc/some-ui `studio-spoken/model.ts`'s `lineOf`).
const SPOKEN: &str = "spoken:";

/// Whether a Studio take is of a script's scene or of a spoken card's part.
///
/// A scene's take is `<project>:<shot>:<take>`, and a spoken card's part's
/// `spoken:<card>.<part>`, which [`studio`] lists as a take with no scene.
/// Read from its `ref` alone: never one an `idea:` ref names. An idea's answer only the `idea_answer` table names
/// (never sent with an `idea:` ref) and whose `ref` has three parts reads as
/// a scene's here; the language is the transcriber's, which sees the `ref`
/// alone.
#[must_use]
pub fn studio_script_or_spoken(of_ref: &str) -> bool {
	if of_ref.starts_with(IDEA) {
		return false;
	}
	of_ref.starts_with(SPOKEN) || of_ref.split(':').count() == 3
}

/// A Studio note: an idea's answer, a scene's take, or a take with no scene.
fn studio(of_ref: &str, idea: Option<(String, String)>) -> About {
	let named = of_ref
		.strip_prefix(IDEA)
		.and_then(|rest| rest.split_once(':'))
		.map(|(idea, question)| (idea.to_owned(), question.to_owned()));
	if let Some((idea, question)) = named.or(idea) {
		return About {
			key: key("idea", &idea),
			detail: Detail::StudioIdea {
				form: StudioForm::Idea,
				idea,
				question,
			},
		};
	}
	let parts: Vec<&str> = of_ref.split(':').collect();
	if let [project, shot, take] = parts.as_slice() {
		return About {
			key: key(Of::Studio.as_str(), project),
			detail: Detail::StudioScene {
				form: StudioForm::Scene,
				project: (*project).to_owned(),
				shot: (*shot).to_owned(),
				take: (*take).to_owned(),
			},
		};
	}
	About {
		key: key(Of::Studio.as_str(), of_ref),
		detail: Detail::StudioLoose {
			form: StudioForm::Loose,
			of_ref: of_ref.to_owned(),
		},
	}
}

/// A Rust Rounds note: a week's ticket (`<week>:<ticket>`) or one of its
/// points (`<week>:<ticket>:<n>`), a pasted error (`<week>:PASTE:<at>`), an
/// answer to the week's opening question n (`<week>:opening:<n>`,
/// paulgsc/some-ui#3269: the week's, with no ticket), or a card (`<round>:<card>[:<note>]`), as [`crate::is_rust_round_ref`] reads
/// them. Keyed by the week, or by the round for a card.
fn rust_round(of_ref: &str) -> About {
	let mut parts = of_ref.splitn(3, ':');
	let first = parts.next().unwrap_or_default();
	let second = parts.next();
	let rest = parts.next();
	let owned = |part: &str| Some(part.to_owned());
	let (week, ticket, point, round, card, pasted_at) = match (is_week_id(first), second, rest) {
		(true, Some(PASTE), Some(at)) => (owned(first), None, None, None, None, owned(at)),
		(true, Some(OPENING), Some(n)) if is_opening_question(n) => (owned(first), None, None, None, None, None),
		(true, Some(ticket), None) if is_ticket_key(ticket) => (owned(first), owned(ticket), None, None, None, None),
		(true, Some(ticket), Some(n)) if is_ticket_key(ticket) && is_point(n) => (owned(first), owned(ticket), n.parse().ok(), None, None, None),
		(_, Some(card), _) => (None, None, None, owned(first), owned(card), None),
		(_, None, _) => (None, None, None, None, None, None),
	};
	About {
		key: key(Of::RustRound.as_str(), first),
		detail: Detail::RustRound {
			week,
			ticket,
			point,
			round,
			card,
			pasted_at,
		},
	}
}

/// An interview take: `<week>:<question>` or `<week>:read:<card>`
/// ([`is_interview_ref`]), keyed by the week. A `ref` that reads as neither
/// (home refuses one) keeps its first part as the week.
fn interview(of_ref: &str) -> About {
	let mut parts = of_ref.splitn(3, ':');
	let week = parts.next().unwrap_or_default().to_owned();
	let second = parts.next();
	let third = parts.next();
	let (question, card) = match (is_interview_ref(of_ref), second, third) {
		(true, Some(READ), Some(card)) => (None, Some(card.to_owned())),
		(true, Some(question), None) => (Some(question.to_owned()), None),
		_ => (None, None),
	};
	About {
		key: key(Of::Interview.as_str(), &week),
		detail: Detail::Interview { week, question, card, rung: None },
	}
}

/// `about`, a game take's, with the game's name, its bite and its newest
/// match filled in. `labels` holds each game's name once read, so a page of
/// one game's bites reads its summary once.
///
/// # Errors
/// [`VoiceError::Unreadable`] for a bite or match this crate did not write,
/// and [`VoiceError::Storage`] for any `sqlx` failure.
pub async fn with_game(conn: &mut SqliteConnection, subject: &str, note_id: &str, about: &mut About, labels: &mut BTreeMap<String, Option<String>>) -> Result<(), VoiceError> {
	let Detail::Game(game) = &mut about.detail else {
		return Ok(());
	};
	game.label = if let Some(label) = labels.get(&game.game) {
		label.clone()
	} else {
		let label = label(conn, &game.game).await?;
		labels.insert(game.game.clone(), label.clone());
		label
	};
	game.bite = bite(conn, subject, note_id).await?;
	let Some(matched) = crate::bite_match::newest_match(conn, subject, note_id).await? else {
		return Ok(());
	};
	let play = match matched.play.as_deref() {
		Some(play) => latest_play(conn, &game.game, play).await?,
		None => None,
	};
	game.matched = Some(MatchAbout {
		method: matched.method,
		scenario: matched.scenario,
		gap_seconds: matched.gap_seconds,
		play,
	});
	Ok(())
}

/// `game`'s name from the newest summary home fetched of it, an indexed
/// read by `game_feed_response_by_game`; none when it has fetched none, or
/// the newest names no two teams. The body is ESPN's, so one that does not
/// parse is no name, never an error.
async fn label(conn: &mut SqliteConnection, game: &str) -> Result<Option<String>, VoiceError> {
	let body = sqlx::query_scalar!(
		r#"SELECT body.body AS "body!: Vec<u8>" FROM game_feed_response AS response
		   JOIN game_feed_body AS body ON body.sha256 = response.sha256
		   WHERE response.game_id = ?1 AND response.kind = 'summary' AND response.status = 200
		   ORDER BY response.response_id DESC LIMIT 1"#,
		game
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(body.and_then(|body| nfl_feed::summary::matchup(&body).ok().flatten()).map(|matchup| matchup.label()))
}

/// The bite of `subject`'s game note `note_id`, its scenario its own or else
/// its newest mapping's.
async fn bite(conn: &mut SqliteConnection, subject: &str, note_id: &str) -> Result<Option<BiteAbout>, VoiceError> {
	let row = sqlx::query!(
		r#"SELECT b.scenario_id, b.how, b.live_strip, b.context_line, b.watching, b.started_at, b.mode,
		          (SELECT m.scenario_id FROM bite_mapping m WHERE m.subject_id = b.subject_id AND m.note_id = b.note_id
		           ORDER BY m.mapping_id DESC LIMIT 1) AS "mapped?: String"
		   FROM game_bite b WHERE b.subject_id = ?1 AND b.note_id = ?2"#,
		subject,
		note_id
	)
	.fetch_optional(&mut *conn)
	.await?;
	row
		.map(|row| {
			Ok(BiteAbout {
				scenario: row.scenario_id.or(row.mapped),
				how: How::named(&row.how).ok_or(VoiceError::Unreadable)?,
				strip: row.live_strip,
				context: row.context_line,
				watching: row.watching != 0,
				started: row.started_at,
				mode: Mode::named(&row.mode).ok_or(VoiceError::Unreadable)?,
			})
		})
		.transpose()
}

/// `play` of `game` as the feed last revised it: the version nothing
/// supersedes.
async fn latest_play(conn: &mut SqliteConnection, game: &str, play: &str) -> Result<Option<Play>, VoiceError> {
	let row = sqlx::query!(
		r#"SELECT p.period, p.clock, p.play_type, p.text FROM game_feed_play AS p
		   WHERE p.game_id = ?1 AND p.espn_play_id = ?2
		     AND NOT EXISTS (SELECT 1 FROM game_feed_play AS later WHERE later.supersedes = p.row_id)
		   ORDER BY p.row_id DESC LIMIT 1"#,
		game,
		play
	)
	.fetch_optional(&mut *conn)
	.await?;
	Ok(row.map(|row| Play {
		id: play.to_owned(),
		period: row.period,
		clock: row.clock,
		play_type: row.play_type,
		text: row.text,
	}))
}

/// Each week's standing send, as [`with_rung`] has read it: `None` for a
/// week with none, or whose send no longer reads as one.
pub type Sends = BTreeMap<String, Option<Sent>>;

/// `about`, an interview take's, with the rung its week's send gave it.
///
/// The send is the one that stands, the week's final or else its draft
/// (`rust_round_repo::draft::standing`), when it names the take. `sends`
/// holds each week's send once read, so a page of one week's takes reads it
/// once. A stored send that no longer reads as one is no rung, never an
/// error: the rung is one detail of one take, and the page answers without
/// it.
///
/// # Errors
/// [`VoiceError::Storage`] for any other `sqlx` failure.
pub async fn with_rung(conn: &mut SqliteConnection, subject: &str, note_id: &str, about: &mut About, sends: &mut Sends) -> Result<(), VoiceError> {
	let Detail::Interview {
		week, question: Some(_), rung, ..
	} = &mut about.detail
	else {
		return Ok(());
	};
	if !sends.contains_key(week.as_str()) {
		let sent = match rust_round_repo::draft::standing::<Sent>(conn, subject, week).await {
			Ok(standing) => standing.map(|standing| standing.sent),
			Err(sqlx::Error::Decode(_)) => None,
			Err(err) => return Err(VoiceError::Storage(err)),
		};
		sends.insert(week.clone(), sent);
	}
	*rung = sends
		.get(week.as_str())
		.and_then(Option::as_ref)
		.and_then(|sent| sent.answers.iter().find(|answer| answer.take == note_id))
		.and_then(|answer| answer.rung);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{about, Detail, StudioForm};
	use crate::Of;

	/// Every kind home accepts says what its takes are about, and keys them
	/// under its own name or its family's. With [`about`]'s match, a kind
	/// added to [`Of`] fails here as well as at compile time.
	#[test]
	fn every_kind_says_what_its_takes_are_about() {
		for of in Of::ALL {
			let said = about(of, "r", None);
			let family = said.key.split_once(':').unwrap().0;
			assert!(family == of.as_str() || (of == Of::Studio && family == "idea"), "{} keyed {}", of.as_str(), said.key);
		}
	}

	#[test]
	fn each_kind_reads_its_ref() {
		let key = |of, r: &str| about(of, r, None).key;
		assert_eq!(key(Of::Game, "401772"), "game:401772");
		assert_eq!(key(Of::Studio, "ep04:s3:2"), "studio:ep04");
		assert_eq!(key(Of::Studio, "idea:i1:why"), "idea:i1");
		assert_eq!(key(Of::Studio, "library:t1"), "studio:library:t1");
		assert_eq!(key(Of::RustRound, "2026-w47:ENG-12"), "rust-round:2026-w47");
		assert_eq!(key(Of::RustRound, "indexing-01:a"), "rust-round:indexing-01");
		assert_eq!(key(Of::Interview, "2026-w47:read:home-first"), "interview:2026-w47");
		assert_eq!(key(Of::Leetype, "f1:s2"), "leetype:f1");

		assert_eq!(
			about(Of::Studio, "loose", Some(("i9".into(), "what".into()))).detail,
			Detail::StudioIdea {
				form: StudioForm::Idea,
				idea: "i9".into(),
				question: "what".into()
			},
			"an idea answer names it"
		);
		assert_eq!(
			about(Of::RustRound, "2026-w47:PASTE:2026-11-17T10:00:00.000Z", None).detail,
			Detail::RustRound {
				week: Some("2026-w47".into()),
				ticket: None,
				point: None,
				round: None,
				card: None,
				pasted_at: Some("2026-11-17T10:00:00.000Z".into())
			}
		);
		assert_eq!(
			about(Of::RustRound, "2026-w47:opening:2", None).detail,
			Detail::RustRound {
				week: Some("2026-w47".into()),
				ticket: None,
				point: None,
				round: None,
				card: None,
				pasted_at: None
			},
			"an opening answer is the week's, never a card's (paulgsc/some-ui#3269)"
		);
		// RR24: a take of one point names its ticket and point, read from the
		// `ref`; a take of the whole ticket names no point.
		for (of_ref, point) in [("2026-w47:QM-312:2", Some(2)), ("2026-w47:QM-312", None)] {
			assert_eq!(
				about(Of::RustRound, of_ref, None).detail,
				Detail::RustRound {
					week: Some("2026-w47".into()),
					ticket: Some("QM-312".into()),
					point,
					round: None,
					card: None,
					pasted_at: None
				},
				"{of_ref}"
			);
		}
		assert_eq!(
			about(Of::Interview, "2026-w47:about-you", None).detail,
			Detail::Interview {
				week: "2026-w47".into(),
				question: Some("about-you".into()),
				card: None,
				rung: None
			}
		);
	}
}
