use crate::transport::{MailboxTransport, Method, Request, Response, Token};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;

/// The most items one listing asks for: the mailbox's own ceiling.
pub const PAGE_LIMIT: u32 = 50;

/// The mailbox's folders, each with its own name pattern and schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folder {
	/// The routine's digests, `digest-YYYY-MM-DD.json`.
	Brief,
	/// The phone's decisions, `decisions-YYYY-MM-DD(-n).json`.
	Decisions,
	/// The phone's scouts, `scouts-YYYYMMDDTHHMMSS.json`.
	Scout,
	/// The reflections routine's word sets, `scene-<reflection>.json`.
	Scene,
	/// The reflections routine's dramas, `drama-<stamp>.json`.
	Drama,
}

impl Folder {
	/// Every folder, in the order a pass visits them.
	pub const ALL: [Self; 5] = [Self::Brief, Self::Decisions, Self::Scout, Self::Scene, Self::Drama];

	/// The name in the mailbox's URLs, and in `mailbox_repo`'s rows.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Brief => "brief",
			Self::Decisions => "decisions",
			Self::Scout => "scout",
			Self::Scene => "scene",
			Self::Drama => "drama",
		}
	}
}

/// One item in a listing, as the mailbox describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemMeta {
	/// The mailbox's own order. It has gaps: page with the highest seen.
	pub seq: i64,
	pub name: String,
	pub size: u64,
	/// Lowercase hex SHA-256 of the stored bytes.
	pub sha256: String,
	pub content_type: String,
	pub put_at: String,
	/// When it was last put back from home, else `put_at`. A mailbox from
	/// before archiving does not send it.
	pub live_since: Option<String>,
}

impl ItemMeta {
	/// When the item went live: what archiving counts its age from.
	#[must_use]
	pub fn live_since(&self) -> &str {
		self.live_since.as_deref().unwrap_or(&self.put_at)
	}
}

/// An archived item the routine asked for again.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wanted {
	pub name: String,
	pub asked_at: String,
}

/// What an archive did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
	/// The mailbox kept a row about the item and dropped its bytes.
	Archived,
	/// It was archived already.
	Unchanged,
}

/// What putting an archived item back did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restore {
	/// The bytes are live again, under a new `seq`.
	Restored { seq: i64 },
	/// The item was live already.
	Unchanged { seq: i64 },
	/// The mailbox archived other bytes under the name.
	Conflict,
}

/// What a put did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Put {
	/// The mailbox did not hold the name: it holds these bytes now.
	Added { seq: i64 },
	/// The mailbox already held these very bytes under the name.
	Unchanged { seq: i64 },
	/// The mailbox holds other bytes under the name, which it never replaces.
	Conflict,
}

/// Why a call came to nothing. None of these holds a token, a URL or a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailboxError {
	/// `401`: no token, or not one the mailbox knows.
	Unauthorized,
	/// `403`: the token may not do this to this folder.
	Forbidden,
	/// `404`: no such item, or no such route.
	NotFound,
	/// `422`: the mailbox refused the request, in its own words.
	Refused(String),
	/// `413`: the body is over the mailbox's ceiling.
	TooBig,
	/// Any other status the call does not expect.
	Http(u16),
	/// Nothing came back: the transport's error, its URL already stripped.
	Transport(String),
	/// An answer that does not read as the API says it should.
	Decode(&'static str),
	/// A fetched body whose size or SHA-256 is not what the listing said.
	Mismatch,
	/// A name that could not be put in a URL as it stands.
	BadName,
}

impl MailboxError {
	/// A fixed word per kind, for a metric label or a log line.
	#[must_use]
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Unauthorized => "unauthorized",
			Self::Forbidden => "forbidden",
			Self::NotFound => "not_found",
			Self::Refused(_) => "refused",
			Self::TooBig => "too_big",
			Self::Http(_) => "http",
			Self::Transport(_) => "transport",
			Self::Decode(_) => "decode",
			Self::Mismatch => "mismatch",
			Self::BadName => "bad_name",
		}
	}
}

