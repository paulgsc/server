//! Home's bookkeeping for the Jobs routine, filed to the mailbox's `index`
//! folder after each pass (paulgsc/server#509, HF7).
//!
//! The morning routine used to rebuild this itself, from a scratch folder of
//! every digest and decisions file it downloaded from Drive: which picks
//! still hold a slot, which files and notes are new since the last digest,
//! and each note's newest transcript (paulgsc/some-ui
//! `packages/job-desk/ROUTINE.md`, §2). Home already holds every one of those
//! files, so it computes them here and the routine reads one small file. The
//! model fills values; it does not do the bookkeeping (paulgsc/some-ui#2162).
//!
//! **What it is computed from** ([`mailbox_repo::jobs_inputs`]): every
//! digest home kept from `brief`, and every decisions file, scouts file and
//! Jobs note's transcript the mailbox took from home as home filed it. No new
//! table: those are home's own rows, and the index is a file it files in
//! `mailbox_outgoing` and hands over like any other.
//!
//! **Deterministic.** [`jobs_index`] is a pure function of those rows, the
//! day and `asOf`: the same rows give the same bytes. Every list is in a
//! fixed order (by name, or by digest day as the queue is), and the body is
//! written from structs whose fields serialise in declaration order.
//!
//! **Filed only when it changes.** A pass first reads when home last took in
//! anything the index counts ([`mailbox_repo::jobs_as_of`]) and the day in
//! [`JOBS_ZONE`]. When both match the newest index filed, nothing is
//! computed or filed: the inputs are append-only and a name holds one body,
//! so nothing it counts can have changed. Otherwise it computes, and files a
//! new name, `index-jobs-<stamp>.json`, only if the bytes differ from the
//! last. So the mailbox gets one a day at least (the day moves at midnight
//! in [`JOBS_ZONE`], and lapses with it), plus one each time a digest, a file
//! or a Jobs transcript arrives. A body over the mailbox's ceiling is not
//! filed, and counted as refused (`too_big`).
//!
//! **What it carries, and what it never does.** File names, pick ids,
//! fingerprints, postings' titles, companies and links, event and scout ids,
//! note ids, and transcripts' names and hashes: what the routine needs to
//! know what to read, never a file's contents beyond those fields. Nothing
//! in it is audio or points at audio: a note is named by its file name and
//! the phone's id, and its transcript by its name in the mailbox
//! (docs/identity.md invariant 19).
//!
//! **The queue is `@some-ui/job-desk`'s.** [`queue_of`] and [`free_slots`]
//! port `queueOf` and `freeSlots` (`packages/job-desk/src/queue.ts`), and
//! [`LAPSE_DAYS`] and [`MAX_OPEN_PICKS`] its policy; a change to either there
//! is a change here. The tests port `queue.test.ts`'s cases.

use crate::auth::DeletionLock;
use chrono::{DateTime, NaiveDate, Utc};
use mailbox_kit::{Mailbox, MailboxTransport};
use mailbox_repo::{Change, HeldFile, HeldTranscript, JobsInputs, MailboxError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::pass::{Failure, Handed, Outcome, MAILBOX_BODY_CEILING};

/// What an index put to the mailbox's `index` folder is (paulgsc/some-ui
/// `apps/mailbox/README.md`, "What a put must satisfy").
pub const INDEX_SCHEMA: &str = "some-ui.routine-index/1";

/// The routine this index is for, as its name and `routine` say.
pub const JOBS: &str = "jobs";

/// The zone the Jobs routine's day is in: "Today is the date in
/// `America/Los_Angeles`" (`ROUTINE.md` §0).
pub const JOBS_ZONE: chrono_tz::Tz = chrono_tz::America::Los_Angeles;

/// The days a pick no decision answers waits before it lapses
/// (`LAPSE_DAYS`, `packages/job-desk/src/policy.ts`).
pub const LAPSE_DAYS: u64 = 3;

/// The tracks, and the most picks each may hold open at once
/// (`MAX_OPEN_PICKS`, `packages/job-desk/src/policy.ts`).
pub const MAX_OPEN_PICKS: [(&str, usize); 2] = [("software", 3), ("adjacent", 2)];

// ---------------------------------------------------------------------------
// The files, as far as the index reads them. A file that does not read as
// its schema is `skipped`, as the routine's `queue` script skips one.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Posting {
	title: String,
	company: String,
	#[serde(rename = "applyUrl")]
	apply_url: String,
}

#[derive(Deserialize)]
struct DigestPick {
	id: String,
	fingerprint: String,
	track: String,
	posting: Posting,
}

#[derive(Deserialize)]
struct Lapse {
	day: String,
	#[serde(rename = "pickId")]
	pick_id: String,
}

#[derive(Deserialize)]
struct DigestQueue {
	#[serde(default)]
	lapsed: Vec<Lapse>,
}

#[derive(Deserialize)]
struct Answered {
	#[serde(rename = "scoutId")]
	scout_id: String,
}

