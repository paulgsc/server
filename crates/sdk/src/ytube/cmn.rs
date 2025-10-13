use http::header::{InvalidHeaderValue, ToStrError};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use hyper::{body::Body, uri::InvalidUri, HeaderMap, Method, Request, Response, StatusCode};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use mime::Mime;
use oauth2::{self, Retry, TokenType};
use std;
use std::error;
use std::fmt::{self, Display};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::pin::Pin;
use std::str::FromStr;
use std::task::{Context, Poll};
use std::thread::sleep;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

use serde_json as json;

pub trait MethodsBuilder {}

pub trait CallBuilder {}

pub trait Resource {}

pub trait ResponseResult {}

pub trait RequestValue {}

pub trait UnusedType {}

pub trait Part {}

pub trait NestedType {}

pub trait ReadSeek: Seek + Read {}
impl<T: Seek + Read> ReadSeek for T {}

pub trait AsyncReadSeek: AsyncRead + AsyncSeek + Unpin {}
impl<T: AsyncRead + AsyncSeek + Unpin> AsyncReadSeek for T {}

pub trait ToParts {
	fn to_parts(&self) -> String;
}

#[derive(serde::Deserialize)]
pub struct JsonServerError {
	pub error: String,
	pub error_description: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct ErrorResponse {
	error: ServerError,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct ServerError {
	errors: Vec<ServerMessage>,
	code: u16,
	message: String,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct ServerMessage {
	domain: String,
	reason: String,
	message: String,
	#[serde(rename = "locationType")]
	location_type: Option<String>,
	location: Option<String>,
}

#[derive(Copy, Clone)]
pub struct DummyNetworkStream;

impl AsyncRead for DummyNetworkStream {
	fn poll_read(self: Pin<&mut Self>, _cx: &mut Context<'_>, _buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
		Poll::Ready(Ok(()))
	}
}

impl AsyncSeek for DummyNetworkStream {
	fn start_seek(self: Pin<&mut Self>, _position: SeekFrom) -> io::Result<()> {
		Ok(())
	}

	fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
		Poll::Ready(Ok(0))
	}
}

pub trait Delegate {
	fn begin(&mut self, method_info: MethodInfo) {}

	fn http_error(&mut self, err: &hyper::Error) -> Retry {
		let _ = err;
		Retry::Abort
	}

	fn api_key(&mut self) -> Option<String> {
		None
	}

	fn token(&mut self, err: &dyn error::Error) -> Option<oauth2::Token> {
		let _ = err;
		None
	}

	fn upload_url(&mut self) -> Option<String> {
		None
	}

	fn store_upload_url(&mut self, url: Option<&str>) {
		let _ = url;
	}

	fn response_json_decode_error(&mut self, json_encoded_value: &str, json_decode_error: &json::Error) {
		let _ = json_encoded_value;
		let _ = json_decode_error;
	}

	fn http_failure(&mut self, status: StatusCode, json_err: Option<JsonServerError>, server_err: Option<ServerError>) -> Retry {
		let _ = (status, json_err, server_err);
		Retry::Abort
	}

	fn pre_request(&mut self) {}

	fn chunk_size(&mut self) -> u64 {
		1 << 23
	}

	fn cancel_chunk_upload(&mut self, chunk: &ContentRange) -> bool {
		let _ = chunk;
		false
	}

	fn finished(&mut self, is_success: bool) {
		let _ = is_success;
	}
}

#[derive(Default)]
pub struct DefaultDelegate;

impl Delegate for DefaultDelegate {}

#[derive(Debug)]
pub enum Error {
	HttpError(hyper::Error),
	UploadSizeLimitExceeded(u64, u64),
	BadRequest(ErrorResponse),
	MissingAPIKey,
	MissingToken(Box<dyn error::Error>),
	Cancelled,
	FieldClash(&'static str),
	JsonDecodeError(String, json::Error),
	Failure(StatusCode),
	HeaderError(InvalidHeaderValue),
	HeaderParseError(ToStrError),
	InvalidUri(InvalidUri),
	BodyRead(String),
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Error::HttpError(ref err) => err.fmt(f),
			Error::UploadSizeLimitExceeded(ref resource_size, ref max_size) => {
				writeln!(f, "The media size {} exceeds the maximum allowed upload size of {}", resource_size, max_size)
			}
			Error::MissingAPIKey => {
				writeln!(f, "The application's API key was not found in the configuration")?;
				writeln!(f, "It is used as there are no Scopes defined for this method.")
			}
			Error::BadRequest(ref err) => {
				writeln!(f, "Bad Request ({}): {}", err.error.code, err.error.message)?;
				for err in err.error.errors.iter() {
					writeln!(
						f,
						"    {}: {}, {}{}",
						err.domain,
						err.message,
						err.reason,
						match &err.location {
							Some(loc) => format!("@{}", loc),
							None => String::new(),
						}
					)?;
				}
				Ok(())
			}
			Error::MissingToken(ref err) => writeln!(f, "Token retrieval failed with error: {}", err),
			Error::Cancelled => writeln!(f, "Operation cancelled by delegate"),
			Error::FieldClash(field) => writeln!(f, "The custom parameter '{}' is already provided natively by the CallBuilder.", field),
			Error::JsonDecodeError(ref json_str, ref err) => writeln!(f, "{}: {}", err, json_str),
			Error::Failure(ref status) => writeln!(f, "Http status indicates failure: {:?}", status),
			Error::HeaderError(ref err) => writeln!(f, "Header error: {}", err),
			Error::HeaderParseError(ref err) => writeln!(f, "Header parse error: {}", err),
		}
	}
}

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			Error::HttpError(ref err) => err.to_string().as_str(),
			Error::JsonDecodeError(_, ref err) => err.to_string().as_str(),
			_ => "NO DESCRIPTION POSSIBLE - use `Display.fmt()` instead",
		}
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		match *self {
			Error::HttpError(ref err) => Some(err),
			Error::JsonDecodeError(_, ref err) => Some(err),
			Error::HeaderError(ref err) => Some(err),
			Error::HeaderParseError(ref err) => Some(err),
			_ => None,
		}
	}
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct MethodInfo {
	pub id: &'static str,
	pub http_method: Method,
}

