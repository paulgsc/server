//! Home's bookkeeping for the Jobs routine, put in the mailbox's `index`
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
//! digest home kept from `brief`, every decisions file, scouts file and Jobs
//! note's transcript the mailbox took from home as home filed it, and every
//! decisions or scouts file only Drive held that the routine put back in `brief` once
//! (the backfill, below). No new table: those are home's own rows.
//!
//! **A view, not a record** (owner, 2026-10-31, paulgsc/server#509). Home
//! recomputes the index from records it keeps, so it keeps no copy of it:
//! nothing in `mailbox_outgoing`, `mailbox_handed` or `mailbox_archive`. The
//! mailbox holds only the latest per routine and drops the one before in
//! the same write, archiving never walks the folder
//! ([`super::pass::ARCHIVES`]), and the routine cannot ask for an old one.
//! What home remembers is in memory, [`LastIndex`]: what the last index the
//! mailbox answered was computed from, so a restart costs one put.
//!
//! **The backfill** (MB2's amendment, the same decision, widened by
//! paulgsc/some-ui#2393). Drive holds Jobs decisions and scouts files home
//! never saw, from before the phone filed every one home. The routine puts
//! each once in `brief`, under its Drive name, which the mailbox takes only
//! for a name dated before 2026-11-04 (`DRIVE_ONLY_ENDED`), so the set is
//! closed. Home keeps it as any `brief` file, and the
//! index counts it with the phone's files and names it in `from.backfill`.
//! It adds no listing: `brief` is read every pass anyway.
//!
//! **Deterministic.** [`jobs_index`] is a pure function of those rows, the
//! day and `asOf`: the same rows give the same bytes, and on any day from
//! `day` through `validThrough` they give the same bytes except `day`. Every
//! list is in a fixed order (by name, or by digest day as the queue is), and
//! the body is written from structs whose fields serialise in declaration
//! order.
//!
//! **Put only when it moves.** A pass first reads when home last took in
//! anything the index counts ([`mailbox_repo::jobs_as_of`]). When that is
//! what the last index the mailbox answered was computed from ([`LastIndex`])
//! and today, in [`JOBS_ZONE`], is no later than that index's `validThrough`,
//! nothing is computed or put: the inputs are append-only and a name holds
//! one body, so nothing it counts can have changed, and the only part of the
//! index the day moves is the queue, which moves only when an open pick no
//! decision answers lapses. `validThrough` is the day before the first such
//! lapse, or `null` when no open pick can lapse. Otherwise it computes and
//! puts a new name, `index-jobs-<stamp>.json`: once each time a digest, a
//! file or a Jobs transcript arrives, and once on a day a pick lapses. A day
//! with neither puts none; `day` in the body is the day it was computed, and
//! the routine reads it as standing through `validThrough`. Every answer is
//! final (a refusal is not put again until what it counts moves); only a failure
//! to reach the mailbox leaves the next pass to try. A body over the
//! mailbox's ceiling is not put, and counted as refused (`too_big`).
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
//!
//! **The reflections routine's index** is [`reflections`]'s, built the same
//! way: a pure function of home's rows and the moment, put only when what it
//! counts moves or a moment it carries (`validUntil`) passes.

use chrono::{DateTime, NaiveDate, Utc};
use mailbox_kit::{Folder, Mailbox, MailboxError, MailboxTransport, Put};
use mailbox_repo::{HeldFile, HeldTranscript, JobsInputs};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use super::pass::{Failure, Outcome, MAILBOX_BODY_CEILING};

pub mod reflections;

pub(super) use reflections::put_reflections_index;

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
	/// Every decisions file counted, the phone's and the backfill's.
	decisions: Vec<String>,
	/// Every scouts file counted, likewise.
	scout: Vec<String>,
	/// Those of `decisions` and `scout` that came from the routine's one-time
	/// backfill of files only Drive held (`brief`), not from the phone.
	backfill: Vec<String>,
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
	/// The last day this index's queue holds with nothing new arriving: the
	/// day before the first open pick would lapse, or `None` when none can.
	#[serde(rename = "validThrough")]
	valid_through: Option<String>,
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

