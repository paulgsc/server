//! Delivering a decision: claim before sending, send each device within the pass's
//! deadline, and hand back a claim nothing was sent for. Split out of `waker` (S9, #489).

use super::deadline::{Deadline, PassClock};
use super::propose::refresh_stale_proposal;
use super::waker::Considered;
use crate::failure::{Failure, Step};
use crate::nudge::payload::NudgePayload;
use crate::{auth::DeletionLock, NudgeContext};
use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use push_kit::SendOutcome;
use push_repo::PushSubscriptionRepository;
use session_repo::SessionRepository;
use sqlx::SqlitePool;
use study_domain::StudyAction;
use tracing::{debug, error, info, warn};

/// What `waker::consider` decided to send, and the recharged charge to claim it
/// with.
pub(super) struct Claim {
	pub(super) action: StudyAction,
	/// When the pass read the subject.
	pub(super) now: DateTime<Utc>,
	pub(super) next_eligible: DateTime<Utc>,
	/// The charge's version as read; the claim is conditional on it.
	pub(super) read_as_of: Option<String>,
	pub(super) levels: Vec<(u16, f64)>,
	pub(super) as_of: DateTime<Utc>,
}

/// Claim the subject, then send: the second half of `waker::consider`.
pub(super) async fn send<C: PassClock>(
	db: &SqlitePool,
	nudge: &NudgeContext,
	engagement: &EngagementRepository,
	subject_id: &str,
	deadline: &Deadline<C>,
	claim: Claim,
) -> Result<Considered, sqlx::Error> {
	let Claim {
		action,
		now,
		next_eligible,
		read_as_of,
		levels,
		as_of,
	} = claim;
	// Disallowed for tracing; this is the stored history column.
	#[allow(clippy::disallowed_methods)]
	let serialized = serde_json::to_string(&action).unwrap_or_default();
	// The recharge is written by the claim itself, version-checked against
	// `read_as_of`: a signal folded in since this pass read the charge means
	// `action` was chosen from deficits that no longer exist, so the claim is
	// refused and nothing stale is sent — see `EngagementRepository::claim`.
	let Some(log_id) = engagement
		.claim(
			subject_id,
			read_as_of.as_deref(),
			&now.to_rfc3339(),
			&next_eligible.to_rfc3339(),
			action.kind(),
			&serialized,
			&levels,
			&as_of.to_rfc3339(),
		)
		.await?
	else {
		info!(subject = %subject_id, "another pass claimed this subject first, or a signal changed their charge while this pass was deciding");
		crate::metrics::waker::record_verdict("claim_lost", "n/a");
		return Ok(Considered::NotSent);
	};

	// #284 (RCM7): refresh only after winning the claim above, not merely on
	// `Verdict::Intervene` — a real `chatgpt-codex-connector` finding on
	// `#335` caught the earlier ordering, where a concurrent pass over the
	// same subject that ultimately *loses* the claim below still ran this
	// refresh beforehand. Because a machine refresh deliberately never moves
	// `updated_at` (`refresh_if_untouched`'s own doc comment), that losing
	// pass's write still lands as "untouched" even after the winning pass
	// has already claimed, saved, and — by the time the losing pass's own
	// catalogue read and write finish — actuated: the recipient can open the
	// proposal the winner just sent while the loser is still silently
	// rewriting it underneath them, for a notification the loser never
	// sends. Only the pass that actually holds `log_id` reaches here, so at
	// most one pass per intervention ever performs this write, immediately
	// before the one send it corresponds to — not one per pass that merely
	// evaluated to `Intervene`.
	if let Some(session_id) = action.session_id() {
		refresh_stale_proposal(db, nudge, &SessionRepository::new(db.clone()), subject_id, session_id, now).await;
	}

	let Delivery {
		accepted,
		timed_out,
		refused,
		tried,
		cut,
	} = actuate(db, nudge, &action, subject_id, deadline).await?;
	if cut && tried == 0 {
		// #575: the deadline came between the claim and the first delivery.
		// Nothing went out, so handing the claim back cannot cause a
		// duplicate, and it is not a verdict: the subject has reached no
		// outcome this pass, like every other one the pass did not reach.
		info!(subject = %subject_id, "waker pass deadline reached before this subject's first delivery; handing the claim back for the next pass");
		engagement.release(subject_id, log_id, &now.to_rfc3339()).await?;
		return Ok(Considered::Deferred);
	}
	if accepted == 0 && timed_out > 0 {
		// #264 (SLI3): a timeout is crash-shaped, and lands on the same side
		// of the claim-before-send line a crash does. Unlike every other
		// failure `actuate` sees, it cannot say whether the push service
		// took the message — a provider that received the request and never
		// answered may still deliver it — so handing the claim back here
		// would let the next pass send a second notification for one
		// intervention. Kept instead: this intervention is spent, the gate
		// already sits at `next_eligible`, and `actuated_at` stays `NULL`
		// because nothing confirmed acceptance.
		warn!(subject = %subject_id, timed_out, "no device confirmed the intervention and at least one delivery timed out; keeping the claim rather than risking a duplicate");
		crate::metrics::waker::record_verdict("delivery_timed_out", "n/a");
		return Ok(Considered::NotSent);
	}
	if accepted == 0 {
		warn!(subject = %subject_id, "no device accepted the intervention; releasing the claim");
		crate::metrics::waker::record_verdict("no_device_accepted", "n/a");
		engagement.release(subject_id, log_id, &now.to_rfc3339()).await?;
		// Handed back, so the next pass meets it again: held open by the
		// pass, never counted here (#1180). With no device tried, nothing
		// refused it.
		return Ok(if tried > 0 { Considered::Unaccepted } else { Considered::NotSent });
	}

	engagement.mark_actuated(log_id, &Utc::now().to_rfc3339()).await?;
	// Spent once it is marked sent: a device that did not take it never
	// will, so each is one count, here (#1180).
	for _ in 0..refused.saturating_add(timed_out) {
		Failure::alone(Step::StudyNudgeDeviceMissed, &"a device did not take an intervention another device accepted");
	}
	info!(subject = %subject_id, action = action.kind(), devices = accepted, "intervened");
	crate::metrics::waker::record_verdict("sent", "n/a");
	Ok(Considered::Sent)
}

