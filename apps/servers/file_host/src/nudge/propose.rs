//! What the waker proposes for a due subject with nothing prepared: the ranking inputs, a
//! provisioned session, and the refresh of a stale one. Split out of `waker` (S9, #489).

use super::actuate::unless_deleted;
use crate::nudge::constraints::{StudyConstraints, Suppressed};
use crate::NudgeContext;
use activity_repo::{
	default_session_name, provision, recommend, total_duration_ms, ActivityHistory, ActivityOutcome, ActivityRecord, ActivityRepository, DEFAULT_RECOMMENDATION_COUNT,
};
use chrono::{DateTime, Utc};
use engagement_repo::EngagementRepository;
use intervention::{Admissibility, Calibration, Charge, Selector, Verdict};
use outcome_repo::{ActivityStats, OutcomeRepository};
use session_repo::{new_session_id, LayoutMode, SessionOrigin, SessionRecord, SessionRepository, SessionStatus};
use sqlx::SqlitePool;
use std::collections::HashSet;
use study_domain::{StudyAction, StudyCalibration, StudySelector, StudyV1};
use tracing::{error, warn};

/// What `recommend()` ranks a subject's proposal with, beyond the catalogue
/// (#289, TEL4).
///
/// `Default` is the cold-start input every provisioned session used before
/// `activity_outcome` existed: no history, so every activity reads as never
/// played. It is still what every subject gets unless
/// `NudgeContext::recommender_uses_outcomes` is on — and what a subject with
/// no outcomes gets even when it is, which is the "zero-data path is
/// unchanged" property #289 requires.
#[derive(Debug, Clone, Default)]
pub struct RankingInputs {
	pub history: Vec<ActivityHistory>,
	pub last_session_at: Option<DateTime<Utc>>,
}

impl RankingInputs {
	/// Turn one subject's per-activity stats into the recommender's history.
	///
	/// - never played (only skipped, or absent) → no entry: still new on
	///   axis 1, full lift on axis 2;
	/// - an assessed mean → `Completed { score: mean }`, so a poorly-scored
	///   activity lifts and a well-scored one does not;
	/// - otherwise, abandoned and never completed → `Abandoned`;
	/// - otherwise, completed but never assessed → `Unassessed`.
	///
	/// `last_session_at` is the latest play across every activity, which is
	/// what axis 1's "republished since their last session" compares against.
	///
	/// Computed once per decision from one query, so the recommender's seeded
	/// shuffle stays deterministic: the same subject on the same day, with the
	/// same outcomes, gets the same proposal.
	#[must_use]
	pub fn from_stats(stats: &[ActivityStats]) -> Self {
		let history = stats
			.iter()
			.filter(|activity| activity.plays > 0)
			.map(|activity| ActivityHistory {
				activity_id: activity.activity_id.clone(),
				outcome: match activity.mean_score {
					Some(score) => ActivityOutcome::Completed { score },
					None if activity.completed == 0 => ActivityOutcome::Abandoned,
					None => ActivityOutcome::Unassessed,
				},
			})
			.collect();
		let last_session_at = stats
			.iter()
			.filter_map(|activity| activity.last_played_at.as_deref())
			.filter_map(crate::nudge::clock::parse_timestamp)
			.max();
		Self { history, last_session_at }
	}
}

/// This subject's ranking inputs, behind the #289 flag.
///
/// With the flag on, a stats read that fails — a storage error, or a subject
/// whose outcomes span more activities than `STATS_CEILING` — is `None`, and
/// the caller composes nothing rather than falling back to the cold-start
/// input: ranking a subject *with* history as though they had none would
/// persist a proposal that looks informed and is not (a real
/// `chatgpt-codex-connector` finding on #361). With the flag off, the
/// cold-start input is the policy, not a fallback.
pub(super) async fn ranking_inputs(db: &SqlitePool, nudge: &NudgeContext, subject_id: &str) -> Option<RankingInputs> {
	if !nudge.recommender_uses_outcomes {
		return Some(RankingInputs::default());
	}
	match OutcomeRepository::new(db.clone()).stats(subject_id).await {
		Ok(stats) => Some(RankingInputs::from_stats(&stats)),
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read outcome stats; composing nothing rather than ranking this subject as though nothing had been played");
			None
		}
	}
}

