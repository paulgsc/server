//! The period read behind the weekly engagement report (paulgsc/some-ui
//! ENG12, #2700): one subject's engagement counts for an ISO week, the week
//! before, the same week a quarter ago and the same week a year ago.
//!
//! - **Raw counts, ids only.** Per pipeline, affordance and surface, how many
//!   times each was offered, taken, passed over and left. What may go public,
//!   and how coarsely (#2701: bands, "fewer than 5", aph and Jobs as one
//!   line), is the report writer's rule in some-ui, applied after this.
//!   No time of day leaves here: [`Seen`] gives days only. No `said` count,
//!   no answer and no ending leave at all.
//! - **A function of the records before the week's end.** Every number reads
//!   only events whose `at` is before the asked week's `to`, so the same
//!   records give the same answer however late it is asked.
//! - **Absence is an observation.** A week before home held any engagement
//!   event is `measured: false`, which the report says as "no data", never
//!   as zero.
//! - **Bounded by the index.** Each week is one range read of
//!   `app_event_by_family` over seven days; [`Seen`] is one grouped read of
//!   the subject's engagement events before `to`, as `summary`'s last taken
//!   is. Nothing is held per signal or per week between reads.
//!
//! Weeks are ISO weeks in UTC: Monday 00:00 to the next Monday, half-open.

use crate::EventError;
use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Utc, Weekday};
use serde::Serialize;
use sqlx::SqliteConnection;

/// An ISO week: `2026-W46`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IsoWeek {
	monday: NaiveDate,
}

/// The years a week may name: four digits, so every instant this writes has
/// the fixed width `at` is compared at.
const YEARS: std::ops::RangeInclusive<i32> = 2000..=9998;

impl IsoWeek {
	/// `2026-W46` (the week number two digits), or `None`.
	#[must_use]
	pub fn parse(text: &str) -> Option<Self> {
		let (year, week) = text.split_once("-W")?;
		if year.len() != 4 || week.len() != 2 || !year.bytes().chain(week.bytes()).all(|byte| byte.is_ascii_digit()) {
			return None;
		}
		Self::of(year.parse().ok()?, week.parse().ok()?)
	}

	fn of(year: i32, week: u32) -> Option<Self> {
		if !YEARS.contains(&year) {
			return None;
		}
		NaiveDate::from_isoywd_opt(year, week, Weekday::Mon).map(|monday| Self { monday })
	}

	/// The week holding `date`.
	#[must_use]
	pub fn containing(date: NaiveDate) -> Self {
		let back = i64::from(date.weekday().num_days_from_monday());
		Self {
			monday: date - Duration::days(back),
		}
	}

	/// `2026-W46`.
	#[must_use]
	pub fn label(self) -> String {
		self.to_string()
	}

	/// `weeks` weeks before this one.
	#[must_use]
	pub fn back(self, weeks: i64) -> Self {
		Self {
			monday: self.monday - Duration::weeks(weeks),
		}
	}

	/// The same week number in the ISO year before, or its W52 when that
	/// year has no W53.
	#[must_use]
	pub fn year_ago(self) -> Self {
		let iso = self.monday.iso_week();
		Self::of(iso.year() - 1, iso.week())
			.or_else(|| Self::of(iso.year() - 1, 52))
			.unwrap_or_else(|| self.back(52))
	}

	/// The week's first instant, as `at` is stored.
	#[must_use]
	pub fn from(self) -> String {
		Self::instant(self.monday)
	}

	/// The instant after the week's last: the next Monday's first.
	#[must_use]
	pub fn to(self) -> String {
		Self::instant(self.monday + Duration::weeks(1))
	}

	fn instant(day: NaiveDate) -> String {
		DateTime::<Utc>::from_naive_utc_and_offset(day.and_time(chrono::NaiveTime::MIN), Utc).to_rfc3339_opts(SecondsFormat::Millis, true)
	}
}

impl std::fmt::Display for IsoWeek {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let iso = self.monday.iso_week();
		write!(f, "{:04}-W{:02}", iso.year(), iso.week())
	}
}

/// Which of the four weeks a [`Week`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
	This,
	Last,
	QuarterAgo,
	YearAgo,
}

/// One affordance on one surface over one week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekUse {
	pub pipeline: String,
	pub affordance: String,
	pub surface: String,
	pub offered: i64,
	pub taken: i64,
	pub passed_over: i64,
	pub left: i64,
}

/// One of the four weeks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Week {
	pub role: Role,
	/// `2026-W46`.
	pub week: String,
	pub from: String,
	pub to: String,
	/// The week has ended.
	pub complete: bool,
	/// Home held an engagement event of the subject's before the week
	/// ended. A week that is not is "no data", never zero.
	pub measured: bool,
	/// Every affordance with an event in the week, in pipeline, affordance
	/// and surface order.
	pub affordances: Vec<WeekUse>,
}

/// One affordance over everything home holds before the asked week's end:
/// the day it was first seen and the day it was last taken (UTC dates, no
/// time of day).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
	pub pipeline: String,
	pub affordance: String,
	pub surface: String,
	pub first_day: String,
	pub last_taken_day: Option<String>,
}

