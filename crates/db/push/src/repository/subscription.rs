use push_kit::PushSubscription;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// What a person agreed to receive.
///
/// A closed set, because "subscribe to notifications" is not a thing anyone can
/// meaningfully consent to — they agree to *particular* interruptions, and the
/// options they were shown have to be the options the sender honours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Topic {
	/// The beta's only one: a session was prepared and is waiting.
	LessonReady,
	/// Coaching that follows from how sessions went.
	Coaching,
	/// New curriculum or exercises.
	NewMaterial,
}

impl Topic {
	pub const ALL: &'static [Self] = &[Self::LessonReady, Self::Coaching, Self::NewMaterial];

	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::LessonReady => "lesson-ready",
			Self::Coaching => "coaching",
			Self::NewMaterial => "new-material",
		}
	}

	/// Total parse, for the same reason every stored enum here has one: a value
	/// written by a release that offered a topic this one does not.
	#[must_use]
	pub fn parse(raw: &str) -> Option<Self> {
		match raw {
			"lesson-ready" => Some(Self::LessonReady),
			"coaching" => Some(Self::Coaching),
			"new-material" => Some(Self::NewMaterial),
			_ => None,
		}
	}
}

/// The explicit grant that has to exist before anything is sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consent {
	pub subject_id: String,
	pub topics: Vec<Topic>,
	/// When they said yes. Kept because "we asked" is a claim that should be
	/// answerable with a timestamp rather than an assumption.
	pub consented_at: String,
}

/// A subscription as stored, with the delivery bookkeeping the browser never
/// sends and should never see.
#[derive(Debug, Clone)]
pub struct StoredSubscription {
	pub subscription: PushSubscription,
	pub subject_id: String,
	pub topics: Vec<Topic>,
	pub created_at: String,
	pub last_success_at: Option<String>,
	pub last_failure_at: Option<String>,
	pub failure_count: i64,
}

impl StoredSubscription {
	/// Whether this device agreed to hear about this.
	#[must_use]
	pub fn accepts(&self, topic: Topic) -> bool {
		self.topics.contains(&topic)
	}
}

#[derive(FromRow)]
struct SubscriptionRow {
	endpoint: String,
	subject_id: String,
	p256dh: String,
	auth: String,
	topics: String,
	created_at: String,
	last_success_at: Option<String>,
	last_failure_at: Option<String>,
	failure_count: i64,
}

impl From<SubscriptionRow> for StoredSubscription {
	fn from(row: SubscriptionRow) -> Self {
		// An unparseable topic list, or a topic name this build does not know,
		// degrades to "receives nothing" rather than "receives everything".
		// Consent that cannot be read is not consent.
		let topics = serde_json::from_str::<Vec<String>>(&row.topics)
			.unwrap_or_default()
			.iter()
			.filter_map(|raw| Topic::parse(raw))
			.collect();

		Self {
			subscription: PushSubscription {
				endpoint: row.endpoint,
				keys: push_kit::SubscriptionKeys {
					p256dh: row.p256dh,
					auth: row.auth,
				},
			},
			subject_id: row.subject_id,
			topics,
			created_at: row.created_at,
			last_success_at: row.last_success_at,
			last_failure_at: row.last_failure_at,
			failure_count: row.failure_count,
		}
	}
}

pub struct PushSubscriptionRepository {
	pool: SqlitePool,
}

impl PushSubscriptionRepository {
	#[must_use]
	pub const fn new(pool: SqlitePool) -> Self {
		Self { pool }
	}

	/// Store a subscription, or refresh one already stored.
	///
	/// Consent is an argument rather than a default: there is no path through
	/// this crate that creates a subscription nobody agreed to. Re-subscribing
	/// can return the same endpoint with rotated keys, so the conflict branch
	/// overwrites both and resets the failure tally — a fresh subscription is
	/// not carrying the previous one's failures.
	///
	/// # Errors
	/// Propagates any `sqlx` failure, or a failure to serialize the topic list.
	pub async fn upsert(&self, subscription: &PushSubscription, consent: &Consent, now: &str) -> Result<(), sqlx::Error> {
		// Infallible: a `Vec<&str>` always serializes. Written this way rather
		// than with `expect` so a future topic type that can fail cannot
		// introduce a panic here silently.
		// `serde_json::to_string` is on clippy.toml's disallowed list to keep
		// eager serialization out of tracing calls. This is the column itself.
		#[allow(clippy::disallowed_methods)]
		let topics = {
			let names: Vec<&str> = consent.topics.iter().map(|topic| topic.as_str()).collect();
			serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_owned())
		};

