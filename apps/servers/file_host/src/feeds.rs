//! The feeds home renders and the owner watches on the phone.
//!
//! FEED4, paulgsc/some-ui#2647: the weekly call (`call_weeks`) and Playoff
//! Delta (`pd_fills`), Rust Rounds' and mock interview's essays (`rust_essays`, RR16,
//! paulgsc/some-ui#2691), and cards essays (`cards_essays`, CARDS7 #854;
//! EXPLAIN6 #878), and the weekly engagement report (`report_weeks`, ENG12,
//! paulgsc/some-ui#2700, #821). Each keeps its episodes, links and receipts in its own
//! tables, so each record is keyed to what it is of; what the phone does with
//! an episode's video (list it, bring it, record its `YouTube` link, let it
//! go) is one implementation over them all, [`videos`].
//!
//! **A cards essay is listed, brought and linked, never let go**
//! ([`Feed::lets_go`]). The owner's ask (2026-11-14) was a place among the
//! episodes; its `YouTube` link (paulgsc/some-ui#2999, call C388) is kept in
//! `cards_essay_link`, beside `cards_essay`, which is append-only and holds
//! the essay alone. Letting home's copy go once the link is recorded is a
//! later story; its receipts will be `cards_essay_video_deletion`'s, which
//! already holds an essay withdrawn because a take its spoken cards play
//! was deleted (paulgsc/server#1055, `cards_essays::pass::tidy`): such an
//! essay is gone, as a withdrawn Rust Rounds week is, so no list shows it.
//!
//! An episode is named by its key, one plain path segment by its table's own
//! checks: the call's week by its first day (`2026-10-30`), or, for a week
//! that is a series of parts (paulgsc/some-ui#2918), each part after the
//! first by `<day>-p<k>` (`2026-11-06-p2`; `call_week_repo::video::split_key`),
//! each its own episode with its own video, link and receipts; Playoff Delta's
//! by `<season>-<team>-<NN>` (`2026-SF-08`), an essay by its week
//! (`2026-w46`), a cards essay by its slug and day
//! (`nixos-upgrade-2026-11-14`), an engagement report by its ISO week
//! (`2026-W46`).
//!
//! **Whose.** The call's, Playoff Delta's and the cards essays' episodes are
//! no one's: what was said by home's speaker about a public week, or about
//! public repositories at stamped commits. An essay's is its subject's,
//! since its video plays the owner's own takes (docs/identity.md invariant
//! 23's one named exception), and so is an engagement report's, written from
//! the owner's own records (`report_week`), so every method that reads or
//! writes the feed's tables takes the caller's subject, and those feeds
//! answer only that subject's weeks.

pub mod mp4;
pub mod renders;
pub mod videos;

use call_week_repo::video::{Link, Receipt, Recorded, Step};
use sqlx::SqliteConnection;
use std::collections::BTreeMap;
use std::path::Path;

/// Held by an export pass for a whole pass (`call_weeks::pass`,
/// `pd_fills::render::export`) and by an episode's let go for a whole step
/// ([`videos::let_go`]), so neither writes or removes an episode's files in
/// the middle of the other.
pub(crate) static FOLDERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A feed home renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
	/// The weekly call (ST36, ST38).
	Call,
	/// Playoff Delta (FEED4).
	PlayoffDelta,
	/// Rust Rounds' essays (RR16): the caller's own.
	RustEssay,
	/// A week of mock interview's essays (IW6, server#899): the caller's own,
	/// by Rust Rounds essays' rules over its own tables.
	InterviewEssay,
	/// Cards essays (CARDS7, EXPLAIN6): listed, brought and linked
	/// (paulgsc/some-ui#2999), never let go ([`Self::lets_go`]).
	CardsEssay,
	/// The weekly engagement report (ENG12, #821): the caller's own, by the
	/// essays' rules over its own tables.
	Report,
}