const BOUNDARY: &'static str = "MDuXWGyeE33QFXGchb2VFWc4Z7945d";
const LINE_ENDING: &'static str = "\r\n";

#[derive(Default)]
pub struct MultiPartReader<'a> {
	raw_parts: Vec<(HeaderMap, &'a mut dyn Read)>,
	current_part: Option<(Cursor<Vec<u8>>, &'a mut dyn Read)>,
	last_part_boundary: Option<Cursor<Vec<u8>>>,
}

impl<'a> MultiPartReader<'a> {
	pub fn reserve_exact(&mut self, cap: usize) {
		self.raw_parts.reserve_exact(cap);
	}

	pub fn add_part(&mut self, reader: &'a mut dyn Read, size: u64, mime_type: Mime) -> &mut MultiPartReader<'a> {
		let mut headers = HeaderMap::new();
		headers.insert(CONTENT_TYPE, HeaderValue::from_str(&mime_type.to_string()).unwrap());
		headers.insert(CONTENT_LENGTH, HeaderValue::from_str(&size.to_string()).unwrap());
		self.raw_parts.push((headers, reader));
		self
	}

	pub fn mime_type(&self) -> Mime {
		format!("multipart/related; boundary={}", BOUNDARY).parse().unwrap()
	}

	fn is_depleted(&self) -> bool {
		self.raw_parts.len() == 0 && self.current_part.is_none() && self.last_part_boundary.is_none()
	}

	fn is_last_part(&self) -> bool {
		self.raw_parts.len() == 0 && self.current_part.is_some()
	}

	fn format_headers(headers: &HeaderMap) -> String {
		let mut result = String::new();
		for (name, value) in headers {
			result.push_str(&format!("{}: {}{}", name, value.to_str().unwrap_or(""), LINE_ENDING));
		}
		result
	}
}

impl<'a> Read for MultiPartReader<'a> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		match (self.raw_parts.len(), self.current_part.is_none(), self.last_part_boundary.is_none()) {
			(_, _, false) => {
				let br = self.last_part_boundary.as_mut().unwrap().read(buf).unwrap_or(0);
				if br < buf.len() {
					self.last_part_boundary = None;
				}
				return Ok(br);
			}
			(0, true, true) => return Ok(0),
			(n, true, _) if n > 0 => {
				let (headers, reader) = self.raw_parts.remove(0);
				let mut c = Cursor::new(Vec::<u8>::new());
				write!(&mut c, "{}--{}{}{}{}", LINE_ENDING, BOUNDARY, LINE_ENDING, Self::format_headers(&headers), LINE_ENDING).unwrap();
				c.seek(SeekFrom::Start(0)).unwrap();
				self.current_part = Some((c, reader));
			}
			_ => {}
		}

		let (hb, rr) = {
			let (ref mut c, ref mut reader) = self.current_part.as_mut().unwrap();
			let b = c.read(buf).unwrap_or(0);
			(b, reader.read(&mut buf[b..]))
		};

		match rr {
			Ok(bytes_read) => {
				if hb < buf.len() && bytes_read == 0 {
					if self.is_last_part() {
						self.last_part_boundary = Some(Cursor::new(format!("{}--{}--", LINE_ENDING, BOUNDARY).into_bytes()))
					}
					self.current_part = None;
				}
				let mut total_bytes_read = hb + bytes_read;
				while total_bytes_read < buf.len() && !self.is_depleted() {
					match self.read(&mut buf[total_bytes_read..]) {
						Ok(br) => total_bytes_read += br,
						Err(err) => return Err(err),
					}
				}
				Ok(total_bytes_read)
			}
			Err(err) => {
				self.current_part = None;
				self.last_part_boundary = None;
				self.raw_parts.clear();
				Err(err)
			}
		}
	}
}