/// What one attempt to compose something to point at produced — see
/// [`propose_a_session`].
pub(super) enum Proposal {
	/// A prepared session now exists for this subject: the one this pass
	/// wrote, or a concurrent writer's that `first_prepared` resolved to
	/// instead.
	Prepared(String),
	/// Nothing could be composed this pass. Both fields are what the caller
	/// needs *only if* it has no sessionless action to fall back on; when it
	/// does (plain absence's `GetStarted`), both are correctly ignored,
	/// because the pass's terminal outcome is then that notification rather
	/// than this failure.
	Unavailable {
		/// The `record_verdict` label for this failure, recorded by the
		/// caller so exactly one verdict is counted per pass.
		label: &'static str,
		/// How far to push `eligible_at` out, or `None` to leave it exactly
		/// where it is so the next pass retries this subject immediately.
		retry_in: Option<chrono::Duration>,
	},
}

/// Compose a session for a subject who has nothing prepared, and write it.
///
/// #279 (RCM2)'s decision, written down per its acceptance criteria:
/// provision a session rather than widen the vocabulary. Two other shapes
/// were weighed and rejected:
///
/// - A fifth `StudyAction` variant (`ProposeSession`) breaks
///   `StudyAction::session_id()`'s totality — every existing variant already
///   carries a real id — and forces `payload::topic_for`/
///   `NudgePayload::for_action` to grow a case for "the same message, before
///   a session exists."
/// - A new `Verdict` arm in `intervention` puts "the domain wants something
///   created" into the generic engine, which `intervention`'s own docs are
///   explicit about keeping free of study vocabulary: "every user story adds
///   a variant [to `study_domain`]; none of them should touch
///   `intervention`."
///
/// So `intervention` and `StudySelector` both stay untouched: provisioning
/// happens here, `prepared_session` becomes `Some`, and the *existing*
/// selector maps the same dominant deficit to `LessonReady`/
/// `ResumeAbandoned`/`SuggestReview`/`NewMaterial` exactly as it would for a
/// session that already existed. RCM3 (#280), RCM4 (#281), and RCM5 (#282)
/// are what actually fill it in — see `materialize_provisioned_session`'s own
/// doc comment for what it writes and why.
///
/// **Crash safety without extra bookkeeping.** The write is a `Scheduled` row
/// `SessionRepository::first_prepared` will find on any later pass (RCM5's own
/// choice — see `materialize_provisioned_session`'s doc comment for why
/// `Scheduled` is now safe where RCM2 originally chose `Draft`). A crash
/// between this write and `consider`'s `claim` costs that pass's
/// notification, not a second session — the next pass reads the row this one
/// already wrote and never calls this function at all.
///
/// **Race safety is a separate concern, and needs its own guard.**
/// `consider`'s `first_prepared` read happened at the *top* of the pass, and
/// a concurrent waker pass or the subject's own `POST /sessions` call can
/// create a prepared session in the window between that read and this write.
/// Because the provisioned row's id is always freshly generated, an
/// unconditional `upsert` would not collide with whatever won that race — it
/// would just add a second, blank draft beside it. `provision_if_absent`
/// closes the window atomically (one statement, targeting #284's own partial
/// unique index — `origin = 'system' AND started_at IS NULL`, not the three
/// statuses `first_prepared` treats as prepared) rather than trusting the
/// read that already happened; see its own doc comment for the mechanism and
/// the #313 review that caught the original race. A race-losing write is a
/// no-op rather than a refresh (#345): the row it would otherwise rewrite is,
/// by construction, another concurrent pass's brand-new proposal — never the
/// stale one #284 wanted refreshed, since that row would have been found by
/// `first_prepared` at the top of `consider` — and the winner may already
/// have claimed and sent a notification pointing at it. Staleness is
/// `refresh_stale_proposal`'s job, after a claim is won; see
/// `docs/study-nudge.md`'s "Never stack proposals" section. The
/// `first_prepared` re-read below is what makes this correct either way:
/// whichever side of the race actually landed is what gets returned, not
/// necessarily the row built here.
///
/// **Logs, but never records a verdict.** Every failure below says in the log
/// what the catalogue could not do, which is true regardless of what the
/// caller does next. Whether that failure is also the *pass's* outcome is the
/// caller's question, not this function's — a `Presence`-dominant subject
/// still gets `GetStarted` out of it — and `record_verdict` counts one
/// terminal outcome per due subject per pass.
pub(super) async fn propose_a_session(db: &SqlitePool, nudge: &NudgeContext, sessions: &SessionRepository, subject_id: &str, now: DateTime<Utc>) -> Proposal {
	let catalogue = match ActivityRepository::new(db.clone()).list().await {
		Ok(catalogue) => catalogue,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the activity catalogue; composing nothing rather than an empty session");
			// A real Codex review finding on server#322 (P2): unlike
			// `first_prepared`'s per-subject read, a catalogue read is
			// global -- if it is failing, it fails identically for every
			// subject reaching this function in the same pass. Leaving
			// `eligible_at` where it was would let
			// `EngagementRepository::due`'s oldest-32 query keep
			// re-selecting exactly these subjects every subsequent pass,
			// crowding the batch and starving subjects who need no
			// catalogue read at all -- one with an existing prepared
			// session, say. Ask for the gate to be advanced instead.
			return Proposal::Unavailable {
				label: "storage_error",
				retry_in: Some(chrono::Duration::hours(1)),
			};
		}
	};

	let Some(inputs) = ranking_inputs(db, nudge, subject_id).await else {
		// Per-subject, like a failed session write below: the gate is pushed
		// out an hour rather than left where it is, since an over-ceiling
		// history will not fix itself by the next pass.
		return Proposal::Unavailable {
			label: "storage_error",
			retry_in: Some(chrono::Duration::hours(1)),
		};
	};
	let provisioned = materialize_provisioned_session(new_session_id(), subject_id, &catalogue, &inputs, now);
	if provisioned.activities.is_empty() {
		// A real Codex review finding on server#322 (P2): every one of
		// `recommend()`'s picks was dropped by `provision()` -- a `NULL`
		// floor on every eligible candidate, or a cap so tight nothing fits.
		// Persisting this anyway would write a `Scheduled` row
		// `first_prepared`/`provision_if_absent` then treats as "already
		// prepared" forever: nothing in this codebase re-provisions once a
		// prepared session exists, so the subject would be stuck pointing at
		// a permanently empty session even after the catalogue is fixed.
		// Retry later instead of writing a session with nothing in it -- a
		// longer backoff than the storage-error case above, since a
		// catalogue that produces nothing timeable needs a content fix, not
		// a quick retry.
		warn!(subject = %subject_id, "recommend()+provision() produced no timeable activities; not persisting an empty session");
		return Proposal::Unavailable {
			label: "nothing_to_provision",
			retry_in: Some(chrono::Duration::hours(6)),
		};
	}

	let provisioned_write = unless_deleted(
		&EngagementRepository::new(db.clone()),
		&nudge.deletion,
		subject_id,
		sessions.provision_if_absent(subject_id, &provisioned),
	)
	.await;
	let Ok(provisioned_write) = provisioned_write else {
		error!(subject = %subject_id, "could not check the subject still exists before provisioning; skipping this subject");
		return Proposal::Unavailable {
			label: "storage_error",
			retry_in: None,
		};
	};
	let Some(provisioned_write) = provisioned_write else {
		// The account was deleted after this pass read the subject, so there
		// is no one left to provision for, and no gate row to retry from.
		return Proposal::Unavailable {
			label: "nothing_to_provision",
			retry_in: None,
		};
	};
	if let Err(err) = provisioned_write {
		error!(subject = %subject_id, error = %err, "could not write a provisioned session; skipping this subject rather than notifying about one that doesn't exist");
		// Per-subject, unlike the catalogue failures above: the gate stays
		// where it is so the very next pass tries this subject again.
		return Proposal::Unavailable {
			label: "storage_error",
			retry_in: None,
		};
	}

	match sessions.first_prepared(subject_id).await {
		Ok(Some(session_id)) => Proposal::Prepared(session_id),
		Ok(None) => {
			// Unreachable in practice: `provision_if_absent` just proved a
			// prepared session exists for this subject, either the one built
			// above or a concurrent writer's. Guarded rather than trusted,
			// per this codebase's refuse-rather-than-guess convention.
			error!(subject = %subject_id, "provisioned a session but none is findable immediately after; skipping this pass");
			Proposal::Unavailable {
				label: "storage_error",
				retry_in: None,
			}
		}
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not re-read the provisioned session; skipping this subject");
			Proposal::Unavailable {
				label: "storage_error",
				retry_in: None,
			}
		}
	}
}