impl fmt::Display for MailboxError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Unauthorized => f.write_str("the mailbox does not know this token"),
			Self::Forbidden => f.write_str("this token may not do that in this folder"),
			Self::NotFound => f.write_str("the mailbox has no such item"),
			Self::Refused(why) => write!(f, "the mailbox refused it: {why}"),
			Self::TooBig => f.write_str("the body is over the mailbox's ceiling"),
			Self::Http(status) => write!(f, "the mailbox answered {status}"),
			Self::Transport(err) => write!(f, "the mailbox could not be reached: {err}"),
			Self::Decode(what) => write!(f, "the mailbox's answer does not read: {what}"),
			Self::Mismatch => f.write_str("a fetched item is not what its listing said"),
			Self::BadName => f.write_str("not a name an item can have"),
		}
	}
}

impl std::error::Error for MailboxError {}

/// The mailbox at one base URL, holding one token.
pub struct Mailbox<T> {
	base: String,
	token: Token,
	transport: T,
}

// Hand-written: neither the base URL (which names the owner's Worker) nor the
// token belongs in a log line.
impl<T> fmt::Debug for Mailbox<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Mailbox").finish_non_exhaustive()
	}
}

impl<T: MailboxTransport> Mailbox<T> {
	/// A mailbox at `base` (`https://…`, no path), reached through `transport`.
	///
	/// # Errors
	/// A `base` that is not `https://` with a host, or that carries a path,
	/// query or fragment, which would put requests somewhere `/v1` is not.
	pub fn new(base: &str, token: Token, transport: T) -> Result<Self, &'static str> {
		let base = base.trim_end_matches('/');
		let Some(host) = base.strip_prefix("https://") else {
			return Err("the mailbox URL must start with https://");
		};
		if host.is_empty() || host.contains(['/', '?', '#', '@']) || host.contains(char::is_whitespace) {
			return Err("the mailbox URL must be https:// and a host, with no path");
		}
		Ok(Self {
			base: base.to_owned(),
			token,
			transport,
		})
	}

	fn url(&self, parts: &[&str]) -> String {
		let mut url = self.base.clone() + "/v1";
		for part in parts {
			url.push('/');
			url.push_str(part);
		}
		url
	}

	async fn send(&self, method: Method, url: String, body: Option<Vec<u8>>) -> Result<Response, MailboxError> {
		self.send_matching(method, url, body, None).await
	}

	async fn send_matching(&self, method: Method, url: String, body: Option<Vec<u8>>, if_match: Option<String>) -> Result<Response, MailboxError> {
		let request = Request {
			method,
			url,
			token: self.token.clone(),
			body,
			if_match,
		};
		self.transport.send(request).await.map_err(|err| MailboxError::Transport(err.to_string()))
	}

	/// At most `limit` items of `folder` with a `seq` above `after`, oldest
	/// first: one page. Ask again from the highest `seq` returned, never
	/// `after + 1`, since `seq` has gaps.
	///
	/// # Errors
	/// The status's [`MailboxError`], [`MailboxError::Transport`], or
	/// [`MailboxError::Decode`] for an answer that is not `{ items }`.
	pub async fn list(&self, folder: Folder, after: i64, limit: u32) -> Result<Vec<ItemMeta>, MailboxError> {
		#[derive(Deserialize)]
		struct Listing {
			items: Vec<ItemMeta>,
		}
		let limit = limit.clamp(1, PAGE_LIMIT);
		let query = String::from("items?after=") + &after.max(0).to_string() + "&limit=" + &limit.to_string();
		let response = self.send(Method::Get, self.url(&[folder.as_str(), &query]), None).await?;
		if response.status != 200 {
			return Err(failure(&response));
		}
		let listing: Listing = serde_json::from_slice(&response.body).map_err(|_| MailboxError::Decode("a listing is not { items }"))?;
		Ok(listing.items)
	}

	/// The bytes of `item`, checked against its listed size and SHA-256.
	///
	/// # Errors
	/// [`MailboxError::BadName`] for a name a URL cannot carry as it stands,
	/// [`MailboxError::Mismatch`] for bytes that are not what the listing
	/// said, and the status's [`MailboxError`] or [`MailboxError::Transport`].
	pub async fn get(&self, folder: Folder, item: &ItemMeta) -> Result<Vec<u8>, MailboxError> {
		let name = plain_name(&item.name)?;
		let response = self.send(Method::Get, self.url(&[folder.as_str(), "items", name]), None).await?;
		if response.status != 200 {
			return Err(failure(&response));
		}
		let digest = hex::encode(Sha256::digest(&response.body));
		if u64::try_from(response.body.len()).ok() != Some(item.size) || digest != item.sha256.to_ascii_lowercase() {
			return Err(MailboxError::Mismatch);
		}
		Ok(response.body)
	}

	/// Put `body` under `name` in `folder`. The mailbox is append-only: a name
	/// it already holds with other bytes is [`Put::Conflict`], never replaced.
	///
	/// # Errors
	/// [`MailboxError::BadName`], the status's [`MailboxError`] (a `422` is
	/// [`MailboxError::Refused`], naming why), or [`MailboxError::Transport`].
	pub async fn put(&self, folder: Folder, name: &str, body: Vec<u8>) -> Result<Put, MailboxError> {
		#[derive(Deserialize)]
		struct Answer {
			change: String,
			seq: i64,
		}
		let name = plain_name(name)?;
		let response = self.send(Method::Put, self.url(&[folder.as_str(), "items", name]), Some(body)).await?;
		match response.status {
			200 | 201 => {
				let answer: Answer = serde_json::from_slice(&response.body).map_err(|_| MailboxError::Decode("a put's answer is not { change, seq }"))?;
				match (response.status, answer.change.as_str()) {
					(201, "added") => Ok(Put::Added { seq: answer.seq }),
					(200, "unchanged") => Ok(Put::Unchanged { seq: answer.seq }),
					_ => Err(MailboxError::Decode("a put's change does not match its status")),
				}
			}
			409 => Ok(Put::Conflict),
			_ => Err(failure(&response)),
		}
	}

	/// The routine's open asks for archived items in `folder`, oldest first,
	/// at most `limit`. Needs the archive token.
	///
	/// # Errors
	/// The status's [`MailboxError`], [`MailboxError::Transport`], or
	/// [`MailboxError::Decode`] for an answer that is not `{ items }`.
	pub async fn wanted(&self, folder: Folder, limit: u32) -> Result<Vec<Wanted>, MailboxError> {
		#[derive(Deserialize)]
		struct Listing {
			items: Vec<Wanted>,
		}
		let query = String::from("wanted?limit=") + &limit.clamp(1, PAGE_LIMIT).to_string();
		let response = self.send(Method::Get, self.url(&[folder.as_str(), &query]), None).await?;
		if response.status != 200 {
			return Err(failure(&response));
		}
		let listing: Listing = serde_json::from_slice(&response.body).map_err(|_| MailboxError::Decode("the asks are not { items }"))?;
		Ok(listing.items)
	}

	/// Archive `item`: the mailbox keeps a row about it and drops its bytes,
	/// but only if they are still the ones `item` lists. Needs the archive
	/// token, and home must hold the bytes first.
	///
	/// # Errors
	/// [`MailboxError::Mismatch`] when the mailbox holds other bytes (`412`),
	/// [`MailboxError::Refused`] when it is too young to archive (`422`),
	/// [`MailboxError::NotFound`], [`MailboxError::BadName`], the status's
	/// other [`MailboxError`], or [`MailboxError::Transport`].
	pub async fn archive(&self, folder: Folder, item: &ItemMeta) -> Result<Archive, MailboxError> {
		#[derive(Deserialize)]
		struct Answer {
			change: String,
		}
		let name = plain_name(&item.name)?;
		let tag = String::from("\"") + &item.sha256.to_ascii_lowercase() + "\"";
		let response = self.send_matching(Method::Delete, self.url(&[folder.as_str(), "items", name]), None, Some(tag)).await?;
		match response.status {
			200 => {
				let answer: Answer = serde_json::from_slice(&response.body).map_err(|_| MailboxError::Decode("an archive's answer is not { change }"))?;
				match answer.change.as_str() {
					"archived" => Ok(Archive::Archived),
					"unchanged" => Ok(Archive::Unchanged),
					_ => Err(MailboxError::Decode("an archive's change is not one it can be")),
				}
			}
			412 => Err(MailboxError::Mismatch),
			_ => Err(failure(&response)),
		}
	}

	/// Put an archived item's own bytes back under `name`. Needs the archive
	/// token.
	///
	/// # Errors
	/// [`MailboxError::NotFound`] when the name was never archived,
	/// [`MailboxError::BadName`], the status's other [`MailboxError`], or
	/// [`MailboxError::Transport`].
	pub async fn restore(&self, folder: Folder, name: &str, body: Vec<u8>) -> Result<Restore, MailboxError> {
		#[derive(Deserialize)]
		struct Answer {
			change: String,
			seq: i64,
		}
		let name = plain_name(name)?;
		let response = self.send(Method::Put, self.url(&[folder.as_str(), "items", name]), Some(body)).await?;
		match response.status {
			200 | 201 => {
				let answer: Answer = serde_json::from_slice(&response.body).map_err(|_| MailboxError::Decode("a restore's answer is not { change, seq }"))?;
				match (response.status, answer.change.as_str()) {
					(201, "restored") => Ok(Restore::Restored { seq: answer.seq }),
					(200, "unchanged") => Ok(Restore::Unchanged { seq: answer.seq }),
					_ => Err(MailboxError::Decode("a restore's change does not match its status")),
				}
			}
			409 => Ok(Restore::Conflict),
			_ => Err(failure(&response)),
		}
	}
}

