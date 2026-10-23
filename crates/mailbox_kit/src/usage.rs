//! How much of its free-plan allowance the mailbox's D1 database has used
//! today, from Cloudflare's GraphQL Analytics API.
//!
//! The mailbox runs on Cloudflare's free plan with no payment method on
//! purpose: past 5,000,000 rows read or 100,000 rows written in a UTC day,
//! every query fails until 00:00 UTC, and nothing is ever billed. [`Usage`]
//! says how close today is, so a dashboard can show it before the limit
//! does. The token is a read-only "Account Analytics" token, a different one
//! from the mailbox's own, and the same [`Token`] type keeps it from
//! printing.

use crate::transport::{MailboxTransport, Method, Request, Token};
use serde::Deserialize;
use std::fmt;

/// Cloudflare's GraphQL endpoint.
pub const ENDPOINT: &str = "https://api.cloudflare.com/client/v4/graphql";

/// Rows a free-plan D1 database may read in a UTC day.
pub const FREE_ROWS_READ_PER_DAY: u64 = 5_000_000;
/// Rows a free-plan D1 database may write in a UTC day.
pub const FREE_ROWS_WRITTEN_PER_DAY: u64 = 100_000;
/// Bytes a free-plan D1 database may hold.
pub const FREE_DATABASE_BYTES: u64 = 500_000_000;

/// The one query: one UTC day's sums for one database, and its latest size.
///
/// Dataset and field names are from Cloudflare's D1 analytics docs
/// (`d1AnalyticsAdaptiveGroups`, `d1StorageAdaptiveGroups`); the docs name
/// `readQueries`/`writeQueries` in `sum` and `rowsRead`/`rowsWritten` among
/// D1's metrics. The fixture in this module's tests is shaped from them and
/// has not yet been checked against a live answer: introspect before trusting
/// a zero (the deploy notes in the PR that added this say how).
pub const QUERY: &str = "query D1Usage($account: String!, $database: String!, $day: Date!) {
  viewer {
    accounts(filter: { accountTag: $account }) {
      usage: d1AnalyticsAdaptiveGroups(limit: 1, filter: { databaseId: $database, date_geq: $day, date_leq: $day }) {
        dimensions { date }
        sum { readQueries writeQueries rowsRead rowsWritten }
        quantiles { queryBatchTimeMsP90 }
      }
      storage: d1StorageAdaptiveGroups(limit: 1, filter: { databaseId: $database, date_leq: $day }, orderBy: [date_DESC]) {
        dimensions { date }
        max { databaseSizeBytes }
      }
    }
  }
}";

/// One UTC day's use of one database.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
	pub rows_read: u64,
	pub rows_written: u64,
	pub read_queries: u64,
	pub write_queries: u64,
	/// `None` when Cloudflare has no size for the database yet.
	pub database_bytes: Option<u64>,
	/// `None` on a day with no queries.
	pub query_batch_p90_ms: Option<f64>,
}

/// Why a read came to nothing. None of these holds the token or the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageError {
	/// `401`/`403`, or an answer with no account: the token cannot read it.
	Unauthorized,
	/// Cloudflare answered with GraphQL errors; its first message, cut short.
	Api(String),
	/// Any other status.
	Http(u16),
	/// Nothing came back: the transport's error, its URL stripped.
	Transport(String),
	/// An answer that does not read as the query asks.
	Decode(&'static str),
}

impl UsageError {
	/// A fixed word per kind, for a metric label or a log line.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Unauthorized => "unauthorized",
			Self::Api(_) => "api",
			Self::Http(_) => "http",
			Self::Transport(_) => "transport",
			Self::Decode(_) => "decode",
		}
	}
}

impl fmt::Display for UsageError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Unauthorized => f.write_str("the analytics token cannot read this account"),
			Self::Api(message) => write!(f, "Cloudflare refused the query: {message}"),
			Self::Http(status) => write!(f, "Cloudflare answered {status}"),
			Self::Transport(err) => write!(f, "Cloudflare could not be reached: {err}"),
			Self::Decode(what) => write!(f, "Cloudflare's answer does not read: {what}"),
		}
	}
}

