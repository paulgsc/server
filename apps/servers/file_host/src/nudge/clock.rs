//! What "today" means, decided once.
//!
//! Every time-of-day judgement in this feature — the day boundary, quiet
//! hours, "have I studied today" — is wrong by the UTC offset unless a zone is
//! chosen explicitly, and `file_host` runs in a container, which is UTC unless
//! told otherwise. The concrete failures in a UTC-07:00 zone:
//!
//! - A session completed at 18:00 local is `2026-08-05T01:00Z`. A UTC day
//!   boundary files it under *tomorrow*, so "have I studied today" answers no
//!   and a nudge fires for a day already done.
//! - Quiet hours of 22:00–08:00 evaluated against UTC silence 15:00–01:00 local
//!   and permit 03:00 local.
//!
//! The client resolved this once, correctly, and the resolution transfers:
//! `decideNudge` compares `getFullYear`/`getMonth`/`getDate` in the viewer's own
//! zone, because "same calendar day" is the only sense of *today* a person
//! means. This module is that comparison, against a configured zone rather than
//! an ambient one.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// Where the zone came from.
///
/// Reported back so `main` can log it. An unset `NUDGE_TIMEZONE` is allowed but
/// is not allowed to be *quiet*: "the reminder came at 3am" is a much harder
/// thing to diagnose than a line at startup saying which clock is in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneSource {
	/// `NUDGE_TIMEZONE` named this zone.
	Configured(String),
	/// `NUDGE_TIMEZONE` was unset. Running on UTC by default.
	Unset,
	/// `NUDGE_TIMEZONE` named something `chrono-tz` does not know.
	Unparseable(String),
}

/// A timezone and the calendar arithmetic that depends on it.
///
/// Not a clock: it never reads the time. `now` is always an argument, which is
/// what lets the policy above it stay a pure function.
#[derive(Debug, Clone)]
pub struct NudgeClock {
	zone: Tz,
}

impl NudgeClock {
	/// Resolve the configured zone, reporting how it went.
	///
	/// Never fails. A misspelled zone falls back to UTC rather than refusing to
	/// boot, but says so — the alternative is a server that will not start
	/// because of a typo in an optional feature's configuration.
	#[must_use]
	pub fn resolve(configured: Option<&str>) -> (Self, ZoneSource) {
		configured.map_or_else(
			|| (Self { zone: Tz::UTC }, ZoneSource::Unset),
			|name| {
				name.parse::<Tz>().map_or_else(
					|_| (Self { zone: Tz::UTC }, ZoneSource::Unparseable(name.to_owned())),
					|zone| (Self { zone }, ZoneSource::Configured(name.to_owned())),
				)
			},
		)
	}

	#[must_use]
	pub const fn zone(&self) -> Tz {
		self.zone
	}

	/// The calendar date `instant` falls on, in this zone.
	#[must_use]
	pub fn local_date(&self, instant: DateTime<Utc>) -> NaiveDate {
		instant.with_timezone(&self.zone).date_naive()
	}

	/// The `YYYY-MM-DD` key `nudge_log` is keyed by.
	#[must_use]
	pub fn local_day_key(&self, instant: DateTime<Utc>) -> String {
		self.local_date(instant).format("%Y-%m-%d").to_string()
	}

	/// The local hour, 0-23, that quiet hours are evaluated against.
	#[must_use]
	pub fn local_hour(&self, instant: DateTime<Utc>) -> u32 {
		instant.with_timezone(&self.zone).hour()
	}

	/// Whether two instants land on the same calendar day here.
	///
	/// Every "today" question goes through this, so that there is exactly one
	/// definition of the day boundary to be wrong in.
	#[must_use]
	pub fn same_local_day(&self, a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
		self.local_date(a) == self.local_date(b)
	}

	/// The half-open UTC instant range `[start, end)` covering the local day
	/// that contains `instant`.
	///
	/// This is what turns "did I study today" into an indexed range scan over
	/// `started_at` / `completed_at` instead of a table scan plus per-row
	/// timezone conversion.
	///
	/// **DST.** On a spring-forward day local midnight does not exist, and on a
	/// fall-back day it exists twice. The ambiguous case takes the earlier
	/// instant and the absent case takes the first instant that does exist, so
	/// the range is always well-formed and never drops an hour of history. Both
	/// are accepted rather than solved: the consequence is at most one nudge
	/// arriving up to an hour off on two days a year, which is not worth a
	/// calendar library's worth of machinery.
	#[must_use]
	pub fn local_day_bounds(&self, instant: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
		let date = self.local_date(instant);
		(self.start_of_local_day(date), self.start_of_local_day(date + Duration::days(1)))
	}

	fn start_of_local_day(&self, date: NaiveDate) -> DateTime<Utc> {
		let midnight = date.and_time(NaiveTime::MIN);

		self
			.zone
			.from_local_datetime(&midnight)
			.earliest()
			.or_else(|| {
				// Spring forward: 00:00 was skipped. Walk forward until the
				// wall clock exists again.
				(1..=3_i64).find_map(|hours| self.zone.from_local_datetime(&(midnight + Duration::hours(hours))).earliest())
			})
			.map_or_else(|| DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc), |local| local.with_timezone(&Utc))
	}
}

