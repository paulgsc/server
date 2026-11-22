//! The walkthrough's friction measures, over the 28 days ending with a week.
//!
//! What a published walkthrough may say about how the owner uses their own
//! apps (paulgsc/some-ui WALK5 #3337, silence #3338; the owner's C861), read
//! beside the weekly report ([`weekly`](crate::weekly)'s `friction`).
//!
//! - **Per affordance, never per module.** Which affordance is a module's
//!   root (`opened`), which are `not-measured`, which place is a stop's last
//!   and which modules the build offers are the registry's
//!   (`@some-ui/engagement`), and the report writer, which holds it, reads
//!   them off these rows. Home keeps no copy of the module list.
//! - **Opened** is, per affordance, how many of the 28 days held an
//!   `offered` or `taken` of it. Only a root that is shown on screen is an
//!   opening; for one offered by notification, `offered` is a notification
//!   sent, which the writer, holding the registry, tells apart. A day
//!   before the affordance's own `seen` `firstDay` is not measured for it,
//!   though `measuredDays` (the subject's first event of any affordance)
//!   counts it: the writer bounds an affordance's days by its `firstDay`, so
//!   a root that began sending late is never read as silent before then.
//!   A day is the owner's ([`ZONE`](crate::ZONE)).
//! - **Places** are, per affordance and place, how many `left`s named it,
//!   how many of them were followed by a `taken` of the same affordance at
//!   the same instant (`movedOn`, as `stepPlace` writes a move), and the
//!   median time in it (`at` less `startedAt`). What these can't tell apart,
//!   since a `taken` names no place: a move forward from one back, a move to
//!   the next place from the next item of the same affordance opened in the
//!   same millisecond, and, at an affordance's last place, finishing from
//!   leaving. The app going to the back writes a `left` with no `taken`, and
//!   coming back a new `taken`, so a step spent across that is two stretches:
//!   `left` counts stretches on screen, and the median is a stretch's.
//!   Since paulgsc/some-ui#3367 (the owner's answer: yes) a `left` may say
//!   the screen it went to (`next`), so `nexts` counts, per place, the
//!   `left`s that went to each screen; the report writer, holding the
//!   registry, reads finishing and leaving part-way off them. A `left` with
//!   no `next` (the app put away, or an older build's) is counted in `left`
//!   and never guessed into one. `nextsBefore` names the surfaces on which a
//!   `left` with a `next` was made by the window's start: there, a `left`
//!   with none is the app put away (the phone says `elsewhere` for a screen
//!   closing to anywhere it can't name), unless a device still runs an older
//!   build.
//!   Taps per step stay the owner's for later.
//! - **Debrief** is Applying's friction, 1 to 5, as the Jobs record holds
//!   it (`jobs_repo::debrief_frictions`): how many said it and their median.
//! - **"Fewer than 5" (ask #2701).** Counts leave raw: banding them is the
//!   report writer's rule, as every count of the weekly read is. A median is
//!   not a count, and one of fewer than [`UNDER`] values is close to a single
//!   person's single visit, so it is not said at all (`null`).
//! - **Ids, counts and medians only.** No time of day, no pick, no answer.
//! - **A function of the records before the week's end**, as the weekly
//!   read is; a day before home held any engagement event is not measured
//!   (`measuredDays`), never read as a day nothing was opened.
//! - **Bounded.** Opened is one read joined against the 28 day edges on
//!   `app_event_by_family`; places one range read of the window, at most
//!   [`WINDOW_READ`](crate::WINDOW_READ) `left`s (`partial` past it);
//!   `nextsBefore` one indexed read per surface back from the window's start
//!   to the newest `left` with a `next` there (the whole earlier record until
//!   a build sends one, then a few rows);
//!   debrief one range read of `jobs_event_order`.

use crate::{EventError, IsoWeek};
use chrono::Duration;
use serde::Serialize;
use sqlx::SqliteConnection;
use std::collections::BTreeMap;

/// The days the measures run over, ending with the asked week: C861's
/// 28-day window.
pub const WINDOW_DAYS: i64 = 28;

