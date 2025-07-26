/**

The MIT License (MIT)
=====================

Copyright 2015–2024 Sebastian Thiel

Permission is hereby granted, free of charge, to any person
obtaining a copy of this software and associated documentation
files (the “Software”), to deal in the Software without
restriction, including without limitation the rights to use,
copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

*
**/
use serde::{Deserialize, Serialize};
use serde_json as json;
// TODO:  I'm pretty sure no more need for this past stone age!
use google_apis_common::auth::GetToken;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use hyper::{body::Body, Method, Request, Response, Uri};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use std::io;
use std::{
	borrow::BorrowMut,
	cell::RefCell,
	collections::{BTreeMap, HashMap},
	io::{Read, Seek},
	time::Duration,
};
use tokio::time::sleep;
// TODO: Wai?
use std::mem;

pub use crate::cmn::{
	remove_json_null_values, CallBuilder, DefaultDelegate, Delegate, Error, ErrorResponse, MethodInfo, MethodsBuilder, MultiPartReader, NestedType, Part, ReadSeek,
	RequestValue, Resource, ResponseResult, Result, ToParts,
};

#[derive(PartialEq, Eq)]
pub enum Scope {
	YoutubeReadonly,

	YtAnalyticReadonly,

	Youtube,

	Youtubepartner,

	YtAnalyticMonetaryReadonly,
}

impl AsRef<str> for Scope {
	fn as_ref(&self) -> &str {
		match *self {
			Scope::YoutubeReadonly => "https://www.googleapis.com/auth/youtube.readonly",
			Scope::YtAnalyticReadonly => "https://www.googleapis.com/auth/yt-analytics.readonly",
			Scope::Youtube => "https://www.googleapis.com/auth/youtube",
			Scope::Youtubepartner => "https://www.googleapis.com/auth/youtubepartner",
			Scope::YtAnalyticMonetaryReadonly => "https://www.googleapis.com/auth/yt-analytics-monetary.readonly",
		}
	}
}

impl Default for Scope {
	fn default() -> Scope {
		Scope::YoutubeReadonly
	}
}

pub struct YouTubeAnalytics<C, A> {
	client: RefCell<C>,
	auth: RefCell<A>,
	_user_agent: String,
	_base_url: String,
	_root_url: String,
}