impl Feed {
	/// The feed as the routes name it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Call => "call",
			Self::PlayoffDelta => "playoff-delta",
			Self::RustEssay => "rust-essay",
			Self::InterviewEssay => "interview-essay",
			Self::CardsEssay => "cards-essay",
			Self::Report => "report",
		}
	}

	/// The feed a name names, if any.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		match name {
			"call" => Some(Self::Call),
			"playoff-delta" => Some(Self::PlayoffDelta),
			"rust-essay" => Some(Self::RustEssay),
			"interview-essay" => Some(Self::InterviewEssay),
			"cards-essay" => Some(Self::CardsEssay),
			"report" => Some(Self::Report),
			_ => None,
		}
	}

	/// Where the render worker keeps each episode's folder.
	#[must_use]
	pub fn renders(self) -> &'static Path {
		Path::new(match self {
			Self::Call => crate::call_weeks::render::RENDERS_FOLDER,
			Self::PlayoffDelta => crate::pd_fills::render::RENDERS_FOLDER,
			Self::RustEssay => crate::rust_essays::RENDERS_FOLDER,
			Self::InterviewEssay => crate::rust_essays::INTERVIEW_RENDERS_FOLDER,
			Self::CardsEssay => crate::cards_essays::RENDERS_FOLDER,
			Self::Report => crate::report_weeks::RENDERS_FOLDER,
		})
	}

	/// Where `file_host` writes each episode for the render worker.
	#[must_use]
	pub fn input(self) -> &'static Path {
		Path::new(match self {
			Self::Call => crate::call_weeks::RENDER_FOLDER,
			Self::PlayoffDelta => crate::pd_fills::render::EPISODES_FOLDER,
			Self::RustEssay => crate::rust_essays::ESSAYS_FOLDER,
			Self::InterviewEssay => crate::rust_essays::INTERVIEW_ESSAYS_FOLDER,
			Self::CardsEssay => crate::cards_essays::ESSAYS_FOLDER,
			Self::Report => crate::report_weeks::REPORTS_FOLDER,
		})
	}

	/// The video's file in an episode's folder.
	#[must_use]
	pub const fn video(self) -> &'static str {
		match self {
			Self::Call => crate::call_weeks::render::VIDEO,
			Self::PlayoffDelta => crate::pd_fills::render::VIDEO,
			Self::RustEssay | Self::InterviewEssay => crate::rust_essays::VIDEO,
			Self::CardsEssay => crate::cards_essays::VIDEO,
			Self::Report => crate::report_weeks::VIDEO,
		}
	}

	/// Whether an episode's files are let go from the phone: every feed's but
	/// a cards essay's, which has no receipt table yet (the module doc). Every
	/// feed's link is kept. The routes answer `404` for one that doesn't.
	#[must_use]
	pub const fn lets_go(self) -> bool {
		!matches!(self, Self::CardsEssay)
	}

	/// What an episode's entry in [`Self::input`] is: a week's, a cards
	/// essay's or a report's file, or a fill's or an essay's folder.
	#[must_use]
	pub fn input_of(self, input: &Path, episode: &str) -> std::path::PathBuf {
		match self {
			Self::Call | Self::CardsEssay | Self::Report => input.join(String::from(episode) + ".json"),
			Self::PlayoffDelta | Self::RustEssay | Self::InterviewEssay => input.join(episode),
		}
	}

	/// Every episode home holds, newest first, with its day: for the call,
	/// each week, whose parts ([`videos::list`]) are read from the drive.
	///
	/// # Errors
	/// A storage failure.
	pub async fn held(self, conn: &mut SqliteConnection, subject: &str) -> Result<Vec<HeldEpisode>, sqlx::Error> {
		Ok(match self {
			// Its first day.
			Self::Call => call_week_repo::held(conn)
				.await?
				.into_iter()
				.map(|week| HeldEpisode {
					date: Some(week.from.clone()),
					key: week.from,
				})
				.collect(),
			// The day home took its fill, by home's clock: its key names the
			// game's week by number, which home holds no date for.
			Self::PlayoffDelta => pd_fill_repo::held(conn)
				.await?
				.iter()
				.rev()
				.map(|fill| HeldEpisode {
					key: fill.key(),
					date: day_taken(&fill.taken_at),
				})
				.collect(),
			Self::RustEssay => essays_held(conn, crate::rust_essays::Kind::RustRounds, subject)
				.await?
				.into_iter()
				.rev()
				.map(HeldEpisode::of_iso_week)
				.collect(),
			Self::InterviewEssay => essays_held(conn, crate::rust_essays::Kind::Interview, subject)
				.await?
				.into_iter()
				.rev()
				.map(HeldEpisode::of_iso_week)
				.collect(),
			// Newest week first, each by its Monday.
			Self::Report => app_event_repo::report::held(conn, subject)
				.await?
				.into_iter()
				.rev()
				.map(|week| HeldEpisode {
					date: app_event_repo::IsoWeek::parse(&week).map(|week| week.monday().format("%Y-%m-%d").to_string()),
					key: week,
				})
				.collect(),
			// Newest day first, as the table orders them oldest first.
			// Less those withdrawn for a deleted take: gone for good.
			Self::CardsEssay => {
				let gone = cards_essay_repo::gone(conn).await?;
				cards_essay_repo::held(conn)
					.await?
					.into_iter()
					.rev()
					.filter(|essay| !gone.contains(&essay.key))
					.map(|essay| HeldEpisode {
						key: essay.key,
						date: Some(essay.day),
					})
					.collect()
			}
		})
	}

	/// Whether home holds the episode `key` names.
	///
	/// # Errors
	/// A storage failure.
	pub async fn holds(self, conn: &mut SqliteConnection, subject: &str, key: &str) -> Result<bool, sqlx::Error> {
		Ok(match self {
			Self::Call => match call_week_repo::video::split_key(key) {
				Some((week, _)) => call_week_repo::body(conn, week).await?.is_some(),
				None => false,
			},
			Self::PlayoffDelta => match pd_fill_repo::parse_key(key) {
				Some((season, team, week)) => pd_fill_repo::bodies(conn, &season, &team, week).await?.is_some(),
				None => false,
			},
			Self::RustEssay => essays_held(conn, crate::rust_essays::Kind::RustRounds, subject).await?.iter().any(|week| week == key),
			Self::InterviewEssay => essays_held(conn, crate::rust_essays::Kind::Interview, subject).await?.iter().any(|week| week == key),
			Self::CardsEssay => {
				crate::cards_essays::pass::is_key(key) && cards_essay_repo::holds(conn, key).await? && !cards_essay_repo::gone(conn).await?.iter().any(|gone| gone == key)
			}
			Self::Report => app_event_repo::report::holds(conn, subject, key).await?,
		})
	}

	/// Every episode's link: the newest recorded of each, by key.
	///
	/// # Errors
	/// A storage failure.
	pub async fn links(self, conn: &mut SqliteConnection, subject: &str) -> Result<BTreeMap<String, Link>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::links(conn).await,
			Self::PlayoffDelta => pd_fill_repo::video::links(conn).await,
			Self::RustEssay => rust_round_repo::essay_video::links(conn, crate::rust_essays::Kind::RustRounds, subject).await,
			Self::InterviewEssay => rust_round_repo::essay_video::links(conn, crate::rust_essays::Kind::Interview, subject).await,
			Self::Report => app_event_repo::report_video::links(conn, subject).await,
			// Oldest first within each essay, so the newest is the one left.
			Self::CardsEssay => Ok(
				cards_essay_repo::links(conn)
					.await?
					.into_iter()
					.map(|link| {
						(
							link.key,
							Link {
								youtube_id: link.youtube_id,
								recorded_at: link.recorded_at,
							},
						)
					})
					.collect(),
			),
		}
	}

	/// Records `youtube_id` as the episode's link, once.
	///
	/// # Errors
	/// A storage failure.
	pub async fn record_link(self, conn: &mut SqliteConnection, subject: &str, key: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::record_link(conn, key, youtube_id, now).await,
			Self::PlayoffDelta => pd_fill_repo::video::record_link(conn, key, youtube_id, now).await,
			Self::RustEssay => {
				let held = self.holds(conn, subject, key).await?;
				rust_round_repo::essay_video::record_link(conn, crate::rust_essays::Kind::RustRounds, subject, key, held, youtube_id, now).await
			}
			Self::InterviewEssay => {
				let held = self.holds(conn, subject, key).await?;
				rust_round_repo::essay_video::record_link(conn, crate::rust_essays::Kind::Interview, subject, key, held, youtube_id, now).await
			}
			Self::Report => app_event_repo::report_video::record_link(conn, subject, key, youtube_id, now).await,
			Self::CardsEssay => Ok(match cards_essay_repo::record_link(conn, key, youtube_id, now).await? {
				cards_essay_repo::Linked::Added => Recorded::Added,
				cards_essay_repo::Linked::Held => Recorded::Held,
				cards_essay_repo::Linked::NoEssay => Recorded::NoWeek,
			}),
		}
	}

	/// Every receipt, by episode and step.
	///
	/// # Errors
	/// A storage failure.
	pub async fn receipts(self, conn: &mut SqliteConnection, subject: &str) -> Result<Vec<Receipt>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::receipts(conn).await,
			Self::PlayoffDelta => pd_fill_repo::video::receipts(conn).await,
			Self::RustEssay => rust_round_repo::essay_video::receipts(conn, crate::rust_essays::Kind::RustRounds, subject).await,
			Self::InterviewEssay => rust_round_repo::essay_video::receipts(conn, crate::rust_essays::Kind::Interview, subject).await,
			Self::CardsEssay => Ok(Vec::new()),
			Self::Report => app_event_repo::report_video::receipts(conn, subject).await,
		}
	}

	/// The receipt of an episode's step, if it was let go.
	///
	/// # Errors
	/// A storage failure.
	pub async fn receipt(self, conn: &mut SqliteConnection, subject: &str, key: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::receipt(conn, key, step).await,
			Self::PlayoffDelta => pd_fill_repo::video::receipt(conn, key, step).await,
			Self::RustEssay => rust_round_repo::essay_video::receipt(conn, crate::rust_essays::Kind::RustRounds, subject, key, step).await,
			Self::InterviewEssay => rust_round_repo::essay_video::receipt(conn, crate::rust_essays::Kind::Interview, subject, key, step).await,
			Self::CardsEssay => Ok(None),
			Self::Report => app_event_repo::report_video::receipt(conn, subject, key, step).await,
		}
	}

	/// Writes a step's receipt, for an episode of `of` parts (a call's week;
	/// every other feed's episode is one).
	///
	/// # Errors
	/// A storage failure, or what the feed's table refuses.
	pub async fn write_receipt(self, conn: &mut SqliteConnection, subject: &str, receipt: &Receipt, of: u32) -> Result<(), sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::write_receipt(conn, receipt, of).await,
			Self::PlayoffDelta => pd_fill_repo::video::write_receipt(conn, receipt).await,
			Self::RustEssay => rust_round_repo::essay_video::write_receipt(conn, crate::rust_essays::Kind::RustRounds, subject, receipt).await,
			Self::InterviewEssay => rust_round_repo::essay_video::write_receipt(conn, crate::rust_essays::Kind::Interview, subject, receipt).await,
			Self::Report => app_event_repo::report_video::write_receipt(conn, subject, receipt).await,
			// [`videos::let_go`] never gets this far for it (`lets_go`).
			Self::CardsEssay => Err(sqlx::Error::Protocol(String::from("a cards essay is never let go"))),
		}
	}
}