/// Below this many values a median is not said (ask #2701's "fewer than 5").
pub const UNDER: usize = 5;

/// One affordance's days opened in the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
	pub pipeline: String,
	pub affordance: String,
	pub surface: String,
	/// Of the window's days, how many held an `offered` or `taken` of it.
	pub days: i64,
}

/// One place of one affordance over the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceTime {
	pub pipeline: String,
	pub affordance: String,
	pub surface: String,
	pub place: String,
	/// `left`s naming it.
	pub left: i64,
	/// Of those, how many had a `taken` of the same affordance at the same
	/// instant: a move, of whatever kind the module doc lists.
	pub moved_on: i64,
	/// The median time in it, in milliseconds; `null` under [`UNDER`].
	pub median_ms: Option<i64>,
	/// Of those `left`s, the ones that said the screen they went to, counted
	/// per screen, in id order. A `left` with no `next` (the app went to the
	/// back, or a build before the field) is in none of them.
	pub nexts: Vec<NextCount>,
}

/// How many `left`s of one place went to one screen (paulgsc/some-ui#3367).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NextCount {
	/// The screen, by the app's id: one of the affordance's places,
	/// `<affordance>.<place>` of its pipeline, or `elsewhere`. Which is the
	/// affordance's own is the registry's, and so the report writer's to tell.
	pub next: String,
	pub left: i64,
}

/// Applying's debrief friction over the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Debrief {
	/// Debriefs that said a friction.
	pub said: i64,
	/// Their median, 1 to 5 (a half between two); `null` under [`UNDER`].
	pub median: Option<f64>,
}

/// The measures, as the weekly read carries them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Friction {
	/// The window's first instant: the Monday 27 days before the asked
	/// week's Sunday, at the owner's midnight.
	pub from: String,
	/// The asked week's end.
	pub to: String,
	/// Of the window's days, how many end after home's first engagement
	/// event of any affordance. 0 is "no data"; one affordance's own start
	/// is its `seen` `firstDay`.
	pub measured_days: i64,
	/// Every affordance opened in the window, in pipeline, affordance and
	/// surface order.
	pub opened: Vec<Opened>,
	/// Every place left in the window, in the same order, then by place.
	pub places: Vec<PlaceTime>,
	/// The surfaces (`apk`, `web`) on which the subject made a `left` that
	/// said where it went at or before the window's first instant, in that
	/// order: on them every build that ran in the window could say it, so the
	/// writer may read their `nexts` as whole (a device kept on an older build
	/// after another of the same surface moved on is the tolerance it
	/// accepts). Per surface, since the web build ships before the phone's.
	pub nexts_before: Vec<String>,
	/// More `left`s than [`WINDOW_READ`](crate::WINDOW_READ) fell in the
	/// window, and only the newest were read.
	pub partial: bool,
	pub debrief: Debrief,
}

/// The window's day edges: `[from, to)` for each of its days, as stored
/// instants, oldest first.
fn day_edges(week: IsoWeek) -> Vec<(String, String)> {
	let last = week.monday() + Duration::days(6);
	(0..WINDOW_DAYS)
		.rev()
		.map(|back| {
			let day = last - Duration::days(back);
			(IsoWeek::midnight(day), IsoWeek::midnight(day + Duration::days(1)))
		})
		.collect()
}

/// The median of `sorted`, or `None` when it holds fewer than [`UNDER`].
fn median<T: Copy>(sorted: &[T], halve: impl Fn(T, T) -> T) -> Option<T> {
	if sorted.len() < UNDER {
		return None;
	}
	let mid = sorted.len() / 2;
	Some(if sorted.len() % 2 == 1 { sorted[mid] } else { halve(sorted[mid - 1], sorted[mid]) })
}

