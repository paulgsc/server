//! The feeds home renders and the owner watches on the phone.
//!
//! FEED4, paulgsc/some-ui#2647: the weekly call (`call_weeks`) and Playoff
//! Delta (`pd_fills`). Each keeps its episodes, links and receipts in its own
//! tables, so each record is keyed to what it is of; what the phone does with
//! an episode's video (list it, bring it, record its `YouTube` link, let it
//! go) is one implementation over them all, [`videos`].
//!
//! An episode is named by its key, one plain path segment by its table's own
//! checks: the call's week by its first day (`2026-10-30`), Playoff Delta's
//! by `<season>-<team>-<NN>` (`2026-SF-08`).

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
}

impl Feed {
	/// The feed as the routes name it.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Call => "call",
			Self::PlayoffDelta => "playoff-delta",
		}
	}

	/// The feed a name names, if any.
	#[must_use]
	pub fn parse(name: &str) -> Option<Self> {
		match name {
			"call" => Some(Self::Call),
			"playoff-delta" => Some(Self::PlayoffDelta),
			_ => None,
		}
	}

	/// Where the render worker keeps each episode's folder.
	#[must_use]
	pub fn renders(self) -> &'static Path {
		Path::new(match self {
			Self::Call => crate::call_weeks::render::RENDERS_FOLDER,
			Self::PlayoffDelta => crate::pd_fills::render::RENDERS_FOLDER,
		})
	}

	/// Where `file_host` writes each episode for the render worker.
	#[must_use]
	pub fn input(self) -> &'static Path {
		Path::new(match self {
			Self::Call => crate::call_weeks::RENDER_FOLDER,
			Self::PlayoffDelta => crate::pd_fills::render::EPISODES_FOLDER,
		})
	}

	/// The video's file in an episode's folder.
	#[must_use]
	pub const fn video(self) -> &'static str {
		match self {
			Self::Call => crate::call_weeks::render::VIDEO,
			Self::PlayoffDelta => crate::pd_fills::render::VIDEO,
		}
	}

	/// What an episode's entry in [`Self::input`] is: a week's file, or a
	/// fill's folder.
	#[must_use]
	pub fn input_of(self, input: &Path, episode: &str) -> std::path::PathBuf {
		match self {
			Self::Call => input.join(String::from(episode) + ".json"),
			Self::PlayoffDelta => input.join(episode),
		}
	}

	/// Every episode home holds, newest first.
	///
	/// # Errors
	/// A storage failure.
	pub async fn held(self, conn: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
		Ok(match self {
			Self::Call => call_week_repo::held(conn).await?.into_iter().map(|week| week.from).collect(),
			Self::PlayoffDelta => pd_fill_repo::held(conn).await?.iter().rev().map(pd_fill_repo::HeldFill::key).collect(),
		})
	}

	/// Whether home holds the episode `key` names.
	///
	/// # Errors
	/// A storage failure.
	pub async fn holds(self, conn: &mut SqliteConnection, key: &str) -> Result<bool, sqlx::Error> {
		Ok(match self {
			Self::Call => call_week_repo::body(conn, key).await?.is_some(),
			Self::PlayoffDelta => match pd_fill_repo::parse_key(key) {
				Some((season, team, week)) => pd_fill_repo::bodies(conn, &season, &team, week).await?.is_some(),
				None => false,
			},
		})
	}

	/// Every episode's link: the newest recorded of each, by key.
	///
	/// # Errors
	/// A storage failure.
	pub async fn links(self, conn: &mut SqliteConnection) -> Result<BTreeMap<String, Link>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::links(conn).await,
			Self::PlayoffDelta => pd_fill_repo::video::links(conn).await,
		}
	}

	/// Records `youtube_id` as the episode's link, once.
	///
	/// # Errors
	/// A storage failure.
	pub async fn record_link(self, conn: &mut SqliteConnection, key: &str, youtube_id: &str, now: &str) -> Result<Recorded, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::record_link(conn, key, youtube_id, now).await,
			Self::PlayoffDelta => pd_fill_repo::video::record_link(conn, key, youtube_id, now).await,
		}
	}

	/// Every receipt, by episode and step.
	///
	/// # Errors
	/// A storage failure.
	pub async fn receipts(self, conn: &mut SqliteConnection) -> Result<Vec<Receipt>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::receipts(conn).await,
			Self::PlayoffDelta => pd_fill_repo::video::receipts(conn).await,
		}
	}

	/// The receipt of an episode's step, if it was let go.
	///
	/// # Errors
	/// A storage failure.
	pub async fn receipt(self, conn: &mut SqliteConnection, key: &str, step: Step) -> Result<Option<Receipt>, sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::receipt(conn, key, step).await,
			Self::PlayoffDelta => pd_fill_repo::video::receipt(conn, key, step).await,
		}
	}

	/// Writes a step's receipt.
	///
	/// # Errors
	/// A storage failure, or what the feed's table refuses.
	pub async fn write_receipt(self, conn: &mut SqliteConnection, receipt: &Receipt) -> Result<(), sqlx::Error> {
		match self {
			Self::Call => call_week_repo::video::write_receipt(conn, receipt).await,
			Self::PlayoffDelta => pd_fill_repo::video::write_receipt(conn, receipt).await,
		}
	}
}
