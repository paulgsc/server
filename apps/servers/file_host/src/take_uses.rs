//! What uses each take (INUSE, paulgsc/server C1380–C1384): the videos home
//! holds whose audio plays it, for the badge `GET /voice-notes/kept` and
//! `GET /voice-notes/takes` put on every note (`usedBy`) and the phone's
//! warning before a delete.
//!
//! **One rule with the withdraw.** A delete (`voice_repo::delete::delete`)
//! withdraws no video itself: each render pass withdraws its own on its next
//! tick, by a take's receipt. So what uses a take is read here with the very
//! function each pass withdraws by, never a copy of it:
//!
//! - a Rust Rounds or mock-interview week's essay video
//!   ([`UseKind::FeedVideo`]): `rust_essays::plays`, which `rust_essays`'
//!   tidy withdraws by;
//! - a cards essay ([`UseKind::Essay`]): `cards_essays::pass::spoken_takes`,
//!   which its tidy and its export withdraw by;
//! - a Studio video queued on home ([`UseKind::StudioVideo`]): the takes its
//!   newest DNA revision names (`studio_video_repo::naming`). Nothing
//!   withdraws one, so its use never does (`withdrawsOnDelete: false`): the
//!   owner's badge still shows it, as a used take must never read as unused
//!   (C1383, claude's call, which the owner can move).
//!
//! A video already gone (let go or withdrawn) uses nothing.
//!
//! **Cost** (docs/home-compute.md: interactive work, bounded by what the
//! subject holds, never by notes × videos): per list, one read of each video
//! family the subject holds, as each pass reads it (its essays, its gone
//! weeks, and per week two indexed reads and one small file; the cards
//! essays and one body each; the queued videos' newest revisions), then one
//! map lookup per note. Nothing runs per note.
//!
//! **Whose.** Every read takes the subject. The cards essays are the mailbox
//! subject's alone (`mailbox::pass_subject`), as their pass reads them, so
//! another subject sees none of them.

use crate::auth::owner::Owner;
use crate::rust_essays::{self, Kind};
use crate::AppState;
use axum::extract::FromRef;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashMap;
use std::path::PathBuf;
use voice_repo::used::{Use, UseKind};

/// Where the uses are read from: the folders the essay passes give the
/// render worker, and whose the cards essays are.
#[derive(Debug, Clone)]
pub struct Sources {
	/// Each essay kind's [`rust_essays::Folders::essays`], where the revision
	/// given is read.
	pub essays: Vec<(Kind, PathBuf)>,
	/// The cards essays' subject, the mailbox's; none without one.
	pub cards_subject: Option<String>,
}

/// What a list handler is given to find the [`Sources`] by: the essay
/// passes' folders and the deployment's `MAILBOX_SUBJECT`. Its own state, so
/// a test gives it its own folders.
#[derive(Debug, Clone)]
pub struct UsesFrom {
	/// Each essay kind's folder given to the render worker.
	pub essays: Vec<(Kind, PathBuf)>,
	/// `MAILBOX_SUBJECT`, when the deployment set one.
	pub configured: Option<String>,
}

impl UsesFrom {
	/// The server's own essay folders, with `configured` as the mailbox's
	/// subject.
	#[must_use]
	pub fn server(configured: Option<String>) -> Self {
		Self {
			essays: rust_essays::KINDS.iter().map(|&kind| (kind, rust_essays::Folders::server(kind).essays)).collect(),
			configured,
		}
	}
}

impl FromRef<AppState> for UsesFrom {
	fn from_ref(state: &AppState) -> Self {
		Self::server(state.mailbox.as_ref().and_then(|mailbox| mailbox.subject.clone()))
	}
}

impl Sources {
	/// What `from` names, the cards essays' subject read as their pass reads
	/// it: `MAILBOX_SUBJECT`, else the owner's account.
	///
	/// # Errors
	/// A storage failure reading the owner.
	pub async fn read(db: &SqlitePool, from: &UsesFrom) -> Result<Self, sqlx::Error> {
		let cards_subject = match crate::mailbox::pass_subject(db, from.configured.as_deref()).await? {
			Owner::Subject(subject) => Some(subject.as_str().to_owned()),
			Owner::NoAccount | Owner::Several => None,
		};
		Ok(Self {
			essays: from.essays.clone(),
			cards_subject,
		})
	}
}

/// The fixed words a week's essay video is shown by.
const fn week_title(kind: Kind) -> &'static str {
	match kind {
		Kind::RustRounds => "Rust Rounds essay, ",
		Kind::Interview => "Mock interview essay, ",
	}
}

/// Puts `used` among `take`'s uses.
fn add(uses: &mut HashMap<String, Vec<Use>>, take: &str, used: Use) {
	uses.entry(take.to_owned()).or_default().push(used);
}

/// Every video `subject` holds that uses a take, by the take's note id:
/// feed videos first, then essays, then Studio videos, each oldest first.
///
/// # Errors
/// A storage failure, or a stored record that no longer reads.
pub async fn of(conn: &mut SqliteConnection, subject: &str, sources: &Sources) -> Result<HashMap<String, Vec<Use>>, sqlx::Error> {
	let mut uses: HashMap<String, Vec<Use>> = HashMap::new();
	for (kind, essays) in &sources.essays {
		for week in rust_essays::uses(conn, *kind, subject, essays).await? {
			for play in &week.plays {
				add(
					&mut uses,
					&play.take,
					Use {
						kind: UseKind::FeedVideo,
						key: week.week.clone(),
						title: String::from(week_title(*kind)) + &week.week,
						at: week.at.clone(),
						withdraws_on_delete: play.withdraws,
					},
				);
			}
		}
	}
	if sources.cards_subject.as_deref() == Some(subject) {
		let gone = cards_essay_repo::gone(conn).await?;
		let titles: HashMap<String, Option<String>> = cards_essay_repo::titles(conn).await?.into_iter().collect();
		for essay in cards_essay_repo::held(conn).await? {
			if gone.contains(&essay.key) {
				continue;
			}
			let Some(body) = cards_essay_repo::body(conn, &essay.key).await? else { continue };
			let title = titles.get(&essay.key).cloned().flatten().unwrap_or_else(|| essay.key.clone());
			for take in crate::cards_essays::pass::spoken_takes(&body) {
				add(
					&mut uses,
					&take,
					Use {
						kind: UseKind::Essay,
						key: essay.key.clone(),
						title: title.clone(),
						at: essay.taken_at.clone(),
						withdraws_on_delete: true,
					},
				);
			}
		}
	}
	for video in studio_video_repo::naming(conn, subject).await? {
		for note in &video.notes {
			add(
				&mut uses,
				note,
				Use {
					kind: UseKind::StudioVideo,
					key: video.video.clone(),
					title: video.title.clone(),
					at: video.queued_at.clone(),
					withdraws_on_delete: false,
				},
			);
		}
	}
	Ok(uses)
}