/// A name the mailbox's patterns allow is letters, digits, `-`, `_` and `.`,
/// which a URL path carries as they are. Anything else is refused rather than
/// escaped: it is not a name the mailbox could hold.
fn plain_name(name: &str) -> Result<&str, MailboxError> {
	let plain = !name.is_empty() && name != "." && name != ".." && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
	if plain {
		Ok(name)
	} else {
		Err(MailboxError::BadName)
	}
}

/// The error a status the call did not expect stands for.
fn failure(response: &Response) -> MailboxError {
	#[derive(Deserialize)]
	struct Refusal {
		error: String,
	}
	match response.status {
		401 => MailboxError::Unauthorized,
		403 => MailboxError::Forbidden,
		404 => MailboxError::NotFound,
		413 => MailboxError::TooBig,
		422 => {
			let why = serde_json::from_slice::<Refusal>(&response.body).map_or_else(|_| String::from("no reason given"), |refusal| refusal.error);
			// The mailbox's refusals are short sentences of its own; a longer
			// one is cut, so a log line stays a line.
			MailboxError::Refused(why.chars().take(200).collect())
		}
		status => MailboxError::Http(status),
	}
}

#[cfg(test)]
mod tests {
	use super::{Archive, Folder, ItemMeta, Mailbox, MailboxError, Put, Restore, Wanted};
	use crate::transport::{MailboxTransport, Method, Request, Response, Token};
	use sha2::{Digest, Sha256};
	use std::sync::Mutex;