/// What `GET /app-events/weeks/:week` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Weekly {
	/// The asked week, `2026-W46`.
	pub week: String,
	/// This week, last week, a quarter ago (13 weeks back) and a year ago,
	/// in that order.
	pub weeks: Vec<Week>,
	/// Every affordance seen before this week's end, in order.
	pub seen: Vec<Seen>,
}

/// `subject`'s engagement over `week` and its three comparison weeks, as of
/// `now` (an instant, as `at` is stored), which only decides `complete`.
///
/// # Errors
/// [`EventError::Storage`] for any `sqlx` failure.
pub async fn weekly(conn: &mut SqliteConnection, subject: &str, week: IsoWeek, now: &str) -> Result<Weekly, EventError> {
	let end = week.to();
	let first = sqlx::query_scalar!(
		r#"SELECT MIN(at) AS "first: String" FROM app_event WHERE subject_id = ?1 AND family = 'engagement'"#,
		subject
	)
	.fetch_one(&mut *conn)
	.await?;

	let mut weeks = Vec::with_capacity(4);
	for (role, asked) in [
		(Role::This, week),
		(Role::Last, week.back(1)),
		(Role::QuarterAgo, week.back(13)),
		(Role::YearAgo, week.year_ago()),
	] {
		let (from, to) = (asked.from(), asked.to());
		let affordances = week_uses(conn, subject, &from, &to).await?;
		weeks.push(Week {
			role,
			week: asked.label(),
			complete: to.as_str() <= now,
			measured: first.as_deref().is_some_and(|first| first < to.as_str()),
			from,
			to,
			affordances,
		});
	}

	let seen = sqlx::query_as!(
		Seen,
		r#"SELECT pipeline AS "pipeline!: String", affordance AS "affordance!: String", surface AS "surface!: String",
			substr(MIN(at), 1, 10) AS "first_day!: String",
			substr(MAX(CASE WHEN kind = 'taken' THEN at END), 1, 10) AS "last_taken_day: String"
		FROM (SELECT at, json_extract(body, '$.pipeline') AS pipeline, json_extract(body, '$.affordance') AS affordance,
				json_extract(body, '$.surface') AS surface, json_extract(body, '$.kind') AS kind
			FROM app_event WHERE subject_id = ?1 AND family = 'engagement' AND at < ?2)
		GROUP BY pipeline, affordance, surface
		ORDER BY pipeline, affordance, surface"#,
		subject,
		end
	)
	.fetch_all(&mut *conn)
	.await?;

	Ok(Weekly { week: week.label(), weeks, seen })
}

/// One week's counts: a range read of `app_event_by_family`.
async fn week_uses(conn: &mut SqliteConnection, subject: &str, from: &str, to: &str) -> Result<Vec<WeekUse>, EventError> {
	Ok(
		sqlx::query_as!(
			WeekUse,
			r#"SELECT pipeline AS "pipeline!: String", affordance AS "affordance!: String", surface AS "surface!: String",
			COALESCE(SUM(kind = 'offered'), 0) AS "offered!: i64",
			COALESCE(SUM(kind = 'taken'), 0) AS "taken!: i64",
			COALESCE(SUM(kind = 'passed-over'), 0) AS "passed_over!: i64",
			COALESCE(SUM(kind = 'left'), 0) AS "left!: i64"
		FROM (SELECT json_extract(body, '$.pipeline') AS pipeline, json_extract(body, '$.affordance') AS affordance,
				json_extract(body, '$.surface') AS surface, json_extract(body, '$.kind') AS kind
			FROM app_event WHERE subject_id = ?1 AND family = 'engagement' AND at >= ?2 AND at < ?3)
		GROUP BY pipeline, affordance, surface
		HAVING SUM(kind IN ('offered', 'taken', 'passed-over', 'left')) > 0
		ORDER BY pipeline, affordance, surface"#,
			subject,
			from,
			to
		)
		.fetch_all(&mut *conn)
		.await?,
	)
}

#[cfg(test)]
mod tests {
	use super::IsoWeek;
	use chrono::NaiveDate;

	#[test]
	fn a_week_parses_and_names_itself() {
		let week = IsoWeek::parse("2026-W46").unwrap();
		assert_eq!(week.label(), "2026-W46");
		assert_eq!(week.from(), "2026-11-09T00:00:00.000Z");
		assert_eq!(week.to(), "2026-11-16T00:00:00.000Z");
		assert_eq!(week.back(1).label(), "2026-W45");
		assert_eq!(week.back(13).label(), "2026-W33");
		assert_eq!(week.year_ago().label(), "2025-W46");
		assert_eq!(IsoWeek::parse("2026-W01").unwrap().back(1).label(), "2025-W52");
		assert_eq!(IsoWeek::containing(NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()).label(), "2026-W46");
	}

	#[test]
	fn a_year_without_week_53_gives_its_52nd() {
		// 2026 has 53 ISO weeks; 2025 has 52.
		let w53 = IsoWeek::parse("2026-W53").unwrap();
		assert_eq!(w53.year_ago().label(), "2025-W52");
		assert!(IsoWeek::parse("2025-W53").is_none());
	}

	#[test]
	fn anything_else_is_not_a_week() {
		for text in ["2026-W00", "2026-W54", "2026-w46", "2026W46", "2026-W4", "26-W46", "1999-W10", "2026-W+1", "", "2026-W46 "] {
			assert!(IsoWeek::parse(text).is_none(), "{text}");
		}
	}
}