impl<'a, C, A> YouTubeAnalytics<C, A>
where
	C: BorrowMut<hyper::Client>,
	A: GetToken,
{
	pub fn new(client: C, authenticator: A) -> YouTubeAnalytics<C, A> {
		YouTubeAnalytics {
			client: RefCell::new(client),
			auth: RefCell::new(authenticator),
			_user_agent: "google-api-rust-client/1.0.8".to_string(),
			_base_url: "https://youtubeanalytics.googleapis.com/".to_string(),
			_root_url: "https://youtubeanalytics.googleapis.com/".to_string(),
		}
	}

	pub fn group_items(&'a self) -> GroupItemMethods<'a, C, A> {
		GroupItemMethods { hub: &self }
	}
	pub fn groups(&'a self) -> GroupMethods<'a, C, A> {
		GroupMethods { hub: &self }
	}
	pub fn reports(&'a self) -> ReportMethods<'a, C, A> {
		ReportMethods { hub: &self }
	}

	pub fn user_agent(&mut self, agent_name: String) -> String {
		mem::replace(&mut self._user_agent, agent_name)
	}

	pub fn base_url(&mut self, new_base_url: String) -> String {
		mem::replace(&mut self._base_url, new_base_url)
	}

	pub fn root_url(&mut self, new_root_url: String) -> String {
		mem::replace(&mut self._root_url, new_root_url)
	}
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Errors {
	pub code: Option<String>,

	#[serde(rename = "requestId")]
	pub request_id: Option<String>,

	pub error: Option<Vec<ErrorProto>>,
}

impl Part for Errors {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Group {
	pub snippet: Option<GroupSnippet>,

	pub kind: Option<String>,

	pub errors: Option<Errors>,

	pub etag: Option<String>,

	#[serde(rename = "contentDetails")]
	pub content_details: Option<GroupContentDetails>,

	pub id: Option<String>,
}

impl RequestValue for Group {}
impl Resource for Group {}
impl ResponseResult for Group {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct GroupContentDetails {
	#[serde(rename = "itemCount")]
	pub item_count: Option<i64>,

	#[serde(rename = "itemType")]
	pub item_type: Option<String>,
}

impl Part for GroupContentDetails {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct EmptyResponse {
	pub errors: Option<Errors>,
}

impl ResponseResult for EmptyResponse {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ErrorProto {
	pub domain: Option<String>,

	pub code: Option<String>,

	pub location: Option<String>,

	#[serde(rename = "externalErrorMessage")]
	pub external_error_message: Option<String>,

	#[serde(rename = "debugInfo")]
	pub debug_info: Option<String>,

	#[serde(rename = "locationType")]
	pub location_type: Option<String>,

	pub argument: Option<Vec<String>>,
}

impl Part for ErrorProto {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListGroupsResponse {
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,

	pub items: Option<Vec<Group>>,

	pub kind: Option<String>,

	pub errors: Option<Errors>,

	pub etag: Option<String>,
}

impl ResponseResult for ListGroupsResponse {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ResultTableColumnHeader {
	#[serde(rename = "dataType")]
	pub data_type: Option<String>,

	#[serde(rename = "columnType")]
	pub column_type: Option<String>,

	pub name: Option<String>,
}

impl Part for ResultTableColumnHeader {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct GroupSnippet {
	#[serde(rename = "publishedAt")]
	pub published_at: Option<String>,

	pub title: Option<String>,
}

impl Part for GroupSnippet {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct GroupItem {
	pub kind: Option<String>,

	pub errors: Option<Errors>,

	pub resource: Option<GroupItemResource>,

	pub etag: Option<String>,

	#[serde(rename = "groupId")]
	pub group_id: Option<String>,

	pub id: Option<String>,
}

impl RequestValue for GroupItem {}
impl Resource for GroupItem {}
impl ResponseResult for GroupItem {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct GroupItemResource {
	pub kind: Option<String>,

	pub id: Option<String>,
}

impl Part for GroupItemResource {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct QueryResponse {
	pub kind: Option<String>,

	pub rows: Option<Vec<Vec<String>>>,

	pub errors: Option<Errors>,

	#[serde(rename = "columnHeaders")]
	pub column_headers: Option<Vec<ResultTableColumnHeader>>,
}

impl ResponseResult for QueryResponse {}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListGroupItemsResponse {
	pub items: Option<Vec<GroupItem>>,

	pub kind: Option<String>,

	pub errors: Option<Errors>,

	pub etag: Option<String>,
}

impl ResponseResult for ListGroupItemsResponse {}

pub struct ReportMethods<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
}

impl<'a, C, A> MethodsBuilder for ReportMethods<'a, C, A> {}

impl<'a, C, A> ReportMethods<'a, C, A> {
	pub fn query(&self) -> ReportQueryCall<'a, C, A> {
		ReportQueryCall {
			hub: self.hub,
			_start_index: Default::default(),
			_start_date: Default::default(),
			_sort: Default::default(),
			_metrics: Default::default(),
			_max_results: Default::default(),
			_include_historical_channel_data: Default::default(),
			_ids: Default::default(),
			_filters: Default::default(),
			_end_date: Default::default(),
			_dimensions: Default::default(),
			_currency: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}
}

pub struct GroupItemMethods<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
}

impl<'a, C, A> MethodsBuilder for GroupItemMethods<'a, C, A> {}

impl<'a, C, A> GroupItemMethods<'a, C, A> {
	pub fn insert(&self, request: GroupItem) -> GroupItemInsertCall<'a, C, A> {
		GroupItemInsertCall {
			hub: self.hub,
			_request: request,
			_on_behalf_of_content_owner: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}

	pub fn list(&self) -> GroupItemListCall<'a, C, A> {
		GroupItemListCall {
			hub: self.hub,
			_on_behalf_of_content_owner: Default::default(),
			_group_id: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}

	pub fn delete(&self) -> GroupItemDeleteCall<'a, C, A> {
		GroupItemDeleteCall {
			hub: self.hub,
			_on_behalf_of_content_owner: Default::default(),
			_id: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}
}

pub struct GroupMethods<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
}

impl<'a, C, A> MethodsBuilder for GroupMethods<'a, C, A> {}

impl<'a, C, A> GroupMethods<'a, C, A> {
	pub fn delete(&self) -> GroupDeleteCall<'a, C, A> {
		GroupDeleteCall {
			hub: self.hub,
			_on_behalf_of_content_owner: Default::default(),
			_id: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}

	pub fn insert(&self, request: Group) -> GroupInsertCall<'a, C, A> {
		GroupInsertCall {
			hub: self.hub,
			_request: request,
			_on_behalf_of_content_owner: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}

	pub fn list(&self) -> GroupListCall<'a, C, A> {
		GroupListCall {
			hub: self.hub,
			_page_token: Default::default(),
			_on_behalf_of_content_owner: Default::default(),
			_mine: Default::default(),
			_id: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}

	pub fn update(&self, request: Group) -> GroupUpdateCall<'a, C, A> {
		GroupUpdateCall {
			hub: self.hub,
			_request: request,
			_on_behalf_of_content_owner: Default::default(),
			_delegate: Default::default(),
			_scopes: Default::default(),
			_additional_params: Default::default(),
		}
	}
}

pub struct ReportQueryCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_start_index: Option<i32>,
	_start_date: Option<String>,
	_sort: Option<String>,
	_metrics: Option<String>,
	_max_results: Option<i32>,
	_include_historical_channel_data: Option<bool>,
	_ids: Option<String>,
	_filters: Option<String>,
	_end_date: Option<String>,
	_dimensions: Option<String>,
	_currency: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for ReportQueryCall<'a, C, A> {}

impl<'a, C, A> ReportQueryCall<'a, C, A>
where
	C: Clone + Send + Sync + 'static,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(Response<Full<Bytes>>, QueryResponse)> {
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};
		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.reports.query",
			http_method: Method::GET,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(13 + self._additional_params.len());
		if let Some(value) = self._start_index {
			params.push(("startIndex", value.to_string()));
		}
		if let Some(value) = self._start_date {
			params.push(("startDate", value.to_string()));
		}
		if let Some(value) = self._sort {
			params.push(("sort", value.to_string()));
		}
		if let Some(value) = self._metrics {
			params.push(("metrics", value.to_string()));
		}
		if let Some(value) = self._max_results {
			params.push(("maxResults", value.to_string()));
		}
		if let Some(value) = self._include_historical_channel_data {
			params.push(("includeHistoricalChannelData", value.to_string()));
		}
		if let Some(value) = self._ids {
			params.push(("ids", value.to_string()));
		}
		if let Some(value) = self._filters {
			params.push(("filters", value.to_string()));
		}
		if let Some(value) = self._end_date {
			params.push(("endDate", value.to_string()));
		}
		if let Some(value) = self._dimensions {
			params.push(("dimensions", value.to_string()));
		}
		if let Some(value) = self._currency {
			params.push(("currency", value.to_string()));
		}

		for &field in [
			"alt",
			"startIndex",
			"startDate",
			"sort",
			"metrics",
			"maxResults",
			"includeHistoricalChannelData",
			"ids",
			"filters",
			"endDate",
			"dimensions",
			"currency",
		]
		.iter()
		{
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}
		for (name, value) in self._additional_params.iter() {
			params.push((&name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/reports";
		if self._scopes.len() == 0 {
			self._scopes.insert(Scope::YoutubeReadonly.as_ref().to_string(), ());
		}

		// Build query string
		let query_string = params
			.iter()
			.map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
			.collect::<Vec<_>>()
			.join("&");

		if !query_string.is_empty() {
			url.push('?');
			url.push_str(&query_string);
		}

		let uri: Uri = url.parse().map_err(|_| Error::InvalidUri(url.clone()))?;

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			let auth_header_value = format!("Bearer {}", token.access_token);

			let req = Request::builder()
				.method(Method::GET)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, auth_header_value)
				.body(Body::empty())
				.map_err(|e| Error::RequestBuild(e))?;

			dlg.pre_request();

			let client = &self.hub.client;
			let req_result = client.request(req).await;

			match req_result {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(Duration::from_secs(d as u64)).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					if !status.is_success() {
						let (parts, body) = res.into_parts();
						let body_bytes = hyper::body::to_bytes(body).await.map_err(Error::BodyRead)?;
						let json_err = String::from_utf8_lossy(&body_bytes).to_string();

						let reconstructed_res = Response::from_parts(parts, Body::from(body_bytes.clone()));

						if let oauth2::Retry::After(d) = dlg.http_failure(&reconstructed_res, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(Duration::from_secs(d as u64)).await;
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => {
								let error_res = Response::from_parts(reconstructed_res.into_parts().0, Body::from(body_bytes));
								Err(Error::Failure(error_res))
							}
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					let (parts, body) = res.into_parts();
					let body_bytes = hyper::body::to_bytes(body).await.map_err(Error::BodyRead)?;
					let json_response = String::from_utf8_lossy(&body_bytes).to_string();

					let result_value = match json::from_str(&json_response) {
						Ok(decoded) => {
							let response = Response::from_parts(parts, Body::from(body_bytes));
							(response, decoded)
						}
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response, err));
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn start_index(mut self, new_value: i32) -> ReportQueryCall<'a, C, A> {
		self._start_index = Some(new_value);
		self
	}

	pub fn start_date(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._start_date = Some(new_value.to_string());
		self
	}

	pub fn sort(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._sort = Some(new_value.to_string());
		self
	}

	pub fn metrics(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._metrics = Some(new_value.to_string());
		self
	}

	pub fn max_results(mut self, new_value: i32) -> ReportQueryCall<'a, C, A> {
		self._max_results = Some(new_value);
		self
	}

	pub fn include_historical_channel_data(mut self, new_value: bool) -> ReportQueryCall<'a, C, A> {
		self._include_historical_channel_data = Some(new_value);
		self
	}

	pub fn ids(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._ids = Some(new_value.to_string());
		self
	}

	pub fn filters(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._filters = Some(new_value.to_string());
		self
	}

	pub fn end_date(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._end_date = Some(new_value.to_string());
		self
	}

	pub fn dimensions(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._dimensions = Some(new_value.to_string());
		self
	}

	pub fn currency(mut self, new_value: &str) -> ReportQueryCall<'a, C, A> {
		self._currency = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> ReportQueryCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> ReportQueryCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> ReportQueryCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => self._scopes.insert(scope.as_ref().to_string(), ()),
			None => None,
		};
		self
	}
}

pub struct GroupItemInsertCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_request: GroupItem,
	_on_behalf_of_content_owner: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupItemInsertCall<'a, C, A> {}

impl<'a, C, A> GroupItemInsertCall<'a, C, A>
where
	C: BorrowMut<Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>>,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(hyper::Response<Full<Bytes>>, GroupItem)> {
		use std::io::{Read, Seek};
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};

		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groupItems.insert",
			http_method: Method::POST,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value.to_string()));
		}

		for &field in ["alt", "onBehalfOfContentOwner"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}

		for (name, value) in self._additional_params.iter() {
			params.push((&name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groupItems";
		if self._scopes.len() == 0 {
			self._scopes.insert(Scope::Youtube.as_ref().to_string(), ());
		}

		// Build query string manually for more control
		if !params.is_empty() {
			url.push('?');
			for (i, (key, value)) in params.iter().enumerate() {
				if i > 0 {
					url.push('&');
				}
				url.push_str(&format!("{}={}", urlencoding::encode(key), urlencoding::encode(value)));
			}
		}

		let uri: Uri = url.parse().map_err(|e| Error::UriParseError(e))?;

		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		// Read the entire body into memory for Hyper 1.x
		let mut body_bytes = Vec::new();
		request_value_reader.read_to_end(&mut body_bytes).unwrap();

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			// Build request with modern Hyper APIs
			let mut req = Request::builder()
				.method(Method::POST)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, format!("Bearer {}", token.access_token))
				.header(CONTENT_TYPE, "application/json")
				.header(CONTENT_LENGTH, request_size.to_string())
				.body(Full::new(Bytes::from(body_bytes.clone())))
				.map_err(|e| Error::RequestBuildError(e))?;

			dlg.pre_request();

			let req_result = {
				let mut client = self.hub.client.borrow_mut();
				client.request(req).await
			};

			match req_result {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						tokio::time::sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					if !status.is_success() {
						let body_bytes = res.into_body().collect().await.map(|collected| collected.to_bytes()).unwrap_or_else(|_| Bytes::new());
						let json_err = String::from_utf8_lossy(&body_bytes);

						if let oauth2::Retry::After(d) = dlg.http_failure(&status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							tokio::time::sleep(d).await;
							continue;
						}

						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(status)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					let (parts, body) = res.into_parts();
					let body_bytes = body.collect().await.map(|collected| collected.to_bytes()).unwrap_or_else(|_| Bytes::new());
					let json_response = String::from_utf8_lossy(&body_bytes);

					let decoded_result = match json::from_str(&json_response) {
						Ok(decoded) => decoded,
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response.to_string(), err));
						}
					};

					// Reconstruct response with Full<Bytes> body
					let response = hyper::Response::from_parts(parts, Full::new(body_bytes.clone()));
					let result_value = (response, decoded_result);

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn request(mut self, new_value: GroupItem) -> GroupItemInsertCall<'a, C, A> {
		self._request = new_value;
		self
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupItemInsertCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupItemInsertCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupItemInsertCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupItemInsertCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => self._scopes.insert(scope.as_ref().to_string(), ()),
			None => None,
		};
		self
	}
}

pub struct GroupItemListCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_on_behalf_of_content_owner: Option<String>,
	_group_id: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupItemListCall<'a, C, A> {}

impl<'a, C, A> GroupItemListCall<'a, C, A>
where
	C: BorrowMut<Client<hyper_rustls::HttpsConnector<HttpConnector>, Empty<Bytes>>>,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(hyper::Response<http_body_util::Full<Bytes>>, ListGroupItemsResponse)> {
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};

		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groupItems.list",
			http_method: Method::GET,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value.to_string()));
		}
		if let Some(value) = self._group_id {
			params.push(("groupId", value.to_string()));
		}

		for &field in ["alt", "onBehalfOfContentOwner", "groupId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}

		for (name, value) in self._additional_params.iter() {
			params.push((&name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groupItems";
		if self._scopes.len() == 0 {
			self._scopes.insert(Scope::YoutubeReadonly.as_ref().to_string(), ());
		}

		// Build query string manually for more control
		if !params.is_empty() {
			url.push('?');
			for (i, (key, value)) in params.iter().enumerate() {
				if i > 0 {
					url.push('&');
				}
				url.push_str(&format!("{}={}", urlencoding::encode(key), urlencoding::encode(value)));
			}
		}

		let uri: Uri = url.parse().map_err(|e| Error::UriParseError(e))?;

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			// Build request with modern Hyper APIs - GET request with empty body
			let req = Request::builder()
				.method(Method::GET)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, format!("Bearer {}", token.access_token))
				.body(Empty::<Bytes>::new())
				.map_err(|e| Error::RequestBuildError(e))?;

			dlg.pre_request();

			let req_result = {
				let mut client = self.hub.client.borrow_mut();
				client.request(req).await
			};

			match req_result {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						tokio::time::sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					if !status.is_success() {
						let body_bytes = res.into_body().collect().await.map(|collected| collected.to_bytes()).unwrap_or_else(|_| Bytes::new());
						let json_err = String::from_utf8_lossy(&body_bytes);

						if let oauth2::Retry::After(d) = dlg.http_failure(&status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							tokio::time::sleep(d).await;
							continue;
						}

						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(status)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					let (parts, body) = res.into_parts();
					let body_bytes = body.collect().await.map(|collected| collected.to_bytes()).unwrap_or_else(|_| Bytes::new());
					let json_response = String::from_utf8_lossy(&body_bytes);

					let decoded_result = match json::from_str(&json_response) {
						Ok(decoded) => decoded,
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response.to_string(), err));
						}
					};

					// Reconstruct response with Full<Bytes> body
					let response = hyper::Response::from_parts(parts, http_body_util::Full::new(body_bytes.clone()));
					let result_value = (response, decoded_result);

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupItemListCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn group_id(mut self, new_value: &str) -> GroupItemListCall<'a, C, A> {
		self._group_id = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupItemListCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupItemListCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupItemListCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => self._scopes.insert(scope.as_ref().to_string(), ()),
			None => None,
		};
		self
	}
}