/// `subject`'s friction measures over the 28 days ending with `week`.
///
/// `first` is home's first engagement event of the subject's, as
/// [`weekly`](crate::weekly) read it.
///
/// # Errors
/// [`EventError::Storage`] for any `sqlx` failure.
pub async fn friction(conn: &mut SqliteConnection, subject: &str, week: IsoWeek, first: Option<&str>) -> Result<Friction, EventError> {
	let edges = day_edges(week);
	let (from, to) = match (edges.first(), edges.last()) {
		(Some((from, _)), Some((_, to))) => (from.clone(), to.clone()),
		_ => return Err(EventError::Unreadable),
	};
	let measured_days = first.map_or(0, |first| edges.iter().filter(|(_, end)| first < end.as_str()).count());
	let days = serde_json::to_value(&edges).map(|json| json.to_string()).map_err(|_| EventError::Unreadable)?;

	let opened = sqlx::query_as!(
		Opened,
		r#"SELECT pipeline AS "pipeline!: String", affordance AS "affordance!: String", surface AS "surface!: String",
			COUNT(DISTINCT day) AS "days!: i64"
		FROM (SELECT d.key AS day, json_extract(e.body, '$.pipeline') AS pipeline, json_extract(e.body, '$.affordance') AS affordance,
				json_extract(e.body, '$.surface') AS surface
			FROM json_each(?2) AS d JOIN app_event AS e
				ON e.subject_id = ?1 AND e.family = 'engagement'
				AND e.at >= json_extract(d.value, '$[0]') AND e.at < json_extract(d.value, '$[1]')
			WHERE json_extract(e.body, '$.kind') IN ('offered', 'taken'))
		GROUP BY pipeline, affordance, surface
		ORDER BY pipeline, affordance, surface"#,
		subject,
		days
	)
	.fetch_all(&mut *conn)
	.await?;

	let (places, partial) = places(conn, subject, &from, &to).await?;
	// Per surface, newest first from the window's start: once builds send
	// `next`, the first few rows back hold one, and the read stops there.
	let mut nexts_before = Vec::new();
	for surface in ["apk", "web"] {
		let held = sqlx::query_scalar!(
			r#"SELECT at AS "at!: String" FROM app_event
			WHERE subject_id = ?1 AND family = 'engagement' AND at <= ?2
				AND json_extract(body, '$.kind') = 'left' AND json_extract(body, '$.surface') = ?3
				AND json_extract(body, '$.next') IS NOT NULL
			ORDER BY at DESC LIMIT 1"#,
			subject,
			from,
			surface
		)
		.fetch_optional(&mut *conn)
		.await?;
		if held.is_some() {
			nexts_before.push(surface.to_owned());
		}
	}

	let frictions = jobs_repo::debrief_frictions(conn, subject, &from, &to).await?;
	#[allow(clippy::cast_precision_loss)] // 1 to 5
	let halves: Vec<f64> = frictions.iter().map(|&friction| friction as f64).collect();
	let debrief = Debrief {
		said: i64::try_from(frictions.len()).unwrap_or(i64::MAX),
		median: median(&halves, f64::midpoint),
	};

	Ok(Friction {
		from,
		to,
		measured_days: i64::try_from(measured_days).unwrap_or(i64::MAX),
		opened,
		places,
		nexts_before,
		partial,
		debrief,
	})
}