#[derive(Deserialize)]
struct DigestFile {
	schema: String,
	day: String,
	picks: Vec<DigestPick>,
	#[serde(default)]
	queue: Option<DigestQueue>,
	#[serde(default)]
	scouts: Vec<Answered>,
	read: Vec<String>,
	/// `notesDone`, which a digest from before it named `transcribed`.
	#[serde(rename = "notesDone", default)]
	notes_done: Option<Vec<String>>,
	#[serde(default)]
	transcribed: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Said {
	id: String,
	#[serde(default)]
	voice: Vec<String>,
}

#[derive(Deserialize)]
struct DecidedPick {
	day: String,
	#[serde(rename = "pickId")]
	pick_id: String,
	fingerprint: String,
	id: Option<String>,
	decision: String,
	voice: Vec<String>,
	debrief: Option<Said>,
	#[serde(default)]
	fit: Vec<Said>,
}

#[derive(Deserialize)]
struct Named {
	id: String,
	fingerprint: String,
}

#[derive(Deserialize)]
struct Withdrawal {
	id: String,
}

#[derive(Deserialize)]
struct DecisionsFile {
	schema: String,
	#[serde(rename = "sharedAt")]
	shared_at: String,
	picks: Vec<DecidedPick>,
	#[serde(default)]
	notes: BTreeMap<String, String>,
	disagreements: Vec<Named>,
	outcomes: Vec<Named>,
	#[serde(default)]
	withdrawals: Vec<Withdrawal>,
}

#[derive(Deserialize)]
struct Scout {
	id: String,
	voice: Vec<String>,
}

#[derive(Deserialize)]
struct ScoutsFile {
	schema: String,
	scouts: Vec<Scout>,
	#[serde(default)]
	notes: BTreeMap<String, String>,
}

/// A real `YYYY-MM-DD`.
fn is_day(text: &str) -> bool {
	text.len() == 10 && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

fn digest_of(file: &HeldFile) -> Option<DigestFile> {
	let digest: DigestFile = serde_json::from_slice(&file.body).ok()?;
	let tracks_known = digest.picks.iter().all(|pick| MAX_OPEN_PICKS.iter().any(|(track, _)| *track == pick.track));
	(digest.schema == "some-ui.job-digest/1" && is_day(&digest.day) && tracks_known).then_some(digest)
}

const DECISIONS: [&str; 4] = ["taken", "passed", "archived", "untouched"];

fn decisions_of(file: &HeldFile) -> Option<DecisionsFile> {
	let decisions: DecisionsFile = serde_json::from_slice(&file.body).ok()?;
	let known = decisions.picks.iter().all(|pick| DECISIONS.contains(&pick.decision.as_str()));
	(decisions.schema == "some-ui.job-decisions/1" && known).then_some(decisions)
}

fn scouts_of(file: &HeldFile) -> Option<ScoutsFile> {
	let scouts: ScoutsFile = serde_json::from_slice(&file.body).ok()?;
	(scouts.schema == "some-ui.job-scouts/1").then_some(scouts)
}

impl DigestFile {
	fn done(&self) -> &[String] {
		self.notes_done.as_deref().or(self.transcribed.as_deref()).unwrap_or_default()
	}
}

impl DecisionsFile {
	/// Every phone event id this file carries.
	fn ids(&self) -> impl Iterator<Item = &str> {
		self
			.picks
			.iter()
			.flat_map(|pick| pick.id.iter().chain(pick.debrief.iter().map(|said| &said.id)).chain(pick.fit.iter().map(|said| &said.id)))
			.chain(self.disagreements.iter().map(|named| &named.id))
			.chain(self.outcomes.iter().map(|named| &named.id))
			.chain(self.withdrawals.iter().map(|withdrawal| &withdrawal.id))
			.map(String::as_str)
	}

	/// Every voice note this file names, by file name, in the order it names them.
	fn voices(&self) -> impl Iterator<Item = &str> {
		self
			.picks
			.iter()
			.flat_map(|pick| {
				pick
					.voice
					.iter()
					.chain(pick.debrief.iter().flat_map(|said| &said.voice))
					.chain(pick.fit.iter().flat_map(|said| &said.voice))
			})
			.map(String::as_str)
	}
}

// ---------------------------------------------------------------------------
// The index, as the mailbox takes it. Field order is the bytes' order.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenPick {
	pub day: String,
	#[serde(rename = "pickId")]
	pub pick_id: String,
	pub track: String,
	pub fingerprint: String,
	pub title: String,
	pub company: String,
	#[serde(rename = "applyUrl")]
	pub apply_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LapsedPick {
	pub day: String,
	#[serde(rename = "pickId")]
	pub pick_id: String,
	pub track: String,
	pub fingerprint: String,
	pub title: String,
	pub company: String,
}

/// Each track's free slots, in `MAX_OPEN_PICKS`' order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Free {
	pub software: usize,
	pub adjacent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Queue {
	pub open: Vec<OpenPick>,
	pub lapsed: Vec<LapsedPick>,
	pub free: Free,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct From {
	digests: Vec<String>,
	decisions: Vec<String>,
	scout: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NewDecisions {
	name: String,
	/// The event ids in it that a file some digest read already carried.
	repeats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NewScouts {
	name: String,
	/// The scouts in it some digest already answered.
	answered: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct New {
	decisions: Vec<NewDecisions>,
	scout: Vec<NewScouts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Transcript {
	name: String,
	sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Note {
	/// The voice note's file name, as the file that names it gives it.
	file: String,
	/// The first file, by folder and name, that names it.
	#[serde(rename = "in")]
	named_in: String,
	/// The phone's id for it, or `None` from a file that gives none.
	#[serde(rename = "note")]
	id: Option<String>,
	/// Its newest transcript the mailbox holds, or `None` while home has
	/// not heard it.
	transcript: Option<Transcript>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct JobsIndex<'a> {
	schema: &'static str,
	routine: &'static str,
	day: &'a str,
	#[serde(rename = "asOf")]
	as_of: Option<&'a str>,
	from: From,
	skipped: Vec<String>,
	queue: Queue,
	new: New,
	notes: Vec<Note>,
	picked: Vec<String>,
}

/// `day` plus `days`, as `YYYY-MM-DD`; `None` for a day that does not read.
fn add_days(day: &str, days: u64) -> Option<String> {
	let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
	Some(date.checked_add_days(chrono::Days::new(days))?.format("%Y-%m-%d").to_string())
}

/// Where a pick stands on `today`: `queue.ts`'s `standing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
	Open,
	Lapsed,
	Done,
}

fn standing(stands: Option<&DecidedPick>, digest_day: &str, today: &str) -> Standing {
	let untouched = || {
		if add_days(digest_day, LAPSE_DAYS).is_some_and(|lapses| lapses.as_str() <= today) {
			Standing::Lapsed
		} else {
			Standing::Open
		}
	};
	match stands.map(|pick| (pick.decision.as_str(), pick.debrief.is_some())) {
		None | Some(("untouched", _)) => untouched(),
		Some(("taken", false)) => Standing::Open,
		_ => Standing::Done,
	}
}

/// The picks of `digests` still open on what `decisions` say, and those that
/// lapse on `today`, oldest digest first: `queueOf`. The newest share's entry
/// for a pick stands; files shared at the same moment keep their order.
fn queue_of(digests: &[&DigestFile], decisions: &[&DecisionsFile], today: &str) -> (Vec<OpenPick>, Vec<LapsedPick>) {
	let mut by_share = decisions.to_vec();
	by_share.sort_by(|a, b| a.shared_at.cmp(&b.shared_at));
	let mut stands: HashMap<(&str, &str), &DecidedPick> = HashMap::new();
	for file in by_share {
		for pick in &file.picks {
			stands.insert((pick.day.as_str(), pick.pick_id.as_str()), pick);
		}
	}
	let recorded: HashSet<(&str, &str)> = digests
		.iter()
		.flat_map(|digest| digest.queue.iter().flat_map(|queue| &queue.lapsed))
		.map(|lapse| (lapse.day.as_str(), lapse.pick_id.as_str()))
		.collect();
	let mut by_day = digests.to_vec();
	by_day.sort_by(|a, b| a.day.cmp(&b.day));
	let (mut open, mut lapsed) = (Vec::new(), Vec::new());
	for digest in by_day {
		for pick in &digest.picks {
			let key = (digest.day.as_str(), pick.id.as_str());
			match standing(stands.get(&key).copied(), &digest.day, today) {
				Standing::Done => {}
				Standing::Lapsed if recorded.contains(&key) => {}
				Standing::Open => open.push(OpenPick {
					day: digest.day.clone(),
					pick_id: pick.id.clone(),
					track: pick.track.clone(),
					fingerprint: pick.fingerprint.clone(),
					title: pick.posting.title.clone(),
					company: pick.posting.company.clone(),
					apply_url: pick.posting.apply_url.clone(),
				}),
				Standing::Lapsed => lapsed.push(LapsedPick {
					day: digest.day.clone(),
					pick_id: pick.id.clone(),
					track: pick.track.clone(),
					fingerprint: pick.fingerprint.clone(),
					title: pick.posting.title.clone(),
					company: pick.posting.company.clone(),
				}),
			}
		}
	}
	(open, lapsed)
}

/// How many new picks each track has room for while `open` holds its slots,
/// none of them closed (the routine checks that): `freeSlots`.
fn free_slots(open: &[OpenPick]) -> Free {
	let free = |track: &str| {
		let limit = MAX_OPEN_PICKS.iter().find(|(name, _)| *name == track).map_or(0, |(_, limit)| *limit);
		limit.saturating_sub(open.iter().filter(|pick| pick.track == track).count())
	};
	Free {
		software: free("software"),
		adjacent: free("adjacent"),
	}
}

/// The newest transcript of each note the mailbox holds, by note id: the
/// highest `<n>` of its `transcript-<note>-<n>.json` names.
fn newest_transcripts(transcripts: &[HeldTranscript]) -> HashMap<&str, (u64, &HeldTranscript)> {
	let mut newest: HashMap<&str, (u64, &HeldTranscript)> = HashMap::new();
	for held in transcripts {
		let Some((note, n)) = held
			.name
			.strip_prefix("transcript-")
			.and_then(|rest| rest.strip_suffix(".json"))
			.and_then(|rest| rest.rsplit_once('-'))
		else {
			continue;
		};
		let Ok(n) = n.parse::<u64>() else {
			continue;
		};
		if newest.get(note).is_none_or(|(held_n, _)| *held_n < n) {
			newest.insert(note, (n, held));
		}
	}
	newest
}

/// Every file of `inputs` that reads as its schema, and the names of those
/// that do not, sorted.
struct Read<'a> {
	digests: Vec<(&'a HeldFile, DigestFile)>,
	decisions: Vec<(&'a HeldFile, DecisionsFile)>,
	scouts: Vec<(&'a HeldFile, ScoutsFile)>,
	skipped: Vec<String>,
}

fn read_all(inputs: &JobsInputs) -> Read<'_> {
	fn each<'a, T>(files: &'a [HeldFile], parse: fn(&HeldFile) -> Option<T>, skipped: &mut Vec<String>) -> Vec<(&'a HeldFile, T)> {
		let mut read = Vec::new();
		for file in files {
			match parse(file) {
				Some(parsed) => read.push((file, parsed)),
				None => skipped.push(file.name.clone()),
			}
		}
		read
	}
	let mut skipped = Vec::new();
	let digests = each(&inputs.digests, digest_of, &mut skipped);
	let decisions = each(&inputs.decisions, decisions_of, &mut skipped);
	let scouts = each(&inputs.scouts, scouts_of, &mut skipped);
	skipped.sort();
	Read {
		digests,
		decisions,
		scouts,
		skipped,
	}
}

/// Sorted and once each.
fn sorted<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
	items.collect::<BTreeSet<_>>().into_iter().map(String::from).collect()
}

/// Each decisions and scouts file no digest names in `read`, with what in it
/// an earlier one already carried.
fn whats_new(read: &Read<'_>) -> New {
	let taken: HashSet<&str> = read.digests.iter().flat_map(|(_, digest)| &digest.read).map(String::as_str).collect();
	let carried: HashSet<&str> = read
		.decisions
		.iter()
		.filter(|(file, _)| taken.contains(file.name.as_str()))
		.flat_map(|(_, file)| file.ids())
		.collect();
	let answered: HashSet<&str> = read.digests.iter().flat_map(|(_, digest)| &digest.scouts).map(|answer| answer.scout_id.as_str()).collect();
	New {
		decisions: read
			.decisions
			.iter()
			.filter(|(file, _)| !taken.contains(file.name.as_str()))
			.map(|(file, decisions)| NewDecisions {
				name: file.name.clone(),
				repeats: sorted(decisions.ids().filter(|id| carried.contains(id))),
			})
			.collect(),
		scout: read
			.scouts
			.iter()
			.filter(|(file, _)| !taken.contains(file.name.as_str()))
			.map(|(file, scouts)| NewScouts {
				name: file.name.clone(),
				answered: sorted(scouts.scouts.iter().map(|scout| scout.id.as_str()).filter(|id| answered.contains(id))),
			})
			.collect(),
	}
}

/// Each voice note a file names that no digest lists as done, once, in the
/// order the files (by folder and name) name them.
fn waiting_notes(read: &Read<'_>, transcripts: &[HeldTranscript]) -> Vec<Note> {
	let done: HashSet<&str> = read.digests.iter().flat_map(|(_, digest)| digest.done()).map(String::as_str).collect();
	let newest = newest_transcripts(transcripts);
	let named = read
		.decisions
		.iter()
		.flat_map(|(file, decisions)| decisions.voices().map(move |voice| (file, voice, &decisions.notes)))
		.chain(
			read
				.scouts
				.iter()
				.flat_map(|(file, scouts)| scouts.scouts.iter().flat_map(|scout| &scout.voice).map(move |voice| (file, voice.as_str(), &scouts.notes))),
		);
	let mut seen: HashSet<&str> = HashSet::new();
	let mut notes = Vec::new();
	for (file, voice, ids) in named {
		if done.contains(voice) || !seen.insert(voice) {
			continue;
		}
		let id = ids.get(voice).cloned();
		let transcript = id.as_deref().and_then(|id| newest.get(id)).map(|(_, held)| Transcript {
			name: held.name.clone(),
			sha256: held.sha256.clone(),
		});
		notes.push(Note {
			file: String::from(voice),
			named_in: file.name.clone(),
			id,
			transcript,
		});
	}
	notes
}

/// Every fingerprint a digest picked or a decisions file names.
fn picked_of(read: &Read<'_>) -> Vec<String> {
	let digests = read.digests.iter().flat_map(|(_, digest)| digest.picks.iter().map(|pick| pick.fingerprint.as_str()));
	let decisions = read.decisions.iter().flat_map(|(_, file)| {
		file
			.picks
			.iter()
			.map(|pick| pick.fingerprint.as_str())
			.chain(file.outcomes.iter().map(|named| named.fingerprint.as_str()))
			.chain(file.disagreements.iter().map(|named| named.fingerprint.as_str()))
	});
	sorted(digests.chain(decisions))
}

/// The Jobs index for `day`, from `inputs`, stamped `as_of`: its bytes.
///
/// Pure: the same inputs give the same bytes (the module doc).
#[must_use]
pub fn jobs_index(inputs: &JobsInputs, day: &str, as_of: Option<&str>) -> Vec<u8> {
	let read = read_all(inputs);
	let digests: Vec<&DigestFile> = read.digests.iter().map(|(_, digest)| digest).collect();
	let decisions: Vec<&DecisionsFile> = read.decisions.iter().map(|(_, file)| file).collect();
	let (open, lapsed) = queue_of(&digests, &decisions, day);
	let free = free_slots(&open);
	let names = |files: &[HeldFile]| files.iter().map(|file| file.name.clone()).collect();
	let index = JobsIndex {
		schema: INDEX_SCHEMA,
		routine: JOBS,
		day,
		as_of,
		from: From {
			digests: names(&inputs.digests),
			decisions: names(&inputs.decisions),
			scout: names(&inputs.scouts),
		},
		skipped: read.skipped.clone(),
		queue: Queue { open, lapsed, free },
		new: whats_new(&read),
		notes: waiting_notes(&read, &inputs.transcripts),
		picked: picked_of(&read),
	};
	let mut body = Vec::new();
	// Writing these structs to a `Vec` cannot fail: every key is a string.
	let _ = serde_json::to_writer(&mut body, &index);
	body
}

/// The day `now` falls on in [`JOBS_ZONE`], as `YYYY-MM-DD`; `None` when
/// `now` does not read.
fn jobs_day(now: &str) -> Option<String> {
	let now = DateTime::parse_from_rfc3339(now).ok()?.with_timezone(&Utc);
	Some(now.with_timezone(&JOBS_ZONE).format("%Y-%m-%d").to_string())
}

/// `now` (`2026-10-31T15:00:00.123Z`) as a name's stamp
/// (`20261031T150000123Z`); `None` when it is not in that form.
fn stamp_of(now: &str) -> Option<String> {
	let stamp: String = now.chars().filter(|c| !matches!(c, '-' | ':' | '.')).collect();
	let shape = stamp.len() == 19 && stamp.as_bytes()[8] == b'T' && stamp.ends_with('Z');
	shape.then_some(stamp)
}

/// The `day` and `asOf` an index body was computed for.
#[derive(Deserialize, PartialEq, Eq)]
struct Stamped {
	day: String,
	#[serde(rename = "asOf")]
	as_of: Option<String>,
}

/// Computes `subject`'s Jobs index and files and hands it, when what it
/// counts has moved since the last one (the module doc).
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// an index filed and not handed is handed by the next pass's outbound.
pub(super) async fn file_jobs_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	deletion: &DeletionLock,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let at = now();
	let (Some(day), Some(stamp)) = (jobs_day(&at), stamp_of(&at)) else {
		return Ok(());
	};
	let (inputs, as_of, last) = {
		let mut tx = db.begin().await.map_err(Failure::Storage)?;
		let as_of = mailbox_repo::jobs_as_of(&mut tx, subject).await.map_err(super::pass::storage)?;
		// Nothing to count yet: no index, and the routine reads Drive.
		if as_of.is_none() {
			return Ok(());
		}
		let last = mailbox_repo::last_index(&mut tx, subject, JOBS).await.map_err(super::pass::storage)?;
		let stamped = last.as_ref().and_then(|last| serde_json::from_slice::<Stamped>(&last.body).ok());
		if stamped.is_some_and(|stamped| stamped.day == day && stamped.as_of == as_of) {
			return Ok(());
		}
		let inputs = mailbox_repo::jobs_inputs(&mut tx, subject).await.map_err(super::pass::storage)?;
		tx.commit().await.map_err(Failure::Storage)?;
		(inputs, as_of, last)
	};
	let body = jobs_index(&inputs, &day, as_of.as_deref());
	if last.is_some_and(|last| last.body == body) {
		return Ok(());
	}
	if body.len() > MAILBOX_BODY_CEILING {
		outcome.refused.push("too_big");
		return Ok(());
	}
	let name = mailbox_repo::index_name(JOBS, &stamp);
	let sha256 = hex::encode(Sha256::digest(&body));
	let filed = {
		let _hold = deletion.hold().await;
		let mut tx = db.begin_with("BEGIN IMMEDIATE").await.map_err(Failure::Storage)?;
		let change = mailbox_repo::file_outgoing(&mut tx, subject, mailbox_repo::INDEX, &name, &sha256, &body, &now()).await;
		tx.commit().await.map_err(Failure::Storage)?;
		change
	};
	match filed {
		Ok(Change::Added | Change::Unchanged) => {}
		Ok(Change::NoAccount) => {
			outcome.no_account = true;
			return Ok(());
		}
		Err(MailboxError::Rewritten) => {
			outcome.refused.push("rewritten");
			return Ok(());
		}
		Err(MailboxError::Storage(err)) => return Err(Failure::Storage(err)),
	}
	let file = mailbox_repo::Outgoing {
		folder: String::from(mailbox_repo::INDEX),
		name,
		body,
	};
	match super::pass::hand_one(db, mailbox, subject, deletion, now, file).await? {
		Handed::Answered(answer) => outcome.indexed.push(answer.as_str()),
		Handed::NotOurs => {}
		Handed::NoAccount => outcome.no_account = true,
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	//! The computation from fixture rows, and its cases ported from
	//! paulgsc/some-ui `packages/job-desk/src/__tests__/queue.test.ts`.

	use super::{jobs_day, jobs_index, stamp_of};
	use mailbox_repo::{HeldFile, HeldTranscript, JobsInputs};
	use serde_json::{json, Value};

	fn pick(id: &str, track: &str) -> Value {
		json!({
			"id": id,
			"fingerprint": String::from("acme|") + id + "|remote",
			"track": track,
			"family": if track == "adjacent" { json!("data") } else { Value::Null },
			"posting": {
				"title": String::from("Role ") + id,
				"company": "Acme",
				"location": "Remote",
				"compensation": null,
				"experience": null,
				"postedOn": null,
				"applyUrl": String::from("https://jobs.lever.co/acme/") + id
			}
		})
	}

	/// A digest for `day` picking `picks` (`(id, track)`), reading `read`.
	fn digest(day: &str, picks: &[(&str, &str)], read: &[&str]) -> Value {
		json!({
			"schema": "some-ui.job-digest/1",
			"day": day,
			"createdAt": String::from(day) + "T15:00:00Z",
			"seen": 7,
			"picks": picks.iter().map(|(id, track)| pick(id, track)).collect::<Vec<_>>(),
			"setAside": [],
			"scouts": [],
			"queue": { "open": [], "lapsed": [] },
			"read": read,
			"notesDone": [],
			"evidence": { "someUi": "abcdef0", "server": "abcdef0" }
		})
	}

	/// A decisions entry for `day`'s pick `pick_id`.
	fn entry(day: &str, pick_id: &str, decision: &str, debriefed: bool) -> Value {
		json!({
			"day": day,
			"pickId": pick_id,
			"fingerprint": String::from("acme|") + pick_id + "|remote",
			"id": String::from("ev-") + pick_id + "-" + decision,
			"decision": decision,
			"reasons": [],
			"note": "",
			"voice": [],
			"debrief": if debriefed { json!({ "id": String::from("ev-debrief-") + pick_id, "minutes": 6, "friction": 2, "frictionChips": [], "what": "", "voice": [], "result": "submitted" }) } else { Value::Null },
			"fit": []
		})
	}

	fn decisions(shared_at: &str, picks: &[Value]) -> Value {
		json!({
			"schema": "some-ui.job-decisions/1",
			"day": &shared_at[..10],
			"digestDay": &shared_at[..10],
			"sharedAt": shared_at,
			"picks": picks,
			"notes": {},
			"disagreements": [],
			"outcomes": [],
			"withdrawals": []
		})
	}

	fn held(name: &str, body: &Value) -> HeldFile {
		HeldFile {
			name: String::from(name),
			body: body.to_string().into_bytes(),
		}
	}

	fn inputs(digests: &[(&str, Value)], decided: &[(&str, Value)]) -> JobsInputs {
		JobsInputs {
			digests: digests.iter().map(|(name, body)| held(name, body)).collect(),
			decisions: decided.iter().map(|(name, body)| held(name, body)).collect(),
			..JobsInputs::default()
		}
	}

	fn index(inputs: &JobsInputs, day: &str) -> Value {
		serde_json::from_slice(&jobs_index(inputs, day, Some("2026-10-18T15:05:00.000Z"))).unwrap()
	}

	fn ids(list: &Value) -> Vec<&str> {
		list.as_array().unwrap().iter().map(|pick| pick["pickId"].as_str().unwrap()).collect()
	}

	/// The sample's shape: three software picks and two adjacent, on 10-18.
	fn sample() -> (&'static str, Value) {
		(
			"digest-2026-10-18.json",
			digest(
				"2026-10-18",
				&[
					("sw-0001", "software"),
					("sw-0002", "software"),
					("sw-0003", "software"),
					("ad-0001", "adjacent"),
					("ad-0002", "adjacent"),
				],
				&[],
			),
		)
	}

	#[test]
	fn the_same_rows_give_the_same_bytes_in_a_fixed_order() {
		let (name, body) = sample();
		let decided = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "passed", false)]);
		let forward = inputs(&[(name, body)], &[("decisions-2026-10-18.json", decided)]);
		let first = jobs_index(&forward, "2026-10-18", Some("2026-10-18T22:05:00.000Z"));
		assert_eq!(first, jobs_index(&forward, "2026-10-18", Some("2026-10-18T22:05:00.000Z")));
		let text = String::from_utf8(first).unwrap();
		let keys: Vec<&str> = [
			"\"schema\"",
			"\"routine\"",
			"\"day\"",
			"\"asOf\"",
			"\"from\"",
			"\"skipped\"",
			"\"queue\"",
			"\"new\"",
			"\"notes\"",
			"\"picked\"",
		]
		.into_iter()
		.collect();
		let positions: Vec<usize> = keys.iter().map(|key| text.find(key).unwrap()).collect();
		assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "keys in their fixed order: {text}");
		let value: Value = serde_json::from_str(&text).unwrap();
		assert_eq!(value["schema"], "some-ui.routine-index/1");
		assert_eq!(value["routine"], "jobs");
		assert_eq!(
			value["from"],
			json!({ "digests": ["digest-2026-10-18.json"], "decisions": ["decisions-2026-10-18.json"], "scout": [] })
		);
		assert_eq!(ids(&value["queue"]["open"]), ["sw-0002", "sw-0003", "ad-0001", "ad-0002"]);
		assert_eq!(value["queue"]["free"], json!({ "software": 1, "adjacent": 0 }));
		assert_eq!(
			value["queue"]["open"][0],
			json!({
				"day": "2026-10-18", "pickId": "sw-0002", "track": "software", "fingerprint": "acme|sw-0002|remote",
				"title": "Role sw-0002", "company": "Acme", "applyUrl": "https://jobs.lever.co/acme/sw-0002"
			})
		);
	}

	#[test]
	fn holds_every_pick_no_shared_file_has_answered() {
		let (name, body) = sample();
		let value = index(&inputs(&[(name, body)], &[]), "2026-10-18");
		assert_eq!(ids(&value["queue"]["open"]), ["sw-0001", "sw-0002", "sw-0003", "ad-0001", "ad-0002"]);
		assert_eq!(value["queue"]["free"], json!({ "software": 0, "adjacent": 0 }));
	}

	#[test]
	fn frees_a_pick_passed_archived_or_debriefed_and_keeps_one_untouched_or_only_taken() {
		let (name, body) = sample();
		let decided = decisions(
			"2026-10-18T22:00:00Z",
			&[
				entry("2026-10-18", "sw-0001", "passed", false),
				entry("2026-10-18", "sw-0002", "archived", false),
				entry("2026-10-18", "sw-0003", "taken", true),
				entry("2026-10-18", "ad-0001", "taken", false),
			],
		);
		let value = index(&inputs(&[(name, body)], &[("decisions-2026-10-18.json", decided)]), "2026-10-18");
		assert_eq!(ids(&value["queue"]["open"]), ["ad-0001", "ad-0002"]);
	}

	#[test]
	fn goes_by_the_newest_share_whatever_order_the_files_come_in() {
		let (name, body) = sample();
		let archived = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "archived", false)]);
		let retaken = decisions("2026-10-19T08:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		for files in [
			[("decisions-2026-10-18.json", archived.clone()), ("decisions-2026-10-19.json", retaken.clone())],
			[("decisions-2026-10-18.json", retaken), ("decisions-2026-10-19.json", archived)],
		] {
			let value = index(&inputs(&[(name, body.clone())], &files), "2026-10-19");
			assert!(ids(&value["queue"]["open"]).contains(&"sw-0001"));
		}
	}

	#[test]
	fn lapses_on_its_digests_day_plus_three_and_not_a_day_before() {
		let (name, body) = sample();
		let held = inputs(&[(name, body)], &[]);
		let before = index(&held, "2026-10-20");
		assert_eq!(before["queue"]["lapsed"], json!([]));
		let after = index(&held, "2026-10-21");
		assert_eq!(after["queue"]["open"], json!([]));
		assert_eq!(ids(&after["queue"]["lapsed"]), ["sw-0001", "sw-0002", "sw-0003", "ad-0001", "ad-0002"]);
		assert_eq!(after["queue"]["free"], json!({ "software": 3, "adjacent": 2 }));
		assert!(after["queue"]["lapsed"][0].get("applyUrl").is_none(), "a lapsed pick is not read again");
	}

	#[test]
	fn lapses_one_reported_untouched_never_one_taken_passed_or_debriefed() {
		let (name, body) = sample();
		let mut untouched = entry("2026-10-18", "sw-0001", "untouched", false);
		untouched["id"] = Value::Null;
		let decided = decisions(
			"2026-10-19T22:00:00Z",
			&[
				untouched,
				entry("2026-10-18", "sw-0002", "taken", false),
				entry("2026-10-18", "sw-0003", "passed", false),
				entry("2026-10-18", "ad-0001", "taken", true),
			],
		);
		let value = index(&inputs(&[(name, body)], &[("decisions-2026-10-19.json", decided)]), "2026-10-25");
		assert_eq!(ids(&value["queue"]["open"]), ["sw-0002"]);
		assert_eq!(ids(&value["queue"]["lapsed"]), ["sw-0001", "ad-0002"]);
	}

	/// A later digest that recorded `pick_id` of 10-18 as lapsed.
	fn recorder(pick_id: &str) -> (&'static str, Value) {
		let mut later = digest("2026-10-21", &[], &[]);
		later["queue"] = json!({ "open": [], "lapsed": [{ "day": "2026-10-18", "pickId": pick_id, "track": "software" }] });
		("digest-2026-10-21.json", later)
	}

	#[test]
	fn opens_a_pick_again_when_a_share_takes_it_after_a_digest_recorded_its_lapse() {
		let late = decisions("2026-10-22T08:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		let value = index(&inputs(&[sample(), recorder("sw-0001")], &[("decisions-2026-10-22.json", late)]), "2026-10-23");
		assert_eq!(ids(&value["queue"]["open"]), ["sw-0001"]);
	}

	#[test]
	fn names_a_lapse_once_not_again_after_a_digest_recorded_it() {
		let value = index(&inputs(&[sample(), recorder("sw-0001")], &[]), "2026-10-22");
		assert_eq!(ids(&value["queue"]["lapsed"]), ["sw-0002", "sw-0003", "ad-0001", "ad-0002"]);
		assert_eq!(value["queue"]["open"], json!([]));
	}

	#[test]
	fn names_each_pick_by_its_own_digests_day_oldest_first() {
		let earlier = ("digest-2026-10-17.json", digest("2026-10-17", &[("sw-0001", "software")], &[]));
		let today = ("digest-2026-10-18.json", digest("2026-10-18", &[("sw-0001", "software")], &[]));
		let decided = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "passed", false)]);
		let value = index(&inputs(&[today, earlier], &[("decisions-2026-10-18.json", decided)]), "2026-10-18");
		let open = value["queue"]["open"].as_array().unwrap();
		assert_eq!(open.len(), 1);
		assert_eq!((open[0]["day"].as_str(), open[0]["track"].as_str()), (Some("2026-10-17"), Some("software")));
	}

	#[test]
	fn whats_new_is_each_file_no_digest_read_with_what_an_earlier_one_already_carried() {
		let first = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "passed", false)]);
		// A share offered again repeats the pass, and adds a take.
		let again = decisions(
			"2026-10-19T07:00:00Z",
			&[entry("2026-10-18", "sw-0001", "passed", false), entry("2026-10-18", "sw-0002", "taken", false)],
		);
		let reader = ("digest-2026-10-19.json", digest("2026-10-19", &[], &["decisions-2026-10-18.json"]));
		let mut rows = inputs(&[sample(), reader], &[("decisions-2026-10-18.json", first), ("decisions-2026-10-19.json", again)]);
		let mut answered = digest("2026-10-20", &[], &[]);
		answered["scouts"] = json!([{ "scoutId": "sc-0001", "verdict": "agree", "note": "Good fit.", "pickId": null, "rule": null, "review": null }]);
		rows.digests.push(held("digest-2026-10-20.json", &answered));
		let scouts = json!({
			"schema": "some-ui.job-scouts/1",
			"sharedAt": "2026-10-20T09:00:00Z",
			"scouts": [
				{ "id": "sc-0001", "url": "https://jobs.lever.co/acme/1", "title": "One", "why": "", "postingText": null, "voice": [], "addedAt": "2026-10-20T09:00:00Z", "track": "software", "family": null },
				{ "id": "sc-0002", "url": "https://jobs.lever.co/acme/2", "title": "Two", "why": "", "postingText": null, "voice": [], "addedAt": "2026-10-20T09:00:00Z", "track": "software", "family": null }
			],
			"notes": {}
		});
		rows.scouts.push(held("scouts-20261020T090000.json", &scouts));
		let value = index(&rows, "2026-10-20");
		assert_eq!(
			value["new"],
			json!({
				"decisions": [{ "name": "decisions-2026-10-19.json", "repeats": ["ev-sw-0001-passed"] }],
				"scout": [{ "name": "scouts-20261020T090000.json", "answered": ["sc-0001"] }]
			})
		);
	}

	#[test]
	fn a_waiting_note_carries_its_id_and_its_newest_transcript_and_a_done_one_is_gone() {
		let mut decided = decisions("2026-10-19T07:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		decided["picks"][0]["voice"] = json!(["decisions-2026-10-19_voice-1.webm", "decisions-2026-10-19_voice-2.webm"]);
		decided["picks"][0]["fit"] = json!([{ "id": "ev-fit", "note": "", "voice": ["decisions-2026-10-19_voice-3.webm"] }]);
		decided["notes"] = json!({ "decisions-2026-10-19_voice-1.webm": "n1", "decisions-2026-10-19_voice-3.webm": "n3" });
		let mut reader = digest("2026-10-19", &[], &[]);
		reader["transcribed"] = json!(["decisions-2026-10-19_voice-2.webm"]);
		reader.as_object_mut().unwrap().remove("notesDone");
		let mut held = inputs(&[sample(), ("digest-2026-10-19.json", reader)], &[("decisions-2026-10-19.json", decided)]);
		held.transcripts = ["transcript-n1-1.json", "transcript-n1-2.json", "transcript-other-1.json"]
			.into_iter()
			.map(|name| HeldTranscript {
				name: String::from(name),
				sha256: String::from("sha-of-") + name,
			})
			.collect();
		let value = index(&held, "2026-10-19");
		assert_eq!(
			value["notes"],
			json!([
				{
					"file": "decisions-2026-10-19_voice-1.webm", "in": "decisions-2026-10-19.json", "note": "n1",
					"transcript": { "name": "transcript-n1-2.json", "sha256": "sha-of-transcript-n1-2.json" }
				},
				{ "file": "decisions-2026-10-19_voice-3.webm", "in": "decisions-2026-10-19.json", "note": "n3", "transcript": null }
			]),
			"voice-2 is done (an older digest's `transcribed`); voice-3 not heard yet"
		);
	}

	#[test]
	fn picked_is_every_fingerprint_a_digest_picked_or_a_file_names_once_each_in_order() {
		let mut decided = decisions("2026-10-19T07:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		decided["outcomes"] = json!([{ "id": "ev-o", "fingerprint": "beta|old|remote", "event": "reply", "on": "2026-10-19", "note": "" }]);
		decided["disagreements"] = json!([{ "id": "ev-d", "fingerprint": "gamma|set aside|remote", "note": "Mine." }]);
		let value = index(&inputs(&[sample()], &[("decisions-2026-10-19.json", decided)]), "2026-10-19");
		assert_eq!(
			value["picked"],
			json!([
				"acme|ad-0001|remote",
				"acme|ad-0002|remote",
				"acme|sw-0001|remote",
				"acme|sw-0002|remote",
				"acme|sw-0003|remote",
				"beta|old|remote",
				"gamma|set aside|remote"
			])
		);
	}

	#[test]
	fn a_file_that_does_not_read_as_its_schema_is_skipped_and_counts_for_nothing() {
		let broken = HeldFile {
			name: String::from("decisions-2026-10-19.json"),
			body: br#"{"schema":"some-ui.job-decisions/1","picks":"no"}"#.to_vec(),
		};
		let mut held = inputs(&[sample()], &[]);
		held.decisions.push(broken);
		held.digests.push(HeldFile {
			name: String::from("digest-2026-10-17.json"),
			body: br#"{"schema":"some-ui.job-digest/1","day":"2026-10-17"}"#.to_vec(),
		});
		let value = index(&held, "2026-10-18");
		assert_eq!(value["skipped"], json!(["decisions-2026-10-19.json", "digest-2026-10-17.json"]));
		assert_eq!(value["from"]["decisions"], json!(["decisions-2026-10-19.json"]), "still named as read");
		assert_eq!(value["new"]["decisions"], json!([]));
		assert_eq!(ids(&value["queue"]["open"]).len(), 5);
	}

	#[test]
	fn an_empty_home_has_an_empty_index() {
		let value: Value = serde_json::from_slice(&jobs_index(&JobsInputs::default(), "2026-10-31", None)).unwrap();
		assert_eq!(
			value,
			json!({
				"schema": "some-ui.routine-index/1", "routine": "jobs", "day": "2026-10-31", "asOf": null,
				"from": { "digests": [], "decisions": [], "scout": [] }, "skipped": [],
				"queue": { "open": [], "lapsed": [], "free": { "software": 3, "adjacent": 2 } },
				"new": { "decisions": [], "scout": [] }, "notes": [], "picked": []
			})
		);
	}

	#[test]
	fn the_day_is_los_angeles_and_the_stamp_is_the_name_the_mailbox_takes() {
		assert_eq!(jobs_day("2026-10-31T06:59:59.999Z").as_deref(), Some("2026-10-30"));
		assert_eq!(jobs_day("2026-10-31T07:00:00.000Z").as_deref(), Some("2026-10-31"));
		assert_eq!(jobs_day("2026-12-01T07:59:00.000Z").as_deref(), Some("2026-11-30"), "PST after the clocks change");
		assert_eq!(stamp_of("2026-10-31T15:00:00.123Z").as_deref(), Some("20261031T150000123Z"));
		assert_eq!(stamp_of("2026-10-31T15:00:00Z"), None);
	}
}