/// An episode home holds, and its date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldEpisode {
	/// Its key: for the call, the week's (its first part's).
	pub key: String,
	/// Its date, `YYYY-MM-DD`, by what its feed knows (server#906): the call's
	/// week's first day; Playoff Delta's, the day home took its fill, by
	/// home's clock (`renders::HOME_ZONE`); an essay's or a report's, the
	/// Monday of the ISO week it is of; a cards essay's own day. `None` only for a stamp or an
	/// essay's week that doesn't read, which the tables never hold.
	pub date: Option<String>,
}

impl HeldEpisode {
	/// An essay's, keyed by the ISO week it is of (`2026-w46`).
	fn of_iso_week(week: String) -> Self {
		Self {
			date: iso_monday(&week).map(|day| day.format("%Y-%m-%d").to_string()),
			key: week,
		}
	}
}

/// The Monday of the ISO week `week` names (`2026-w46`), if it names one.
fn iso_monday(week: &str) -> Option<chrono::NaiveDate> {
	let (year, number) = week.split_once("-w")?;
	chrono::NaiveDate::from_isoywd_opt(year.parse().ok()?, number.parse().ok()?, chrono::Weekday::Mon)
}

/// The day, by home's clock, of an ISO-8601 stamp.
fn day_taken(taken_at: &str) -> Option<String> {
	let at = chrono::DateTime::parse_from_rfc3339(taken_at).ok()?;
	Some(at.with_timezone(&renders::HOME_ZONE).format("%Y-%m-%d").to_string())
}