/// Run `write` for `subject_id` only if the subject still has its gate row,
/// holding off account deletion between that check and the write
/// (`docs/identity.md` invariant 9). `None` when the subject is gone.
///
/// A pass reads its subjects well before it writes for them, with push
/// deliveries in between, and some of its writes (a provisioned `sessions`
/// draft, a first charge) check nothing that a deletion removes. Holding the
/// lock only here, around one check and one statement, keeps a deletion from
/// ever waiting on a pass's deliveries.
///
/// # Errors
/// The existence check's storage failure.
pub(crate) async fn unless_deleted<T>(
	engagement: &EngagementRepository,
	deletion: &DeletionLock,
	subject_id: &str,
	write: impl std::future::Future<Output = T>,
) -> Result<Option<T>, sqlx::Error> {
	let _hold = deletion.hold().await;
	if engagement.gate(subject_id).await?.is_none() {
		return Ok(None);
	}
	Ok(Some(write.await))
}

/// Write the waker's verdict for a subject unless a signal folded in since it
/// read their charge (#360).
///
/// If one did, that signal has already written a newer charge and re-solved
/// `eligible_at` from it, and this pass's verdict — computed from the older
/// read — is the stale one: dropping it leaves the subject exactly where the
/// signal put them, to be reconsidered on a later pass. The intervention path
/// does not come through here: its recharge is written by the version-checked
/// claim itself (`EngagementRepository::claim`), so a signal that lands first
/// refuses the claim rather than letting a stale action be sent.
pub(super) async fn save_unless_superseded(
	deletion: &DeletionLock,
	engagement: &EngagementRepository,
	subject_id: &str,
	read_as_of: Option<&str>,
	levels: &[(u16, f64)],
	as_of: DateTime<Utc>,
	eligible_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
	// A subject with no charge yet (`read_as_of` `None`) has nothing for the
	// version check to find missing, so a deletion is checked for directly.
	let saved = unless_deleted(
		engagement,
		deletion,
		subject_id,
		engagement.save_if_unchanged(subject_id, read_as_of, levels, &as_of.to_rfc3339(), &eligible_at.to_rfc3339()),
	)
	.await?;
	if !saved.transpose()?.unwrap_or(true) {
		// Not a verdict of its own: the caller has already recorded this
		// subject's terminal outcome for the pass, and the breakdown counts
		// exactly one per subject.
		debug!(subject = %subject_id, "a signal folded in while this pass was deciding; keeping its charge and eligibility rather than this pass's");
	}
	Ok(())
}