/// What [`queue_of`] gives on a day.
struct QueueOn {
	open: Vec<OpenPick>,
	lapsed: Vec<LapsedPick>,
	/// The last day `open` and `lapsed` stay as they are with nothing new
	/// arriving: the day before the first open pick no decision answers
	/// lapses, or `None` when no open pick can lapse (none, or each taken
	/// and waiting on a debrief). Only a lapse moves the queue with the day.
	valid_through: Option<String>,
}

/// The picks of `digests` still open on what `decisions` say, and those that
/// lapse on `today`, oldest digest first: `queueOf`. The newest share's entry
/// for a pick stands; files shared at the same moment keep their order.
fn queue_of(digests: &[&DigestFile], decisions: &[&DecisionsFile], today: &str) -> QueueOn {
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
	let mut valid_through: Option<String> = None;
	for digest in by_day {
		for pick in &digest.picks {
			let key = (digest.day.as_str(), pick.id.as_str());
			let stands_now = stands.get(&key).copied();
			let can_lapse = stands_now.is_none_or(|pick| pick.decision == "untouched");
			let standing_now = standing(stands_now, &digest.day, today);
			if standing_now == Standing::Open && can_lapse {
				// Open and untouched: it lapses on its digest's day plus
				// `LAPSE_DAYS`, so the queue holds through the day before.
				let last = LAPSE_DAYS.checked_sub(1).and_then(|days| add_days(&digest.day, days));
				if let Some(last) = last {
					if valid_through.as_ref().is_none_or(|held| last < *held) {
						valid_through = Some(last);
					}
				}
			}
			match standing_now {
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
	QueueOn { open, lapsed, valid_through }
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
		// A file names a note by its Drive file name and maps it to the note's
		// id in `notes`, or names the note by its id directly, which is what
		// `PUT /jobs/files/:name` takes once the phone sends voice home. A
		// direct id is known only once home holds a transcript under it; until
		// then `note` stays null, and the transcript's arrival moves `asOf`, so
		// the next index carries both.
		let id = ids.get(voice).cloned().or_else(|| newest.contains_key(voice).then(|| String::from(voice)));
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

/// `phone`'s files and those of `backfill` it lacks a name of, by name, and
/// the names taken from `backfill`. The phone's copy of a name wins: it is
/// the one the mailbox's own folder holds.
fn with_backfill(phone: &[HeldFile], backfill: &[HeldFile]) -> (Vec<HeldFile>, Vec<String>) {
	let held: HashSet<&str> = phone.iter().map(|file| file.name.as_str()).collect();
	let added: Vec<&HeldFile> = backfill.iter().filter(|file| !held.contains(file.name.as_str())).collect();
	let names = added.iter().map(|file| file.name.clone()).collect();
	let mut all: Vec<HeldFile> = phone.iter().chain(added).cloned().collect();
	all.sort_by(|a, b| a.name.cmp(&b.name));
	(all, names)
}

/// The Jobs index for `day`, from `inputs`, stamped `as_of`: its bytes.
///
/// Pure: the same inputs give the same bytes (the module doc).
#[must_use]
pub fn jobs_index(inputs: &JobsInputs, day: &str, as_of: Option<&str>) -> Vec<u8> {
	computed_index(inputs, day, as_of).0
}

/// [`jobs_index`]'s bytes, and its `validThrough`.
fn computed_index(inputs: &JobsInputs, day: &str, as_of: Option<&str>) -> (Vec<u8>, Option<String>) {
	let (decisions, mut backfill) = with_backfill(&inputs.decisions, &inputs.backfill_decisions);
	let (scouts, backfill_scouts) = with_backfill(&inputs.scouts, &inputs.backfill_scouts);
	backfill.extend(backfill_scouts);
	backfill.sort();
	let inputs = &JobsInputs {
		digests: inputs.digests.clone(),
		decisions,
		scouts,
		transcripts: inputs.transcripts.clone(),
		..JobsInputs::default()
	};
	let read = read_all(inputs);
	let digests: Vec<&DigestFile> = read.digests.iter().map(|(_, digest)| digest).collect();
	let decisions: Vec<&DecisionsFile> = read.decisions.iter().map(|(_, file)| file).collect();
	let QueueOn { open, lapsed, valid_through } = queue_of(&digests, &decisions, day);
	let free = free_slots(&open);
	let names = |files: &[HeldFile]| files.iter().map(|file| file.name.clone()).collect();
	let index = JobsIndex {
		schema: INDEX_SCHEMA,
		routine: JOBS,
		day,
		valid_through: valid_through.clone(),
		as_of,
		from: From {
			digests: names(&inputs.digests),
			decisions: names(&inputs.decisions),
			scout: names(&inputs.scouts),
			backfill,
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
	(body, valid_through)
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

/// What an index was computed from, and how long it holds: whose, `asOf`,
/// and the last day (`validThrough`) or the moment (`validUntil`) it says
/// the same.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Computed {
	subject: String,
	as_of: Option<String>,
	valid: Option<String>,
}

impl Computed {
	/// Whether a fresh index for `subject`, with home's newest arrival at
	/// `as_of`, would say what this one says (its `day` aside): the same
	/// subject, nothing arrived since, and `within` its `valid` (`None` is
	/// valid for good).
	fn holds(&self, subject: &str, as_of: Option<&str>, within: impl FnOnce(&str) -> bool) -> bool {
		self.subject == subject && self.as_of.as_deref() == as_of && self.valid.as_deref().is_none_or(within)
	}
}

/// What the last index of one routine the mailbox answered was computed from.
///
/// It is kept in memory for as long as the poller runs, never in the
/// database: an index is a view, and home keeps no copy of it (the module
/// doc). After a restart it is empty, so the first pass puts one again, and
/// the mailbox drops the one before it.
#[derive(Debug, Default)]
pub struct LastIndex(Mutex<Option<Computed>>);

impl LastIndex {
	fn holds(&self, subject: &str, as_of: Option<&str>, within: impl FnOnce(&str) -> bool) -> bool {
		self
			.0
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.as_ref()
			.is_some_and(|last| last.holds(subject, as_of, within))
	}

	fn remember(&self, computed: Computed) {
		*self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(computed);
	}
}

/// Home's memory of each routine's last index: one [`LastIndex`] each.
#[derive(Debug, Default)]
pub struct LastIndexes {
	pub jobs: LastIndex,
	pub reflections: LastIndex,
}

/// The name an index for `routine`, put at `stamp` (`YYYYMMDDTHHMMSSmmmZ`),
/// goes under in the mailbox.
fn index_name(routine: &str, stamp: &str) -> String {
	String::from("index-") + routine + "-" + stamp + ".json"
}

/// Computes `subject`'s Jobs index and puts it, when what it counts has moved
/// since the last one the mailbox answered (the module doc). It writes
/// nothing to the database, so it takes no deletion hold.
///
/// # Errors
/// A [`Failure`] when the database fails or the mailbox cannot be reached;
/// the next pass computes and puts it again.
pub(super) async fn put_jobs_index<T: MailboxTransport>(
	db: &SqlitePool,
	mailbox: &Mailbox<T>,
	subject: &str,
	last: &LastIndex,
	now: &(impl Fn() -> String + Sync),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	let at = now();
	let (Some(day), Some(stamp)) = (jobs_day(&at), stamp_of(&at)) else {
		return Ok(());
	};
	let (as_of, inputs) = {
		let mut tx = db.begin().await.map_err(Failure::Storage)?;
		let as_of = mailbox_repo::jobs_as_of(&mut tx, subject).await.map_err(super::pass::storage)?;
		// Nothing to count yet: no index, and the routine reads Drive.
		if as_of.is_none() {
			return Ok(());
		}
		// A new day alone is no reason: the last index still says what a
		// fresh one would, `day` aside, until an open pick lapses.
		if last.holds(subject, as_of.as_deref(), |through| day.as_str() <= through) {
			return Ok(());
		}
		let inputs = mailbox_repo::jobs_inputs(&mut tx, subject).await.map_err(super::pass::storage)?;
		tx.commit().await.map_err(Failure::Storage)?;
		(as_of, inputs)
	};
	let (body, valid_through) = computed_index(&inputs, &day, as_of.as_deref());
	let computed = Computed {
		subject: subject.to_owned(),
		as_of,
		valid: valid_through,
	};
	put_index(mailbox, &index_name(JOBS, &stamp), body, (last, computed), outcome).await
}

/// Puts one routine's index under `name`, unless it is over the mailbox's
/// ceiling, and remembers what it was computed from once the mailbox has
/// answered (a refusal is final too).
///
/// # Errors
/// A [`Failure`] when the mailbox cannot be reached; nothing is remembered,
/// so the next pass computes and puts it again.
async fn put_index<T: MailboxTransport>(
	mailbox: &Mailbox<T>,
	name: &str,
	body: Vec<u8>,
	(last, computed): (&LastIndex, Computed),
	outcome: &mut Outcome,
) -> Result<(), Failure> {
	if body.len() > MAILBOX_BODY_CEILING {
		outcome.refused.push("too_big");
		last.remember(computed);
		return Ok(());
	}
	let answer = match mailbox.put(Folder::Index, name, body).await {
		Ok(Put::Added { .. }) => "added",
		Ok(Put::Unchanged { .. }) => "unchanged",
		Ok(Put::Conflict) => "conflict",
		Err(MailboxError::Refused(_) | MailboxError::TooBig | MailboxError::BadName) => "refused",
		Err(err) => return Err(Failure::Mailbox(err)),
	};
	outcome.indexed.push(answer);
	last.remember(computed);
	Ok(())
}

#[cfg(test)]
mod tests {
	//! The computation from fixture rows, and its cases ported from
	//! paulgsc/some-ui `packages/job-desk/src/__tests__/queue.test.ts`.

	use super::{index_name, jobs_day, jobs_index, stamp_of};
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
			"\"validThrough\"",
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
			json!({ "digests": ["digest-2026-10-18.json"], "decisions": ["decisions-2026-10-18.json"], "scout": [], "backfill": [] })
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
	fn a_note_named_by_its_id_carries_its_transcript_once_home_holds_one() {
		let mut decided = decisions("2026-10-19T07:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		decided["picks"][0]["voice"] = json!(["0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f", "a1b2c3d4-0000-4000-8000-000000000001"]);
		let mut held = inputs(&[sample()], &[("decisions-2026-10-19.json", decided)]);
		held.transcripts = vec![HeldTranscript {
			name: String::from("transcript-0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f-1.json"),
			sha256: String::from("sha-1"),
		}];
		let value = index(&held, "2026-10-19");
		assert_eq!(
			value["notes"],
			json!([
				{
					"file": "0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f", "in": "decisions-2026-10-19.json", "note": "0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f",
					"transcript": { "name": "transcript-0b6f0f8e-1f2a-4c55-9d7e-3a1b2c3d4e5f-1.json", "sha256": "sha-1" }
				},
				{ "file": "a1b2c3d4-0000-4000-8000-000000000001", "in": "decisions-2026-10-19.json", "note": null, "transcript": null }
			]),
			"a direct id resolves through home's transcript; one not heard yet waits with no id"
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
				"schema": "some-ui.routine-index/1", "routine": "jobs", "day": "2026-10-31", "validThrough": null, "asOf": null,
				"from": { "digests": [], "decisions": [], "scout": [], "backfill": [] }, "skipped": [],
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
		assert_eq!(index_name("jobs", "20261031T150000123Z"), "index-jobs-20261031T150000123Z.json");
	}

	/// The queue moves with the day only when an open pick no decision
	/// answers lapses, so the index holds through the day before the first
	/// such lapse; a pick taken and waiting on its debrief never lapses.
	#[test]
	fn valid_through_is_the_day_before_the_first_open_pick_lapses() {
		let early = ("digest-2026-10-18.json", digest("2026-10-18", &[("sw-0001", "software")], &[]));
		let late = ("digest-2026-10-20.json", digest("2026-10-20", &[("sw-0002", "software")], &[]));
		let held = inputs(&[early.clone(), late.clone()], &[]);
		assert_eq!(index(&held, "2026-10-18")["validThrough"], "2026-10-20", "10-18's pick lapses on 10-21");
		let lapsed = index(&held, "2026-10-21");
		assert_eq!(ids(&lapsed["queue"]["lapsed"]), ["sw-0001"]);
		assert_eq!(lapsed["validThrough"], "2026-10-22", "now 10-20's, which lapses on 10-23");
		// Each day through validThrough gives the same index but its `day`.
		let through = |day: &str| {
			let mut value = index(&held, day);
			value["day"] = Value::Null;
			value
		};
		assert_eq!(through("2026-10-21"), through("2026-10-22"));
		assert_ne!(through("2026-10-22"), through("2026-10-23"));

		let taken = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "taken", false)]);
		let waiting = inputs(std::slice::from_ref(&early), &[("decisions-2026-10-18.json", taken)]);
		assert_eq!(ids(&index(&waiting, "2026-10-30")["queue"]["open"]), ["sw-0001"]);
		assert_eq!(index(&waiting, "2026-10-18")["validThrough"], Value::Null, "taken, waiting on a debrief: never lapses");
		let passed = decisions("2026-10-18T22:00:00Z", &[entry("2026-10-18", "sw-0001", "passed", false)]);
		let done = inputs(&[early], &[("decisions-2026-10-18.json", passed)]);
		assert_eq!(index(&done, "2026-10-18")["validThrough"], Value::Null, "no open pick");
	}

	/// MB2's backfill (owner, #509): a file only Drive held that the routine put back is
	/// counted as the phone's would be, named in `from.backfill`, and loses to
	/// the phone's copy of the same name.
	#[test]
	fn a_backfilled_file_counts_as_the_phones_and_says_where_it_came_from() {
		let decided = |name: &str| {
			held(
				name,
				&json!({ "schema": "some-ui.job-decisions/1", "sharedAt": "2026-10-20T14:00:00Z", "picks": [], "notes": {}, "disagreements": [], "outcomes": [] }),
			)
		};
		let scouts = held("scouts-20261021T101500.json", &json!({ "schema": "some-ui.job-scouts/1", "scouts": [] }));
		let mut held_inputs = inputs(&[], &[]);
		held_inputs.decisions.push(decided("decisions-2026-10-21.json"));
		held_inputs.backfill_decisions = vec![decided("decisions-2026-10-20.json"), decided("decisions-2026-10-21.json")];
		held_inputs.backfill_scouts = vec![scouts];
		let value = index(&held_inputs, "2026-10-31");
		assert_eq!(value["from"]["decisions"], json!(["decisions-2026-10-20.json", "decisions-2026-10-21.json"]));
		assert_eq!(value["from"]["scout"], json!(["scouts-20261021T101500.json"]));
		assert_eq!(
			value["from"]["backfill"],
			json!(["decisions-2026-10-20.json", "scouts-20261021T101500.json"]),
			"the phone's copy of a name is not the backfill's"
		);
		assert_eq!(
			value["new"]["decisions"],
			json!([{ "name": "decisions-2026-10-20.json", "repeats": [] }, { "name": "decisions-2026-10-21.json", "repeats": [] }])
		);
	}
}