	const BASE: &str = "https://mailbox.example.test";
	const SECRET: &str = "a-token-that-must-never-print";

	/// Answers each request with the next canned response, and keeps every
	/// request it was sent.
	struct Fake {
		answers: Mutex<Vec<Response>>,
		sent: Mutex<Vec<Request>>,
	}

	impl MailboxTransport for Fake {
		type Error = &'static str;

		async fn send(&self, request: Request) -> Result<Response, Self::Error> {
			self.sent.lock().unwrap().push(request);
			let mut answers = self.answers.lock().unwrap();
			if answers.is_empty() {
				Err("connection refused")
			} else {
				Ok(answers.remove(0))
			}
		}
	}

	fn mailbox(answers: Vec<(u16, &str)>) -> Mailbox<Fake> {
		let answers = answers
			.into_iter()
			.map(|(status, body)| Response {
				status,
				body: body.as_bytes().to_vec(),
			})
			.collect();
		let fake = Fake {
			answers: Mutex::new(answers),
			sent: Mutex::new(Vec::new()),
		};
		Mailbox::new(BASE, Token::new(SECRET.to_owned()), fake).unwrap()
	}

	fn sent(mailbox: &Mailbox<Fake>) -> Vec<(Method, String)> {
		mailbox.transport.sent.lock().unwrap().iter().map(|request| (request.method, request.url.clone())).collect()
	}