/// The weeks `subject` holds an essay of `kind` of, oldest first, less those
/// withdrawn for a deleted take (gone for good, with no video to list) and
/// those another account holds an essay of too: the drive keys an essay's
/// files by its week alone, so a shared week is none of theirs to reach
/// (`rust_essays::export` never gives one).
async fn essays_held(conn: &mut SqliteConnection, kind: crate::rust_essays::Kind, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	let withdrawn: Vec<String> = rust_round_repo::essay_video::withdrawn(conn, kind, subject).await?.into_iter().map(|w| w.week).collect();
	let mut held = Vec::new();
	for week in crate::rust_essays::essay_weeks(conn, kind, subject).await? {
		if !withdrawn.contains(&week) && crate::rust_essays::holders(conn, kind, &week).await? == 1 {
			held.push(week);
		}
	}
	Ok(held)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_episodes_day_is_what_its_feed_knows() {
		assert_eq!(HeldEpisode::of_iso_week(String::from("2026-w46")).date.as_deref(), Some("2026-11-09"));
		assert_eq!(HeldEpisode::of_iso_week(String::from("2027-w01")).date.as_deref(), Some("2027-01-04"));
		assert_eq!(HeldEpisode::of_iso_week(String::from("2026-w54")).date, None, "no such week");
		assert_eq!(HeldEpisode::of_iso_week(String::from("2026-11-09")).date, None);
		// Home's clock: a fill taken late on a Tuesday evening at home is
		// Tuesday's, though it is already Wednesday in UTC.
		assert_eq!(day_taken("2026-11-11T05:30:00.000Z").as_deref(), Some("2026-11-10"));
		assert_eq!(day_taken("2026-11-10T20:00:00.000Z").as_deref(), Some("2026-11-10"));
		assert_eq!(day_taken("yesterday"), None);
	}
}
