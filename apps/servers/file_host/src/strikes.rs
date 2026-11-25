//! A device that keeps failing on its own is set aside (#1180, #1222, epic
//! #1174).
//!
//! The owner's words on #1180: "stop sending to a phone after a few
//! timeouts". After [`SET_ASIDE_AFTER`] failures in a row that were the
//! device's own, it is set aside: not deleted, marked as no longer sent to,
//! with when and why. A delivery it accepts later, or registering again,
//! brings it back. One family for both senders that reach a person's device,
//! the study nudge (web push, `push_subscriptions`) and the Jobs notice (FCM,
//! `jobs_notice_device`): the columns sit on each device table, and which
//! failures strike is closed per sender here (the coordinator's call C1513).
//!
//! **A strike is the device's own failure.** Anything that could be home's
//! (a key, a payload, a signature, a service having a bad hour) never
//! strikes, since it would set every device aside at once:
//!
//! - web push ([`web_push`]): a delivery that got no answer in time, or no
//!   answer at all, strikes. `400` (our payload or VAPID JWT), `403` (our
//!   key), `413`, `429`, `5xx` and a send we never made do not; `404`/`410`
//!   prune the subscription, as before.
//! - FCM ([`fcm`]): `403 SENDER_ID_MISMATCH` strikes. A `400` strikes only
//!   when another phone took the same notice in that pass, which shows the
//!   notice and the key were fine. A lone phone's `400` never strikes: its
//!   token can't be told apart from our payload (the bound). `401`, the token
//!   endpoint, a JWT home could not sign, `429`, `5xx` and no answer never
//!   strike; `404`/`UNREGISTERED` forget the phone, as before.
//!
//! **Brought back without a clock.** A set-aside device is not probed on a
//! schedule. When a real send happens anyway, it is included at most once a
//! day ([`PROBE_EVERY_HOURS`]); if it accepts, it is back, and if not it stays
//! aside with no new strike and no new count. Work only per real send, O(1)
//! per device (docs/study-nudge.md, "Scaling invariants";
//! docs/home-compute.md, "no clock").
//!
//! **Counted once.** Setting a device aside is one event, counted when the
//! mark is written (`study-nudge`/`device_set_aside`,
//! `jobs-notice`/`phone_set_aside`). It is not a level: only a person can
//! bring it back sooner, so a level would page for good with nothing to do
//! (docs/home-compute.md, invariant 9).

use chrono::{DateTime, Duration, Utc};

/// Failures of its own in a row that set a device aside: the owner's number
/// on #1180, a policy kept in code.
pub const SET_ASIDE_AFTER: i64 = 3;

/// How often, at most, a set-aside device is included in a real send.
pub const PROBE_EVERY_HOURS: i64 = 24;

/// Whether a device is sent to on this send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
	/// It is not set aside.
	Send,
	/// It is set aside and its daily probe is due: sent to, and a failure
	/// takes no strike.
	Probe,
	/// It is set aside and was probed within the day.
	Skip,
	/// It is sent to, and already took a strike in this pass: a pass is one
	/// try, however many records or interventions it sends (#1180).
	Struck,
}

/// This send's turn for a device set aside at `set_aside_at` (`None`: it is
/// not) and last probed at `last_probe_at`, at `now`. A time that does not
/// read is due: a probe too many is the side to err on.
#[must_use]
pub fn turn(set_aside_at: Option<&str>, last_probe_at: Option<&str>, now: DateTime<Utc>) -> Turn {
	if set_aside_at.is_none() {
		return Turn::Send;
	}
	let due = last_probe_at.is_none_or(|at| DateTime::parse_from_rfc3339(at).map_or(true, |at| now - at.with_timezone(&Utc) >= Duration::hours(PROBE_EVERY_HOURS)));
	if due {
		Turn::Probe
	} else {
		Turn::Skip
	}
}

/// Why a web push delivery strikes its device, if it does: `None` for an
/// answer (`Some(outcome)`) that was home's or the service's, or that was
/// accepted.
#[must_use]
pub const fn web_push(outcome: Option<&push_kit::SendOutcome>) -> Option<&'static str> {
	match outcome {
		None => Some("timeouts"),
		Some(push_kit::SendOutcome::Transport(_)) => Some("no_answer"),
		Some(_) => None,
	}
}

/// Why an FCM answer strikes its phone, if it does. `another_took`: another
/// phone took the same notice in this pass.
#[must_use]
pub const fn fcm(outcome: &fcm_kit::SendOutcome, another_took: bool) -> Option<&'static str> {
	match outcome {
		fcm_kit::SendOutcome::Mismatched => Some("sender_mismatch"),
		fcm_kit::SendOutcome::Refused(400) if another_took => Some("refused"),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::{fcm, turn, web_push, Turn};
	use chrono::{DateTime, Utc};

	fn at(text: &str) -> DateTime<Utc> {
		DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
	}

	/// #1180: a device sent to is sent to; one set aside is included once a
	/// day, when a send happens anyway; a time that does not read is due.
	#[test]
	fn a_set_aside_device_is_included_at_most_once_a_day() {
		let now = at("2026-11-25T12:00:00Z");
		assert_eq!(turn(None, None, now), Turn::Send);
		let aside = Some("2026-11-20T00:00:00Z");
		assert_eq!(turn(aside, None, now), Turn::Probe, "never probed");
		assert_eq!(turn(aside, Some("2026-11-25T06:00:00Z"), now), Turn::Skip, "probed this morning");
		assert_eq!(turn(aside, Some("2026-11-24T12:00:00Z"), now), Turn::Probe, "a day ago");
		assert_eq!(turn(aside, Some("not a time"), now), Turn::Probe);
	}

	/// C1513: only the device's own failures strike. Family: every web push
	/// outcome, and FCM's with and without another phone taking the notice.
	#[test]
	fn only_a_devices_own_failure_strikes() {
		use fcm_kit::SendOutcome as Fcm;
		use push_kit::SendOutcome as Push;
		assert_eq!(web_push(None), Some("timeouts"));
		assert_eq!(web_push(Some(&Push::Transport(String::from("reset")))), Some("no_answer"));
		for home in [
			Push::Accepted,
			Push::Expired,
			Push::KeyMismatch,
			Push::Rejected(String::from("bad jwt")),
			Push::TooLarge,
			Push::RateLimited { retry_after: None },
			Push::ServiceError { status: 503, body: String::new() },
			Push::NotSent(String::from("encrypt")),
		] {
			assert_eq!(web_push(Some(&home)), None, "{home:?}");
		}

		assert_eq!(fcm(&Fcm::Mismatched, false), Some("sender_mismatch"));
		assert_eq!(fcm(&Fcm::Refused(400), true), Some("refused"), "another phone took it: the token's");
		assert_eq!(fcm(&Fcm::Refused(400), false), None, "a lone phone's 400 could be our payload");
		for home in [
			Fcm::Delivered,
			Fcm::Unregistered,
			Fcm::Refused(0),
			Fcm::Refused(401),
			Fcm::Refused(403),
			Fcm::Refused(429),
			Fcm::Refused(503),
			Fcm::Transport(String::from("reset")),
		] {
			assert_eq!(fcm(&home, true), None, "{home:?}");
		}
	}
}