	fn item(name: &str, body: &str) -> ItemMeta {
		ItemMeta {
			seq: 9,
			name: name.to_owned(),
			size: body.len() as u64,
			sha256: hex::encode(Sha256::digest(body.as_bytes())),
			content_type: "application/json".to_owned(),
			put_at: "2026-10-22T14:57:51.443Z".to_owned(),
			live_since: None,
		}
	}

	#[test]
	fn only_an_https_host_is_a_base() {
		let fake = || Fake {
			answers: Mutex::new(Vec::new()),
			sent: Mutex::new(Vec::new()),
		};
		let token = || Token::new(SECRET.to_owned());
		assert!(Mailbox::new("https://mailbox.example.test/", token(), fake()).is_ok());
		for base in [
			"http://mailbox.example.test",
			"https://",
			"https://host/v1",
			"https://host?x",
			"https://user@host",
			"mailbox.example.test",
		] {
			assert!(Mailbox::new(base, token(), fake()).is_err(), "{base}");
		}
	}

	#[tokio::test]
	async fn a_listing_asks_after_the_cursor_and_reads_the_items() {
		let listing = r#"{"items":[{"seq":9,"name":"digest-2026-10-22.json","size":625,"sha256":"91dc","contentType":"application/json","putAt":"2026-10-22T14:57:51.443Z"}]}"#;
		let mailbox = mailbox(vec![(200, listing), (200, r#"{"items":[]}"#)]);
		let items = mailbox.list(Folder::Brief, 8, 20).await.unwrap();
		assert_eq!(items.len(), 1);
		assert_eq!((items[0].seq, items[0].name.as_str(), items[0].size), (9, "digest-2026-10-22.json", 625));
		assert!(mailbox.list(Folder::Scout, 9, 500).await.unwrap().is_empty());
		assert_eq!(
			sent(&mailbox),
			[
				(Method::Get, String::from(BASE) + "/v1/brief/items?after=8&limit=20"),
				(Method::Get, String::from(BASE) + "/v1/scout/items?after=9&limit=50"),
			],
			"the cursor goes as after, and the limit stays within the mailbox's ceiling"
		);
	}

	#[tokio::test]
	async fn a_listing_that_is_not_items_is_an_error() {
		let mailbox = mailbox(vec![(200, "<html>"), (401, "{}"), (503, r#"{"error":"unavailable"}"#)]);
		assert!(matches!(mailbox.list(Folder::Brief, 0, 20).await, Err(MailboxError::Decode(_))));
		assert_eq!(mailbox.list(Folder::Brief, 0, 20).await, Err(MailboxError::Unauthorized));
		assert_eq!(mailbox.list(Folder::Brief, 0, 20).await, Err(MailboxError::Http(503)));
		assert_eq!(mailbox.list(Folder::Brief, 0, 20).await.unwrap_err().kind(), "transport");
	}

	#[tokio::test]
	async fn a_get_checks_the_bytes_against_the_listing() {
		let body = r#"{"schema":"some-ui.job-digest/1"}"#;
		let mailbox = mailbox(vec![(200, body), (200, r#"{"schema":"other"}"#), (404, r#"{"error":"not found"}"#)]);
		let listed = item("digest-2026-10-22.json", body);
		assert_eq!(mailbox.get(Folder::Brief, &listed).await.unwrap(), body.as_bytes());
		assert_eq!(mailbox.get(Folder::Brief, &listed).await, Err(MailboxError::Mismatch), "other bytes than listed");
		assert_eq!(mailbox.get(Folder::Brief, &listed).await, Err(MailboxError::NotFound));
		assert_eq!(sent(&mailbox)[0], (Method::Get, String::from(BASE) + "/v1/brief/items/digest-2026-10-22.json"));

		let mailbox = super::Mailbox::new(BASE, Token::new(SECRET.to_owned()), mailbox.transport).unwrap();
		for name in ["../v1/decisions/items", "a b.json", "", ".."] {
			assert_eq!(mailbox.get(Folder::Brief, &item(name, body)).await, Err(MailboxError::BadName), "{name}");
		}
	}

	#[tokio::test]
	async fn each_put_answer_maps_to_what_it_means() {
		let mailbox = mailbox(vec![
			(201, r#"{"change":"added","seq":12}"#),
			(200, r#"{"change":"unchanged","seq":12}"#),
			(409, r#"{"error":"conflict"}"#),
			(422, r#"{"error":"the schema does not match the folder"}"#),
			(413, "{}"),
			(403, r#"{"error":"forbidden"}"#),
			(200, r#"{"change":"added","seq":13}"#),
		]);
		let body = || br#"{"schema":"some-ui.job-decisions/1"}"#.to_vec();
		let put = || mailbox.put(Folder::Decisions, "decisions-2026-10-22.json", body());
		assert_eq!(put().await, Ok(Put::Added { seq: 12 }));
		assert_eq!(put().await, Ok(Put::Unchanged { seq: 12 }));
		assert_eq!(put().await, Ok(Put::Conflict));
		assert_eq!(put().await, Err(MailboxError::Refused("the schema does not match the folder".to_owned())));
		assert_eq!(put().await, Err(MailboxError::TooBig));
		assert_eq!(put().await, Err(MailboxError::Forbidden));
		assert!(matches!(put().await, Err(MailboxError::Decode(_))), "a 200 that says added");
		let request = &mailbox.transport.sent.lock().unwrap()[0];
		assert_eq!(
			(request.method, request.url.as_str()),
			(Method::Put, "https://mailbox.example.test/v1/decisions/items/decisions-2026-10-22.json")
		);
		assert_eq!(request.body.as_deref(), Some(body().as_slice()));
	}

	#[tokio::test]
	async fn a_listing_reads_live_since_and_falls_back_to_put_at() {
		let listing = r#"{"items":[
			{"seq":9,"name":"a.json","size":1,"sha256":"aa","contentType":"application/json","putAt":"2026-01-01T00:00:00.000Z","liveSince":"2026-06-01T00:00:00.000Z"},
			{"seq":10,"name":"b.json","size":1,"sha256":"bb","contentType":"application/json","putAt":"2026-02-01T00:00:00.000Z"}
		]}"#;
		let items = mailbox(vec![(200, listing)]).list(Folder::Brief, 0, 50).await.unwrap();
		assert_eq!(items[0].live_since(), "2026-06-01T00:00:00.000Z", "put back from home");
		assert_eq!(items[1].live_since(), "2026-02-01T00:00:00.000Z", "a mailbox from before archiving");
	}

	#[tokio::test]
	async fn an_archive_names_the_bytes_and_each_answer_maps_to_what_it_means() {
		let body = r#"{"schema":"some-ui.job-digest/1"}"#;
		let mailbox = mailbox(vec![
			(200, r#"{"change":"archived"}"#),
			(200, r#"{"change":"unchanged"}"#),
			(412, r#"{"error":"mismatch"}"#),
			(422, r#"{"error":"an item is archived only after 30 days live"}"#),
			(404, r#"{"error":"not_found"}"#),
			(200, r#"{"change":"restored"}"#),
		]);
		let listed = item("digest-2026-10-22.json", body);
		assert_eq!(mailbox.archive(Folder::Brief, &listed).await, Ok(Archive::Archived));
		assert_eq!(mailbox.archive(Folder::Brief, &listed).await, Ok(Archive::Unchanged));
		assert_eq!(mailbox.archive(Folder::Brief, &listed).await, Err(MailboxError::Mismatch));
		assert!(matches!(mailbox.archive(Folder::Brief, &listed).await, Err(MailboxError::Refused(_))));
		assert_eq!(mailbox.archive(Folder::Brief, &listed).await, Err(MailboxError::NotFound));
		assert!(matches!(mailbox.archive(Folder::Brief, &listed).await, Err(MailboxError::Decode(_))));
		let request = mailbox.transport.sent.lock().unwrap()[0].clone();
		assert_eq!(
			(request.method, request.url.as_str(), request.if_match.as_deref(), request.body.is_none()),
			(
				Method::Delete,
				"https://mailbox.example.test/v1/brief/items/digest-2026-10-22.json",
				Some(String::from("\"") + &listed.sha256 + "\"").as_deref(),
				true
			)
		);
		assert_eq!(mailbox.archive(Folder::Brief, &item("../x", body)).await, Err(MailboxError::BadName));
	}

	#[tokio::test]
	async fn the_asks_are_listed_and_a_restore_answer_maps_to_what_it_means() {
		let mailbox = mailbox(vec![
			(200, r#"{"items":[{"name":"digest-2026-07-01.json","askedAt":"2026-10-23T15:00:00.000Z"}]}"#),
			(201, r#"{"change":"restored","seq":40}"#),
			(200, r#"{"change":"unchanged","seq":40}"#),
			(409, r#"{"error":"conflict"}"#),
			(404, r#"{"error":"not_found"}"#),
			(201, r#"{"change":"added","seq":41}"#),
		]);
		assert_eq!(
			mailbox.wanted(Folder::Brief, 500).await.unwrap(),
			[Wanted {
				name: String::from("digest-2026-07-01.json"),
				asked_at: String::from("2026-10-23T15:00:00.000Z"),
			}]
		);
		let restore = || mailbox.restore(Folder::Brief, "digest-2026-07-01.json", b"{}".to_vec());
		assert_eq!(restore().await, Ok(Restore::Restored { seq: 40 }));
		assert_eq!(restore().await, Ok(Restore::Unchanged { seq: 40 }));
		assert_eq!(restore().await, Ok(Restore::Conflict));
		assert_eq!(restore().await, Err(MailboxError::NotFound));
		assert!(matches!(restore().await, Err(MailboxError::Decode(_))), "a 201 that says added");
		assert_eq!(
			sent(&mailbox)[..2],
			[
				(Method::Get, String::from(BASE) + "/v1/brief/wanted?limit=50"),
				(Method::Put, String::from(BASE) + "/v1/brief/items/digest-2026-07-01.json"),
			]
		);
	}

	#[tokio::test]
	async fn the_token_never_appears_in_a_debug_or_an_error() {
		let mailbox = mailbox(vec![(401, SECRET)]);
		let err = mailbox.list(Folder::Brief, 0, 1).await.unwrap_err();
		let request = mailbox.transport.sent.lock().unwrap()[0].clone();
		for text in [debug(&mailbox), debug(&request), debug(&request.token), debug(&err), err.to_string()] {
			assert!(!text.contains(SECRET), "{text}");
		}
		assert!(!debug(&mailbox).contains("mailbox.example.test"), "nor the base URL");
		assert_eq!(debug(&request.token), "[redacted]");
		assert_eq!(request.token.expose(), SECRET, "the transport still reads it");
	}

	fn debug(value: &impl std::fmt::Debug) -> String {
		let mut text = String::new();
		std::fmt::Write::write_fmt(&mut text, format_args!("{value:?}")).unwrap();
		text
	}
}