/// Re-run the half of [`intervention::Engine::evaluate`] that happens *after*
/// warrant, now that there is a session to point at.
///
/// Deliberately not a second `evaluate` call: warrant (refractory, then the
/// solved eligibility instant) was settled before anything was written, and
/// nothing this pass does afterwards can change it — `StudySignal::
/// SessionProvisioned` carries a delta of `0.0` precisely so that composing a
/// proposal is not itself evidence of engagement. What *does* change is the
/// selector's input, so only selection and admission are redone, in the same
/// order and with the same `retry_at` arithmetic `evaluate` uses.
///
/// The provisioned action still has to clear the same admission gate any
/// other action would — quiet hours and consent do not stop applying just
/// because this action came from provisioning rather than an existing
/// session. Presence is re-checked against the new action's own context
/// automatically: `consider`'s constraints hold one fixed `PresenceLeases`
/// snapshot, fetched once at the top of the pass, and `admit` asks it about
/// whatever action it is given.
pub(super) fn decide_with_a_proposal(admissibility: &StudyConstraints, charge: &Charge<StudyV1>, now: DateTime<Utc>, session_id: String) -> Verdict<StudyAction, Suppressed> {
	let deficits = charge.deficits::<StudyCalibration>(now);
	let Some(action) = (StudySelector {
		prepared_session: Some(session_id),
	})
	.select(&deficits) else {
		// Cannot happen: `select` is exhaustive over all four classes once
		// `prepared_session` is `Some`. Handed back as a verdict rather than
		// `.expect`ed so `consider`'s own defensive arm is the single place
		// that decides what an impossible state costs.
		return Verdict::NothingToSay;
	};

	match admissibility.admit(now, &action) {
		Ok(()) => Verdict::Intervene(action),
		Err(reason) => Verdict::Suppressed {
			reason,
			retry_at: now + StudyCalibration::REFRACTORY.min(chrono::Duration::hours(1)),
		},
	}
}