		sqlx::query!(
			r#"
			INSERT INTO push_subscriptions
			    (endpoint, subject_id, p256dh, auth, topics, consented_at, created_at, failure_count)
			VALUES (?, ?, ?, ?, ?, ?, ?, 0)
			ON CONFLICT(endpoint) DO UPDATE SET
			    subject_id      = excluded.subject_id,
			    p256dh          = excluded.p256dh,
			    auth            = excluded.auth,
			    topics          = excluded.topics,
			    consented_at    = excluded.consented_at,
			    failure_count   = 0,
			    last_failure_at = NULL
			"#,
			subscription.endpoint,
			consent.subject_id,
			subscription.keys.p256dh,
			subscription.keys.auth,
			topics,
			consent.consented_at,
			now,
		)
		.execute(&self.pool)
		.await?;

		Ok(())
	}

	/// Every device this subject consented on.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn for_subject(&self, subject_id: &str) -> Result<Vec<StoredSubscription>, sqlx::Error> {
		let rows = sqlx::query_as!(
			SubscriptionRow,
			r#"
			SELECT endpoint as "endpoint!", subject_id, p256dh, auth, topics, created_at,
			       last_success_at, last_failure_at, failure_count
			FROM push_subscriptions
			WHERE subject_id = ?
			ORDER BY created_at ASC
			"#,
			subject_id
		)
		.fetch_all(&self.pool)
		.await?;

		Ok(rows.into_iter().map(Into::into).collect())
	}

	/// Every subject who has ever subscribed, deduplicated.
	///
	/// The one caller is the first-contact backfill run at startup —
	/// reconciling subscriptions that predate `waker::first_contact` against
	/// `engagement_gate`. [`Self::for_subject`] is not reusable here: this asks
	/// for the distinct *owners*, not one owner's devices.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn distinct_subject_ids(&self) -> Result<Vec<String>, sqlx::Error> {
		sqlx::query_scalar!("SELECT DISTINCT subject_id FROM push_subscriptions").fetch_all(&self.pool).await
	}

	/// Remove a subscription. Idempotent: an explicit opt-out and a `410 Gone`
	/// are saying the same thing, and neither is an error when it has already
	/// happened.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn delete_by_endpoint(&self, endpoint: &str) -> Result<u64, sqlx::Error> {
		let result = sqlx::query!("DELETE FROM push_subscriptions WHERE endpoint = ?", endpoint).execute(&self.pool).await?;
		Ok(result.rows_affected())
	}

	/// Record that the push service accepted a message.
	///
	/// Note what this does *not* record: delivery. `push_kit` is explicit that
	/// a `201` means accepted, and this column inherits that meaning exactly.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn record_success(&self, endpoint: &str, now: &str) -> Result<(), sqlx::Error> {
		sqlx::query!("UPDATE push_subscriptions SET last_success_at = ?, failure_count = 0 WHERE endpoint = ?", now, endpoint)
			.execute(&self.pool)
			.await?;
		Ok(())
	}

	/// Record a non-acceptance. The count distinguishes a push service having a
	/// bad afternoon from a subscription that is quietly dead but not yet
	/// reporting `410`.
	///
	/// # Errors
	/// Propagates any `sqlx` failure.
	pub async fn record_failure(&self, endpoint: &str, now: &str) -> Result<(), sqlx::Error> {
		sqlx::query!(
			"UPDATE push_subscriptions SET last_failure_at = ?, failure_count = failure_count + 1 WHERE endpoint = ?",
			now,
			endpoint
		)
		.execute(&self.pool)
		.await?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{StoredSubscription, Topic};
	use push_kit::{PushSubscription, SubscriptionKeys};

	fn stored(topics: Vec<Topic>) -> StoredSubscription {
		StoredSubscription {
			subscription: PushSubscription {
				endpoint: "https://example.com/p".to_owned(),
				keys: SubscriptionKeys {
					p256dh: "a".to_owned(),
					auth: "b".to_owned(),
				},
			},
			subject_id: "s".to_owned(),
			topics,
			created_at: "now".to_owned(),
			last_success_at: None,
			last_failure_at: None,
			failure_count: 0,
		}
	}

	#[test]
	fn a_device_only_accepts_what_it_agreed_to() {
		let device = stored(vec![Topic::LessonReady]);
		assert!(device.accepts(Topic::LessonReady));
		assert!(!device.accepts(Topic::Coaching));
	}

	#[test]
	fn consenting_to_nothing_is_a_real_state_not_a_fallback_to_everything() {
		let device = stored(Vec::new());
		for topic in Topic::ALL {
			assert!(!device.accepts(*topic));
		}
	}

	#[test]
	fn topics_round_trip_and_an_unknown_one_is_dropped() {
		for topic in Topic::ALL {
			assert_eq!(Topic::parse(topic.as_str()), Some(*topic));
		}
		assert_eq!(Topic::parse("marketing-blast"), None);
	}
}
