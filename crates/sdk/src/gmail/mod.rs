use crate::{GoogleServiceFilePath, SecretFilePathError};
use base64::{engine::general_purpose::URL_SAFE, Engine};
use chrono::{DateTime, Utc};
use google_gmail1::api::{Message, MessagePart};
use google_gmail1::yup_oauth2::Error as OAuth2Error;
use google_gmail1::yup_oauth2::ServiceAccountAuthenticator;
use google_gmail1::{Error as GmailError, Gmail};
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use once_cell::unsync::OnceCell;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::str::FromStr;
use std::sync::Arc;

const SCOPES: [&str; 5] = [
	"https://www.googleapis.com/auth/gmail.readonly",
	"https://www.googleapis.com/auth/gmail.modify",
	"https://mail.google.com/",
	"https://www.googleapis.com/auth/gmail.settings.basic",
	"https://www.googleapis.com/auth/gmail.addons.current.message.readonly",
];

type HttpsConnectorType = HttpsConnector<HttpConnector>;
type GmailClient = Gmail<HttpsConnectorType>;

#[derive(Debug, thiserror::Error)]
pub enum GmailServiceError {
	#[error("OAuth2 error: {0}")]
	OAuth2(#[from] OAuth2Error),

	#[error("Gmail API error: {0}")]
	Gmail(#[from] GmailError),

	#[error("HTTP client error: {0}")]
	Hyper(#[from] HyperError),

	#[error("IO error: {0}")]
	Io(#[from] io::Error),

	#[error("Service initialization failed: {0}")]
	ServiceInit(String),

	#[error("Missing credentials file: {0}")]
	MissingCredentials(String),

	#[error("Message error: {0}")]
	Message(String),

	#[error("MIME parsing error: {0}")]
	MimeParse(#[from] mime::FromStrError),

	#[error("Base64 decode error: {0}")]
	Base64(#[from] base64::DecodeError),

	#[error("Secret file path rrror: {0}")]
	SecretFilePath(#[from] SecretFilePathError),

	#[error("JSON error: {0}")]
	JsonError(#[from] serde_json::Error),

	#[error("Unexpected error: {0}")]
	TokenError(#[from] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailMetadata {
	pub id: String,
	pub thread_id: String,
	pub subject: String,
	pub from: String,
	pub to: Vec<String>,
	pub date: DateTime<Utc>,
}

impl fmt::Display for EmailMetadata {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"Email ID: {}\nThread ID: {}\nSubject: {}\nFrom: {}\nTo: {}\nDate: {}",
			self.id,
			self.thread_id,
			self.subject,
			self.from,
			self.to.join(", "),
			self.date
		)
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailContent {
	pub metadata: EmailMetadata,
	pub body: String,
	pub attachments: Vec<AttachmentMetadata>,
}

impl fmt::Display for EmailContent {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}\n\nBody:\n{}\n\nAttachments: {}", self.metadata, self.body, self.attachments.len())
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttachmentMetadata {
	pub filename: String,
	pub mime_type: String,
	pub size: i32,
}

pub enum ServiceMode {
	ServiceAccount,
	Oauth,
}

impl Default for ServiceMode {
	fn default() -> Self {
		Self::ServiceAccount
	}
}

impl FromStr for ServiceMode {
	type Err = GmailServiceError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s.to_lowercase() {
			s if ["service_acct", "acct", "service"].contains(&s.as_str()) => Ok(Self::ServiceAccount),
			s if ["oauth", "2.0"].contains(&s.as_str()) => Ok(Self::Oauth),
			_ => Err(GmailServiceError::ServiceInit(format!("Incorrect service mode set: '{}'", s))),
		}
	}
}

pub struct GoogleGmailClient {
	user_email: &'static str,
	service: OnceCell<Arc<GmailClient>>,
	client_secret_path: GoogleServiceFilePath,
	service_mode: ServiceMode,
}

impl GoogleGmailClient {
	pub fn new(user_email: &'static str, client_secret_path: String, service_option: Option<&str>) -> Result<Self, GmailServiceError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;
		let service_str = service_option.unwrap_or("service");
		let service_mode = ServiceMode::from_str(service_str)?;

		Ok(Self {
			user_email,
			service: OnceCell::new(),
			client_secret_path: validated_path,
			service_mode,
		})
	}

	async fn initialize_service(&self) -> Result<GmailClient, GmailServiceError> {
		let secret = google_gmail1::yup_oauth2::read_service_account_key(&self.client_secret_path.as_ref()).await?;

		let auth = ServiceAccountAuthenticator::builder(secret).build().await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		let service = Gmail::new(client, auth);
		let auth = &service.auth;
		auth.get_token(&SCOPES).await?;

		Ok(service)
	}

	#[allow(dead_code)]
	async fn initialize_oauth_service(&self) -> Result<GmailClient, GmailServiceError> {
		// rustls::crypto::ring::default_provider()
		// 	.install_default()
		// 	.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

		let secret = google_gmail1::yup_oauth2::read_application_secret(&self.client_secret_path.as_ref()).await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor.clone()).build(connector.clone());

		let auth = google_gmail1::yup_oauth2::InstalledFlowAuthenticator::with_client(
			secret,
			google_gmail1::yup_oauth2::InstalledFlowReturnMethod::HTTPRedirect,
			hyper_util::client::legacy::Client::builder(executor).build(connector),
		)
		.persist_tokens_to_disk(".googleapis/gmail")
		.build()
		.await?;

		auth.token(&SCOPES).await?;

		Ok(Gmail::new(client, auth))
	}

	pub async fn get_service(&self) -> Result<&Arc<GmailClient>, GmailServiceError> {
		if self.service.get().is_none() {
			let service = match self.service_mode {
				ServiceMode::ServiceAccount => self.initialize_service().await?,
				ServiceMode::Oauth => self.initialize_oauth_service().await?,
			};
			self
				.service
				.set(Arc::new(service))
				.map_err(|_| GmailServiceError::ServiceInit("Failed to set service".to_string()))?;
		}
		Ok(self.service.get().unwrap())
	}
}

pub struct ReadGmail {
	client: GoogleGmailClient,
}

impl ReadGmail {
	pub fn new(user_email: &'static str, client_secret_path: String, service_option: Option<&str>) -> Result<Self, GmailServiceError> {
		Ok(Self {
			client: GoogleGmailClient::new(user_email, client_secret_path, service_option)?,
		})
	}

	pub async fn list_message_ids(&self, query: Option<&str>, max_results: u32) -> Result<Vec<String>, GmailServiceError> {
		let service = self.client.get_service().await?;
		let mut message_ids = Vec::new();
		let mut page_token = None;
		let mut total_fetched = 0;
		let mut consecutive_errors = 0;
		const MAX_RETRIES: u32 = 3;

		loop {
			println!("Fetching messages page. Current count: {}", total_fetched);

			let mut request = service.users().messages_list(&self.client.user_email);

			let remaining = max_results.saturating_sub(total_fetched);
			if remaining == 0 {
				break;
			}
			request = request.max_results(remaining);

			if let Some(token) = page_token.as_deref() {
				request = request.page_token(token);
			}

			if let Some(q) = query {
				request = request.q(q);
			}

			println!("Sending request with query: {:?}", query);

			match request.add_scopes(&SCOPES).doit().await {
				Ok(response) => {
					consecutive_errors = 0;

					match response.1.messages {
						Some(messages) => {
							println!("Received {} messages in this page", messages.len());

							for message in messages {
								if let Some(id) = message.id {
									message_ids.push(id);
									total_fetched += 1;

									if total_fetched >= max_results {
										return Ok(message_ids);
									}
								}
							}
						}
						None => {
							println!("No messages found in this page");
							break;
						}
					}

					page_token = response.1.next_page_token;

					if page_token.is_none() {
						println!("No more pages available");
						break;
					}
				}
				Err(e) => {
					println!("API Error: {:?}", e);
					consecutive_errors += 1;
					if consecutive_errors >= MAX_RETRIES {
						return Err(GmailServiceError::Gmail(e));
					}
					tokio::time::sleep(std::time::Duration::from_secs(1)).await;
				}
			}
		}

		println!("Total messages fetched: {}", message_ids.len());
		Ok(message_ids)
	}

	pub async fn get_message_metadata(&self, message_id: &str) -> Result<EmailMetadata, GmailServiceError> {
		let service = self.client.get_service().await?;
		let mut request = service.users().messages_get(&self.client.user_email, message_id).format("metadata");

		for header in ["From", "To", "Subject", "Date"] {
			request = request.add_metadata_headers(header);
		}

		let (_, message) = request.add_scopes(&SCOPES).doit().await?;
		self.parse_message_metadata(message).await
	}

	pub async fn get_message_content(&self, message_id: &str) -> Result<EmailContent, GmailServiceError> {
		let service = self.client.get_service().await?;
		println!("get message_content starting after init service...");
		let message = service
			.users()
			.messages_get(&self.client.user_email, message_id)
			.format("full")
			.add_scopes(&SCOPES)
			.doit()
			.await?
			.1;
		println!("recieved message, start parsing metadata...");

		let metadata = self.parse_message_metadata(message.clone()).await?;
		println!("finished parsing metadata...");
		let (body, attachments) = self.parse_message_parts(message.payload)?;

		Ok(EmailContent { metadata, body, attachments })
	}

	async fn parse_message_metadata(&self, message: Message) -> Result<EmailMetadata, GmailServiceError> {
		println!("begin parsing metadata...");
		let _ = self.client.get_service().await?;
		println!("retrieved service...");
		let headers = message
			.payload
			.and_then(|p| p.headers)
			.ok_or_else(|| GmailServiceError::Message("No headers found".to_string()))?;
		println!("retrieved headers...");

		let mut subject = String::new();
		let mut from = String::new();
		let mut to = Vec::new();
		let mut date = Utc::now();

		for header in headers {
			match header.name.as_deref() {
				Some("Subject") => subject = header.value.unwrap_or_default(),
				Some("From") => from = header.value.unwrap_or_default(),
				Some("To") => to = header.value.unwrap_or_default().split(',').map(|s| s.trim().to_string()).collect(),
				Some("Date") => {
					if let Some(date_str) = header.value {
						date = DateTime::parse_from_rfc2822(&date_str).map(|d| d.with_timezone(&Utc)).unwrap_or(Utc::now());
					}
				}
				_ => {}
			}
		}

		Ok(EmailMetadata {
			id: message.id.unwrap_or_default(),
			thread_id: message.thread_id.unwrap_or_default(),
			subject,
			from,
			to,
			date,
		})
	}

	fn parse_message_parts(&self, payload: Option<MessagePart>) -> Result<(String, Vec<AttachmentMetadata>), GmailServiceError> {
		let mut body = String::new();
		let mut attachments = Vec::new();

		if let Some(part) = payload {
			if let Some(parts) = part.parts {
				for part in parts {
					if let Some(part_body) = part.body {
						if part.filename.is_some() && part.filename.as_ref().unwrap().len() > 0 {
							attachments.push(AttachmentMetadata {
								filename: part.filename.unwrap_or_default(),
								mime_type: part.mime_type.unwrap_or_default(),
								size: part_body.size.unwrap_or(0),
							});
						} else if let Some(data) = part_body.data {
							if let Ok(decoded) = URL_SAFE.decode(data) {
								body.push_str(&String::from_utf8_lossy(&decoded));
							}
						}
					}
				}
			} else if let Some(part_body) = part.body {
				if let Some(data) = part_body.data {
					if let Ok(decoded) = URL_SAFE.decode(data) {
						body.push_str(&String::from_utf8_lossy(&decoded));
					}
				}
			}
		}

		Ok((body, attachments))
	}
}

pub struct SendGmail {
	client: GoogleGmailClient,
}

impl SendGmail {
	pub fn new(user_email: &'static str, client_secret_path: String, service_option: Option<&str>) -> Result<Self, GmailServiceError> {
		Ok(Self {
			client: GoogleGmailClient::new(user_email, client_secret_path, service_option)?,
		})
	}

	pub async fn send_email(&self, to: Vec<String>, subject: &str, body: &str) -> Result<String, GmailServiceError> {
		let message = self.create_message(to, subject, body)?;
		let service = self.client.get_service().await?;

		let json_message = serde_json::to_vec(&message)?;
		let reader = std::io::Cursor::new(json_message);

		let mime_type = "application/json".parse::<mime::Mime>()?;

		let result = service.users().messages_send(message, &self.client.user_email).upload(reader, mime_type).await?;

		Ok(result.1.id.unwrap_or_default())
	}

	fn create_message(&self, to: Vec<String>, subject: &str, body: &str) -> Result<Message, GmailServiceError> {
		let to_header = to.join(", ");
		let email_content = format!("From: {}\r\nTo: {}\r\nSubject: {}\r\n\r\n{}", self.client.user_email, to_header, subject, body);

		let encoded = URL_SAFE.encode(email_content.as_bytes());

		Ok(Message {
			raw: Some(encoded.into()),
			..Default::default()
		})
	}
}
