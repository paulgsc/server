//! What home keeps (`crate::storage`, STORAGE1 #542): the database's size
//! on the 10 s sweep, and the daily sample's figures.
//!
//! Two cadences, two sets of names:
//!
//! - `sqlite_db_bytes`, `sqlite_freelist_bytes` and `sqlite_wal_bytes`, set
//!   on `metrics::periodic`'s sweep. The WAL is its own series so a checkpoint's rise and fall never reads as the
//!   database growing.
//! - `storage_*`, set from `storage_sample` after each hourly check. The
//!   windows (7 and 90 days) are longer than Prometheus keeps (200 h), so the
//!   server reads them from the table and exports the results; Grafana only
//!   divides. Labels are a kind and a table's or index's name: the schema's,
//!   bounded by it, never anything a person wrote.

use std::time::Duration;
use storage_repo::{Figures, Kind, Runway, Window};

/// The 10 s sweep's three sizes. `wal` is `None` when the database has no
/// file (in memory), and the series keeps its last value.
pub fn record_footprint(database: i64, freelist: i64, wal: Option<u64>) {
	metrics::gauge!("sqlite_db_bytes").set(bytes(database));
	metrics::gauge!("sqlite_freelist_bytes").set(bytes(freelist));
	if let Some(wal) = wal {
		#[allow(clippy::cast_precision_loss)] // a file's length stays far inside f64's exact range
		metrics::gauge!("sqlite_wal_bytes").set(wal as f64);
	}
}

/// The daily pass's deadline, published once at spawn so the dashboard
/// draws the pass's cost against its bound rather than a default.
pub fn record_pass_deadline(deadline: Duration) {
	metrics::gauge!("storage_sample_pass_deadline_seconds").set(deadline.as_secs_f64());
}

/// How long the last pass that measured took, kept or stopped.
pub fn record_pass_duration(elapsed: Duration) {
	metrics::gauge!("storage_sample_last_pass_duration_seconds").set(elapsed.as_secs_f64());
}

/// Whether the last check failed: 1 until one succeeds. A check that finds
/// the day already held succeeds.
pub fn record_failing(failing: bool) {
	metrics::gauge!("storage_sample_failing").set(if failing { 1.0 } else { 0.0 });
}

/// Everything the newest set and its windows say.
///
/// A figure with nothing to say is NaN, which the panels show as "no data"
/// (not measured) rather than 0: a rate, and each table's growth, while the
/// window reaches no earlier day; the runway while there is no rate. NaN
/// rather than left alone, so a figure that stops being known never keeps
/// its last value. A runway of -1 is "not growing".
pub fn record_figures(figures: &Figures) {
	if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&figures.sampled_at) {
		#[allow(clippy::cast_precision_loss)] // seconds since 1970
		metrics::gauge!("storage_sample_timestamp_seconds").set(at.timestamp() as f64);
	}
	for size in &figures.sizes {
		metrics::gauge!("storage_sample_bytes", "kind" => size.kind.as_str(), "name" => size.name.clone()).set(bytes(size.bytes));
	}
	window("7d", &figures.short);
	window("90d", &figures.long);
	if figures.growth.is_empty() {
		for size in figures.sizes.iter().filter(|size| matches!(size.kind, Kind::Table | Kind::Index)) {
			metrics::gauge!("storage_table_growth_7d_bytes", "kind" => size.kind.as_str(), "name" => size.name.clone()).set(f64::NAN);
		}
	}
	for growth in &figures.growth {
		metrics::gauge!("storage_table_growth_7d_bytes", "kind" => growth.kind.as_str(), "name" => growth.name.clone()).set(bytes(growth.bytes));
	}
	match figures.runway {
		Runway::Unknown => metrics::gauge!("storage_runway_days").set(f64::NAN),
		Runway::NotGrowing => metrics::gauge!("storage_runway_days").set(-1.0),
		Runway::Days(days) => metrics::gauge!("storage_runway_days").set(days),
	}
}

fn window(label: &'static str, window: &Window) {
	#[allow(clippy::cast_precision_loss)] // a span in days
	metrics::gauge!("storage_growth_span_days", "window" => label).set(window.span_days as f64);
	metrics::gauge!("storage_growth_bytes_per_day", "window" => label).set(window.bytes_per_day.unwrap_or(f64::NAN));
}

#[allow(clippy::cast_precision_loss)] // byte counts stay far inside f64's exact range
const fn bytes(bytes: i64) -> f64 {
	bytes as f64
}