pub struct GroupItemDeleteCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_on_behalf_of_content_owner: Option<String>,
	_id: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupItemDeleteCall<'a, C, A> {}

impl<'a, C, A> GroupItemDeleteCall<'a, C, A>
where
	C: BorrowMut<hyper::Client>,
	A: GetToken,
{
	pub fn doit(mut self) -> Result<(hyper::client::Response, EmptyResponse)> {
		use hyper::header::{Authorization, Bearer, ContentLength, ContentType, Location, UserAgent};
		use std::io::{Read, Seek};
		let mut dd = DefaultDelegate;
		let mut dlg: &mut Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};
		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groupItems.delete",
			http_method: hyper::method::Method::Delete,
		});
		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value.to_string()));
		}
		if let Some(value) = self._id {
			params.push(("id", value.to_string()));
		}
		for &field in ["alt", "onBehalfOfContentOwner", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}
		for (name, value) in self._additional_params.iter() {
			params.push((&name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groupItems";
		if self._scopes.len() == 0 {
			self._scopes.insert(Scope::Youtube.as_ref().to_string(), ());
		}

		let url = hyper::Url::parse_with_params(&url, params).unwrap();

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};
			let auth_header = Authorization(Bearer { token: token.access_token });
			let mut req_result = {
				let mut client = &mut *self.hub.client.borrow_mut();
				let mut req = client
					.borrow_mut()
					.request(Method::Delete, url.clone())
					.header(UserAgent(self.hub._user_agent.clone()))
					.header(auth_header.clone());

				dlg.pre_request();
				req.send()
			};

			match req_result {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(d);
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status.is_success() {
						let mut json_err = String::new();
						res.read_to_string(&mut json_err).unwrap();
						if let oauth2::Retry::After(d) = dlg.http_failure(&res, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d);
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(res)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}
					let result_value = {
						let mut json_response = String::new();
						res.read_to_string(&mut json_response).unwrap();
						match json::from_str(&json_response) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&json_response, &err);
								return Err(Error::JsonDecodeError(json_response, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupItemDeleteCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn id(mut self, new_value: &str) -> GroupItemDeleteCall<'a, C, A> {
		self._id = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupItemDeleteCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupItemDeleteCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupItemDeleteCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => self._scopes.insert(scope.as_ref().to_string(), ()),
			None => None,
		};
		self
	}
}

pub struct GroupDeleteCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_on_behalf_of_content_owner: Option<String>,
	_id: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupDeleteCall<'a, C, A> {}

impl<'a, C, A> GroupDeleteCall<'a, C, A>
where
	C: BorrowMut<hyper::Client>,
	A: GetToken,
{
	pub fn doit(mut self) -> Result<(hyper::client::Response, EmptyResponse)> {
		use hyper::header::{Authorization, Bearer, ContentLength, ContentType, Location, UserAgent};
		use std::io::{Read, Seek};
		let mut dd = DefaultDelegate;
		let mut dlg: &mut Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};
		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groups.delete",
			http_method: hyper::method::Method::Delete,
		});
		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value.to_string()));
		}
		if let Some(value) = self._id {
			params.push(("id", value.to_string()));
		}
		for &field in ["alt", "onBehalfOfContentOwner", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}
		for (name, value) in self._additional_params.iter() {
			params.push((&name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groups";
		if self._scopes.len() == 0 {
			self._scopes.insert(Scope::Youtube.as_ref().to_string(), ());
		}

		let url = hyper::Url::parse_with_params(&url, params).unwrap();

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};
			let auth_header = Authorization(Bearer { token: token.access_token });
			let mut req_result = {
				let mut client = &mut *self.hub.client.borrow_mut();
				let mut req = client
					.borrow_mut()
					.request(Method::Delete, url.clone())
					.header(UserAgent(self.hub._user_agent.clone()))
					.header(auth_header.clone());

				dlg.pre_request();
				req.send()
			};

			match req_result {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(d);
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status.is_success() {
						let mut json_err = String::new();
						res.read_to_string(&mut json_err).unwrap();
						if let oauth2::Retry::After(d) = dlg.http_failure(&res, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d);
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(res)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}
					let result_value = {
						let mut json_response = String::new();
						res.read_to_string(&mut json_response).unwrap();
						match json::from_str(&json_response) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&json_response, &err);
								return Err(Error::JsonDecodeError(json_response, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupDeleteCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn id(mut self, new_value: &str) -> GroupDeleteCall<'a, C, A> {
		self._id = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupDeleteCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupDeleteCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupDeleteCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => self._scopes.insert(scope.as_ref().to_string(), ()),
			None => None,
		};
		self
	}
}

pub struct GroupInsertCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_request: Group,
	_on_behalf_of_content_owner: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupInsertCall<'a, C, A> {}

impl<'a, C, A> GroupInsertCall<'a, C, A>
where
	C: std::borrow::BorrowMut<Client<HttpConnector>>,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(Response<Full<Bytes>>, Group)> {
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};

		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groups.insert",
			http_method: Method::POST,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value));
		}

		for &field in ["alt", "onBehalfOfContentOwner"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}

		for (name, value) in self._additional_params.iter() {
			params.push((name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groups";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Youtube.as_ref().to_string(), ());
		}

		// Build query string manually for more control
		if !params.is_empty() {
			url.push('?');
			for (i, (key, value)) in params.iter().enumerate() {
				if i > 0 {
					url.push('&');
				}
				url.push_str(&urlencoding::encode(key));
				url.push('=');
				url.push_str(&urlencoding::encode(value));
			}
		}

		let uri: Uri = url.parse().map_err(|e| Error::InvalidUri(e))?;

		// Prepare request body
		let mut request_value = json::value::to_value(&self._request).expect("serde to work");
		remove_json_null_values(&mut request_value);

		let request_json = json::to_string(&request_value).unwrap();
		let request_bytes = Bytes::from(request_json);
		let content_length = request_bytes.len();

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			let req = Request::builder()
				.method(Method::POST)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, format!("Bearer {}", token.access_token))
				.header(CONTENT_TYPE, "application/json")
				.header(CONTENT_LENGTH, content_length)
				.body(Body::from(request_bytes.clone()))
				.map_err(|e| Error::RequestBuild(e))?;

			dlg.pre_request();

			let mut client = self.hub.client.borrow_mut();
			let res = client.request(req).await;

			match res {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					let (parts, body) = res.into_parts();

					if !status.is_success() {
						// Collect body bytes using modern http-body-util approach
						let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
						let body_bytes = collected.to_bytes();
						let json_err = String::from_utf8_lossy(&body_bytes);

						if let oauth2::Retry::After(d) = dlg.http_failure(status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d).await;
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(status)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					// Collect body bytes for successful response
					let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
					let body_bytes = collected.to_bytes();
					let json_response = String::from_utf8_lossy(&body_bytes);

					let decoded: Group = match json::from_str(&json_response) {
						Ok(decoded) => decoded,
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response.to_string(), err));
						}
					};

					dlg.finished(true);

					// Reconstruct response with original body for return
					let response = Response::from_parts(parts, Body::from(body_bytes));
					return Ok((response, decoded));
				}
			}
		}
	}

	pub fn request(mut self, new_value: Group) -> GroupInsertCall<'a, C, A> {
		self._request = new_value;
		self
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupInsertCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupInsertCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupInsertCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupInsertCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => {
				self._scopes.insert(scope.as_ref().to_string(), ());
			}
			None => {}
		};
		self
	}
}