impl std::error::Error for UsageError {}

/// One account's analytics, for one database, through one token.
pub struct Analytics<T> {
	account: String,
	database: String,
	token: Token,
	transport: T,
}

// Hand-written: neither the account id nor the token belongs in a log line.
impl<T> fmt::Debug for Analytics<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Analytics").finish_non_exhaustive()
	}
}

#[derive(Deserialize)]
struct Answer {
	data: Option<Data>,
	#[serde(default)]
	errors: Option<Vec<Message>>,
}

#[derive(Deserialize)]
struct Message {
	message: String,
}

#[derive(Deserialize)]
struct Data {
	viewer: Viewer,
}

#[derive(Deserialize)]
struct Viewer {
	accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
	usage: Vec<UsageGroup>,
	storage: Vec<StorageGroup>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageGroup {
	sum: Sums,
	quantiles: Option<Quantiles>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sums {
	read_queries: u64,
	write_queries: u64,
	rows_read: u64,
	rows_written: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Quantiles {
	query_batch_time_ms_p90: Option<f64>,
}

#[derive(Deserialize)]
struct StorageGroup {
	max: StorageMax,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageMax {
	database_size_bytes: Option<u64>,
}

impl<T: MailboxTransport> Analytics<T> {
	#[must_use]
	pub const fn new(account: String, database: String, token: Token, transport: T) -> Self {
		Self {
			account,
			database,
			token,
			transport,
		}
	}

	/// The database's use on `day` (`YYYY-MM-DD`, a UTC day).
	///
	/// # Errors
	/// A [`UsageError`] naming what failed.
	pub async fn day(&self, day: &str) -> Result<Usage, UsageError> {
		let body = serde_json::json!({
			"query": QUERY,
			"variables": { "account": self.account, "database": self.database, "day": day },
		});
		let request = Request {
			method: Method::Post,
			url: ENDPOINT.to_owned(),
			token: self.token.clone(),
			body: Some(body.to_string().into_bytes()),
		};
		let response = self.transport.send(request).await.map_err(|err| UsageError::Transport(err.to_string()))?;
		match response.status {
			200 => read(&response.body),
			401 | 403 => Err(UsageError::Unauthorized),
			status => Err(UsageError::Http(status)),
		}
	}
}

fn read(body: &[u8]) -> Result<Usage, UsageError> {
	let answer: Answer = serde_json::from_slice(body).map_err(|_| UsageError::Decode("not a GraphQL answer of the query's shape"))?;
	if let Some(first) = answer.errors.as_deref().and_then(<[Message]>::first) {
		return Err(UsageError::Api(first.message.chars().take(200).collect()));
	}
	let account = answer.data.and_then(|data| data.viewer.accounts.into_iter().next()).ok_or(UsageError::Unauthorized)?;
	let mut usage = Usage {
		database_bytes: account.storage.first().and_then(|group| group.max.database_size_bytes),
		..Usage::default()
	};
	// No group at all is a day with no queries: zeros, not a failure.
	if let Some(group) = account.usage.first() {
		usage.rows_read = group.sum.rows_read;
		usage.rows_written = group.sum.rows_written;
		usage.read_queries = group.sum.read_queries;
		usage.write_queries = group.sum.write_queries;
		usage.query_batch_p90_ms = group.quantiles.as_ref().and_then(|quantiles| quantiles.query_batch_time_ms_p90);
	}
	Ok(usage)
}

#[cfg(test)]
mod tests {
	use super::{Analytics, Usage, UsageError, ENDPOINT};
	use crate::transport::{MailboxTransport, Method, Request, Response, Token};
	use std::sync::Mutex;

	const ACCOUNT: &str = "account-id-that-must-never-print";
	const SECRET: &str = "analytics-token-that-must-never-print";

	/// A recorded answer, shaped from Cloudflare's documented D1 datasets.
	const ANSWER: &str = r#"{
		"data": { "viewer": { "accounts": [{
			"usage": [{
				"dimensions": { "date": "2026-10-23" },
				"sum": { "readQueries": 41, "writeQueries": 9, "rowsRead": 1234, "rowsWritten": 27 },
				"quantiles": { "queryBatchTimeMsP90": 1.7 }
			}],
			"storage": [{ "dimensions": { "date": "2026-10-23" }, "max": { "databaseSizeBytes": 126976 } }]
		}] } },
		"errors": null
	}"#;

	struct Fake {
		answer: (u16, String),
		sent: Mutex<Vec<Request>>,
	}

	impl MailboxTransport for Fake {
		type Error = &'static str;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			self.sent.lock().unwrap().push(request);
			Ok(Response {
				status: self.answer.0,
				body: self.answer.1.as_bytes().to_vec(),
			})
		}
	}

	fn analytics(status: u16, body: &str) -> Analytics<Fake> {
		let fake = Fake {
			answer: (status, body.to_owned()),
			sent: Mutex::new(Vec::new()),
		};
		Analytics::new(ACCOUNT.to_owned(), "db-id".to_owned(), Token::new(SECRET.to_owned()), fake)
	}

	#[tokio::test]
	async fn a_day_reads_its_sums_and_the_latest_size() {
		let analytics = analytics(200, ANSWER);
		assert_eq!(
			analytics.day("2026-10-23").await.unwrap(),
			Usage {
				rows_read: 1234,
				rows_written: 27,
				read_queries: 41,
				write_queries: 9,
				database_bytes: Some(126_976),
				query_batch_p90_ms: Some(1.7),
			}
		);
		let request = &analytics.transport.sent.lock().unwrap()[0];
		assert_eq!((request.method, request.url.as_str()), (Method::Post, ENDPOINT));
		let body: serde_json::Value = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
		assert_eq!(body["variables"], serde_json::json!({ "account": ACCOUNT, "database": "db-id", "day": "2026-10-23" }));
		assert_eq!(request.token.expose(), SECRET);
	}

	#[tokio::test]
	async fn a_day_with_no_queries_is_zeros_not_a_failure() {
		let quiet = r#"{"data":{"viewer":{"accounts":[{"usage":[],"storage":[]}]}}}"#;
		assert_eq!(analytics(200, quiet).day("2026-10-23").await.unwrap(), Usage::default());
	}

	#[tokio::test]
	async fn each_failure_is_named_by_its_kind() {
		let refused = r#"{"data":null,"errors":[{"message":"unknown field rowsRead"}]}"#;
		assert_eq!(analytics(200, refused).day("d").await, Err(UsageError::Api("unknown field rowsRead".to_owned())));
		let no_account = r#"{"data":{"viewer":{"accounts":[]}}}"#;
		assert_eq!(
			analytics(200, no_account).day("d").await,
			Err(UsageError::Unauthorized),
			"a token that cannot see the account"
		);
		assert_eq!(analytics(403, "{}").day("d").await, Err(UsageError::Unauthorized));
		assert_eq!(analytics(502, "").day("d").await, Err(UsageError::Http(502)));
		assert_eq!(analytics(200, "<html>").day("d").await.unwrap_err().kind(), "decode");
	}

	#[tokio::test]
	async fn neither_the_token_nor_the_account_prints() {
		let analytics = analytics(200, ANSWER);
		analytics.day("2026-10-23").await.unwrap();
		let mut text = String::new();
		std::fmt::Write::write_fmt(&mut text, format_args!("{analytics:?} {:?}", analytics.transport.sent.lock().unwrap()[0].token)).unwrap();
		assert!(!text.contains(SECRET) && !text.contains(ACCOUNT), "{text}");
	}
}