/// Every `left` in `[from, to)`, grouped per affordance and place.
async fn places(conn: &mut SqliteConnection, subject: &str, from: &str, to: &str) -> Result<(Vec<PlaceTime>, bool), EventError> {
	let limit = crate::WINDOW_READ + 1;
	let rows = sqlx::query!(
		r#"SELECT pipeline AS "pipeline!: String", affordance AS "affordance!: String", surface AS "surface!: String",
			place AS "place!: String", ms AS "ms!: i64", moved AS "moved!: bool", at AS "at!: String", next AS "next: String"
		FROM (SELECT l.at AS at, json_extract(l.body, '$.pipeline') AS pipeline, json_extract(l.body, '$.affordance') AS affordance,
				json_extract(l.body, '$.surface') AS surface, json_extract(l.body, '$.place') AS place,
				json_extract(l.body, '$.next') AS next,
				CAST(ROUND((julianday(l.at) - julianday(json_extract(l.body, '$.startedAt'))) * 86400000) AS INTEGER) AS ms,
				EXISTS (SELECT 1 FROM app_event AS t
					WHERE t.subject_id = l.subject_id AND t.family = 'engagement' AND t.at = l.at
					AND json_extract(t.body, '$.kind') = 'taken'
					AND json_extract(t.body, '$.pipeline') = json_extract(l.body, '$.pipeline')
					AND json_extract(t.body, '$.affordance') = json_extract(l.body, '$.affordance')
					AND json_extract(t.body, '$.surface') = json_extract(l.body, '$.surface')) AS moved
			FROM app_event AS l
			WHERE l.subject_id = ?1 AND l.family = 'engagement' AND l.at >= ?2 AND l.at < ?3
				AND json_extract(l.body, '$.kind') = 'left'
				AND json_extract(l.body, '$.place') IS NOT NULL AND json_extract(l.body, '$.startedAt') IS NOT NULL
			ORDER BY l.at DESC
			LIMIT ?4)
		ORDER BY pipeline, affordance, surface, place, ms"#,
		subject,
		from,
		to,
		limit
	)
	.fetch_all(&mut *conn)
	.await?;
	let mut rows = rows;
	let partial = i64::try_from(rows.len()).unwrap_or(i64::MAX) > crate::WINDOW_READ;
	if partial {
		// The one row past the read is the oldest `left`, wherever the
		// place order put it, so what is read is exactly the newest.
		if let Some(oldest) = rows.iter().enumerate().min_by(|(_, a), (_, b)| a.at.cmp(&b.at)).map(|(index, _)| index) {
			rows.remove(oldest);
		}
	}

	let mut places: Vec<PlaceTime> = Vec::new();
	let mut times: Vec<Vec<i64>> = Vec::new();
	let mut onward: Vec<BTreeMap<String, i64>> = Vec::new();
	for row in rows {
		let same = places
			.last()
			.is_some_and(|last| last.pipeline == row.pipeline && last.affordance == row.affordance && last.surface == row.surface && last.place == row.place);
		if !same {
			places.push(PlaceTime {
				pipeline: row.pipeline,
				affordance: row.affordance,
				surface: row.surface,
				place: row.place,
				left: 0,
				moved_on: 0,
				median_ms: None,
				nexts: Vec::new(),
			});
			times.push(Vec::new());
			onward.push(BTreeMap::new());
		}
		if let (Some(place), Some(time), Some(went)) = (places.last_mut(), times.last_mut(), onward.last_mut()) {
			place.left += 1;
			place.moved_on += i64::from(row.moved);
			time.push(row.ms);
			if let Some(next) = row.next {
				*went.entry(next).or_insert(0) += 1;
			}
		}
	}
	for ((place, time), went) in places.iter_mut().zip(&times).zip(onward) {
		place.median_ms = median(time, i64::midpoint);
		place.nexts = went.into_iter().map(|(next, left)| NextCount { next, left }).collect();
	}
	Ok((places, partial))
}

#[cfg(test)]
mod tests {
	use super::{day_edges, median, IsoWeek};

	#[test]
	fn the_window_is_the_28_owner_days_ending_with_the_week() {
		let edges = day_edges(IsoWeek::parse("2026-W46").unwrap());
		assert_eq!(edges.len(), 28);
		assert_eq!(edges[0].0, "2026-10-19T07:00:00.000Z", "Monday 2026-10-19, daylight time");
		assert_eq!(edges[27].1, "2026-11-16T08:00:00.000Z", "the week's end");
		assert!(edges.windows(2).all(|pair| pair[0].1 == pair[1].0), "the days are contiguous");
		assert_eq!(
			edges[13],
			("2026-11-01T07:00:00.000Z".to_owned(), "2026-11-02T08:00:00.000Z".to_owned()),
			"the clock change's day is 25 hours"
		);
	}

	#[test]
	fn a_median_is_said_from_five_values() {
		assert_eq!(median(&[1, 2, 3, 4], i64::midpoint), None);
		assert_eq!(median(&[1, 2, 3, 4, 9], i64::midpoint), Some(3));
		assert_eq!(median(&[1, 2, 3, 4, 9, 10], i64::midpoint), Some(3));
		assert_eq!(median(&[1.0, 2.0, 3.0, 4.0, 4.0, 5.0], f64::midpoint), Some(3.5));
	}
}
