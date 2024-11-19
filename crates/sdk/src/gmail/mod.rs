use crate::{GoogleServiceFilePath, SecretFilePathError};
use base64::{engine::general_purpose::URL_SAFE, Engine};
use chrono::{DateTime, Utc};
use google_gmail1::api::{Message, MessagePart};
use google_gmail1::yup_oauth2::Error as OAuth2Error;
use google_gmail1::yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod};
use google_gmail1::{Error as GmailError, Gmail};
use hyper::Error as HyperError;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use once_cell::unsync::OnceCell;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

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

	#[error("Secret file path error: {0}")]
	SecretFilePath(#[from] SecretFilePathError),

	#[error("JSON error: {0}")]
	JsonError(#[from] serde_json::Error),
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

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailContent {
	pub metadata: EmailMetadata,
	pub body: String,
	pub attachments: Vec<AttachmentMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttachmentMetadata {
	pub filename: String,
	pub mime_type: String,
	pub size: i32,
}

pub struct GoogleGmailClient {
	user_email: String,
	service: OnceCell<Arc<GmailClient>>,
	client_secret_path: GoogleServiceFilePath,
}

impl GoogleGmailClient {
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, GmailServiceError> {
		let validated_path = GoogleServiceFilePath::new(client_secret_path)?;

		Ok(Self {
			user_email,
			service: OnceCell::new(),
			client_secret_path: validated_path,
		})
	}

	async fn initialize_service(&self) -> Result<GmailClient, GmailServiceError> {
		// rustls::crypto::ring::default_provider()
		// 	.install_default()
		// 	.map_err(|_| SheetError::ServiceInit(format!("Failed to initialize crypto provider: ")))?;

		let secret = google_gmail1::yup_oauth2::read_application_secret(&self.client_secret_path.as_ref()).await?;

		let cache_path = PathBuf::from("app").join("gmail_pickle").join("token_gmail_v1.json");

		fs::create_dir_all(cache_path.parent().unwrap()).map_err(GmailServiceError::Io)?;

		let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
			.persist_tokens_to_disk(cache_path)
			.build()
			.await?;

		let connector = hyper_rustls::HttpsConnectorBuilder::new()
			.with_native_roots()
			.unwrap()
			.https_or_http()
			.enable_http1()
			.build();

		let executor = hyper_util::rt::TokioExecutor::new();
		let client = hyper_util::client::legacy::Client::builder(executor).build(connector);

		Ok(Gmail::new(client, auth))
	}

	pub async fn get_service(&self) -> Result<&Arc<GmailClient>, GmailServiceError> {
		if self.service.get().is_none() {
			let service = self.initialize_service().await?;
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
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, GmailServiceError> {
		Ok(Self {
			client: GoogleGmailClient::new(user_email, client_secret_path)?,
		})
	}

	pub async fn list_message(&self, query: Option<&str>, max_results: u32) -> Result<Vec<EmailMetadata>, GmailServiceError> {
		let service = self.client.get_service().await?;
		let mut result = Vec::new();
		let mut page_token = None;

		loop {
			let mut request = service.users().messages_list(&self.client.user_email).max_results(max_results);

			match page_token.as_deref() {
				Some(token) => request = request.page_token(token),
				None => {}
			}

			match query {
				Some(q) => request = request.q(q),
				None => {}
			}

			let response = request.doit().await?;

			match response.1.messages {
				Some(messages) => {
					for message in messages {
						match message.id {
							Some(id) => match self.get_message_metadata(&id).await {
								Ok(metadata) => result.push(metadata),
								Err(_) => continue,
							},
							None => continue,
						}
					}
				}
				None => {}
			}

			page_token = response.1.next_page_token;
			match page_token {
				None => break,
				Some(_) => continue,
			}
		}

		Ok(result)
	}

	pub async fn get_message_metadata(&self, message_id: &str) -> Result<EmailMetadata, GmailServiceError> {
		let service = self.client.get_service().await?;
		let mut request = service.users().messages_get(&self.client.user_email, message_id).format("metadata");

		for header in ["From", "To", "Subject", "Date"] {
			request = request.add_metadata_headers(header);
		}

		let (_, message) = request.doit().await?;
		self.parse_message_metadata(message)
	}

	pub async fn get_message_content(&self, message_id: &str) -> Result<EmailContent, GmailServiceError> {
		let service = self.client.get_service().await?;
		let message = service.users().messages_get(&self.client.user_email, message_id).format("full").doit().await?.1;

		let metadata = self.parse_message_metadata(message.clone())?;
		let (body, attachments) = self.parse_message_parts(message.payload)?;

		Ok(EmailContent { metadata, body, attachments })
	}

	fn parse_message_metadata(&self, message: Message) -> Result<EmailMetadata, GmailServiceError> {
		let headers = message
			.payload
			.and_then(|p| p.headers)
			.ok_or_else(|| GmailServiceError::Message("No headers found".to_string()))?;

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
	pub fn new(user_email: String, client_secret_path: String) -> Result<Self, GmailServiceError> {
		Ok(Self {
			client: GoogleGmailClient::new(user_email, client_secret_path)?,
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