/// #284 (RCM7): refresh `prepared`'s content in place if it is an un-started
/// `system` proposal — same id, freshly recommended contents — so an
/// ordinary pass over someone who keeps ignoring the same proposal does not
/// keep pointing at whatever `recommend()` produced the day it was first
/// written.
///
/// **Called only after `consider` has already won the claim on the
/// intervention it is about to send** — not from the moment `first_prepared`
/// resolves, and not merely on `Verdict::Intervene` either. Both were real
/// `chatgpt-codex-connector` findings on `#335`. The first caught an earlier
/// version that ran this before admission was ever checked: it could rewrite
/// a proposal's content while the subject held a fresh presence lease on
/// that exact session — actively viewing it — only for `evaluate`'s own
/// `admit` call to then suppress the notification on `Present` anyway,
/// mutating a session out from under someone looking at it for nothing.
/// Moving the call to gate on `Verdict::Intervene` fixed that, but not a
/// second race one level up: two concurrent passes over the same subject can
/// both reach `Intervene`, and gating on that verdict alone meant *both* ran
/// this refresh — including the one that goes on to lose the claim below
/// and therefore never sends anything. Because a machine refresh
/// deliberately never moves `updated_at`, the loser's write still lands as
/// "untouched" after the winner has already claimed and sent, letting the
/// loser silently rewrite the exact session the recipient just opened from
/// the winner's notification. Gating on a successfully claimed `log_id`
/// instead closes that: at most one pass per intervention ever reaches this
/// call, and it is always the one that goes on to actuate.
///
/// **Best-effort, not fatal.** Unlike `propose_a_session` (where a
/// catalogue-read failure or an empty candidate set means there may be
/// nothing to offer this pass at all, and only plain absence's `GetStarted`
/// saves it), a failure here still has a session to fall back to — the existing,
/// unrefreshed proposal `prepared` already names. So every failure path
/// below just logs and leaves that proposal exactly as it was, rather than
/// failing `consider` or advancing any gate: the person still gets notified
/// about *something* real, only not this pass's freshest recommendation.
///
/// **Reads before it writes.** `prepared` is the id `evaluate` is about to
/// act on — which, because `first_prepared`'s own priority order is
/// `paused`, then `scheduled`, then `draft`, is not always the `system`/
/// un-started proposal even when one exists (a person's own `paused`
/// session outranks it). A single indexed lookup by id decides whether this
/// is the row to refresh at all; nothing here assumes `prepared` already is
/// it.
///
/// **`created_at == updated_at` is the second gate, and it is load-bearing —
/// a real `chatgpt-codex-connector` finding on `#335`.** `origin = 'system'`
/// alone is not proof nobody has touched this row: `docs/study-nudge.md`'s
/// own "Origin" section already documents that, before `paulgsc/some-ui`
/// PRO1 ships, the live client never sends an `origin` field on an edit, so
/// `update_session` never promotes it — a person can rename this exact
/// proposal, or replace its activities, and it still reads as `system` and
/// `started_at IS NULL` afterwards. Before this refresh existed, that gap's
/// only cost was a `Momentum` misclassification if the row was later
/// abandoned; refreshing turns the same gap into silent data loss, since it
/// would overwrite the person's own edit with a fresh recommendation.
/// `update_session` advances `updated_at` unconditionally on every write
/// (`upsert`'s own doc comment), so a person's edit — even an unpromoted one
/// — always moves it away from `created_at`. `SessionRepository::
/// refresh_if_untouched`'s own `UPDATE` deliberately never touches
/// `updated_at` either, which is what makes `created_at == updated_at`
/// survive any number of machine refreshes and still mean exactly one thing:
/// nothing but the waker has ever written to this row.
///
/// **The checks below are an optimisation, not the safety boundary — a real
/// `chatgpt-codex-connector` finding on `#335` caught an earlier version
/// treating them as if they were.** `record` is read here, then `evaluate`,
/// admission, a catalogue read, and `recommend()`/`provision()` all run
/// before anything is written — a real window for a person's own
/// `PATCH`/`DELETE` to land in. Re-deciding from a now-stale `record` and
/// writing unconditionally would silently overwrite whatever changed in
/// that window, or orphan a new row while `consider`'s already-selected
/// `StudyAction` keeps pointing at the old id. So the actual enforcement
/// lives one level down, in `refresh_if_untouched`'s own `WHERE` clause,
/// which re-checks the identical predicate atomically at write time; see
/// its own doc comment for the three ways the row can have changed and why
/// each one safely no-ops instead. Everything here only decides whether
/// it is worth doing the catalogue read and `recommend()` call at all.
pub(super) async fn refresh_stale_proposal(db: &SqlitePool, nudge: &NudgeContext, sessions: &SessionRepository, subject_id: &str, prepared: &str, now: DateTime<Utc>) {
	let record = match sessions.get(subject_id, prepared).await {
		Ok(Some(record)) => record,
		Ok(None) => return,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the prepared session; leaving it as-is rather than guessing whether it needs refreshing");
			return;
		}
	};
	if !(matches!(record.origin, SessionOrigin::System) && record.started_at.is_none() && record.created_at == record.updated_at) {
		return;
	}

	let catalogue = match ActivityRepository::new(db.clone()).list().await {
		Ok(catalogue) => catalogue,
		Err(err) => {
			error!(subject = %subject_id, error = %err, "could not read the activity catalogue; leaving the existing proposal stale rather than failing this pass");
			return;
		}
	};
	let Some(inputs) = ranking_inputs(db, nudge, subject_id).await else {
		// The existing proposal is a real, playable session; leave it rather
		// than replace it with one ranked on no history.
		return;
	};
	let refreshed = materialize_provisioned_session(new_session_id(), subject_id, &catalogue, &inputs, now);
	if refreshed.activities.is_empty() {
		// Same trap #322 (P2) named for the original provisioning path: a
		// catalogue with nothing currently timeable must not clobber a real
		// proposal with an empty one. Leaving the stale row in place is
		// strictly better than that — it is still a real, playable session.
		warn!(subject = %subject_id, "recommend()+provision() currently produces nothing timeable; leaving the existing proposal as-is");
		return;
	}
	match sessions.refresh_if_untouched(subject_id, prepared, &refreshed).await {
		// `false` means the row changed state (edited, started, promoted,
		// or deleted) in the window since `record` was read above — not an
		// error, just nothing left to do; the row is already exactly as
		// whatever touched it last left it.
		Ok(_) => {}
		Err(err) => error!(subject = %subject_id, error = %err, "could not refresh the existing proposal; leaving it as-is"),
	}
}