pub struct GroupListCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_page_token: Option<String>,
	_on_behalf_of_content_owner: Option<String>,
	_mine: Option<bool>,
	_id: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupListCall<'a, C, A> {}

impl<'a, C, A> GroupListCall<'a, C, A>
where
	C: std::borrow::BorrowMut<Client<HttpConnector>>,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(Response<Full<Bytes>>, ListGroupsResponse)> {
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};

		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groups.list",
			http_method: Method::GET,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(6 + self._additional_params.len());
		if let Some(value) = self._page_token {
			params.push(("pageToken", value));
		}
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value));
		}
		if let Some(value) = self._mine {
			params.push(("mine", value.to_string()));
		}
		if let Some(value) = self._id {
			params.push(("id", value));
		}

		for &field in ["alt", "pageToken", "onBehalfOfContentOwner", "mine", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}

		for (name, value) in self._additional_params.iter() {
			params.push((name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groups";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::YoutubeReadonly.as_ref().to_string(), ());
		}

		// Build query string manually for more control
		if !params.is_empty() {
			url.push('?');
			for (i, (key, value)) in params.iter().enumerate() {
				if i > 0 {
					url.push('&');
				}
				url.push_str(&urlencoding::encode(key));
				url.push('=');
				url.push_str(&urlencoding::encode(value));
			}
		}

		let uri: Uri = url.parse().map_err(|e| Error::InvalidUri(e))?;

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			let req = Request::builder()
				.method(Method::GET)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, format!("Bearer {}", token.access_token))
				.body(Body::empty())
				.map_err(|e| Error::RequestBuild(e))?;

			dlg.pre_request();

			let mut client = self.hub.client.borrow_mut();
			let res = client.request(req).await;

			match res {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					let (parts, body) = res.into_parts();

					if !status.is_success() {
						// Collect body bytes using modern http-body-util approach
						let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
						let body_bytes = collected.to_bytes();
						let json_err = String::from_utf8_lossy(&body_bytes);

						if let oauth2::Retry::After(d) = dlg.http_failure(status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d).await;
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(status)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					// Collect body bytes for successful response
					let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
					let body_bytes = collected.to_bytes();
					let json_response = String::from_utf8_lossy(&body_bytes);

					let decoded: ListGroupsResponse = match json::from_str(&json_response) {
						Ok(decoded) => decoded,
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response.to_string(), err));
						}
					};

					dlg.finished(true);

					// Reconstruct response with original body for return
					let response = Response::from_parts(parts, Body::from(body_bytes));
					return Ok((response, decoded));
				}
			}
		}
	}

	pub fn page_token(mut self, new_value: &str) -> GroupListCall<'a, C, A> {
		self._page_token = Some(new_value.to_string());
		self
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupListCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn mine(mut self, new_value: bool) -> GroupListCall<'a, C, A> {
		self._mine = Some(new_value);
		self
	}

	pub fn id(mut self, new_value: &str) -> GroupListCall<'a, C, A> {
		self._id = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupListCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupListCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupListCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => {
				self._scopes.insert(scope.as_ref().to_string(), ());
			}
			None => {}
		};
		self
	}
}

