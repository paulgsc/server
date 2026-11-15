//! The feeds home renders and the owner watches on the phone.
//!
//! FEED4, paulgsc/some-ui#2647: the weekly call (`call_weeks`) and Playoff
//! Delta (`pd_fills`), Rust Rounds' essays (`rust_essays`, RR16,
//! paulgsc/some-ui#2691), and cards essays (`cards_essays`, CARDS7 #854;
//! EXPLAIN6 #878). Each keeps its episodes, links and receipts in its own
//! tables, so each record is keyed to what it is of; what the phone does with
//! an episode's video (list it, bring it, record its `YouTube` link, let it
//! go) is one implementation over them all, [`videos`].
//!
//! **A cards essay is listed, brought and linked, never let go**
//! ([`Feed::lets_go`]). The owner's ask (2026-11-14) was a place among the
//! episodes; its `YouTube` link (paulgsc/some-ui#2999, call C388) is kept in
//! `cards_essay_link`, beside `cards_essay`, which is append-only and holds
//! the essay alone. Letting home's copy go once the link is recorded needs a
//! receipt table too, and is a later story.
//!
//! An episode is named by its key, one plain path segment by its table's own
//! checks: the call's week by its first day (`2026-10-30`), or, for a week
//! that is a series of parts (paulgsc/some-ui#2918), each part after the
//! first by `<day>-p<k>` (`2026-11-06-p2`; `call_week_repo::video::split_key`),
//! each its own episode with its own video, link and receipts; Playoff Delta's
//! by `<season>-<team>-<NN>` (`2026-SF-08`), an essay by its week
//! (`2026-w46`), a cards essay by its slug and day
//! (`nixos-upgrade-2026-11-14`).
//!
//! **Whose.** The call's, Playoff Delta's and the cards essays' episodes are
//! no one's: what was said by home's speaker about a public week, or about
//! public repositories at stamped commits. An essay's is its subject's,
//! since its video plays the owner's own takes (docs/identity.md invariant
//! 23's one named exception), so every method that reads or writes the
//! feed's tables takes the caller's subject, and the essay feed answers only
//! that subject's weeks.

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
	/// Cards essays (CARDS7, EXPLAIN6): listed, brought and linked
	/// (paulgsc/some-ui#2999), never let go ([`Self::lets_go`]).
	CardsEssay,
}

impl Feed {
	/// The feed as the routes name it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Call => "call",
			Self::PlayoffDelta => "playoff-delta",
			Self::RustEssay => "rust-essay",
			Self::CardsEssay => "cards-essay",
		}
	}

	/// The feed a name names, if any.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		match name {
			"call" => Some(Self::Call),
			"playoff-delta" => Some(Self::PlayoffDelta),
			"rust-essay" => Some(Self::RustEssay),
			"cards-essay" => Some(Self::CardsEssay),
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
			Self::CardsEssay => crate::cards_essays::RENDERS_FOLDER,
		})
	}

	/// Where `file_host` writes each episode for the render worker.
	#[must_use]
	pub fn input(self) -> &'static Path {
		Path::new(match self {
			Self::Call => crate::call_weeks::RENDER_FOLDER,
			Self::PlayoffDelta => crate::pd_fills::render::EPISODES_FOLDER,
			Self::RustEssay => crate::rust_essays::ESSAYS_FOLDER,
			Self::CardsEssay => crate::cards_essays::ESSAYS_FOLDER,
		})
	}

	/// The video's file in an episode's folder.
	#[must_use]
	pub const fn video(self) -> &'static str {
		match self {
			Self::Call => crate::call_weeks::render::VIDEO,
			Self::PlayoffDelta => crate::pd_fills::render::VIDEO,
			Self::RustEssay => crate::rust_essays::VIDEO,
			Self::CardsEssay => crate::cards_essays::VIDEO,
		}
	}

	/// Whether an episode's files are let go from the phone: every feed's but
	/// a cards essay's, which has no receipt table yet (the module doc). Every
	/// feed's link is kept. The routes answer `404` for one that doesn't.
	#[must_use]
	pub const fn lets_go(self) -> bool {
		!matches!(self, Self::CardsEssay)
	}

	/// What an episode's entry in [`Self::input`] is: a week's or a cards
	/// essay's file, or a fill's or an essay's folder.
	#[must_use]
	pub fn input_of(self, input: &Path, episode: &str) -> std::path::PathBuf {
		match self {
			Self::Call | Self::CardsEssay => input.join(String::from(episode) + ".json"),
			Self::PlayoffDelta | Self::RustEssay => input.join(episode),
		}
	}

	/// Every episode home holds, newest first: for the call, each week, whose
	/// parts ([`videos::list`]) are read from the drive.
	///
	/// # Errors
	/// A storage failure.
	pub async fn held(self, conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
		Ok(match self {
			Self::Call => call_week_repo::held(conn).await?.into_iter().map(|week| week.from).collect(),
			Self::PlayoffDelta => pd_fill_repo::held(conn).await?.iter().rev().map(pd_fill_repo::HeldFill::key).collect(),
			Self::RustEssay => essays_held(conn, subject).await?.into_iter().rev().collect(),
			// Newest day first, as the table orders them oldest first.
			Self::CardsEssay => cards_essay_repo::held(conn).await?.into_iter().rev().map(|essay| essay.key).collect(),
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
			Self::RustEssay => essays_held(conn, subject).await?.iter().any(|week| week == key),
			Self::CardsEssay => crate::cards_essays::pass::is_key(key) && cards_essay_repo::holds(conn, key).await?,
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
			Self::RustEssay => rust_round_repo::essay_video::links(conn, subject).await,
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
				rust_round_repo::essay_video::record_link(conn, subject, key, held, youtube_id, now).await
			}
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
			Self::RustEssay => rust_round_repo::essay_video::receipts(conn, subject).await,
			Self::CardsEssay => Ok(Vec::new()),
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
			Self::RustEssay => rust_round_repo::essay_video::receipt(conn, subject, key, step).await,
			Self::CardsEssay => Ok(None),
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
			Self::RustEssay => rust_round_repo::essay_video::write_receipt(conn, subject, receipt).await,
			// [`videos::let_go`] never gets this far for it (`lets_go`).
			Self::CardsEssay => Err(sqlx::Error::Protocol(String::from("a cards essay is never let go"))),
		}
	}
}

/// The weeks `subject` holds an essay of, oldest first, less those withdrawn
/// for a deleted take (gone for good, with no video to list) and those
/// another account holds an essay of too: the drive keys an essay's files by
/// its week alone, so a shared week is none of theirs to reach
/// (`rust_essays::export` never gives one).
async fn essays_held(conn: &mut SqliteConnection, subject: &str) -> Result<Vec<String>, sqlx::Error> {
	let storage = |err| match err {
		mailbox_repo::MailboxError::Storage(err) => err,
		mailbox_repo::MailboxError::Rewritten => sqlx::Error::Protocol(String::from("a mailbox file does not read back")),
	};
	let withdrawn: Vec<String> = rust_round_repo::essay_video::withdrawn(conn, subject).await?.into_iter().map(|w| w.week).collect();
	let mut held = Vec::new();
	for week in mailbox_repo::rust_essay_weeks(conn, subject).await.map_err(storage)? {
		if !withdrawn.contains(&week) && mailbox_repo::rust_essay_holders(conn, &week).await.map_err(storage)? == 1 {
			held.push(week);
		}
	}
	Ok(held)
}