#[derive(PartialEq, Debug, Clone)]
pub struct XUploadContentType(pub Mime);

impl XUploadContentType {
	pub fn header_name() -> &'static str {
		"X-Upload-Content-Type"
	}

	pub fn parse_header_value(value: &HeaderValue) -> Result<Self> {
		let s = value.to_str().map_err(Error::HeaderParseError)?;
		let mime = s.parse().map_err(|_| Error::HeaderError(InvalidHeaderValue::from_static("Invalid MIME type")))?;
		Ok(XUploadContentType(mime))
	}

	pub fn to_header_value(&self) -> Result<HeaderValue> {
		HeaderValue::from_str(&self.0.to_string()).map_err(Error::HeaderError)
	}
}

impl std::ops::Deref for XUploadContentType {
	type Target = Mime;
	fn deref(&self) -> &Mime {
		&self.0
	}
}

impl std::ops::DerefMut for XUploadContentType {
	fn deref_mut(&mut self) -> &mut Mime {
		&mut self.0
	}
}

impl Display for XUploadContentType {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		Display::fmt(&self.0, f)
	}
}

#[derive(Clone, PartialEq, Debug)]
pub struct Chunk {
	pub first: u64,
	pub last: u64,
}

impl fmt::Display for Chunk {
	fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
		write!(fmt, "{}-{}", self.first, self.last)
	}
}

impl FromStr for Chunk {
	type Err = &'static str;

	fn from_str(s: &str) -> std::result::Result<Chunk, &'static str> {
		let parts: Vec<&str> = s.split('-').collect();
		if parts.len() != 2 {
			return Err("Expected two parts: %i-%i");
		}
		Ok(Chunk {
			first: match FromStr::from_str(parts[0]) {
				Ok(d) => d,
				_ => return Err("Couldn't parse 'first' as digit"),
			},
			last: match FromStr::from_str(parts[1]) {
				Ok(d) => d,
				_ => return Err("Couldn't parse 'last' as digit"),
			},
		})
	}
}

#[derive(Clone, PartialEq, Debug)]
pub struct ContentRange {
	pub range: Option<Chunk>,
	pub total_length: u64,
}

impl ContentRange {
	pub fn header_name() -> &'static str {
		"Content-Range"
	}

	pub fn parse_header_value(_value: &HeaderValue) -> Result<Self> {
		Err(Error::HeaderError(InvalidHeaderValue::from_static("Content-Range parsing not implemented")))
	}

	pub fn to_header_value(&self) -> Result<HeaderValue> {
		let value = match self.range {
			Some(ref c) => format!("bytes {}/{}", c, self.total_length),
			None => format!("bytes */{}", self.total_length),
		};
		HeaderValue::from_str(&value).map_err(Error::HeaderError)
	}
}

#[derive(Clone, PartialEq, Debug)]
pub struct RangeResponseHeader(pub Chunk);

impl RangeResponseHeader {
	pub fn header_name() -> &'static str {
		"Range"
	}

	pub fn parse_header_value(value: &HeaderValue) -> Result<Self> {
		let s = value.to_str().map_err(Error::HeaderParseError)?;
		const PREFIX: &'static str = "bytes ";
		if s.starts_with(PREFIX) {
			if let Ok(c) = <Chunk as FromStr>::from_str(&s[PREFIX.len()..]) {
				return Ok(RangeResponseHeader(c));
			}
		}
		Err(Error::HeaderError(InvalidHeaderValue::from_static("Invalid Range header")))
	}

	pub fn to_header_value(&self) -> Result<HeaderValue> {
		HeaderValue::from_str(&format!("bytes {}", self.0)).map_err(Error::HeaderError)
	}
}

pub struct ResumableUploadHelper<'a, A: 'a> {
	pub client: &'a Client<HttpConnector, Full<Bytes>>,
	pub delegate: &'a mut dyn Delegate,
	pub start_at: Option<u64>,
	pub auth: &'a mut A,
	pub user_agent: &'a str,
	pub auth_header: String,
	pub url: &'a str,
	pub reader: &'a mut dyn ReadSeek,
	pub media_type: Mime,
	pub content_length: u64,
}