pub struct GroupUpdateCall<'a, C, A>
where
	C: 'a,
	A: 'a,
{
	hub: &'a YouTubeAnalytics<C, A>,
	_request: Group,
	_on_behalf_of_content_owner: Option<String>,
	_delegate: Option<&'a mut dyn Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeMap<String, ()>,
}

impl<'a, C, A> CallBuilder for GroupUpdateCall<'a, C, A> {}

impl<'a, C, A> GroupUpdateCall<'a, C, A>
where
	C: std::borrow::BorrowMut<Client<HttpConnector, Full<Bytes>>>,
	A: GetToken,
{
	pub async fn doit(mut self) -> Result<(Response<Full<Bytes>>, Group)> {
		let mut dd = DefaultDelegate;
		let mut dlg: &mut dyn Delegate = match self._delegate {
			Some(d) => d,
			None => &mut dd,
		};

		dlg.begin(MethodInfo {
			id: "youtubeAnalytics.groups.update",
			http_method: Method::PUT,
		});

		let mut params: Vec<(&str, String)> = Vec::with_capacity(4 + self._additional_params.len());
		if let Some(value) = self._on_behalf_of_content_owner {
			params.push(("onBehalfOfContentOwner", value));
		}

		for &field in ["alt", "onBehalfOfContentOwner"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(Error::FieldClash(field));
			}
		}

		for (name, value) in self._additional_params.iter() {
			params.push((name, value.clone()));
		}

		params.push(("alt", "json".to_string()));

		let mut url = self.hub._base_url.clone() + "v2/groups";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Youtube.as_ref().to_string(), ());
		}

		// Build query string manually for more control
		if !params.is_empty() {
			url.push('?');
			for (i, (key, value)) in params.iter().enumerate() {
				if i > 0 {
					url.push('&');
				}
				url.push_str(&urlencoding::encode(key));
				url.push('=');
				url.push_str(&urlencoding::encode(value));
			}
		}

		let uri: Uri = url.parse().map_err(|e| Error::InvalidUri(e))?;

		// Prepare request body
		let mut request_value = json::value::to_value(&self._request).expect("serde to work");
		remove_json_null_values(&mut request_value);

		let request_json = json::to_string(&request_value).unwrap();
		let request_bytes = Bytes::from(request_json);
		let content_length = request_bytes.len();

		loop {
			let token = match self.hub.auth.borrow_mut().get_token(self._scopes.keys()) {
				Ok(token) => token,
				Err(err) => match dlg.token(&*err) {
					Some(token) => token,
					None => {
						dlg.finished(false);
						return Err(Error::MissingToken(err));
					}
				},
			};

			let req = Request::builder()
				.method(Method::PUT)
				.uri(uri.clone())
				.header(USER_AGENT, self.hub._user_agent.clone())
				.header(AUTHORIZATION, format!("Bearer {}", token.access_token))
				.header(CONTENT_TYPE, "application/json")
				.header(CONTENT_LENGTH, content_length)
				.body(Full::new(request_bytes.clone()))
				.map_err(|e| Error::RequestBuild(e))?;

			dlg.pre_request();

			let mut client = self.hub.client.borrow_mut();
			let res = client.request(req).await;

			match res {
				Err(err) => {
					if let oauth2::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(Error::HttpError(err));
				}
				Ok(res) => {
					let status = res.status();
					let (parts, body) = res.into_parts();

					if !status.is_success() {
						// Collect body bytes using modern http-body-util approach
						let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
						let body_bytes = collected.to_bytes();
						let json_err = String::from_utf8_lossy(&body_bytes);

						if let oauth2::Retry::After(d) = dlg.http_failure(status, json::from_str(&json_err).ok(), json::from_str(&json_err).ok()) {
							sleep(d).await;
							continue;
						}
						dlg.finished(false);
						return match json::from_str::<ErrorResponse>(&json_err) {
							Err(_) => Err(Error::Failure(status)),
							Ok(serr) => Err(Error::BadRequest(serr)),
						};
					}

					// Collect body bytes for successful response
					let collected = body.collect().await.map_err(|e| Error::BodyRead(e))?;
					let body_bytes = collected.to_bytes();
					let json_response = String::from_utf8_lossy(&body_bytes);

					let decoded: Group = match json::from_str(&json_response) {
						Ok(decoded) => decoded,
						Err(err) => {
							dlg.response_json_decode_error(&json_response, &err);
							return Err(Error::JsonDecodeError(json_response.to_string(), err));
						}
					};

					dlg.finished(true);

					// Reconstruct response with Full<Bytes> body for return
					let response = Response::from_parts(parts, Full::new(body_bytes));
					return Ok((response, decoded));
				}
			}
		}
	}

	pub fn request(mut self, new_value: Group) -> GroupUpdateCall<'a, C, A> {
		self._request = new_value;
		self
	}

	pub fn on_behalf_of_content_owner(mut self, new_value: &str) -> GroupUpdateCall<'a, C, A> {
		self._on_behalf_of_content_owner = Some(new_value.to_string());
		self
	}

	pub fn delegate(mut self, new_value: &'a mut dyn Delegate) -> GroupUpdateCall<'a, C, A> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> GroupUpdateCall<'a, C, A>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<T, S>(mut self, scope: T) -> GroupUpdateCall<'a, C, A>
	where
		T: Into<Option<S>>,
		S: AsRef<str>,
	{
		match scope.into() {
			Some(scope) => {
				self._scopes.insert(scope.as_ref().to_string(), ());
			}
			None => {}
		};
		self
	}
}