/// Put an action on the wire, to every device that consented to its topic.
///
/// Returns how many were **accepted** — which, as `push_kit` is at pains to
/// say, is not how many were delivered — and how many timed out.
///
/// Each delivery is bounded by `NudgeContext::delivery_timeout`, or by what
/// is left of the pass's `deadline` if that is sooner (#264, SLI3). A
/// provider that has not answered by then is recorded exactly like any other
/// non-acceptance — `record_failure`, never a prune, never `Accepted` — but
/// counted separately, because what the caller may do next differs: see
/// `consider`. A device reached after the deadline is not tried and not
/// recorded against: nothing was sent to it, so there is nothing ambiguous to
/// protect and nothing to count as its failure.
pub(super) async fn actuate<C: PassClock>(
	db: &SqlitePool,
	nudge: &NudgeContext,
	action: &StudyAction,
	subject_id: &str,
	deadline: &Deadline<C>,
) -> Result<Delivery, sqlx::Error> {
	let subscriptions_repo = PushSubscriptionRepository::new(db.clone());
	let subscriptions = subscriptions_repo.for_subject(subject_id).await?;

	let payload = NudgePayload::for_action(&nudge.base_url, action);
	let encoded = match payload.to_bytes() {
		Ok(bytes) => bytes,
		Err(err) => {
			error!(error = %err, "could not serialize the notification payload");
			return Ok(Delivery::default());
		}
	};

	let topic = crate::nudge::payload::topic_for(action);
	let mut delivery = Delivery::default();

	for stored in subscriptions {
		if !stored.accepts(topic) {
			continue;
		}

		let endpoint = &stored.subscription.endpoint;
		// What a log line may name: a recognised push service or `other`, never
		// the endpoint or its host, which a client chooses (docs/identity.md).
		let service = stored.subscription.service();
		let budget = nudge.delivery_timeout.min(deadline.left());
		if budget.is_zero() {
			warn!(subject = %subject_id, "waker pass deadline reached; not trying this subject's remaining devices");
			delivery.cut = true;
			break;
		}
		delivery.tried += 1;
		let Some(outcome) = deadline.clock.timeout(budget, nudge.sender.deliver(&stored.subscription, &encoded)).await else {
			warn!(service, timeout_ms = budget.as_millis(), "push service did not answer in time");
			subscriptions_repo.record_failure(endpoint, &Utc::now().to_rfc3339()).await?;
			delivery.timed_out += 1;
			continue;
		};
		let stamp = Utc::now().to_rfc3339();

		if outcome.should_prune() {
			info!(service, "subscription is gone; pruning");
			subscriptions_repo.delete_by_endpoint(endpoint).await?;
			continue;
		}

		if outcome.is_failure() {
			warn!(service, outcome = outcome.label(), detail = %outcome.log_detail(), "push was not accepted");
			subscriptions_repo.record_failure(endpoint, &stamp).await?;
			delivery.refused += 1;
			continue;
		}

		delivery.accepted += 1;
		// Deliberately not "delivered": the push service accepted it, and
		// whether anyone ever sees it is not observable from here.
		debug_assert_eq!(outcome, SendOutcome::Accepted);
		subscriptions_repo.record_success(endpoint, &stamp).await?;
	}

	Ok(delivery)
}

/// What [`actuate`] reports back for one intervention.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Delivery {
	/// Push services that answered `Accepted`.
	accepted: usize,
	/// Push services that did not answer within their budget — the delivery
	/// timeout, or what was left of the pass.
	timed_out: usize,
	/// Push services that answered, and did not accept it (and the
	/// subscription is not gone).
	refused: usize,
	/// Deliveries begun, whatever came of them.
	tried: usize,
	/// Whether the pass deadline stopped the devices before the last was
	/// tried. With `tried` zero, it came before the first (#575).
	cut: bool,
}