impl<'a, A> ResumableUploadHelper<'a, A>
where
	A: oauth2::GetToken,
{
	async fn query_transfer_status(&mut self) -> std::result::Result<u64, hyper::Result<Response<Body>>> {
		loop {
			let content_range = ContentRange {
				range: None,
				total_length: self.content_length,
			};

			let mut request = Request::builder()
				.method(Method::POST)
				.uri(self.url)
				.header(USER_AGENT, self.user_agent)
				.header(AUTHORIZATION, &self.auth_header)
				.header("Content-Range", content_range.to_header_value().unwrap());

			let request = request.body(Full::new(Bytes::new())).unwrap();

			match self.client.request(request).await {
				Ok(response) => {
					let headers = response.headers().clone();
					let status = response.status();

					if status == StatusCode::PERMANENT_REDIRECT {
						if let Some(range_header) = headers.get("Range") {
							if let Ok(range_resp) = RangeResponseHeader::parse_header_value(range_header) {
								return Ok(range_resp.0.last);
							}
						}
					}

					if let Retry::After(d) = self.delegate.http_failure(status, None, None) {
						sleep(d);
						continue;
					}
					return Err(Ok(response));
				}
				Err(err) => {
					if let Retry::After(d) = self.delegate.http_error(&err) {
						sleep(d);
						continue;
					}
					return Err(Err(err));
				}
			}
		}
	}

	pub async fn upload(&mut self) -> Option<hyper::Result<Response<Body>>> {
		let mut start = match self.start_at {
			Some(s) => s,
			None => match self.query_transfer_status().await {
				Ok(s) => s,
				Err(result) => return Some(result),
			},
		};

		const MIN_CHUNK_SIZE: u64 = 1 << 18;
		let chunk_size = match self.delegate.chunk_size() {
			cs if cs > MIN_CHUNK_SIZE => cs,
			_ => MIN_CHUNK_SIZE,
		};

		self.reader.seek(SeekFrom::Start(start)).unwrap();

		loop {
			let request_size = match self.content_length - start {
				rs if rs > chunk_size => chunk_size,
				rs => rs,
			};

			let mut buffer = vec![0u8; request_size as usize];
			let bytes_read = self.reader.read(&mut buffer).unwrap();
			buffer.truncate(bytes_read);

			let range_header = ContentRange {
				range: Some(Chunk {
					first: start,
					last: start + bytes_read as u64 - 1,
				}),
				total_length: self.content_length,
			};

			start += bytes_read as u64;

			if self.delegate.cancel_chunk_upload(&range_header) {
				return None;
			}

			let mut request = Request::builder()
				.method(Method::POST)
				.uri(self.url)
				.header("Content-Range", range_header.to_header_value().unwrap())
				.header(CONTENT_TYPE, self.media_type.to_string())
				.header(USER_AGENT, self.user_agent)
				.header(AUTHORIZATION, &self.auth_header);

			let request = request.body(Full::new(Bytes::from(buffer))).unwrap();

			match self.client.request(request).await {
				Ok(response) => {
					let status = response.status();

					if status == StatusCode::PERMANENT_REDIRECT {
						continue;
					}
					if !status.is_success() {
						let body_bytes = response.into_body().collect().await.map(|collected| collected.to_bytes()).unwrap_or_else(|_| Bytes::new());
						let json_err = String::from_utf8_lossy(&body_bytes).to_string();

						if let Retry::After(d) = self.delegate.http_failure(status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d);
							continue;
						}
					}

					// Reconstruct response with original body for return
					let (parts, _) = response.into_parts();
					let new_response = Response::from_parts(parts, Full::new(Bytes::new()));
					return Some(Ok(new_response));
				}
				Err(err) => {
					if let Retry::After(d) = self.delegate.http_error(&err) {
						sleep(d);
						continue;
					}
					return Some(Err(err));
				}
			}
		}
	}
}

pub fn remove_json_null_values(value: &mut json::value::Value) {
	match *value {
		json::value::Value::Object(ref mut map) => {
			let mut for_removal = Vec::new();

			for (key, mut value) in map.iter_mut() {
				if value.is_null() {
					for_removal.push(key.clone());
				} else {
					remove_json_null_values(&mut value);
				}
			}

			for key in &for_removal {
				map.remove(key);
			}
		}
		json::value::Value::Array(ref mut arr) => {
			let mut i = 0;
			while i < arr.len() {
				if arr[i].is_null() {
					arr.remove(i);
				} else {
					remove_json_null_values(&mut arr[i]);
					i += 1;
				}
			}
		}
		_ => {}
	}
}
