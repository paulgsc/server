//! Signals in: fold an engagement signal into a subject's charge and solve when it is next
//! due, and a subject's first contact. Split out of `waker` (S9, #489).

use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use intervention::Charge;
use push_repo::PushSubscriptionRepository;
use sqlx::SqlitePool;
use study_domain::{StudyCalibration, StudySignal, StudyV1};
use tracing::{debug, error, info};

/// Fold a signal into a subject's charge and re-solve their eligibility.
///
/// This is the *only* place work is created. Everything the waker later does
/// was decided here, by arithmetic, at the moment something actually happened.
///
/// # Errors
/// Propagates any storage failure.
pub async fn observe(db: &SqlitePool, subject_id: &str, signal: &study_domain::StudySignal) -> Result<chrono::DateTime<Utc>, sqlx::Error> {
	let now = Utc::now();

	// One write transaction for the read, the fold, and the save — see
	// `EngagementRepository::fold`. Two signals for the same subject arriving
	// together (a poor score from `POST /outcomes` beside a
	// `session-completed` from `/signals`, say) used to be able to read the
	// same stored charge and have the second save erase the first; a real
	// `chatgpt-codex-connector` finding on #360.
	let eligible_at = EngagementRepository::new(db.clone()).fold(subject_id, |stored| fold_signal(stored, signal, now)).await?;

	debug!(subject = %subject_id, signal = signal.kind(), eligible_at = %eligible_at, "signal folded in");
	Ok(eligible_at)
}

/// Apply `signal` to a stored charge and re-solve eligibility — the body of
/// every charge fold, whether a signal arriving ([`observe`]) or a subject
/// catching up to new material ([`super::sweep::catch_up`]). Returns what
/// `EngagementRepository::fold`/`catch_up` store, plus the solved `eligible_at`.
pub(super) fn fold_signal(stored: &[engagement_repo::ChargeRow], signal: &StudySignal, now: DateTime<Utc>) -> (Vec<(u16, f64)>, String, String, DateTime<Utc>) {
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	let levels: Vec<(u16, f64)> = stored.iter().map(|row| (row.class as u16, row.level)).collect();
	let as_of = stored.first().map_or(now, |row| crate::nudge::clock::parse_timestamp(&row.as_of).unwrap_or(now));

	// No rows means never seen, and `from_storage` starts such a subject
	// **full** rather than empty — an empty charge is instantly eligible, so
	// the alternative would nudge a brand-new account before it did anything.
	let mut charge = Charge::<StudyV1>::from_storage::<StudyCalibration>(&levels, as_of);

	charge.apply::<StudyCalibration>(signal, now);
	let eligible_at = charge.eligible_at::<StudyCalibration>(now);

	let (levels, stamp) = charge.to_storage();
	(levels, stamp.to_rfc3339(), eligible_at.to_rfc3339(), eligible_at)
}

/// First contact: give a subject who has never been observed a gate row,
/// seeded full, so `due` can eventually find them without a signal ever
/// having arrived.
///
/// Before this, the chain was: no session created ⇒ no signal ⇒ no gate row
/// ⇒ never in `due` ⇒ never nudged. Not late — never. The only caller is
/// `POST /push/subscriptions`: it is a person explicitly saying "you may
/// interrupt me", the strongest statement of intent this deployment has, and
/// (unlike a page load, which is too broad, or a dedicated "hello" route,
/// which would just re-say what this one already implies) it happens exactly
/// once per device before any study behaviour exists. The alternative of
/// seeding lazily when the waker runs is not available at all: the waker can
/// only see rows that already exist, which is the circularity this function
/// closes.
///
/// Seeded **full**, by the same `Charge::full`/`eligible_at` arithmetic
/// `observe` falls back to for an unseen subject — see the comment there.
/// Starting empty would make a brand-new account instantly eligible; someone
/// who installs the app at 9am must not be interrupted at 9:05.
///
/// Idempotent: [`EngagementRepository::seed_if_absent`] is a no-op for a
/// subject who already has a row, whether from a prior signal or from a
/// previous device subscribing.
///
/// Returns whether this call actually seeded the row, so [`backfill_first_contact`]
/// can report how much of its reconciliation pass was real work.
///
/// # Errors
/// Propagates any storage failure.
pub async fn first_contact(db: &SqlitePool, subject_id: &str) -> Result<bool, sqlx::Error> {
	let engagement = EngagementRepository::new(db.clone());
	let now = Utc::now();

	let charge = Charge::<StudyV1>::full::<StudyCalibration>(now);
	let eligible_at = charge.eligible_at::<StudyCalibration>(now);
	let (levels, as_of) = charge.to_storage();

	let created = engagement.seed_if_absent(subject_id, &levels, &as_of.to_rfc3339(), &eligible_at.to_rfc3339()).await?;

	if created {
		debug!(subject = %subject_id, eligible_at = %eligible_at, "first contact: gate row seeded full");
	}

	Ok(created)
}

/// Reconcile pre-existing subscriptions against `engagement_gate`, once, at
/// boot.
///
/// `first_contact` only runs on a live `POST /push/subscriptions` call, and a
/// browser that already holds a subscription from before this landed has no
/// reason to send it again — the Push API does not re-announce an unchanged
/// subscription on its own. Without this pass, every subject who subscribed
/// before this release stays exactly the silence #1 victim this feature
/// exists to fix: a `push_subscriptions` row with no path into
/// `engagement_gate`, forever, unless they happen to unsubscribe and
/// resubscribe.
///
/// Run once at startup rather than folded into the waker's per-tick loop: this
/// is a one-time reconciliation against rows that predate the fix, not
/// ongoing work, and `first_contact`'s own idempotency makes repeating it on
/// every restart a cheap no-op rather than a hazard.
///
/// # Errors
/// Propagates a failure to read `push_subscriptions` itself. A failure to
/// seed one particular subject is logged and skipped rather than aborting the
/// pass — one bad row must not leave everyone else ungated, and the next boot
/// tries again.
pub async fn backfill_first_contact(db: &SqlitePool) -> Result<usize, sqlx::Error> {
	let subject_ids = PushSubscriptionRepository::new(db.clone()).distinct_subject_ids().await?;

	let mut seeded = 0;
	for subject_id in subject_ids {
		match first_contact(db, &subject_id).await {
			Ok(true) => seeded += 1,
			Ok(false) => {}
			Err(err) => error!(subject = %subject_id, error = %err, "could not backfill a gate row for an existing subscription; will retry next boot"),
		}
	}

	if seeded > 0 {
		info!(seeded, "backfilled gate rows for subscriptions that predate first contact");
	}

	Ok(seeded)
}