/// A provisioned session, fully materialised.
///
/// #282 (RCM5), the story that closes out what #279 (RCM2) deliberately left
/// empty. `recommend()` (#280,
/// RCM3) picks the activities; `provision()` (#281, RCM4) sets each one's
/// floor duration; this function's own naming and duration logic
/// (`activity_repo::naming`, `activity_repo::provisioning::total_duration_ms`)
/// fill in everything else #282's issue table asks for.
///
/// **`status: Scheduled`, not `Draft`.** A deliberate change from what this
/// function wrote before RCM3/RCM4/RCM5 existed to fill a session in. RCM2's
/// original `Draft` choice defended itself on crash-safety grounds specific
/// to a session with nothing in it yet — see `handlers::db::session::
/// create_session`'s identical reasoning for why a session born anything but
/// `Draft` could be offered before it was finished being composed. That
/// concern does not apply here: `recommend()`, `provision()`, and this
/// function's naming/duration all run synchronously inside one call, before
/// the row is ever written, so there is no partially-composed state a crash
/// could expose between "written" and "filled in." `Scheduled` is what
/// #282's own table asks for, so `first_prepared` and `StudySelector` treat
/// a provisioned session exactly like one a person scheduled themselves.
///
/// **`scenes: Vec::new()`.** #282's own "hard" decision — see
/// `docs/study-nudge.md`'s "Materialising a session: the `scenes` decision"
/// section for the full argument. In short: #272 already decided this
/// server cannot compute a scene's `props` (the closure stays client-side),
/// so the only honest options are an empty `scenes` array or a shape that
/// merely *looks* complete. This writes `[]`. `total_duration_ms` is
/// computed directly from the provisioned activities
/// (`activity_repo::total_duration_ms`) rather than derived from `scenes` —
/// `session_repo::total_duration_of(&[])` would read this session as
/// zero-length, which is wrong; it has real, timed blocks, only not yet
/// materialised into playable scenes. See `total_duration_ms`'s own doc
/// comment for why the two numbers are provably equal for the `basic`
/// layout this always produces.
///
/// **`layout: None`.** SQL `NULL`, not the JSON string `"null"` —
/// `deserialize_explicit_null`/the double-`Option` treatment on
/// `SessionRecord::layout` exists for exactly this distinction. Absent means
/// the client's naive default layout applies, which is right for a proposal
/// the server has no business asserting a layout for.
///
/// `origin: SessionOrigin::System` (`#283`, RCM6) is the real field, landed
/// after RCM5 kept this diff readable on its own. Before it existed, `name`
/// was the only signal that this was proposed rather than authored; now
/// `Momentum` reads `origin` directly (see `session_repo::model::
/// session_abandonment_is_real`) rather than inferring intent from a string.
///
/// `pub` rather than crate-private: `dump-provisioned-session` (the bin
/// this crate ships next to `dump-proposed-session`) calls this directly so
/// the fixture it emits is the exact same code path `consider` runs in
/// production, not a re-implementation of it — the same reason
/// `dump_proposed_session.rs` calls `activity_repo::provision` rather than
/// re-deriving its output.
#[must_use]
pub fn materialize_provisioned_session(id: String, subject_id: &str, catalogue: &[ActivityRecord], inputs: &RankingInputs, now: DateTime<Utc>) -> SessionRecord {
	let stamp = now.to_rfc3339();
	let picks = recommend(subject_id, DEFAULT_RECOMMENDATION_COUNT, catalogue, &inputs.history, inputs.last_session_at, now);
	let provisioned = provision(&picks);

	// `name` is built from whichever picks actually survived `provision` —
	// a `min_duration_ms: NULL` activity, or one that would have pushed the
	// session over `CLIENT_MAX_TOTAL_DURATION_MS`, has no business in the
	// name of a session it is not in. `provision` preserves `recommend`'s
	// order (see its own `provision_preserves_the_recommenders_order`
	// test), so filtering `picks` down to the surviving ids, in `picks`'
	// own order, reconstructs exactly what `provisioned` contains.
	let provisioned_ids: HashSet<&str> = provisioned.iter().map(|activity| activity.activity_id.as_str()).collect();
	let named: Vec<ActivityRecord> = picks.into_iter().filter(|activity| provisioned_ids.contains(activity.id.as_str())).collect();

	SessionRecord {
		id,
		name: default_session_name(&named),
		status: SessionStatus::Scheduled,
		origin: SessionOrigin::System,
		activities: provisioned
			.iter()
			.map(|activity| serde_json::to_value(activity).unwrap_or(serde_json::Value::Null))
			.collect(),
		scenes: Vec::new(),
		layout_mode: LayoutMode::Basic,
		layout: None,
		total_duration_ms: total_duration_ms(&provisioned),
		created_at: stamp.clone(),
		updated_at: stamp,
		started_at: None,
		completed_at: None,
		final_elapsed_ms: None,
	}
}