/// Parse an ISO-8601 timestamp as the client writes it.
///
/// `new Date().toISOString()` produces RFC 3339 with a `Z`, which is the
/// common case; the offset form is accepted too. `None` for anything else,
/// which callers must treat as "this stamp tells me nothing" rather than as
/// zero — see `hours_since` in `policy`.
#[must_use]
pub fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(raw).ok().map(|parsed| parsed.with_timezone(&Utc))
}

/// Is `hour` inside the half-open local-hour range `[start, end)`?
///
/// A direct port of `isWithinQuietHours`. The range may wrap midnight — 22→8 is
/// the default and the common case, so the wrapping branch is the one that has
/// to be right rather than the edge case. Equal bounds mean "no quiet hours"
/// rather than "always quiet": someone who drags both ends together has
/// flattened the range, not muted the app, and `NUDGE_ENABLED` is the control
/// for muting it.
#[must_use]
pub const fn is_within_quiet_hours(hour: u32, start: u32, end: u32) -> bool {
	if start == end {
		return false;
	}
	if start < end {
		hour >= start && hour < end
	} else {
		hour >= start || hour < end
	}
}

#[cfg(test)]
mod tests {
	use super::{is_within_quiet_hours, parse_timestamp, NudgeClock, ZoneSource};

	fn denver() -> NudgeClock {
		NudgeClock::resolve(Some("America/Denver")).0
	}

	#[test]
	fn an_unset_zone_is_utc_but_says_so() {
		let (clock, source) = NudgeClock::resolve(None);
		assert_eq!(source, ZoneSource::Unset);
		assert_eq!(clock.zone().to_string(), "UTC");
	}

	#[test]
	fn a_misspelled_zone_falls_back_loudly_rather_than_refusing_to_boot() {
		let (clock, source) = NudgeClock::resolve(Some("America/Denvr"));
		assert_eq!(source, ZoneSource::Unparseable("America/Denvr".to_owned()));
		assert_eq!(clock.zone().to_string(), "UTC");
	}

	#[test]
	fn an_evening_session_counts_as_today_whatever_the_container_thinks() {
		// 18:00 in UTC-06:00 is tomorrow in UTC. The whole point of the zone.
		let clock = denver();
		let evening = parse_timestamp("2026-08-05T00:00:00Z").unwrap(); // 18:00 Aug 4 local
		let same_evening = parse_timestamp("2026-08-04T20:00:00Z").unwrap(); // 14:00 Aug 4 local

		assert_eq!(clock.local_day_key(evening), "2026-08-04");
		assert!(clock.same_local_day(evening, same_evening));
	}

	#[test]
	fn the_boundary_falls_where_the_local_calendar_says() {
		let clock = denver();
		let before = parse_timestamp("2026-08-05T05:59:00Z").unwrap(); // 23:59 Aug 4
		let after = parse_timestamp("2026-08-05T06:01:00Z").unwrap(); // 00:01 Aug 5

		assert_eq!(clock.local_day_key(before), "2026-08-04");
		assert_eq!(clock.local_day_key(after), "2026-08-05");
		assert!(!clock.same_local_day(before, after));
	}

	#[test]
	fn day_bounds_bracket_exactly_the_local_day() {
		let clock = denver();
		let noon = parse_timestamp("2026-08-04T18:00:00Z").unwrap();
		let (from, to) = clock.local_day_bounds(noon);

		assert_eq!(clock.local_day_key(from), "2026-08-04");
		assert!(from <= noon && noon < to);
		assert_eq!(to - from, chrono::Duration::days(1));
	}

	#[test]
	fn day_bounds_survive_the_spring_forward_that_has_no_midnight() {
		// Lord Howe and a handful of zones skip midnight itself on DST day.
		let clock = NudgeClock::resolve(Some("America/Santiago")).0;
		let during = parse_timestamp("2026-09-06T15:00:00Z").unwrap();
		let (from, to) = clock.local_day_bounds(during);

		assert!(from < to);
		assert!(from <= during && during < to);
	}

	#[test]
	fn quiet_hours_handle_a_same_day_range() {
		assert!(is_within_quiet_hours(3, 1, 6));
		assert!(!is_within_quiet_hours(6, 1, 6));
		assert!(is_within_quiet_hours(1, 1, 6));
	}

	#[test]
	fn quiet_hours_wrap_midnight_for_the_default_range() {
		assert!(is_within_quiet_hours(23, 22, 8));
		assert!(is_within_quiet_hours(3, 22, 8));
		assert!(!is_within_quiet_hours(8, 22, 8));
		assert!(!is_within_quiet_hours(14, 22, 8));
	}

	#[test]
	fn equal_bounds_mean_no_quiet_hours_rather_than_always_quiet() {
		assert!(!is_within_quiet_hours(0, 9, 9));
		assert!(!is_within_quiet_hours(9, 9, 9));
	}

	#[test]
	fn an_unparseable_stamp_is_none_rather_than_the_epoch() {
		assert!(parse_timestamp("not-a-date").is_none());
		assert!(parse_timestamp("").is_none());
		assert!(parse_timestamp("2026-08-05T00:00:00Z").is_some());
		assert!(parse_timestamp("2026-08-04T18:00:00-06:00").is_some());
	}
}
