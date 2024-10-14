use serde_json as json;
use std::cell::RefCell;
use std::collections::BTreeSet;
/// This is adapted from @see
/// https://github.com/Byron/google-apis-rs/blob/main/gen/gmail1/src/api.rs
/// The original source is under MIT license.
use std::collections::HashMap;
use std::default::Default;
use std::error::Error as StdError;
use std::fs;
use std::io;
use std::mem;

use hyper::client::connect;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::sleep;
use tower_service;

use crate::{client, client::serde_with, client::GetToken};

#[derive(PartialEq, Eq, Ord, PartialOrd, Hash, Debug, Clone, Copy)]
pub enum Scope {
	/// Read, compose, send, and permanently delete all your email from Gmail
	Gmai,

	/// Manage drafts and send emails when you interact with the add-on
	AddonCurrentActionCompose,

	/// View your email messages when you interact with the add-on
	AddonCurrentMessageAction,

	/// View your email message metadata when the add-on is running
	AddonCurrentMessageMetadata,

	/// View your email messages when the add-on is running
	AddonCurrentMessageReadonly,

	/// Manage drafts and send emails
	Compose,

	/// Add emails into your Gmail mailbox
	Insert,

	/// See and edit your email labels
	Label,

	/// View your email message metadata such as labels and headers, but not the email body
	Metadata,

	/// Read, compose, and send emails from your Gmail account
	Modify,

	/// View your email messages and settings
	Readonly,

	/// Send email on your behalf
	Send,

	/// See, edit, create, or change your email settings and filters in Gmail
	SettingBasic,

	/// Manage your sensitive mail settings, including who can manage your mail
	SettingSharing,
}

impl AsRef<str> for Scope {
	fn as_ref(&self) -> &str {
		match *self {
			Scope::Gmai => "https://mail.google.com/",
			Scope::AddonCurrentActionCompose => "https://www.googleapis.com/auth/gmail.addons.current.action.compose",
			Scope::AddonCurrentMessageAction => "https://www.googleapis.com/auth/gmail.addons.current.message.action",
			Scope::AddonCurrentMessageMetadata => "https://www.googleapis.com/auth/gmail.addons.current.message.metadata",
			Scope::AddonCurrentMessageReadonly => "https://www.googleapis.com/auth/gmail.addons.current.message.readonly",
			Scope::Compose => "https://www.googleapis.com/auth/gmail.compose",
			Scope::Insert => "https://www.googleapis.com/auth/gmail.insert",
			Scope::Label => "https://www.googleapis.com/auth/gmail.labels",
			Scope::Metadata => "https://www.googleapis.com/auth/gmail.metadata",
			Scope::Modify => "https://www.googleapis.com/auth/gmail.modify",
			Scope::Readonly => "https://www.googleapis.com/auth/gmail.readonly",
			Scope::Send => "https://www.googleapis.com/auth/gmail.send",
			Scope::SettingBasic => "https://www.googleapis.com/auth/gmail.settings.basic",
			Scope::SettingSharing => "https://www.googleapis.com/auth/gmail.settings.sharing",
		}
	}
}

impl Default for Scope {
	fn default() -> Scope {
		Scope::AddonCurrentMessageReadonly
	}
}

#[derive(Clone)]
pub struct Gmail<S> {
	pub client: hyper::Client<S, hyper::body::Body>,
	pub auth: Box<dyn client::GetToken>,
	_user_agent: String,
	_base_url: String,
	_root_url: String,
}

impl<'a, S> client::Hub for Gmail<S> {}

impl<'a, S> Gmail<S> {
	pub fn new<A: 'static + client::GetToken>(client: hyper::Client<S, hyper::body::Body>, auth: A) -> Gmail<S> {
		Gmail {
			client,
			auth: Box::new(auth),
			_user_agent: "google-api-rust-client/5.0.5".to_string(),
			_base_url: "https://gmail.googleapis.com/".to_string(),
			_root_url: "https://gmail.googleapis.com/".to_string(),
		}
	}

	pub fn users(&'a self) -> UserMethods<'a, S> {
		UserMethods { hub: &self }
	}

	/// Set the user-agent header field to use in all requests to the server.
	/// It defaults to `google-api-rust-client/5.0.5`.
	///
	/// Returns the previously set user-agent.
	pub fn user_agent(&mut self, agent_name: String) -> String {
		mem::replace(&mut self._user_agent, agent_name)
	}

	/// Set the base url to use in all requests to the server.
	/// It defaults to `https://gmail.googleapis.com/`.
	///
	/// Returns the previously set base url.
	pub fn base_url(&mut self, new_base_url: String) -> String {
		mem::replace(&mut self._base_url, new_base_url)
	}

	/// Set the root url to use in all requests to the server.
	/// It defaults to `https://gmail.googleapis.com/`.
	///
	/// Returns the previously set root url.
	pub fn root_url(&mut self, new_root_url: String) -> String {
		mem::replace(&mut self._root_url, new_root_url)
	}
}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct AutoForwarding {
	/// The state that a message should be left in after it has been forwarded.
	pub disposition: Option<String>,
	/// Email address to which all incoming messages are forwarded. This email address must be a verified member of the forwarding addresses.
	#[serde(rename = "emailAddress")]
	pub email_address: Option<String>,
	/// Whether all incoming mail is automatically forwarded to another address.
	pub enabled: Option<bool>,
}

impl client::RequestValue for AutoForwarding {}
impl client::ResponseResult for AutoForwarding {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct BatchDeleteMessagesRequest {
	/// The IDs of the messages to delete.
	pub ids: Option<Vec<String>>,
}

impl client::RequestValue for BatchDeleteMessagesRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct BatchModifyMessagesRequest {
	/// A list of label IDs to add to messages.
	#[serde(rename = "addLabelIds")]
	pub add_label_ids: Option<Vec<String>>,
	/// The IDs of the messages to modify. There is a limit of 1000 ids per request.
	pub ids: Option<Vec<String>>,
	/// A list of label IDs to remove from messages.
	#[serde(rename = "removeLabelIds")]
	pub remove_label_ids: Option<Vec<String>>,
}

impl client::RequestValue for BatchModifyMessagesRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct CseIdentity {
	/// The email address for the sending identity. The email address must be the primary email address of the authenticated user.
	#[serde(rename = "emailAddress")]
	pub email_address: Option<String>,
	/// If a key pair is associated, the ID of the key pair, CseKeyPair.
	#[serde(rename = "primaryKeyPairId")]
	pub primary_key_pair_id: Option<String>,
	/// The configuration of a CSE identity that uses different key pairs for signing and encryption.
	#[serde(rename = "signAndEncryptKeyPairs")]
	pub sign_and_encrypt_key_pairs: Option<SignAndEncryptKeyPairs>,
}

impl client::RequestValue for CseIdentity {}
impl client::ResponseResult for CseIdentity {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct CseKeyPair {
	/// Output only. If a key pair is set to `DISABLED`, the time that the key pair's state changed from `ENABLED` to `DISABLED`. This field is present only when the key pair is in state `DISABLED`.
	#[serde(rename = "disableTime")]
	pub disable_time: Option<client::chrono::DateTime<client::chrono::offset::Utc>>,
	/// Output only. The current state of the key pair.
	#[serde(rename = "enablementState")]
	pub enablement_state: Option<String>,
	/// Output only. The immutable ID for the client-side encryption S/MIME key pair.
	#[serde(rename = "keyPairId")]
	pub key_pair_id: Option<String>,
	/// Output only. The public key and its certificate chain, in [PEM](https://en.wikipedia.org/wiki/Privacy-Enhanced_Mail) format.
	pub pem: Option<String>,
	/// Input only. The public key and its certificate chain. The chain must be in [PKCS#7](https://en.wikipedia.org/wiki/PKCS_7) format and use PEM encoding and ASCII armor.
	pub pkcs7: Option<String>,
	/// Metadata for instances of this key pair's private key.
	#[serde(rename = "privateKeyMetadata")]
	pub private_key_metadata: Option<Vec<CsePrivateKeyMetadata>>,
	/// Output only. The email address identities that are specified on the leaf certificate.
	#[serde(rename = "subjectEmailAddresses")]
	pub subject_email_addresses: Option<Vec<String>>,
}

impl client::RequestValue for CseKeyPair {}
impl client::ResponseResult for CseKeyPair {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct CsePrivateKeyMetadata {
	/// Metadata for hardware keys.
	#[serde(rename = "hardwareKeyMetadata")]
	pub hardware_key_metadata: Option<HardwareKeyMetadata>,
	/// Metadata for a private key instance managed by an external key access control list service.
	#[serde(rename = "kaclsKeyMetadata")]
	pub kacls_key_metadata: Option<KaclsKeyMetadata>,
	/// Output only. The immutable ID for the private key metadata instance.
	#[serde(rename = "privateKeyMetadataId")]
	pub private_key_metadata_id: Option<String>,
}

impl client::Part for CsePrivateKeyMetadata {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Delegate {
	/// The email address of the delegate.
	#[serde(rename = "delegateEmail")]
	pub delegate_email: Option<String>,
	/// Indicates whether this address has been verified and can act as a delegate for the account. Read-only.
	#[serde(rename = "verificationStatus")]
	pub verification_status: Option<String>,
}

impl client::RequestValue for Delegate {}
impl client::ResponseResult for Delegate {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct DisableCseKeyPairRequest {
	_never_set: Option<bool>,
}

impl client::RequestValue for DisableCseKeyPairRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
	/// The immutable ID of the draft.
	pub id: Option<String>,
	/// The message content of the draft.
	pub message: Option<Message>,
}

impl client::RequestValue for Draft {}
impl client::ResponseResult for Draft {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct EnableCseKeyPairRequest {
	_never_set: Option<bool>,
}

impl client::RequestValue for EnableCseKeyPairRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Filter {
	/// Action that the filter performs.
	pub action: Option<FilterAction>,
	/// Matching criteria for the filter.
	pub criteria: Option<FilterCriteria>,
	/// The server assigned ID of the filter.
	pub id: Option<String>,
}

impl client::RequestValue for Filter {}
impl client::ResponseResult for Filter {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct FilterAction {
	/// List of labels to add to the message.
	#[serde(rename = "addLabelIds")]
	pub add_label_ids: Option<Vec<String>>,
	/// Email address that the message should be forwarded to.
	pub forward: Option<String>,
	/// List of labels to remove from the message.
	#[serde(rename = "removeLabelIds")]
	pub remove_label_ids: Option<Vec<String>>,
}

impl client::Part for FilterAction {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct FilterCriteria {
	/// Whether the response should exclude chats.
	#[serde(rename = "excludeChats")]
	pub exclude_chats: Option<bool>,
	/// The sender's display name or email address.
	pub from: Option<String>,
	/// Whether the message has any attachment.
	#[serde(rename = "hasAttachment")]
	pub has_attachment: Option<bool>,
	/// Only return messages not matching the specified query. Supports the same query format as the Gmail search box. For example, `"from:someuser@example.com rfc822msgid: is:unread"`.
	#[serde(rename = "negatedQuery")]
	pub negated_query: Option<String>,
	/// Only return messages matching the specified query. Supports the same query format as the Gmail search box. For example, `"from:someuser@example.com rfc822msgid: is:unread"`.
	pub query: Option<String>,
	/// The size of the entire RFC822 message in bytes, including all headers and attachments.
	pub size: Option<i32>,
	/// How the message size in bytes should be in relation to the size field.
	#[serde(rename = "sizeComparison")]
	pub size_comparison: Option<String>,
	/// Case-insensitive phrase found in the message's subject. Trailing and leading whitespace are be trimmed and adjacent spaces are collapsed.
	pub subject: Option<String>,
	/// The recipient's display name or email address. Includes recipients in the "to", "cc", and "bcc" header fields. You can use simply the local part of the email address. For example, "example" and "example@" both match "example@gmail.com". This field is case-insensitive.
	pub to: Option<String>,
}

impl client::Part for FilterCriteria {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ForwardingAddress {
	/// An email address to which messages can be forwarded.
	#[serde(rename = "forwardingEmail")]
	pub forwarding_email: Option<String>,
	/// Indicates whether this address has been verified and is usable for forwarding. Read-only.
	#[serde(rename = "verificationStatus")]
	pub verification_status: Option<String>,
}

impl client::RequestValue for ForwardingAddress {}
impl client::ResponseResult for ForwardingAddress {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HardwareKeyMetadata {
	/// Description about the hardware key.
	pub description: Option<String>,
}

impl client::Part for HardwareKeyMetadata {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct History {
	/// The mailbox sequence ID.

	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub id: Option<u64>,
	/// Labels added to messages in this history record.
	#[serde(rename = "labelsAdded")]
	pub labels_added: Option<Vec<HistoryLabelAdded>>,
	/// Labels removed from messages in this history record.
	#[serde(rename = "labelsRemoved")]
	pub labels_removed: Option<Vec<HistoryLabelRemoved>>,
	/// List of messages changed in this history record. The fields for specific change types, such as `messagesAdded` may duplicate messages in this field. We recommend using the specific change-type fields instead of this.
	pub messages: Option<Vec<Message>>,
	/// Messages added to the mailbox in this history record.
	#[serde(rename = "messagesAdded")]
	pub messages_added: Option<Vec<HistoryMessageAdded>>,
	/// Messages deleted (not Trashed) from the mailbox in this history record.
	#[serde(rename = "messagesDeleted")]
	pub messages_deleted: Option<Vec<HistoryMessageDeleted>>,
}

impl client::Part for History {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HistoryLabelAdded {
	/// Label IDs added to the message.
	#[serde(rename = "labelIds")]
	pub label_ids: Option<Vec<String>>,
	/// no description provided
	pub message: Option<Message>,
}

impl client::Part for HistoryLabelAdded {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HistoryLabelRemoved {
	/// Label IDs removed from the message.
	#[serde(rename = "labelIds")]
	pub label_ids: Option<Vec<String>>,
	/// no description provided
	pub message: Option<Message>,
}

impl client::Part for HistoryLabelRemoved {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HistoryMessageAdded {
	/// no description provided
	pub message: Option<Message>,
}

impl client::Part for HistoryMessageAdded {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct HistoryMessageDeleted {
	/// no description provided
	pub message: Option<Message>,
}

impl client::Part for HistoryMessageDeleted {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ImapSettings {
	/// If this value is true, Gmail will immediately expunge a message when it is marked as deleted in IMAP. Otherwise, Gmail will wait for an update from the client before expunging messages marked as deleted.
	#[serde(rename = "autoExpunge")]
	pub auto_expunge: Option<bool>,
	/// Whether IMAP is enabled for the account.
	pub enabled: Option<bool>,
	/// The action that will be executed on a message when it is marked as deleted and expunged from the last visible IMAP folder.
	#[serde(rename = "expungeBehavior")]
	pub expunge_behavior: Option<String>,
	/// An optional limit on the number of messages that an IMAP folder may contain. Legal values are 0, 1000, 2000, 5000 or 10000. A value of zero is interpreted to mean that there is no limit.
	#[serde(rename = "maxFolderSize")]
	pub max_folder_size: Option<i32>,
}

impl client::RequestValue for ImapSettings {}
impl client::ResponseResult for ImapSettings {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct KaclsKeyMetadata {
	/// Opaque data generated and used by the key access control list service. Maximum size: 8 KiB.
	#[serde(rename = "kaclsData")]
	pub kacls_data: Option<String>,
	/// The URI of the key access control list service that manages the private key.
	#[serde(rename = "kaclsUri")]
	pub kacls_uri: Option<String>,
}

impl client::Part for KaclsKeyMetadata {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Label {
	/// The color to assign to the label. Color is only available for labels that have their `type` set to `user`.
	pub color: Option<LabelColor>,
	/// The immutable ID of the label.
	pub id: Option<String>,
	/// The visibility of the label in the label list in the Gmail web interface.
	#[serde(rename = "labelListVisibility")]
	pub label_list_visibility: Option<String>,
	/// The visibility of messages with this label in the message list in the Gmail web interface.
	#[serde(rename = "messageListVisibility")]
	pub message_list_visibility: Option<String>,
	/// The total number of messages with the label.
	#[serde(rename = "messagesTotal")]
	pub messages_total: Option<i32>,
	/// The number of unread messages with the label.
	#[serde(rename = "messagesUnread")]
	pub messages_unread: Option<i32>,
	/// The display name of the label.
	pub name: Option<String>,
	/// The total number of threads with the label.
	#[serde(rename = "threadsTotal")]
	pub threads_total: Option<i32>,
	/// The number of unread threads with the label.
	#[serde(rename = "threadsUnread")]
	pub threads_unread: Option<i32>,
	/// The owner type for the label. User labels are created by the user and can be modified and deleted by the user and can be applied to any message or thread. System labels are internally created and cannot be added, modified, or deleted. System labels may be able to be applied to or removed from messages and threads under some circumstances but this is not guaranteed. For example, users can apply and remove the `INBOX` and `UNREAD` labels from messages and threads, but cannot apply or remove the `DRAFTS` or `SENT` labels from messages or threads.
	#[serde(rename = "type")]
	pub type_: Option<String>,
}

impl client::RequestValue for Label {}
impl client::ResponseResult for Label {}

/// There is no detailed description.
///
/// This type is not used in any activity, and only used as *part* of another schema.
///
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct LabelColor {
	/// The background color represented as hex string #RRGGBB (ex #000000). This field is required in order to set the color of a label. Only the following predefined set of color values are allowed: \#000000, #434343, #666666, #999999, #cccccc, #efefef, #f3f3f3, #ffffff, \#fb4c2f, #ffad47, #fad165, #16a766, #43d692, #4a86e8, #a479e2, #f691b3, \#f6c5be, #ffe6c7, #fef1d1, #b9e4d0, #c6f3de, #c9daf8, #e4d7f5, #fcdee8, \#efa093, #ffd6a2, #fce8b3, #89d3b2, #a0eac9, #a4c2f4, #d0bcf1, #fbc8d9, \#e66550, #ffbc6b, #fcda83, #44b984, #68dfa9, #6d9eeb, #b694e8, #f7a7c0, \#cc3a21, #eaa041, #f2c960, #149e60, #3dc789, #3c78d8, #8e63ce, #e07798, \#ac2b16, #cf8933, #d5ae49, #0b804b, #2a9c68, #285bac, #653e9b, #b65775, \#822111, #a46a21, #aa8831, #076239, #1a764d, #1c4587, #41236d, #83334c \#464646, #e7e7e7, #0d3472, #b6cff5, #0d3b44, #98d7e4, #3d188e, #e3d7ff, \#711a36, #fbd3e0, #8a1c0a, #f2b2a8, #7a2e0b, #ffc8af, #7a4706, #ffdeb5, \#594c05, #fbe983, #684e07, #fdedc1, #0b4f30, #b3efd3, #04502e, #a2dcc1, \#c2c2c2, #4986e7, #2da2bb, #b99aff, #994a64, #f691b2, #ff7537, #ffad46, \#662e37, #ebdbde, #cca6ac, #094228, #42d692, #16a765
	#[serde(rename = "backgroundColor")]
	pub background_color: Option<String>,
	/// The text color of the label, represented as hex string. This field is required in order to set the color of a label. Only the following predefined set of color values are allowed: \#000000, #434343, #666666, #999999, #cccccc, #efefef, #f3f3f3, #ffffff, \#fb4c2f, #ffad47, #fad165, #16a766, #43d692, #4a86e8, #a479e2, #f691b3, \#f6c5be, #ffe6c7, #fef1d1, #b9e4d0, #c6f3de, #c9daf8, #e4d7f5, #fcdee8, \#efa093, #ffd6a2, #fce8b3, #89d3b2, #a0eac9, #a4c2f4, #d0bcf1, #fbc8d9, \#e66550, #ffbc6b, #fcda83, #44b984, #68dfa9, #6d9eeb, #b694e8, #f7a7c0, \#cc3a21, #eaa041, #f2c960, #149e60, #3dc789, #3c78d8, #8e63ce, #e07798, \#ac2b16, #cf8933, #d5ae49, #0b804b, #2a9c68, #285bac, #653e9b, #b65775, \#822111, #a46a21, #aa8831, #076239, #1a764d, #1c4587, #41236d, #83334c \#464646, #e7e7e7, #0d3472, #b6cff5, #0d3b44, #98d7e4, #3d188e, #e3d7ff, \#711a36, #fbd3e0, #8a1c0a, #f2b2a8, #7a2e0b, #ffc8af, #7a4706, #ffdeb5, \#594c05, #fbe983, #684e07, #fdedc1, #0b4f30, #b3efd3, #04502e, #a2dcc1, \#c2c2c2, #4986e7, #2da2bb, #b99aff, #994a64, #f691b2, #ff7537, #ffad46, \#662e37, #ebdbde, #cca6ac, #094228, #42d692, #16a765
	#[serde(rename = "textColor")]
	pub text_color: Option<String>,
}

impl client::Part for LabelColor {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct LanguageSettings {
	/// The language to display Gmail in, formatted as an RFC 3066 Language Tag (for example `en-GB`, `fr` or `ja` for British English, French, or Japanese respectively). The set of languages supported by Gmail evolves over time, so please refer to the "Language" dropdown in the Gmail settings for all available options, as described in the language settings help article. A table of sample values is also provided in the Managing Language Settings guide Not all Gmail clients can display the same set of languages. In the case that a user's display language is not available for use on a particular client, said client automatically chooses to display in the closest supported variant (or a reasonable default).
	#[serde(rename = "displayLanguage")]
	pub display_language: Option<String>,
}

impl client::RequestValue for LanguageSettings {}
impl client::ResponseResult for LanguageSettings {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListCseIdentitiesResponse {
	/// One page of the list of CSE identities configured for the user.
	#[serde(rename = "cseIdentities")]
	pub cse_identities: Option<Vec<CseIdentity>>,
	/// Pagination token to be passed to a subsequent ListCseIdentities call in order to retrieve the next page of identities. If this value is not returned or is the empty string, then no further pages remain.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
}

impl client::ResponseResult for ListCseIdentitiesResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListCseKeyPairsResponse {
	/// One page of the list of CSE key pairs installed for the user.
	#[serde(rename = "cseKeyPairs")]
	pub cse_key_pairs: Option<Vec<CseKeyPair>>,
	/// Pagination token to be passed to a subsequent ListCseKeyPairs call in order to retrieve the next page of key pairs. If this value is not returned, then no further pages remain.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
}

impl client::ResponseResult for ListCseKeyPairsResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListDelegatesResponse {
	/// List of the user's delegates (with any verification status). If an account doesn't have delegates, this field doesn't appear.
	pub delegates: Option<Vec<Delegate>>,
}

impl client::ResponseResult for ListDelegatesResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListDraftsResponse {
	/// List of drafts. Note that the `Message` property in each `Draft` resource only contains an `id` and a `threadId`. The messages.get method can fetch additional message details.
	pub drafts: Option<Vec<Draft>>,
	/// Token to retrieve the next page of results in the list.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
	/// Estimated total number of results.
	#[serde(rename = "resultSizeEstimate")]
	pub result_size_estimate: Option<u32>,
}

impl client::ResponseResult for ListDraftsResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListFiltersResponse {
	/// List of a user's filters.
	pub filter: Option<Vec<Filter>>,
}

impl client::ResponseResult for ListFiltersResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListForwardingAddressesResponse {
	/// List of addresses that may be used for forwarding.
	#[serde(rename = "forwardingAddresses")]
	pub forwarding_addresses: Option<Vec<ForwardingAddress>>,
}

impl client::ResponseResult for ListForwardingAddressesResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListHistoryResponse {
	/// List of history records. Any `messages` contained in the response will typically only have `id` and `threadId` fields populated.
	pub history: Option<Vec<History>>,
	/// The ID of the mailbox's current history record.
	#[serde(rename = "historyId")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub history_id: Option<u64>,
	/// Page token to retrieve the next page of results in the list.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
}

impl client::ResponseResult for ListHistoryResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListLabelsResponse {
	/// List of labels. Note that each label resource only contains an `id`, `name`, `messageListVisibility`, `labelListVisibility`, and `type`. The labels.get method can fetch additional label details.
	pub labels: Option<Vec<Label>>,
}

impl client::ResponseResult for ListLabelsResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListMessagesResponse {
	/// List of messages. Note that each message resource contains only an `id` and a `threadId`. Additional message details can be fetched using the messages.get method.
	pub messages: Option<Vec<Message>>,
	/// Token to retrieve the next page of results in the list.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
	/// Estimated total number of results.
	#[serde(rename = "resultSizeEstimate")]
	pub result_size_estimate: Option<u32>,
}

impl client::ResponseResult for ListMessagesResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListSendAsResponse {
	/// List of send-as aliases.
	#[serde(rename = "sendAs")]
	pub send_as: Option<Vec<SendAs>>,
}

impl client::ResponseResult for ListSendAsResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListSmimeInfoResponse {
	/// List of SmimeInfo.
	#[serde(rename = "smimeInfo")]
	pub smime_info: Option<Vec<SmimeInfo>>,
}

impl client::ResponseResult for ListSmimeInfoResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ListThreadsResponse {
	/// Page token to retrieve the next page of results in the list.
	#[serde(rename = "nextPageToken")]
	pub next_page_token: Option<String>,
	/// Estimated total number of results.
	#[serde(rename = "resultSizeEstimate")]
	pub result_size_estimate: Option<u32>,
	/// List of threads. Note that each thread resource does not contain a list of `messages`. The list of `messages` for a given thread can be fetched using the threads.get method.
	pub threads: Option<Vec<Thread>>,
}

impl client::ResponseResult for ListThreadsResponse {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Message {
	/// The ID of the last history record that modified this message.
	#[serde(rename = "historyId")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub history_id: Option<u64>,
	/// The immutable ID of the message.
	pub id: Option<String>,
	/// The internal message creation timestamp (epoch ms), which determines ordering in the inbox. For normal SMTP-received email, this represents the time the message was originally accepted by Google, which is more reliable than the `Date` header. However, for API-migrated mail, it can be configured by client to be based on the `Date` header.
	#[serde(rename = "internalDate")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub internal_date: Option<i64>,
	/// List of IDs of labels applied to this message.
	#[serde(rename = "labelIds")]
	pub label_ids: Option<Vec<String>>,
	/// The parsed email structure in the message parts.
	pub payload: Option<MessagePart>,
	/// The entire email message in an RFC 2822 formatted and base64url encoded string. Returned in `messages.get` and `drafts.get` responses when the `format=RAW` parameter is supplied.

	#[serde_as(as = "Option<::client::serde::urlsafe_base64::Wrapper>")]
	pub raw: Option<Vec<u8>>,
	/// Estimated size in bytes of the message.
	#[serde(rename = "sizeEstimate")]
	pub size_estimate: Option<i32>,
	/// A short part of the message text.
	pub snippet: Option<String>,
	/// The ID of the thread the message belongs to. To add a message or draft to a thread, the following criteria must be met: 1. The requested `threadId` must be specified on the `Message` or `Draft.Message` you supply with your request. 2. The `References` and `In-Reply-To` headers must be set in compliance with the [RFC 2822](https://tools.ietf.org/html/rfc2822) standard. 3. The `Subject` headers must match.
	#[serde(rename = "threadId")]
	pub thread_id: Option<String>,
}

impl client::RequestValue for Message {}
impl client::ResponseResult for Message {}

/// A single MIME message part.
///
/// This type is not used in any activity, and only used as *part* of another schema.
///
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct MessagePart {
	/// The message part body for this part, which may be empty for container MIME message parts.
	pub body: Option<MessagePartBody>,
	/// The filename of the attachment. Only present if this message part represents an attachment.
	pub filename: Option<String>,
	/// List of headers on this message part. For the top-level message part, representing the entire message payload, it will contain the standard RFC 2822 email headers such as `To`, `From`, and `Subject`.
	pub mime_type: Option<String>,
	/// The immutable ID of the message part.
	#[serde(rename = "partId")]
	pub part_id: Option<String>,
	/// The child MIME message parts of this part. This only applies to container MIME message parts, for example `multipart/*`. For non- container MIME message part types, such as `text/plain`, this field is empty. For more information, see RFC 1521.
	pub parts: Option<Vec<MessagePart>>,
}

impl client::Part for MessagePart {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct MessagePartBody {
	/// When present, contains the ID of an external attachment that can be retrieved in a separate `messages.attachments.get` request. When not present, the entire content of the message part body is contained in the data field.
	#[serde(rename = "attachmentId")]
	pub attachment_id: Option<String>,
	/// The body data of a MIME message part as a base64url encoded string. May be empty for MIME container types that have no message body or when the body data is sent as a separate attachment. An attachment ID is present if the body data is contained in a separate attachment.

	#[serde_as(as = "Option<::client::serde::urlsafe_base64::Wrapper>")]
	pub data: Option<Vec<u8>>,
	/// Number of bytes for the message part data (encoding notwithstanding).
	pub size: Option<i32>,
}

impl client::ResponseResult for MessagePartBody {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct MessagePartHeader {
	/// The name of the header before the `:` separator. For example, `To`.
	pub name: Option<String>,
	/// The value of the header after the `:` separator. For example, `someuser@example.com`.
	pub value: Option<String>,
}

impl client::Part for MessagePartHeader {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ModifyMessageRequest {
	/// A list of IDs of labels to add to this message. You can add up to 100 labels with each update.
	#[serde(rename = "addLabelIds")]
	pub add_label_ids: Option<Vec<String>>,
	/// A list IDs of labels to remove from this message. You can remove up to 100 labels with each update.
	#[serde(rename = "removeLabelIds")]
	pub remove_label_ids: Option<Vec<String>>,
}

impl client::RequestValue for ModifyMessageRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ModifyThreadRequest {
	/// A list of IDs of labels to add to this thread. You can add up to 100 labels with each update.
	#[serde(rename = "addLabelIds")]
	pub add_label_ids: Option<Vec<String>>,
	/// A list of IDs of labels to remove from this thread. You can remove up to 100 labels with each update.
	#[serde(rename = "removeLabelIds")]
	pub remove_label_ids: Option<Vec<String>>,
}

impl client::RequestValue for ModifyThreadRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ObliterateCseKeyPairRequest {
	_never_set: Option<bool>,
}

impl client::RequestValue for ObliterateCseKeyPairRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct PopSettings {
	/// The range of messages which are accessible via POP.
	#[serde(rename = "accessWindow")]
	pub access_window: Option<String>,
	/// The action that will be executed on a message after it has been fetched via POP.
	pub disposition: Option<String>,
}

impl client::RequestValue for PopSettings {}
impl client::ResponseResult for PopSettings {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
	/// The user's email address.
	#[serde(rename = "emailAddress")]
	pub email_address: Option<String>,
	/// The ID of the mailbox's current history record.
	#[serde(rename = "historyId")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub history_id: Option<u64>,
	/// The total number of messages in the mailbox.
	#[serde(rename = "messagesTotal")]
	pub messages_total: Option<i32>,
	/// The total number of threads in the mailbox.
	#[serde(rename = "threadsTotal")]
	pub threads_total: Option<i32>,
}

impl client::ResponseResult for Profile {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct SendAs {
	/// A name that appears in the "From:" header for mail sent using this alias. For custom "from" addresses, when this is empty, Gmail will populate the "From:" header with the name that is used for the primary address associated with the account. If the admin has disabled the ability for users to update their name format, requests to update this field for the primary login will silently fail.
	#[serde(rename = "displayName")]
	pub display_name: Option<String>,
	/// Whether this address is selected as the default "From:" address in situations such as composing a new message or sending a vacation auto-reply. Every Gmail account has exactly one default send-as address, so the only legal value that clients may write to this field is `true`. Changing this from `false` to `true` for an address will result in this field becoming `false` for the other previous default address.
	#[serde(rename = "isDefault")]
	pub is_default: Option<bool>,
	/// Whether this address is the primary address used to login to the account. Every Gmail account has exactly one primary address, and it cannot be deleted from the collection of send-as aliases. This field is read-only.
	#[serde(rename = "isPrimary")]
	pub is_primary: Option<bool>,
	/// An optional email address that is included in a "Reply-To:" header for mail sent using this alias. If this is empty, Gmail will not generate a "Reply-To:" header.
	#[serde(rename = "replyToAddress")]
	pub reply_to_address: Option<String>,
	/// The email address that appears in the "From:" header for mail sent using this alias. This is read-only for all operations except create.
	#[serde(rename = "sendAsEmail")]
	pub send_as_email: Option<String>,
	/// An optional HTML signature that is included in messages composed with this alias in the Gmail web UI. This signature is added to new emails only.
	pub signature: Option<String>,
	/// An optional SMTP service that will be used as an outbound relay for mail sent using this alias. If this is empty, outbound mail will be sent directly from Gmail's servers to the destination SMTP service. This setting only applies to custom "from" aliases.
	#[serde(rename = "smtpMsa")]
	pub smtp_msa: Option<SmtpMsa>,
	/// Whether Gmail should treat this address as an alias for the user's primary email address. This setting only applies to custom "from" aliases.
	#[serde(rename = "treatAsAlias")]
	pub treat_as_alias: Option<bool>,
	/// Indicates whether this address has been verified for use as a send-as alias. Read-only. This setting only applies to custom "from" aliases.
	#[serde(rename = "verificationStatus")]
	pub verification_status: Option<String>,
}

impl client::RequestValue for SendAs {}
impl client::ResponseResult for SendAs {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct SignAndEncryptKeyPairs {
	/// The ID of the CseKeyPair that encrypts signed outgoing mail.
	#[serde(rename = "encryptionKeyPairId")]
	pub encryption_key_pair_id: Option<String>,
	/// The ID of the CseKeyPair that signs outgoing mail.
	#[serde(rename = "signingKeyPairId")]
	pub signing_key_pair_id: Option<String>,
}

impl client::Part for SignAndEncryptKeyPairs {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct SmimeInfo {
	/// Encrypted key password, when key is encrypted.
	#[serde(rename = "encryptedKeyPassword")]
	pub encrypted_key_password: Option<String>,
	/// When the certificate expires (in milliseconds since epoch).

	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub expiration: Option<i64>,
	/// The immutable ID for the SmimeInfo.
	pub id: Option<String>,
	/// Whether this SmimeInfo is the default one for this user's send-as address.
	#[serde(rename = "isDefault")]
	pub is_default: Option<bool>,
	/// The S/MIME certificate issuer's common name.
	#[serde(rename = "issuerCn")]
	pub issuer_cn: Option<String>,
	/// PEM formatted X509 concatenated certificate string (standard base64 encoding). Format used for returning key, which includes public key as well as certificate chain (not private key).
	pub pem: Option<String>,
	/// PKCS#12 format containing a single private/public key pair and certificate chain. This format is only accepted from client for creating a new SmimeInfo and is never returned, because the private key is not intended to be exported. PKCS#12 may be encrypted, in which case encryptedKeyPassword should be set appropriately.

	#[serde_as(as = "Option<::client::serde::standard_base64::Wrapper>")]
	pub pkcs12: Option<Vec<u8>>,
}

impl client::RequestValue for SmimeInfo {}
impl client::ResponseResult for SmimeInfo {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct SmtpMsa {
	/// The hostname of the SMTP service. Required.
	pub host: Option<String>,
	/// The password that will be used for authentication with the SMTP service. This is a write-only field that can be specified in requests to create or update SendAs settings; it is never populated in responses.
	pub password: Option<String>,
	/// The port of the SMTP service. Required.
	pub port: Option<i32>,
	/// The protocol that will be used to secure communication with the SMTP service. Required.
	#[serde(rename = "securityMode")]
	pub security_mode: Option<String>,
	/// The username that will be used for authentication with the SMTP service. This is a write-only field that can be specified in requests to create or update SendAs settings; it is never populated in responses.
	pub username: Option<String>,
}

impl client::Part for SmtpMsa {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Thread {
	/// The ID of the last history record that modified this thread.
	#[serde(rename = "historyId")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub history_id: Option<u64>,
	/// The unique ID of the thread.
	pub id: Option<String>,
	/// The list of messages in the thread.
	pub messages: Option<Vec<Message>>,
	/// A short part of the message text.
	pub snippet: Option<String>,
}

impl client::ResponseResult for Thread {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct VacationSettings {
	/// Flag that controls whether Gmail automatically replies to messages.
	#[serde(rename = "enableAutoReply")]
	pub enable_auto_reply: Option<bool>,
	/// An optional end time for sending auto-replies (epoch ms). When this is specified, Gmail will automatically reply only to messages that it receives before the end time. If both `startTime` and `endTime` are specified, `startTime` must precede `endTime`.
	#[serde(rename = "endTime")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub end_time: Option<i64>,
	/// Response body in HTML format. Gmail will sanitize the HTML before storing it. If both `response_body_plain_text` and `response_body_html` are specified, `response_body_html` will be used.
	#[serde(rename = "responseBodyHtml")]
	pub response_body_html: Option<String>,
	/// Response body in plain text format. If both `response_body_plain_text` and `response_body_html` are specified, `response_body_html` will be used.
	#[serde(rename = "responseBodyPlainText")]
	pub response_body_plain_text: Option<String>,
	/// Optional text to prepend to the subject line in vacation responses. In order to enable auto-replies, either the response subject or the response body must be nonempty.
	#[serde(rename = "responseSubject")]
	pub response_subject: Option<String>,
	/// Flag that determines whether responses are sent to recipients who are not in the user's list of contacts.
	#[serde(rename = "restrictToContacts")]
	pub restrict_to_contacts: Option<bool>,
	/// Flag that determines whether responses are sent to recipients who are outside of the user's domain. This feature is only available for Google Workspace users.
	#[serde(rename = "restrictToDomain")]
	pub restrict_to_domain: Option<bool>,
	/// An optional start time for sending auto-replies (epoch ms). When this is specified, Gmail will automatically reply only to messages that it receives after the start time. If both `startTime` and `endTime` are specified, `startTime` must precede `endTime`.
	#[serde(rename = "startTime")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub start_time: Option<i64>,
}

impl client::RequestValue for VacationSettings {}
impl client::ResponseResult for VacationSettings {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct WatchRequest {
	/// Filtering behavior of `labelIds list` specified. This field is deprecated because it caused incorrect behavior in some cases; use `label_filter_behavior` instead.
	#[serde(rename = "labelFilterAction")]
	pub label_filter_action: Option<String>,
	/// Filtering behavior of `labelIds list` specified. This field replaces `label_filter_action`; if set, `label_filter_action` is ignored.
	#[serde(rename = "labelFilterBehavior")]
	pub label_filter_behavior: Option<String>,
	/// List of label_ids to restrict notifications about. By default, if unspecified, all changes are pushed out. If specified then dictates which labels are required for a push notification to be generated.
	#[serde(rename = "labelIds")]
	pub label_ids: Option<Vec<String>>,
	/// A fully qualified Google Cloud Pub/Sub API topic name to publish the events to. This topic name **must** already exist in Cloud Pub/Sub and you **must** have already granted gmail "publish" permission on it. For example, "projects/my-project-identifier/topics/my-topic-name" (using the Cloud Pub/Sub "v1" topic naming format). Note that the "my-project-identifier" portion must exactly match your Google developer project id (the one executing this watch request).
	#[serde(rename = "topicName")]
	pub topic_name: Option<String>,
}

impl client::RequestValue for WatchRequest {}

#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde_with::serde_as(crate = "::client::serde_with")]
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct WatchResponse {
	/// When Gmail will stop sending notifications for mailbox updates (epoch millis). Call `watch` again before this time to renew the watch.

	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub expiration: Option<i64>,
	/// The ID of the mailbox's current history record.
	#[serde(rename = "historyId")]
	#[serde_as(as = "Option<::client::serde_with::DisplayFromStr>")]
	pub history_id: Option<u64>,
}

impl client::ResponseResult for WatchResponse {}

pub struct UserMethods<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
}

impl<'a, S> client::MethodsBuilder for UserMethods<'a, S> {}

impl<'a, S> UserMethods<'a, S> {
	/// Create a builder to help you perform the following task:
	///
	/// Creates a new draft with the `DRAFT` label.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn drafts_create(&self, request: Draft, user_id: &str) -> UserDraftCreateCall<'a, S> {
		UserDraftCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Immediately and permanently deletes the specified draft. Does not simply trash it.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the draft to delete.
	pub fn drafts_delete(&self, user_id: &str, id: &str) -> UserDraftDeleteCall<'a, S> {
		UserDraftDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified draft.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the draft to retrieve.
	pub fn drafts_get(&self, user_id: &str, id: &str) -> UserDraftGetCall<'a, S> {
		UserDraftGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_format: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the drafts in the user's mailbox.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn drafts_list(&self, user_id: &str) -> UserDraftListCall<'a, S> {
		UserDraftListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_q: Default::default(),
			_page_token: Default::default(),
			_max_results: Default::default(),
			_include_spam_trash: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Sends the specified, existing draft to the recipients in the `To`, `Cc`, and `Bcc` headers.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn drafts_send(&self, request: Draft, user_id: &str) -> UserDraftSendCall<'a, S> {
		UserDraftSendCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Replaces a draft's content.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the draft to update.
	pub fn drafts_update(&self, request: Draft, user_id: &str, id: &str) -> UserDraftUpdateCall<'a, S> {
		UserDraftUpdateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the history of all changes to the given mailbox. History results are returned in chronological order (increasing `historyId`).
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn history_list(&self, user_id: &str) -> UserHistoryListCall<'a, S> {
		UserHistoryListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_start_history_id: Default::default(),
			_page_token: Default::default(),
			_max_results: Default::default(),
			_label_id: Default::default(),
			_history_types: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates a new label.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn labels_create(&self, request: Label, user_id: &str) -> UserLabelCreateCall<'a, S> {
		UserLabelCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Immediately and permanently deletes the specified label and removes it from any messages and threads that it is applied to.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the label to delete.
	pub fn labels_delete(&self, user_id: &str, id: &str) -> UserLabelDeleteCall<'a, S> {
		UserLabelDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified label.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the label to retrieve.
	pub fn labels_get(&self, user_id: &str, id: &str) -> UserLabelGetCall<'a, S> {
		UserLabelGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists all labels in the user's mailbox.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn labels_list(&self, user_id: &str) -> UserLabelListCall<'a, S> {
		UserLabelListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Patch the specified label.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the label to update.
	pub fn labels_patch(&self, request: Label, user_id: &str, id: &str) -> UserLabelPatchCall<'a, S> {
		UserLabelPatchCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates the specified label.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the label to update.
	pub fn labels_update(&self, request: Label, user_id: &str, id: &str) -> UserLabelUpdateCall<'a, S> {
		UserLabelUpdateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified message attachment.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `messageId` - The ID of the message containing the attachment.
	/// * `id` - The ID of the attachment.
	pub fn messages_attachments_get(&self, user_id: &str, message_id: &str, id: &str) -> UserMessageAttachmentGetCall<'a, S> {
		UserMessageAttachmentGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_message_id: message_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes many messages by message ID. Provides no guarantees that messages were not already deleted or even existed at all.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_batch_delete(&self, request: BatchDeleteMessagesRequest, user_id: &str) -> UserMessageBatchDeleteCall<'a, S> {
		UserMessageBatchDeleteCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Modifies the labels on the specified messages.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_batch_modify(&self, request: BatchModifyMessagesRequest, user_id: &str) -> UserMessageBatchModifyCall<'a, S> {
		UserMessageBatchModifyCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Immediately and permanently deletes the specified message. This operation cannot be undone. Prefer `messages.trash` instead.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the message to delete.
	pub fn messages_delete(&self, user_id: &str, id: &str) -> UserMessageDeleteCall<'a, S> {
		UserMessageDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified message.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the message to retrieve. This ID is usually retrieved using `messages.list`. The ID is also contained in the result when a message is inserted (`messages.insert`) or imported (`messages.import`).
	pub fn messages_get(&self, user_id: &str, id: &str) -> UserMessageGetCall<'a, S> {
		UserMessageGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_metadata_headers: Default::default(),
			_format: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Imports a message into only this user's mailbox, with standard email delivery scanning and classification similar to receiving via SMTP. This method doesn't perform SPF checks, so it might not work for some spam messages, such as those attempting to perform domain spoofing. This method does not send a message.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_import(&self, request: Message, user_id: &str) -> UserMessageImportCall<'a, S> {
		UserMessageImportCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_process_for_calendar: Default::default(),
			_never_mark_spam: Default::default(),
			_internal_date_source: Default::default(),
			_deleted: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Directly inserts a message into only this user's mailbox similar to `IMAP APPEND`, bypassing most scanning and classification. Does not send a message.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_insert(&self, request: Message, user_id: &str) -> UserMessageInsertCall<'a, S> {
		UserMessageInsertCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_internal_date_source: Default::default(),
			_deleted: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the messages in the user's mailbox.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_list(&self, user_id: &str) -> UserMessageListCall<'a, S> {
		UserMessageListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_q: Default::default(),
			_page_token: Default::default(),
			_max_results: Default::default(),
			_label_ids: Default::default(),
			_include_spam_trash: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Modifies the labels on the specified message.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the message to modify.
	pub fn messages_modify(&self, request: ModifyMessageRequest, user_id: &str, id: &str) -> UserMessageModifyCall<'a, S> {
		UserMessageModifyCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Sends the specified message to the recipients in the `To`, `Cc`, and `Bcc` headers. For example usage, see [Sending email](https://developers.google.com/gmail/api/guides/sending).
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn messages_send(&self, request: Message, user_id: &str) -> UserMessageSendCall<'a, S> {
		UserMessageSendCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Moves the specified message to the trash.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the message to Trash.
	pub fn messages_trash(&self, user_id: &str, id: &str) -> UserMessageTrashCall<'a, S> {
		UserMessageTrashCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Removes the specified message from the trash.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the message to remove from Trash.
	pub fn messages_untrash(&self, user_id: &str, id: &str) -> UserMessageUntrashCall<'a, S> {
		UserMessageUntrashCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates and configures a client-side encryption identity that's authorized to send mail from the user account. Google publishes the S/MIME certificate to a shared domain-wide directory so that people within a Google Workspace organization can encrypt and send mail to the identity.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	pub fn settings_cse_identities_create(&self, request: CseIdentity, user_id: &str) -> UserSettingCseIdentityCreateCall<'a, S> {
		UserSettingCseIdentityCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes a client-side encryption identity. The authenticated user can no longer use the identity to send encrypted messages. You cannot restore the identity after you delete it. Instead, use the CreateCseIdentity method to create another identity with the same configuration.
	///
	/// # Arguments
	///
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `cseEmailAddress` - The primary email address associated with the client-side encryption identity configuration that's removed.
	pub fn settings_cse_identities_delete(&self, user_id: &str, cse_email_address: &str) -> UserSettingCseIdentityDeleteCall<'a, S> {
		UserSettingCseIdentityDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_cse_email_address: cse_email_address.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Retrieves a client-side encryption identity configuration.
	///
	/// # Arguments
	///
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `cseEmailAddress` - The primary email address associated with the client-side encryption identity configuration that's retrieved.
	pub fn settings_cse_identities_get(&self, user_id: &str, cse_email_address: &str) -> UserSettingCseIdentityGetCall<'a, S> {
		UserSettingCseIdentityGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_cse_email_address: cse_email_address.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the client-side encrypted identities for an authenticated user.
	///
	/// # Arguments
	///
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	pub fn settings_cse_identities_list(&self, user_id: &str) -> UserSettingCseIdentityListCall<'a, S> {
		UserSettingCseIdentityListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_page_token: Default::default(),
			_page_size: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Associates a different key pair with an existing client-side encryption identity. The updated key pair must validate against Google's [S/MIME certificate profiles](https://support.google.com/a/answer/7300887).
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `emailAddress` - The email address of the client-side encryption identity to update.
	pub fn settings_cse_identities_patch(&self, request: CseIdentity, user_id: &str, email_address: &str) -> UserSettingCseIdentityPatchCall<'a, S> {
		UserSettingCseIdentityPatchCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_email_address: email_address.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates and uploads a client-side encryption S/MIME public key certificate chain and private key metadata for the authenticated user.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	pub fn settings_cse_keypairs_create(&self, request: CseKeyPair, user_id: &str) -> UserSettingCseKeypairCreateCall<'a, S> {
		UserSettingCseKeypairCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Turns off a client-side encryption key pair. The authenticated user can no longer use the key pair to decrypt incoming CSE message texts or sign outgoing CSE mail. To regain access, use the EnableCseKeyPair to turn on the key pair. After 30 days, you can permanently delete the key pair by using the ObliterateCseKeyPair method.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `keyPairId` - The identifier of the key pair to turn off.
	pub fn settings_cse_keypairs_disable(&self, request: DisableCseKeyPairRequest, user_id: &str, key_pair_id: &str) -> UserSettingCseKeypairDisableCall<'a, S> {
		UserSettingCseKeypairDisableCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_key_pair_id: key_pair_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Turns on a client-side encryption key pair that was turned off. The key pair becomes active again for any associated client-side encryption identities.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `keyPairId` - The identifier of the key pair to turn on.
	pub fn settings_cse_keypairs_enable(&self, request: EnableCseKeyPairRequest, user_id: &str, key_pair_id: &str) -> UserSettingCseKeypairEnableCall<'a, S> {
		UserSettingCseKeypairEnableCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_key_pair_id: key_pair_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Retrieves an existing client-side encryption key pair.
	///
	/// # Arguments
	///
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `keyPairId` - The identifier of the key pair to retrieve.
	pub fn settings_cse_keypairs_get(&self, user_id: &str, key_pair_id: &str) -> UserSettingCseKeypairGetCall<'a, S> {
		UserSettingCseKeypairGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_key_pair_id: key_pair_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists client-side encryption key pairs for an authenticated user.
	///
	/// # Arguments
	///
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	pub fn settings_cse_keypairs_list(&self, user_id: &str) -> UserSettingCseKeypairListCall<'a, S> {
		UserSettingCseKeypairListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_page_token: Default::default(),
			_page_size: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes a client-side encryption key pair permanently and immediately. You can only permanently delete key pairs that have been turned off for more than 30 days. To turn off a key pair, use the DisableCseKeyPair method. Gmail can't restore or decrypt any messages that were encrypted by an obliterated key. Authenticated users and Google Workspace administrators lose access to reading the encrypted messages.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	/// * `keyPairId` - The identifier of the key pair to obliterate.
	pub fn settings_cse_keypairs_obliterate(&self, request: ObliterateCseKeyPairRequest, user_id: &str, key_pair_id: &str) -> UserSettingCseKeypairObliterateCall<'a, S> {
		UserSettingCseKeypairObliterateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_key_pair_id: key_pair_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Adds a delegate with its verification status set directly to `accepted`, without sending any verification email. The delegate user must be a member of the same Google Workspace organization as the delegator user. Gmail imposes limitations on the number of delegates and delegators each user in a Google Workspace organization can have. These limits depend on your organization, but in general each user can have up to 25 delegates and up to 10 delegators. Note that a delegate user must be referred to by their primary email address, and not an email alias. Also note that when a new delegate is created, there may be up to a one minute delay before the new delegate is available for use. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_delegates_create(&self, request: Delegate, user_id: &str) -> UserSettingDelegateCreateCall<'a, S> {
		UserSettingDelegateCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Removes the specified delegate (which can be of any verification status), and revokes any verification that may have been required for using it. Note that a delegate user must be referred to by their primary email address, and not an email alias. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `delegateEmail` - The email address of the user to be removed as a delegate.
	pub fn settings_delegates_delete(&self, user_id: &str, delegate_email: &str) -> UserSettingDelegateDeleteCall<'a, S> {
		UserSettingDelegateDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate_email: delegate_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified delegate. Note that a delegate user must be referred to by their primary email address, and not an email alias. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `delegateEmail` - The email address of the user whose delegate relationship is to be retrieved.
	pub fn settings_delegates_get(&self, user_id: &str, delegate_email: &str) -> UserSettingDelegateGetCall<'a, S> {
		UserSettingDelegateGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate_email: delegate_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the delegates for the specified account. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_delegates_list(&self, user_id: &str) -> UserSettingDelegateListCall<'a, S> {
		UserSettingDelegateListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates a filter. Note: you can only create a maximum of 1,000 filters.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_filters_create(&self, request: Filter, user_id: &str) -> UserSettingFilterCreateCall<'a, S> {
		UserSettingFilterCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Immediately and permanently deletes the specified filter.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `id` - The ID of the filter to be deleted.
	pub fn settings_filters_delete(&self, user_id: &str, id: &str) -> UserSettingFilterDeleteCall<'a, S> {
		UserSettingFilterDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets a filter.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `id` - The ID of the filter to be fetched.
	pub fn settings_filters_get(&self, user_id: &str, id: &str) -> UserSettingFilterGetCall<'a, S> {
		UserSettingFilterGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the message filters of a Gmail user.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_filters_list(&self, user_id: &str) -> UserSettingFilterListCall<'a, S> {
		UserSettingFilterListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates a forwarding address. If ownership verification is required, a message will be sent to the recipient and the resource's verification status will be set to `pending`; otherwise, the resource will be created with verification status set to `accepted`. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_forwarding_addresses_create(&self, request: ForwardingAddress, user_id: &str) -> UserSettingForwardingAddressCreateCall<'a, S> {
		UserSettingForwardingAddressCreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes the specified forwarding address and revokes any verification that may have been required. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `forwardingEmail` - The forwarding address to be deleted.
	pub fn settings_forwarding_addresses_delete(&self, user_id: &str, forwarding_email: &str) -> UserSettingForwardingAddressDeleteCall<'a, S> {
		UserSettingForwardingAddressDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_forwarding_email: forwarding_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified forwarding address.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `forwardingEmail` - The forwarding address to be retrieved.
	pub fn settings_forwarding_addresses_get(&self, user_id: &str, forwarding_email: &str) -> UserSettingForwardingAddressGetCall<'a, S> {
		UserSettingForwardingAddressGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_forwarding_email: forwarding_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the forwarding addresses for the specified account.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_forwarding_addresses_list(&self, user_id: &str) -> UserSettingForwardingAddressListCall<'a, S> {
		UserSettingForwardingAddressListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes the specified S/MIME config for the specified send-as alias.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The email address that appears in the "From:" header for mail sent using this alias.
	/// * `id` - The immutable ID for the SmimeInfo.
	pub fn settings_send_as_smime_info_delete(&self, user_id: &str, send_as_email: &str, id: &str) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		UserSettingSendASmimeInfoDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified S/MIME config for the specified send-as alias.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The email address that appears in the "From:" header for mail sent using this alias.
	/// * `id` - The immutable ID for the SmimeInfo.
	pub fn settings_send_as_smime_info_get(&self, user_id: &str, send_as_email: &str, id: &str) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		UserSettingSendASmimeInfoGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Insert (upload) the given S/MIME config for the specified send-as alias. Note that pkcs12 format is required for the key.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The email address that appears in the "From:" header for mail sent using this alias.
	pub fn settings_send_as_smime_info_insert(&self, request: SmimeInfo, user_id: &str, send_as_email: &str) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		UserSettingSendASmimeInfoInsertCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists S/MIME configs for the specified send-as alias.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The email address that appears in the "From:" header for mail sent using this alias.
	pub fn settings_send_as_smime_info_list(&self, user_id: &str, send_as_email: &str) -> UserSettingSendASmimeInfoListCall<'a, S> {
		UserSettingSendASmimeInfoListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Sets the default S/MIME config for the specified send-as alias.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The email address that appears in the "From:" header for mail sent using this alias.
	/// * `id` - The immutable ID for the SmimeInfo.
	pub fn settings_send_as_smime_info_set_default(&self, user_id: &str, send_as_email: &str, id: &str) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		UserSettingSendASmimeInfoSetDefaultCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Creates a custom "from" send-as alias. If an SMTP MSA is specified, Gmail will attempt to connect to the SMTP service to validate the configuration before creating the alias. If ownership verification is required for the alias, a message will be sent to the email address and the resource's verification status will be set to `pending`; otherwise, the resource will be created with verification status set to `accepted`. If a signature is provided, Gmail will sanitize the HTML before saving it with the alias. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_send_as_create(&self, request: SendAs, user_id: &str) -> UserSettingSendACreateCall<'a, S> {
		UserSettingSendACreateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Deletes the specified send-as alias. Revokes any verification that may have been required for using it. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The send-as alias to be deleted.
	pub fn settings_send_as_delete(&self, user_id: &str, send_as_email: &str) -> UserSettingSendADeleteCall<'a, S> {
		UserSettingSendADeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified send-as alias. Fails with an HTTP 404 error if the specified address is not a member of the collection.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The send-as alias to be retrieved.
	pub fn settings_send_as_get(&self, user_id: &str, send_as_email: &str) -> UserSettingSendAGetCall<'a, S> {
		UserSettingSendAGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the send-as aliases for the specified account. The result includes the primary send-as address associated with the account as well as any custom "from" aliases.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_send_as_list(&self, user_id: &str) -> UserSettingSendAListCall<'a, S> {
		UserSettingSendAListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Patch the specified send-as alias.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The send-as alias to be updated.
	pub fn settings_send_as_patch(&self, request: SendAs, user_id: &str, send_as_email: &str) -> UserSettingSendAPatchCall<'a, S> {
		UserSettingSendAPatchCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates a send-as alias. If a signature is provided, Gmail will sanitize the HTML before saving it with the alias. Addresses other than the primary address for the account can only be updated by service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The send-as alias to be updated.
	pub fn settings_send_as_update(&self, request: SendAs, user_id: &str, send_as_email: &str) -> UserSettingSendAUpdateCall<'a, S> {
		UserSettingSendAUpdateCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Sends a verification email to the specified send-as alias address. The verification status must be `pending`. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	/// * `sendAsEmail` - The send-as alias to be verified.
	pub fn settings_send_as_verify(&self, user_id: &str, send_as_email: &str) -> UserSettingSendAVerifyCall<'a, S> {
		UserSettingSendAVerifyCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_send_as_email: send_as_email.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the auto-forwarding setting for the specified account.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_get_auto_forwarding(&self, user_id: &str) -> UserSettingGetAutoForwardingCall<'a, S> {
		UserSettingGetAutoForwardingCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets IMAP settings.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_get_imap(&self, user_id: &str) -> UserSettingGetImapCall<'a, S> {
		UserSettingGetImapCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets language settings.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_get_language(&self, user_id: &str) -> UserSettingGetLanguageCall<'a, S> {
		UserSettingGetLanguageCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets POP settings.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_get_pop(&self, user_id: &str) -> UserSettingGetPopCall<'a, S> {
		UserSettingGetPopCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets vacation responder settings.
	///
	/// # Arguments
	///
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_get_vacation(&self, user_id: &str) -> UserSettingGetVacationCall<'a, S> {
		UserSettingGetVacationCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates the auto-forwarding setting for the specified account. A verified forwarding address must be specified when auto-forwarding is enabled. This method is only available to service account clients that have been delegated domain-wide authority.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_update_auto_forwarding(&self, request: AutoForwarding, user_id: &str) -> UserSettingUpdateAutoForwardingCall<'a, S> {
		UserSettingUpdateAutoForwardingCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates IMAP settings.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_update_imap(&self, request: ImapSettings, user_id: &str) -> UserSettingUpdateImapCall<'a, S> {
		UserSettingUpdateImapCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates language settings. If successful, the return object contains the `displayLanguage` that was saved for the user, which may differ from the value passed into the request. This is because the requested `displayLanguage` may not be directly supported by Gmail but have a close variant that is, and so the variant may be chosen and saved instead.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_update_language(&self, request: LanguageSettings, user_id: &str) -> UserSettingUpdateLanguageCall<'a, S> {
		UserSettingUpdateLanguageCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates POP settings.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_update_pop(&self, request: PopSettings, user_id: &str) -> UserSettingUpdatePopCall<'a, S> {
		UserSettingUpdatePopCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Updates vacation responder settings.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - User's email address. The special value "me" can be used to indicate the authenticated user.
	pub fn settings_update_vacation(&self, request: VacationSettings, user_id: &str) -> UserSettingUpdateVacationCall<'a, S> {
		UserSettingUpdateVacationCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Immediately and permanently deletes the specified thread. Any messages that belong to the thread are also deleted. This operation cannot be undone. Prefer `threads.trash` instead.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - ID of the Thread to delete.
	pub fn threads_delete(&self, user_id: &str, id: &str) -> UserThreadDeleteCall<'a, S> {
		UserThreadDeleteCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the specified thread.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the thread to retrieve.
	pub fn threads_get(&self, user_id: &str, id: &str) -> UserThreadGetCall<'a, S> {
		UserThreadGetCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_metadata_headers: Default::default(),
			_format: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Lists the threads in the user's mailbox.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn threads_list(&self, user_id: &str) -> UserThreadListCall<'a, S> {
		UserThreadListCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_q: Default::default(),
			_page_token: Default::default(),
			_max_results: Default::default(),
			_label_ids: Default::default(),
			_include_spam_trash: Default::default(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Modifies the labels applied to the thread. This applies to all messages in the thread.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the thread to modify.
	pub fn threads_modify(&self, request: ModifyThreadRequest, user_id: &str, id: &str) -> UserThreadModifyCall<'a, S> {
		UserThreadModifyCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Moves the specified thread to the trash. Any messages that belong to the thread are also moved to the trash.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the thread to Trash.
	pub fn threads_trash(&self, user_id: &str, id: &str) -> UserThreadTrashCall<'a, S> {
		UserThreadTrashCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Removes the specified thread from the trash. Any messages that belong to the thread are also removed from the trash.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	/// * `id` - The ID of the thread to remove from Trash.
	pub fn threads_untrash(&self, user_id: &str, id: &str) -> UserThreadUntrashCall<'a, S> {
		UserThreadUntrashCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_id: id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Gets the current user's Gmail profile.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn get_profile(&self, user_id: &str) -> UserGetProfileCall<'a, S> {
		UserGetProfileCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Stop receiving push notifications for the given user mailbox.
	///
	/// # Arguments
	///
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn stop(&self, user_id: &str) -> UserStopCall<'a, S> {
		UserStopCall {
			hub: self.hub,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}

	/// Create a builder to help you perform the following task:
	///
	/// Set up or update a push notification watch on the given user mailbox.
	///
	/// # Arguments
	///
	/// * `request` - No description provided.
	/// * `userId` - The user's email address. The special value `me` can be used to indicate the authenticated user.
	pub fn watch(&self, request: WatchRequest, user_id: &str) -> UserWatchCall<'a, S> {
		UserWatchCall {
			hub: self.hub,
			_request: request,
			_user_id: user_id.to_string(),
			_delegate: Default::default(),
			_additional_params: Default::default(),
			_scopes: Default::default(),
		}
	}
}

// ###################
// CallBuilders   ###
// #################

/// Creates a new draft with the `DRAFT` label.
///
/// A builder for the *drafts.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Draft;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Draft::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_create(req, "userId")
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserDraftCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Draft,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftCreateCall<'a, S> {}

impl<'a, S> UserDraftCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/drafts", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/drafts", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 36700160 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::POST)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 36700160 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Draft) -> UserDraftCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Immediately and permanently deletes the specified draft. Does not simply trash it.
///
/// A builder for the *drafts.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_delete("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserDraftDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftDeleteCall<'a, S> {}

impl<'a, S> UserDraftDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/drafts/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the draft to delete.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserDraftDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified draft.
///
/// A builder for the *drafts.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_get("userId", "id")
///              .format("ipsum")
///              .doit().await;
/// # }
/// ```
pub struct UserDraftGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_format: Option<String>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftGetCall<'a, S> {}

impl<'a, S> UserDraftGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "id", "format"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);
		if let Some(value) = self._format.as_ref() {
			params.push("format", value);
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/drafts/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the draft to retrieve.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserDraftGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The format to return the draft in.
	///
	/// Sets the *format* query property to the given value.
	pub fn format(mut self, new_value: &str) -> UserDraftGetCall<'a, S> {
		self._format = Some(new_value.to_string());
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the drafts in the user's mailbox.
///
/// A builder for the *drafts.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_list("userId")
///              .q("est")
///              .page_token("gubergren")
///              .max_results(84)
///              .include_spam_trash(false)
///              .doit().await;
/// # }
/// ```
pub struct UserDraftListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_q: Option<String>,
	_page_token: Option<String>,
	_max_results: Option<u32>,
	_include_spam_trash: Option<bool>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftListCall<'a, S> {}

impl<'a, S> UserDraftListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListDraftsResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "q", "pageToken", "maxResults", "includeSpamTrash"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(7 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._q.as_ref() {
			params.push("q", value);
		}
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._max_results.as_ref() {
			params.push("maxResults", value.to_string());
		}
		if let Some(value) = self._include_spam_trash.as_ref() {
			params.push("includeSpamTrash", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/drafts";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Only return draft messages matching the specified query. Supports the same query format as the Gmail search box. For example, `"from:someuser@example.com rfc822msgid: is:unread"`.
	///
	/// Sets the *q* query property to the given value.
	pub fn q(mut self, new_value: &str) -> UserDraftListCall<'a, S> {
		self._q = Some(new_value.to_string());
		self
	}
	/// Page token to retrieve a specific page of results in the list.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserDraftListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// Maximum number of drafts to return. This field defaults to 100. The maximum allowed value for this field is 500.
	///
	/// Sets the *max results* query property to the given value.
	pub fn max_results(mut self, new_value: u32) -> UserDraftListCall<'a, S> {
		self._max_results = Some(new_value);
		self
	}
	/// Include drafts from `SPAM` and `TRASH` in the results.
	///
	/// Sets the *include spam trash* query property to the given value.
	pub fn include_spam_trash(mut self, new_value: bool) -> UserDraftListCall<'a, S> {
		self._include_spam_trash = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Sends the specified, existing draft to the recipients in the `To`, `Cc`, and `Bcc` headers.
///
/// A builder for the *drafts.send* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Draft;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Draft::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_send(req, "userId")
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserDraftSendCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Draft,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftSendCall<'a, S> {}

impl<'a, S> UserDraftSendCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.send",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/drafts/send", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/drafts/send", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 36700160 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::POST)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 36700160 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Draft) -> UserDraftSendCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftSendCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftSendCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftSendCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftSendCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftSendCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftSendCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Replaces a draft's content.
///
/// A builder for the *drafts.update* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Draft;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Draft::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().drafts_update(req, "userId", "id")
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserDraftUpdateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Draft,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserDraftUpdateCall<'a, S> {}

impl<'a, S> UserDraftUpdateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.drafts.update",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/drafts/{id}", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/drafts/{id}", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 36700160 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::PUT)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 36700160 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Draft)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Draft) -> UserDraftUpdateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserDraftUpdateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the draft to update.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserDraftUpdateCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserDraftUpdateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserDraftUpdateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserDraftUpdateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserDraftUpdateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserDraftUpdateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the history of all changes to the given mailbox. History results are returned in chronological order (increasing `historyId`).
///
/// A builder for the *history.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().history_list("userId")
///              .start_history_id(31)
///              .page_token("sed")
///              .max_results(40)
///              .label_id("Stet")
///              .add_history_types("kasd")
///              .doit().await;
/// # }
/// ```
pub struct UserHistoryListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_start_history_id: Option<u64>,
	_page_token: Option<String>,
	_max_results: Option<u32>,
	_label_id: Option<String>,
	_history_types: Vec<String>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserHistoryListCall<'a, S> {}

impl<'a, S> UserHistoryListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListHistoryResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.history.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "startHistoryId", "pageToken", "maxResults", "labelId", "historyTypes"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(8 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._start_history_id.as_ref() {
			params.push("startHistoryId", value.to_string());
		}
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._max_results.as_ref() {
			params.push("maxResults", value.to_string());
		}
		if let Some(value) = self._label_id.as_ref() {
			params.push("labelId", value);
		}
		if self._history_types.len() > 0 {
			for f in self._history_types.iter() {
				params.push("historyTypes", f);
			}
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/history";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserHistoryListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Required. Returns history records after the specified `startHistoryId`. The supplied `startHistoryId` should be obtained from the `historyId` of a message, thread, or previous `list` response. History IDs increase chronologically but are not contiguous with random gaps in between valid IDs. Supplying an invalid or out of date `startHistoryId` typically returns an `HTTP 404` error code. A `historyId` is typically valid for at least a week, but in some rare circumstances may be valid for only a few hours. If you receive an `HTTP 404` error response, your application should perform a full sync. If you receive no `nextPageToken` in the response, there are no updates to retrieve and you can store the returned `historyId` for a future request.
	///
	/// Sets the *start history id* query property to the given value.
	pub fn start_history_id(mut self, new_value: u64) -> UserHistoryListCall<'a, S> {
		self._start_history_id = Some(new_value);
		self
	}
	/// Page token to retrieve a specific page of results in the list.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserHistoryListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// Maximum number of history records to return. This field defaults to 100. The maximum allowed value for this field is 500.
	///
	/// Sets the *max results* query property to the given value.
	pub fn max_results(mut self, new_value: u32) -> UserHistoryListCall<'a, S> {
		self._max_results = Some(new_value);
		self
	}
	/// Only return messages with a label matching the ID.
	///
	/// Sets the *label id* query property to the given value.
	pub fn label_id(mut self, new_value: &str) -> UserHistoryListCall<'a, S> {
		self._label_id = Some(new_value.to_string());
		self
	}
	/// History types to be returned by the function
	///
	/// Append the given value to the *history types* query property.
	/// Each appended value will retain its original ordering and be '/'-separated in the URL's parameters.
	pub fn add_history_types(mut self, new_value: &str) -> UserHistoryListCall<'a, S> {
		self._history_types.push(new_value.to_string());
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserHistoryListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserHistoryListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserHistoryListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserHistoryListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserHistoryListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates a new label.
///
/// A builder for the *labels.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Label;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Label::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Label,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelCreateCall<'a, S> {}

impl<'a, S> UserLabelCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Label)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Label) -> UserLabelCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Immediately and permanently deletes the specified label and removes it from any messages and threads that it is applied to.
///
/// A builder for the *labels.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_delete("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelDeleteCall<'a, S> {}

impl<'a, S> UserLabelDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the label to delete.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserLabelDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified label.
///
/// A builder for the *labels.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_get("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelGetCall<'a, S> {}

impl<'a, S> UserLabelGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Label)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the label to retrieve.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserLabelGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists all labels in the user's mailbox.
///
/// A builder for the *labels.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_list("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelListCall<'a, S> {}

impl<'a, S> UserLabelListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListLabelsResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Patch the specified label.
///
/// A builder for the *labels.patch* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Label;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Label::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_patch(req, "userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelPatchCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Label,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelPatchCall<'a, S> {}

impl<'a, S> UserLabelPatchCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Label)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.patch",
			http_method: hyper::Method::PATCH,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PATCH)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Label) -> UserLabelPatchCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelPatchCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the label to update.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserLabelPatchCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelPatchCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelPatchCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelPatchCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelPatchCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelPatchCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates the specified label.
///
/// A builder for the *labels.update* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Label;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Label::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().labels_update(req, "userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserLabelUpdateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Label,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserLabelUpdateCall<'a, S> {}

impl<'a, S> UserLabelUpdateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Label)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.labels.update",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/labels/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Label) -> UserLabelUpdateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserLabelUpdateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the label to update.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserLabelUpdateCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserLabelUpdateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserLabelUpdateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserLabelUpdateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserLabelUpdateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserLabelUpdateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified message attachment.
///
/// A builder for the *messages.attachments.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_attachments_get("userId", "messageId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageAttachmentGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_message_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageAttachmentGetCall<'a, S> {}

impl<'a, S> UserMessageAttachmentGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, MessagePartBody)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.attachments.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "messageId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("messageId", self._message_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{messageId}/attachments/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::AddonCurrentMessageReadonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{messageId}", "messageId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "messageId", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageAttachmentGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message containing the attachment.
	///
	/// Sets the *message id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn message_id(mut self, new_value: &str) -> UserMessageAttachmentGetCall<'a, S> {
		self._message_id = new_value.to_string();
		self
	}
	/// The ID of the attachment.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageAttachmentGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageAttachmentGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageAttachmentGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::AddonCurrentMessageReadonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageAttachmentGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageAttachmentGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageAttachmentGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes many messages by message ID. Provides no guarantees that messages were not already deleted or even existed at all.
///
/// A builder for the *messages.batchDelete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::BatchDeleteMessagesRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = BatchDeleteMessagesRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_batch_delete(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageBatchDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: BatchDeleteMessagesRequest,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageBatchDeleteCall<'a, S> {}

impl<'a, S> UserMessageBatchDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.batchDelete",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/batchDelete";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: BatchDeleteMessagesRequest) -> UserMessageBatchDeleteCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageBatchDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageBatchDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageBatchDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageBatchDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageBatchDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageBatchDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Modifies the labels on the specified messages.
///
/// A builder for the *messages.batchModify* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::BatchModifyMessagesRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = BatchModifyMessagesRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_batch_modify(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageBatchModifyCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: BatchModifyMessagesRequest,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageBatchModifyCall<'a, S> {}

impl<'a, S> UserMessageBatchModifyCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.batchModify",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/batchModify";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: BatchModifyMessagesRequest) -> UserMessageBatchModifyCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageBatchModifyCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageBatchModifyCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageBatchModifyCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageBatchModifyCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageBatchModifyCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageBatchModifyCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Immediately and permanently deletes the specified message. This operation cannot be undone. Prefer `messages.trash` instead.
///
/// A builder for the *messages.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_delete("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageDeleteCall<'a, S> {}

impl<'a, S> UserMessageDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message to delete.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified message.
///
/// A builder for the *messages.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_get("userId", "id")
///              .add_metadata_headers("dolor")
///              .format("duo")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_metadata_headers: Vec<String>,
	_format: Option<String>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageGetCall<'a, S> {}

impl<'a, S> UserMessageGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Message)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "id", "metadataHeaders", "format"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(6 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);
		if self._metadata_headers.len() > 0 {
			for f in self._metadata_headers.iter() {
				params.push("metadataHeaders", f);
			}
		}
		if let Some(value) = self._format.as_ref() {
			params.push("format", value);
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::AddonCurrentMessageReadonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message to retrieve. This ID is usually retrieved using `messages.list`. The ID is also contained in the result when a message is inserted (`messages.insert`) or imported (`messages.import`).
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// When given and format is `METADATA`, only include headers specified.
	///
	/// Append the given value to the *metadata headers* query property.
	/// Each appended value will retain its original ordering and be '/'-separated in the URL's parameters.
	pub fn add_metadata_headers(mut self, new_value: &str) -> UserMessageGetCall<'a, S> {
		self._metadata_headers.push(new_value.to_string());
		self
	}
	/// The format to return the message in.
	///
	/// Sets the *format* query property to the given value.
	pub fn format(mut self, new_value: &str) -> UserMessageGetCall<'a, S> {
		self._format = Some(new_value.to_string());
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::AddonCurrentMessageReadonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Imports a message into only this user's mailbox, with standard email delivery scanning and classification similar to receiving via SMTP. This method doesn't perform SPF checks, so it might not work for some spam messages, such as those attempting to perform domain spoofing. This method does not send a message.
///
/// A builder for the *messages.import* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Message;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Message::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_import(req, "userId")
///              .process_for_calendar(false)
///              .never_mark_spam(false)
///              .internal_date_source("Stet")
///              .deleted(false)
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserMessageImportCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Message,
	_user_id: String,
	_process_for_calendar: Option<bool>,
	_never_mark_spam: Option<bool>,
	_internal_date_source: Option<String>,
	_deleted: Option<bool>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageImportCall<'a, S> {}

impl<'a, S> UserMessageImportCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.import",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "processForCalendar", "neverMarkSpam", "internalDateSource", "deleted"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(8 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._process_for_calendar.as_ref() {
			params.push("processForCalendar", value.to_string());
		}
		if let Some(value) = self._never_mark_spam.as_ref() {
			params.push("neverMarkSpam", value.to_string());
		}
		if let Some(value) = self._internal_date_source.as_ref() {
			params.push("internalDateSource", value);
		}
		if let Some(value) = self._deleted.as_ref() {
			params.push("deleted", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/messages/import", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/messages/import", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 52428800 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 52428800));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::POST)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 52428800 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 52428800));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 52428800
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 52428800
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Message) -> UserMessageImportCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageImportCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Process calendar invites in the email and add any extracted meetings to the Google Calendar for this user.
	///
	/// Sets the *process for calendar* query property to the given value.
	pub fn process_for_calendar(mut self, new_value: bool) -> UserMessageImportCall<'a, S> {
		self._process_for_calendar = Some(new_value);
		self
	}
	/// Ignore the Gmail spam classifier decision and never mark this email as SPAM in the mailbox.
	///
	/// Sets the *never mark spam* query property to the given value.
	pub fn never_mark_spam(mut self, new_value: bool) -> UserMessageImportCall<'a, S> {
		self._never_mark_spam = Some(new_value);
		self
	}
	/// Source for Gmail's internal date of the message.
	///
	/// Sets the *internal date source* query property to the given value.
	pub fn internal_date_source(mut self, new_value: &str) -> UserMessageImportCall<'a, S> {
		self._internal_date_source = Some(new_value.to_string());
		self
	}
	/// Mark the email as permanently deleted (not TRASH) and only visible in Google Vault to a Vault administrator. Only used for Google Workspace accounts.
	///
	/// Sets the *deleted* query property to the given value.
	pub fn deleted(mut self, new_value: bool) -> UserMessageImportCall<'a, S> {
		self._deleted = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageImportCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageImportCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageImportCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageImportCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageImportCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Directly inserts a message into only this user's mailbox similar to `IMAP APPEND`, bypassing most scanning and classification. Does not send a message.
///
/// A builder for the *messages.insert* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Message;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Message::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_insert(req, "userId")
///              .internal_date_source("Lorem")
///              .deleted(true)
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserMessageInsertCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Message,
	_user_id: String,
	_internal_date_source: Option<String>,
	_deleted: Option<bool>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageInsertCall<'a, S> {}

impl<'a, S> UserMessageInsertCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.insert",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "internalDateSource", "deleted"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(6 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._internal_date_source.as_ref() {
			params.push("internalDateSource", value);
		}
		if let Some(value) = self._deleted.as_ref() {
			params.push("deleted", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/messages", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/messages", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 52428800 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 52428800));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::POST)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 52428800 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 52428800));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 52428800
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 52428800
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Message) -> UserMessageInsertCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageInsertCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Source for Gmail's internal date of the message.
	///
	/// Sets the *internal date source* query property to the given value.
	pub fn internal_date_source(mut self, new_value: &str) -> UserMessageInsertCall<'a, S> {
		self._internal_date_source = Some(new_value.to_string());
		self
	}
	/// Mark the email as permanently deleted (not TRASH) and only visible in Google Vault to a Vault administrator. Only used for Google Workspace accounts.
	///
	/// Sets the *deleted* query property to the given value.
	pub fn deleted(mut self, new_value: bool) -> UserMessageInsertCall<'a, S> {
		self._deleted = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageInsertCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageInsertCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageInsertCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageInsertCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageInsertCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the messages in the user's mailbox.
///
/// A builder for the *messages.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_list("userId")
///              .q("accusam")
///              .page_token("takimata")
///              .max_results(55)
///              .add_label_ids("voluptua.")
///              .include_spam_trash(false)
///              .doit().await;
/// # }
/// ```
pub struct UserMessageListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_q: Option<String>,
	_page_token: Option<String>,
	_max_results: Option<u32>,
	_label_ids: Vec<String>,
	_include_spam_trash: Option<bool>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageListCall<'a, S> {}

impl<'a, S> UserMessageListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListMessagesResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "q", "pageToken", "maxResults", "labelIds", "includeSpamTrash"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(8 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._q.as_ref() {
			params.push("q", value);
		}
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._max_results.as_ref() {
			params.push("maxResults", value.to_string());
		}
		if self._label_ids.len() > 0 {
			for f in self._label_ids.iter() {
				params.push("labelIds", f);
			}
		}
		if let Some(value) = self._include_spam_trash.as_ref() {
			params.push("includeSpamTrash", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Only return messages matching the specified query. Supports the same query format as the Gmail search box. For example, `"from:someuser@example.com rfc822msgid: is:unread"`. Parameter cannot be used when accessing the api using the gmail.metadata scope.
	///
	/// Sets the *q* query property to the given value.
	pub fn q(mut self, new_value: &str) -> UserMessageListCall<'a, S> {
		self._q = Some(new_value.to_string());
		self
	}
	/// Page token to retrieve a specific page of results in the list.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserMessageListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// Maximum number of messages to return. This field defaults to 100. The maximum allowed value for this field is 500.
	///
	/// Sets the *max results* query property to the given value.
	pub fn max_results(mut self, new_value: u32) -> UserMessageListCall<'a, S> {
		self._max_results = Some(new_value);
		self
	}
	/// Only return messages with labels that match all of the specified label IDs. Messages in a thread might have labels that other messages in the same thread don't have. To learn more, see [Manage labels on messages and threads](https://developers.google.com/gmail/api/guides/labels#manage_labels_on_messages_threads).
	///
	/// Append the given value to the *label ids* query property.
	/// Each appended value will retain its original ordering and be '/'-separated in the URL's parameters.
	pub fn add_label_ids(mut self, new_value: &str) -> UserMessageListCall<'a, S> {
		self._label_ids.push(new_value.to_string());
		self
	}
	/// Include messages from `SPAM` and `TRASH` in the results.
	///
	/// Sets the *include spam trash* query property to the given value.
	pub fn include_spam_trash(mut self, new_value: bool) -> UserMessageListCall<'a, S> {
		self._include_spam_trash = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Modifies the labels on the specified message.
///
/// A builder for the *messages.modify* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::ModifyMessageRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = ModifyMessageRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_modify(req, "userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageModifyCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: ModifyMessageRequest,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageModifyCall<'a, S> {}

impl<'a, S> UserMessageModifyCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Message)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.modify",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{id}/modify";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: ModifyMessageRequest) -> UserMessageModifyCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageModifyCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message to modify.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageModifyCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageModifyCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageModifyCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageModifyCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageModifyCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageModifyCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Sends the specified message to the recipients in the `To`, `Cc`, and `Bcc` headers. For example usage, see [Sending email](https://developers.google.com/gmail/api/guides/sending).
///
/// A builder for the *messages.send* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Message;
/// use std::fs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Message::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `upload_resumable(...)`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_send(req, "userId")
///              .upload_resumable(fs::File::open("file.ext").unwrap(), "application/octet-stream".parse().unwrap()).await;
/// # }
/// ```
pub struct UserMessageSendCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Message,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageSendCall<'a, S> {}

impl<'a, S> UserMessageSendCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	async fn doit<RS>(mut self, mut reader: RS, reader_mime_type: mime::Mime, protocol: client::UploadProtocol) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.send",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let (mut url, upload_type) = if protocol == client::UploadProtocol::Resumable {
			(self.hub._root_url.clone() + "resumable/upload/gmail/v1/users/{userId}/messages/send", "resumable")
		} else if protocol == client::UploadProtocol::Simple {
			(self.hub._root_url.clone() + "upload/gmail/v1/users/{userId}/messages/send", "multipart")
		} else {
			unreachable!()
		};
		params.push("uploadType", upload_type);
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		let mut should_ask_dlg_for_url = false;
		let mut upload_url_from_server;
		let mut upload_url: Option<String> = None;

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				if should_ask_dlg_for_url && (upload_url = dlg.upload_url()) == () && upload_url.is_some() {
					should_ask_dlg_for_url = false;
					upload_url_from_server = false;
					Ok(
						hyper::Response::builder()
							.status(hyper::StatusCode::OK)
							.header("Location", upload_url.as_ref().unwrap().clone())
							.body(hyper::body::Body::empty())
							.unwrap(),
					)
				} else {
					let mut mp_reader: client::MultiPartReader = Default::default();
					let (mut body_reader, content_type) = match protocol {
						client::UploadProtocol::Simple => {
							mp_reader.reserve_exact(2);
							let size = reader.seek(io::SeekFrom::End(0)).unwrap();
							reader.seek(io::SeekFrom::Start(0)).unwrap();
							if size > 36700160 {
								return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
							}
							mp_reader
								.add_part(&mut request_value_reader, request_size, json_mime_type.clone())
								.add_part(&mut reader, size, reader_mime_type.clone());
							(&mut mp_reader as &mut (dyn io::Read + Send), client::MultiPartReader::mime_type())
						}
						_ => (&mut request_value_reader as &mut (dyn io::Read + Send), json_mime_type.clone()),
					};
					let client = &self.hub.client;
					dlg.pre_request();
					let mut req_builder = hyper::Request::builder()
						.method(hyper::Method::POST)
						.uri(url.as_str())
						.header(USER_AGENT, self.hub._user_agent.clone());

					if let Some(token) = token.as_ref() {
						req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
					}

					upload_url_from_server = true;
					if protocol == client::UploadProtocol::Resumable {
						req_builder = req_builder.header("X-Upload-Content-Type", format!("{}", reader_mime_type));
					}

					let mut body_reader_bytes = vec![];
					body_reader.read_to_end(&mut body_reader_bytes).unwrap();
					let request = req_builder.header(CONTENT_TYPE, content_type.to_string()).body(hyper::body::Body::from(body_reader_bytes));

					client.request(request.unwrap()).await
				}
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					if protocol == client::UploadProtocol::Resumable {
						let size = reader.seek(io::SeekFrom::End(0)).unwrap();
						reader.seek(io::SeekFrom::Start(0)).unwrap();
						if size > 36700160 {
							return Err(client::Error::UploadSizeLimitExceeded(size, 36700160));
						}
						let upload_result = {
							let url_str = &res.headers().get("Location").expect("LOCATION header is part of protocol").to_str().unwrap();
							if upload_url_from_server {
								dlg.store_upload_url(Some(url_str));
							}

							client::ResumableUploadHelper {
								client: &self.hub.client,
								delegate: dlg,
								start_at: if upload_url_from_server { Some(0) } else { None },
								auth: &self.hub.auth,
								user_agent: &self.hub._user_agent,
								// TODO: Check this assumption
								auth_header: format!(
									"Bearer {}",
									token.ok_or_else(|| client::Error::MissingToken("resumable upload requires token".into()))?.as_str()
								),
								url: url_str,
								reader: &mut reader,
								media_type: reader_mime_type.clone(),
								content_length: size,
							}
							.upload()
							.await
						};
						match upload_result {
							None => {
								dlg.finished(false);
								return Err(client::Error::Cancelled);
							}
							Some(Err(err)) => {
								dlg.finished(false);
								return Err(client::Error::HttpError(err));
							}
							Some(Ok(upload_result)) => {
								res = upload_result;
								if !res.status().is_success() {
									dlg.store_upload_url(None);
									dlg.finished(false);
									return Err(client::Error::Failure(res));
								}
							}
						}
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// Upload media in a resumable fashion.
	/// Even if the upload fails or is interrupted, it can be resumed for a
	/// certain amount of time as the server maintains state temporarily.
	///
	/// The delegate will be asked for an `upload_url()`, and if not provided, will be asked to store an upload URL
	/// that was provided by the server, using `store_upload_url(...)`. The upload will be done in chunks, the delegate
	/// may specify the `chunk_size()` and may cancel the operation before each chunk is uploaded, using
	/// `cancel_chunk_upload(...)`.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload_resumable<RS>(self, resumeable_stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(resumeable_stream, mime_type, client::UploadProtocol::Resumable).await
	}
	/// Upload media all at once.
	/// If the upload fails for whichever reason, all progress is lost.
	///
	/// * *multipart*: yes
	/// * *max size*: 36700160
	/// * *valid mime types*: 'message/*'
	pub async fn upload<RS>(self, stream: RS, mime_type: mime::Mime) -> client::Result<(hyper::Response<hyper::body::Body>, Message)>
	where
		RS: client::ReadSeek,
	{
		self.doit(stream, mime_type, client::UploadProtocol::Simple).await
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Message) -> UserMessageSendCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageSendCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageSendCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageSendCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageSendCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageSendCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageSendCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Moves the specified message to the trash.
///
/// A builder for the *messages.trash* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_trash("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageTrashCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageTrashCall<'a, S> {}

impl<'a, S> UserMessageTrashCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Message)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.trash",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{id}/trash";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageTrashCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message to Trash.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageTrashCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageTrashCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageTrashCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageTrashCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageTrashCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageTrashCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Removes the specified message from the trash.
///
/// A builder for the *messages.untrash* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().messages_untrash("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserMessageUntrashCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserMessageUntrashCall<'a, S> {}

impl<'a, S> UserMessageUntrashCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Message)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.messages.untrash",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/messages/{id}/untrash";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserMessageUntrashCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the message to remove from Trash.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserMessageUntrashCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserMessageUntrashCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserMessageUntrashCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserMessageUntrashCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserMessageUntrashCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserMessageUntrashCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates and configures a client-side encryption identity that's authorized to send mail from the user account. Google publishes the S/MIME certificate to a shared domain-wide directory so that people within a Google Workspace organization can encrypt and send mail to the identity.
///
/// A builder for the *settings.cse.identities.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::CseIdentity;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = CseIdentity::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_identities_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseIdentityCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: CseIdentity,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseIdentityCreateCall<'a, S> {}

impl<'a, S> UserSettingCseIdentityCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseIdentity)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.identities.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/identities";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: CseIdentity) -> UserSettingCseIdentityCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseIdentityCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseIdentityCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseIdentityCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseIdentityCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseIdentityCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseIdentityCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes a client-side encryption identity. The authenticated user can no longer use the identity to send encrypted messages. You cannot restore the identity after you delete it. Instead, use the CreateCseIdentity method to create another identity with the same configuration.
///
/// A builder for the *settings.cse.identities.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_identities_delete("userId", "cseEmailAddress")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseIdentityDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_cse_email_address: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseIdentityDeleteCall<'a, S> {}

impl<'a, S> UserSettingCseIdentityDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.identities.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "cseEmailAddress"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("cseEmailAddress", self._cse_email_address);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/identities/{cseEmailAddress}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{cseEmailAddress}", "cseEmailAddress")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["cseEmailAddress", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseIdentityDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The primary email address associated with the client-side encryption identity configuration that's removed.
	///
	/// Sets the *cse email address* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn cse_email_address(mut self, new_value: &str) -> UserSettingCseIdentityDeleteCall<'a, S> {
		self._cse_email_address = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseIdentityDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseIdentityDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseIdentityDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseIdentityDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseIdentityDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Retrieves a client-side encryption identity configuration.
///
/// A builder for the *settings.cse.identities.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_identities_get("userId", "cseEmailAddress")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseIdentityGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_cse_email_address: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseIdentityGetCall<'a, S> {}

impl<'a, S> UserSettingCseIdentityGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseIdentity)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.identities.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "cseEmailAddress"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("cseEmailAddress", self._cse_email_address);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/identities/{cseEmailAddress}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{cseEmailAddress}", "cseEmailAddress")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["cseEmailAddress", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseIdentityGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The primary email address associated with the client-side encryption identity configuration that's retrieved.
	///
	/// Sets the *cse email address* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn cse_email_address(mut self, new_value: &str) -> UserSettingCseIdentityGetCall<'a, S> {
		self._cse_email_address = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseIdentityGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseIdentityGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseIdentityGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseIdentityGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseIdentityGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the client-side encrypted identities for an authenticated user.
///
/// A builder for the *settings.cse.identities.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_identities_list("userId")
///              .page_token("voluptua.")
///              .page_size(-2)
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseIdentityListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_page_token: Option<String>,
	_page_size: Option<i32>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseIdentityListCall<'a, S> {}

impl<'a, S> UserSettingCseIdentityListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListCseIdentitiesResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.identities.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "pageToken", "pageSize"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._page_size.as_ref() {
			params.push("pageSize", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/identities";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseIdentityListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Pagination token indicating which page of identities to return. If the token is not supplied, then the API will return the first page of results.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserSettingCseIdentityListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// The number of identities to return. If not provided, the page size will default to 20 entries.
	///
	/// Sets the *page size* query property to the given value.
	pub fn page_size(mut self, new_value: i32) -> UserSettingCseIdentityListCall<'a, S> {
		self._page_size = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseIdentityListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseIdentityListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseIdentityListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseIdentityListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseIdentityListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Associates a different key pair with an existing client-side encryption identity. The updated key pair must validate against Google's [S/MIME certificate profiles](https://support.google.com/a/answer/7300887).
///
/// A builder for the *settings.cse.identities.patch* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::CseIdentity;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = CseIdentity::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_identities_patch(req, "userId", "emailAddress")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseIdentityPatchCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: CseIdentity,
	_user_id: String,
	_email_address: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseIdentityPatchCall<'a, S> {}

impl<'a, S> UserSettingCseIdentityPatchCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseIdentity)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.identities.patch",
			http_method: hyper::Method::PATCH,
		});

		for &field in ["alt", "userId", "emailAddress"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("emailAddress", self._email_address);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/identities/{emailAddress}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{emailAddress}", "emailAddress")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["emailAddress", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PATCH)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: CseIdentity) -> UserSettingCseIdentityPatchCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseIdentityPatchCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address of the client-side encryption identity to update.
	///
	/// Sets the *email address* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn email_address(mut self, new_value: &str) -> UserSettingCseIdentityPatchCall<'a, S> {
		self._email_address = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseIdentityPatchCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseIdentityPatchCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseIdentityPatchCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseIdentityPatchCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseIdentityPatchCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates and uploads a client-side encryption S/MIME public key certificate chain and private key metadata for the authenticated user.
///
/// A builder for the *settings.cse.keypairs.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::CseKeyPair;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = CseKeyPair::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: CseKeyPair,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairCreateCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseKeyPair)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: CseKeyPair) -> UserSettingCseKeypairCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Turns off a client-side encryption key pair. The authenticated user can no longer use the key pair to decrypt incoming CSE message texts or sign outgoing CSE mail. To regain access, use the EnableCseKeyPair to turn on the key pair. After 30 days, you can permanently delete the key pair by using the ObliterateCseKeyPair method.
///
/// A builder for the *settings.cse.keypairs.disable* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::DisableCseKeyPairRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = DisableCseKeyPairRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_disable(req, "userId", "keyPairId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairDisableCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: DisableCseKeyPairRequest,
	_user_id: String,
	_key_pair_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairDisableCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairDisableCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseKeyPair)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.disable",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "keyPairId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("keyPairId", self._key_pair_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs/{keyPairId}:disable";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{keyPairId}", "keyPairId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["keyPairId", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: DisableCseKeyPairRequest) -> UserSettingCseKeypairDisableCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairDisableCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The identifier of the key pair to turn off.
	///
	/// Sets the *key pair id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn key_pair_id(mut self, new_value: &str) -> UserSettingCseKeypairDisableCall<'a, S> {
		self._key_pair_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairDisableCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairDisableCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairDisableCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairDisableCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairDisableCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Turns on a client-side encryption key pair that was turned off. The key pair becomes active again for any associated client-side encryption identities.
///
/// A builder for the *settings.cse.keypairs.enable* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::EnableCseKeyPairRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = EnableCseKeyPairRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_enable(req, "userId", "keyPairId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairEnableCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: EnableCseKeyPairRequest,
	_user_id: String,
	_key_pair_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairEnableCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairEnableCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseKeyPair)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.enable",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "keyPairId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("keyPairId", self._key_pair_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs/{keyPairId}:enable";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{keyPairId}", "keyPairId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["keyPairId", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: EnableCseKeyPairRequest) -> UserSettingCseKeypairEnableCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairEnableCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The identifier of the key pair to turn on.
	///
	/// Sets the *key pair id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn key_pair_id(mut self, new_value: &str) -> UserSettingCseKeypairEnableCall<'a, S> {
		self._key_pair_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairEnableCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairEnableCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairEnableCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairEnableCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairEnableCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Retrieves an existing client-side encryption key pair.
///
/// A builder for the *settings.cse.keypairs.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_get("userId", "keyPairId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_key_pair_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairGetCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, CseKeyPair)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "keyPairId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("keyPairId", self._key_pair_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs/{keyPairId}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{keyPairId}", "keyPairId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["keyPairId", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The identifier of the key pair to retrieve.
	///
	/// Sets the *key pair id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn key_pair_id(mut self, new_value: &str) -> UserSettingCseKeypairGetCall<'a, S> {
		self._key_pair_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists client-side encryption key pairs for an authenticated user.
///
/// A builder for the *settings.cse.keypairs.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_list("userId")
///              .page_token("tempor")
///              .page_size(-32)
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_page_token: Option<String>,
	_page_size: Option<i32>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairListCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListCseKeyPairsResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "pageToken", "pageSize"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._page_size.as_ref() {
			params.push("pageSize", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Pagination token indicating which page of key pairs to return. If the token is not supplied, then the API will return the first page of results.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserSettingCseKeypairListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// The number of key pairs to return. If not provided, the page size will default to 20 entries.
	///
	/// Sets the *page size* query property to the given value.
	pub fn page_size(mut self, new_value: i32) -> UserSettingCseKeypairListCall<'a, S> {
		self._page_size = Some(new_value);
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes a client-side encryption key pair permanently and immediately. You can only permanently delete key pairs that have been turned off for more than 30 days. To turn off a key pair, use the DisableCseKeyPair method. Gmail can't restore or decrypt any messages that were encrypted by an obliterated key. Authenticated users and Google Workspace administrators lose access to reading the encrypted messages.
///
/// A builder for the *settings.cse.keypairs.obliterate* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::ObliterateCseKeyPairRequest;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = ObliterateCseKeyPairRequest::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_cse_keypairs_obliterate(req, "userId", "keyPairId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingCseKeypairObliterateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: ObliterateCseKeyPairRequest,
	_user_id: String,
	_key_pair_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingCseKeypairObliterateCall<'a, S> {}

impl<'a, S> UserSettingCseKeypairObliterateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.cse.keypairs.obliterate",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId", "keyPairId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("keyPairId", self._key_pair_id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/cse/keypairs/{keyPairId}:obliterate";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{keyPairId}", "keyPairId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["keyPairId", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: ObliterateCseKeyPairRequest) -> UserSettingCseKeypairObliterateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The requester's primary email address. To indicate the authenticated user, you can use the special value `me`.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingCseKeypairObliterateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The identifier of the key pair to obliterate.
	///
	/// Sets the *key pair id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn key_pair_id(mut self, new_value: &str) -> UserSettingCseKeypairObliterateCall<'a, S> {
		self._key_pair_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingCseKeypairObliterateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingCseKeypairObliterateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingCseKeypairObliterateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingCseKeypairObliterateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingCseKeypairObliterateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Adds a delegate with its verification status set directly to `accepted`, without sending any verification email. The delegate user must be a member of the same Google Workspace organization as the delegator user. Gmail imposes limitations on the number of delegates and delegators each user in a Google Workspace organization can have. These limits depend on your organization, but in general each user can have up to 25 delegates and up to 10 delegators. Note that a delegate user must be referred to by their primary email address, and not an email alias. Also note that when a new delegate is created, there may be up to a one minute delay before the new delegate is available for use. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.delegates.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Delegate;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Delegate::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_delegates_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingDelegateCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Delegate,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingDelegateCreateCall<'a, S> {}

impl<'a, S> UserSettingDelegateCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Delegate)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.delegates.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/delegates";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Delegate) -> UserSettingDelegateCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingDelegateCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingDelegateCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingDelegateCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingDelegateCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingDelegateCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingDelegateCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Removes the specified delegate (which can be of any verification status), and revokes any verification that may have been required for using it. Note that a delegate user must be referred to by their primary email address, and not an email alias. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.delegates.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_delegates_delete("userId", "delegateEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingDelegateDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingDelegateDeleteCall<'a, S> {}

impl<'a, S> UserSettingDelegateDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.delegates.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "delegateEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("delegateEmail", self._delegate_email);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/delegates/{delegateEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{delegateEmail}", "delegateEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["delegateEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingDelegateDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address of the user to be removed as a delegate.
	///
	/// Sets the *delegate email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn delegate_email(mut self, new_value: &str) -> UserSettingDelegateDeleteCall<'a, S> {
		self._delegate_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingDelegateDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingDelegateDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingDelegateDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingDelegateDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingDelegateDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified delegate. Note that a delegate user must be referred to by their primary email address, and not an email alias. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.delegates.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_delegates_get("userId", "delegateEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingDelegateGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingDelegateGetCall<'a, S> {}

impl<'a, S> UserSettingDelegateGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Delegate)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.delegates.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "delegateEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("delegateEmail", self._delegate_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/delegates/{delegateEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{delegateEmail}", "delegateEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["delegateEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingDelegateGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address of the user whose delegate relationship is to be retrieved.
	///
	/// Sets the *delegate email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn delegate_email(mut self, new_value: &str) -> UserSettingDelegateGetCall<'a, S> {
		self._delegate_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingDelegateGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingDelegateGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingDelegateGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingDelegateGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingDelegateGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the delegates for the specified account. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.delegates.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_delegates_list("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingDelegateListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingDelegateListCall<'a, S> {}

impl<'a, S> UserSettingDelegateListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListDelegatesResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.delegates.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/delegates";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingDelegateListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingDelegateListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingDelegateListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingDelegateListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingDelegateListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingDelegateListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates a filter. Note: you can only create a maximum of 1,000 filters.
///
/// A builder for the *settings.filters.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::Filter;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = Filter::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_filters_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingFilterCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: Filter,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingFilterCreateCall<'a, S> {}

impl<'a, S> UserSettingFilterCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Filter)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.filters.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/filters";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: Filter) -> UserSettingFilterCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingFilterCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingFilterCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingFilterCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingFilterCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingFilterCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingFilterCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Immediately and permanently deletes the specified filter.
///
/// A builder for the *settings.filters.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_filters_delete("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingFilterDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingFilterDeleteCall<'a, S> {}

impl<'a, S> UserSettingFilterDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.filters.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/filters/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingFilterDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the filter to be deleted.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserSettingFilterDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingFilterDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingFilterDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingFilterDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingFilterDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingFilterDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets a filter.
///
/// A builder for the *settings.filters.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_filters_get("userId", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingFilterGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingFilterGetCall<'a, S> {}

impl<'a, S> UserSettingFilterGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Filter)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.filters.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/filters/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingFilterGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the filter to be fetched.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserSettingFilterGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingFilterGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingFilterGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingFilterGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingFilterGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingFilterGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the message filters of a Gmail user.
///
/// A builder for the *settings.filters.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_filters_list("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingFilterListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingFilterListCall<'a, S> {}

impl<'a, S> UserSettingFilterListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListFiltersResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.filters.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/filters";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingFilterListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingFilterListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingFilterListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingFilterListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingFilterListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingFilterListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates a forwarding address. If ownership verification is required, a message will be sent to the recipient and the resource's verification status will be set to `pending`; otherwise, the resource will be created with verification status set to `accepted`. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.forwardingAddresses.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::ForwardingAddress;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = ForwardingAddress::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_forwarding_addresses_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingForwardingAddressCreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: ForwardingAddress,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingForwardingAddressCreateCall<'a, S> {}

impl<'a, S> UserSettingForwardingAddressCreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ForwardingAddress)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.forwardingAddresses.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/forwardingAddresses";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: ForwardingAddress) -> UserSettingForwardingAddressCreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingForwardingAddressCreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingForwardingAddressCreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingForwardingAddressCreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingForwardingAddressCreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingForwardingAddressCreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingForwardingAddressCreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes the specified forwarding address and revokes any verification that may have been required. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.forwardingAddresses.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_forwarding_addresses_delete("userId", "forwardingEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingForwardingAddressDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_forwarding_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingForwardingAddressDeleteCall<'a, S> {}

impl<'a, S> UserSettingForwardingAddressDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.forwardingAddresses.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "forwardingEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("forwardingEmail", self._forwarding_email);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/forwardingAddresses/{forwardingEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{forwardingEmail}", "forwardingEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["forwardingEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingForwardingAddressDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The forwarding address to be deleted.
	///
	/// Sets the *forwarding email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn forwarding_email(mut self, new_value: &str) -> UserSettingForwardingAddressDeleteCall<'a, S> {
		self._forwarding_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingForwardingAddressDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingForwardingAddressDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingForwardingAddressDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingForwardingAddressDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingForwardingAddressDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified forwarding address.
///
/// A builder for the *settings.forwardingAddresses.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_forwarding_addresses_get("userId", "forwardingEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingForwardingAddressGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_forwarding_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingForwardingAddressGetCall<'a, S> {}

impl<'a, S> UserSettingForwardingAddressGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ForwardingAddress)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.forwardingAddresses.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "forwardingEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("forwardingEmail", self._forwarding_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/forwardingAddresses/{forwardingEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{forwardingEmail}", "forwardingEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["forwardingEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingForwardingAddressGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The forwarding address to be retrieved.
	///
	/// Sets the *forwarding email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn forwarding_email(mut self, new_value: &str) -> UserSettingForwardingAddressGetCall<'a, S> {
		self._forwarding_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingForwardingAddressGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingForwardingAddressGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingForwardingAddressGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingForwardingAddressGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingForwardingAddressGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the forwarding addresses for the specified account.
///
/// A builder for the *settings.forwardingAddresses.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_forwarding_addresses_list("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingForwardingAddressListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingForwardingAddressListCall<'a, S> {}

impl<'a, S> UserSettingForwardingAddressListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListForwardingAddressesResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.forwardingAddresses.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/forwardingAddresses";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingForwardingAddressListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingForwardingAddressListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingForwardingAddressListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingForwardingAddressListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingForwardingAddressListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingForwardingAddressListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes the specified S/MIME config for the specified send-as alias.
///
/// A builder for the *settings.sendAs.smimeInfo.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_smime_info_delete("userId", "sendAsEmail", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendASmimeInfoDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendASmimeInfoDeleteCall<'a, S> {}

impl<'a, S> UserSettingSendASmimeInfoDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.smimeInfo.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "sendAsEmail", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/smimeInfo/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address that appears in the "From:" header for mail sent using this alias.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The immutable ID for the SmimeInfo.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendASmimeInfoDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendASmimeInfoDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendASmimeInfoDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendASmimeInfoDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified S/MIME config for the specified send-as alias.
///
/// A builder for the *settings.sendAs.smimeInfo.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_smime_info_get("userId", "sendAsEmail", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendASmimeInfoGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendASmimeInfoGetCall<'a, S> {}

impl<'a, S> UserSettingSendASmimeInfoGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SmimeInfo)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.smimeInfo.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "sendAsEmail", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/smimeInfo/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address that appears in the "From:" header for mail sent using this alias.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The immutable ID for the SmimeInfo.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendASmimeInfoGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendASmimeInfoGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendASmimeInfoGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendASmimeInfoGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Insert (upload) the given S/MIME config for the specified send-as alias. Note that pkcs12 format is required for the key.
///
/// A builder for the *settings.sendAs.smimeInfo.insert* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::SmimeInfo;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = SmimeInfo::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_smime_info_insert(req, "userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendASmimeInfoInsertCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: SmimeInfo,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendASmimeInfoInsertCall<'a, S> {}

impl<'a, S> UserSettingSendASmimeInfoInsertCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SmimeInfo)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.smimeInfo.insert",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/smimeInfo";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: SmimeInfo) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		self._request = new_value;
		self
	}
	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address that appears in the "From:" header for mail sent using this alias.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendASmimeInfoInsertCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendASmimeInfoInsertCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendASmimeInfoInsertCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendASmimeInfoInsertCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists S/MIME configs for the specified send-as alias.
///
/// A builder for the *settings.sendAs.smimeInfo.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_smime_info_list("userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendASmimeInfoListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendASmimeInfoListCall<'a, S> {}

impl<'a, S> UserSettingSendASmimeInfoListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListSmimeInfoResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.smimeInfo.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/smimeInfo";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendASmimeInfoListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address that appears in the "From:" header for mail sent using this alias.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendASmimeInfoListCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendASmimeInfoListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendASmimeInfoListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendASmimeInfoListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendASmimeInfoListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendASmimeInfoListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Sets the default S/MIME config for the specified send-as alias.
///
/// A builder for the *settings.sendAs.smimeInfo.setDefault* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_smime_info_set_default("userId", "sendAsEmail", "id")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendASmimeInfoSetDefaultCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendASmimeInfoSetDefaultCall<'a, S> {}

impl<'a, S> UserSettingSendASmimeInfoSetDefaultCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.smimeInfo.setDefault",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId", "sendAsEmail", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/smimeInfo/{id}/setDefault";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The email address that appears in the "From:" header for mail sent using this alias.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The immutable ID for the SmimeInfo.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendASmimeInfoSetDefaultCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Creates a custom "from" send-as alias. If an SMTP MSA is specified, Gmail will attempt to connect to the SMTP service to validate the configuration before creating the alias. If ownership verification is required for the alias, a message will be sent to the email address and the resource's verification status will be set to `pending`; otherwise, the resource will be created with verification status set to `accepted`. If a signature is provided, Gmail will sanitize the HTML before saving it with the alias. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.sendAs.create* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::SendAs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = SendAs::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_create(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendACreateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: SendAs,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendACreateCall<'a, S> {}

impl<'a, S> UserSettingSendACreateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SendAs)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.create",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: SendAs) -> UserSettingSendACreateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendACreateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendACreateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendACreateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendACreateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendACreateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendACreateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Deletes the specified send-as alias. Revokes any verification that may have been required for using it. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.sendAs.delete* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_delete("userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendADeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendADeleteCall<'a, S> {}

impl<'a, S> UserSettingSendADeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendADeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The send-as alias to be deleted.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendADeleteCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendADeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendADeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendADeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendADeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendADeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the specified send-as alias. Fails with an HTTP 404 error if the specified address is not a member of the collection.
///
/// A builder for the *settings.sendAs.get* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_get("userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendAGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendAGetCall<'a, S> {}

impl<'a, S> UserSettingSendAGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SendAs)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendAGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The send-as alias to be retrieved.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendAGetCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendAGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendAGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendAGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendAGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendAGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Lists the send-as aliases for the specified account. The result includes the primary send-as address associated with the account as well as any custom "from" aliases.
///
/// A builder for the *settings.sendAs.list* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_list("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendAListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendAListCall<'a, S> {}

impl<'a, S> UserSettingSendAListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListSendAsResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendAListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendAListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendAListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendAListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendAListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendAListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Patch the specified send-as alias.
///
/// A builder for the *settings.sendAs.patch* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::SendAs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = SendAs::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_patch(req, "userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendAPatchCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: SendAs,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendAPatchCall<'a, S> {}

impl<'a, S> UserSettingSendAPatchCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SendAs)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.patch",
			http_method: hyper::Method::PATCH,
		});

		for &field in ["alt", "userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PATCH)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: SendAs) -> UserSettingSendAPatchCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendAPatchCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The send-as alias to be updated.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendAPatchCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendAPatchCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendAPatchCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendAPatchCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendAPatchCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendAPatchCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates a send-as alias. If a signature is provided, Gmail will sanitize the HTML before saving it with the alias. Addresses other than the primary address for the account can only be updated by service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.sendAs.update* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::SendAs;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = SendAs::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_update(req, "userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendAUpdateCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: SendAs,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendAUpdateCall<'a, S> {}

impl<'a, S> UserSettingSendAUpdateCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, SendAs)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.update",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: SendAs) -> UserSettingSendAUpdateCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendAUpdateCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The send-as alias to be updated.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendAUpdateCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendAUpdateCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendAUpdateCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendAUpdateCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendAUpdateCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendAUpdateCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Sends a verification email to the specified send-as alias address. The verification status must be `pending`. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.sendAs.verify* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_send_as_verify("userId", "sendAsEmail")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingSendAVerifyCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_send_as_email: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingSendAVerifyCall<'a, S> {}

impl<'a, S> UserSettingSendAVerifyCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.sendAs.verify",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId", "sendAsEmail"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("sendAsEmail", self._send_as_email);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/sendAs/{sendAsEmail}/verify";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{sendAsEmail}", "sendAsEmail")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["sendAsEmail", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingSendAVerifyCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The send-as alias to be verified.
	///
	/// Sets the *send as email* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn send_as_email(mut self, new_value: &str) -> UserSettingSendAVerifyCall<'a, S> {
		self._send_as_email = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingSendAVerifyCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingSendAVerifyCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingSendAVerifyCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingSendAVerifyCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingSendAVerifyCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets the auto-forwarding setting for the specified account.
///
/// A builder for the *settings.getAutoForwarding* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_get_auto_forwarding("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingGetAutoForwardingCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingGetAutoForwardingCall<'a, S> {}

impl<'a, S> UserSettingGetAutoForwardingCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, AutoForwarding)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.getAutoForwarding",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/autoForwarding";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingGetAutoForwardingCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingGetAutoForwardingCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingGetAutoForwardingCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingGetAutoForwardingCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingGetAutoForwardingCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingGetAutoForwardingCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets IMAP settings.
///
/// A builder for the *settings.getImap* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_get_imap("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingGetImapCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingGetImapCall<'a, S> {}

impl<'a, S> UserSettingGetImapCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ImapSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.getImap",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/imap";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingGetImapCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingGetImapCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingGetImapCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingGetImapCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingGetImapCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingGetImapCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets language settings.
///
/// A builder for the *settings.getLanguage* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_get_language("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingGetLanguageCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingGetLanguageCall<'a, S> {}

impl<'a, S> UserSettingGetLanguageCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, LanguageSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.getLanguage",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/language";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingGetLanguageCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingGetLanguageCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingGetLanguageCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingGetLanguageCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingGetLanguageCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingGetLanguageCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets POP settings.
///
/// A builder for the *settings.getPop* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_get_pop("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingGetPopCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingGetPopCall<'a, S> {}

impl<'a, S> UserSettingGetPopCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, PopSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.getPop",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/pop";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingGetPopCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingGetPopCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingGetPopCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingGetPopCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingGetPopCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingGetPopCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Gets vacation responder settings.
///
/// A builder for the *settings.getVacation* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_get_vacation("userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingGetVacationCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingGetVacationCall<'a, S> {}

impl<'a, S> UserSettingGetVacationCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, VacationSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.getVacation",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/vacation";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingGetVacationCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingGetVacationCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingGetVacationCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingGetVacationCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingGetVacationCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingGetVacationCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates the auto-forwarding setting for the specified account. A verified forwarding address must be specified when auto-forwarding is enabled. This method is only available to service account clients that have been delegated domain-wide authority.
///
/// A builder for the *settings.updateAutoForwarding* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::AutoForwarding;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = AutoForwarding::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_update_auto_forwarding(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingUpdateAutoForwardingCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: AutoForwarding,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingUpdateAutoForwardingCall<'a, S> {}

impl<'a, S> UserSettingUpdateAutoForwardingCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, AutoForwarding)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.updateAutoForwarding",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/autoForwarding";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingSharing.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: AutoForwarding) -> UserSettingUpdateAutoForwardingCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingUpdateAutoForwardingCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingUpdateAutoForwardingCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingUpdateAutoForwardingCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingSharing`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingUpdateAutoForwardingCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingUpdateAutoForwardingCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingUpdateAutoForwardingCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates IMAP settings.
///
/// A builder for the *settings.updateImap* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::ImapSettings;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = ImapSettings::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_update_imap(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingUpdateImapCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: ImapSettings,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingUpdateImapCall<'a, S> {}

impl<'a, S> UserSettingUpdateImapCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ImapSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.updateImap",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/imap";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: ImapSettings) -> UserSettingUpdateImapCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingUpdateImapCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingUpdateImapCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingUpdateImapCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingUpdateImapCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingUpdateImapCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingUpdateImapCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates language settings. If successful, the return object contains the `displayLanguage` that was saved for the user, which may differ from the value passed into the request. This is because the requested `displayLanguage` may not be directly supported by Gmail but have a close variant that is, and so the variant may be chosen and saved instead.
///
/// A builder for the *settings.updateLanguage* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::LanguageSettings;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = LanguageSettings::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_update_language(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingUpdateLanguageCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: LanguageSettings,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingUpdateLanguageCall<'a, S> {}

impl<'a, S> UserSettingUpdateLanguageCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, LanguageSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.updateLanguage",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/language";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: LanguageSettings) -> UserSettingUpdateLanguageCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingUpdateLanguageCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingUpdateLanguageCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	/// Set any additional parameter of the query string used in the request.
	/// It should be used to set parameters which are not yet available through their own
	/// setters.
	///
	/// Please note that this method must not be used to set any of the known parameters
	/// which have their own setter method. If done anyway, the request will fail.
	///
	/// # Additional Parameters
	///
	/// * *$.xgafv* (query-string) - V1 error format.
	/// * *access_token* (query-string) - OAuth access token.
	/// * *alt* (query-string) - Data format for response.
	/// * *callback* (query-string) - JSONP
	/// * *fields* (query-string) - Selector specifying which fields to include in a partial response.
	/// * *key* (query-string) - API key. Your API key identifies your project and provides you with API access, quota, and reports. Required unless you provide an OAuth 2.0 token.
	/// * *oauth_token* (query-string) - OAuth 2.0 token for the current user.
	/// * *prettyPrint* (query-boolean) - Returns response with indentations and line breaks.
	/// * *quotaUser* (query-string) - Available to use for quota purposes for server-side applications. Can be any arbitrary string assigned to a user, but should not exceed 40 characters.
	/// * *uploadType* (query-string) - Legacy upload protocol for media (e.g. "media", "multipart").
	/// * *upload_protocol* (query-string) - Upload protocol for media (e.g. "raw", "multipart").
	pub fn param<T>(mut self, name: T, value: T) -> UserSettingUpdateLanguageCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingUpdateLanguageCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingUpdateLanguageCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingUpdateLanguageCall<'a, S> {
		self._scopes.clear();
		self
	}
}

/// Updates POP settings.
///
/// A builder for the *settings.updatePop* method supported by a *user* resource.
/// It is not used directly, but through a [`UserMethods`] instance.
///
/// # Example
///
/// Instantiate a resource method builder
///
/// ```test_harness,no_run
/// # extern crate hyper;
/// # extern crate hyper_rustls;
/// # extern crate google_gmail1 as gmail1;
/// use gmail1::api::PopSettings;
/// # async fn dox() {
/// # use std::default::Default;
/// # use gmail1::{Gmail, oauth2, hyper, hyper_rustls, chrono, FieldMask};
///
/// # let secret: oauth2::ApplicationSecret = Default::default();
/// # let auth = oauth2::InstalledFlowAuthenticator::builder(
/// #         secret,
/// #         oauth2::InstalledFlowReturnMethod::HTTPRedirect,
/// #     ).build().await.unwrap();
/// # let mut hub = Gmail::new(hyper::Client::builder().build(hyper_rustls::HttpsConnectorBuilder::new().with_native_roots().unwrap().https_or_http().enable_http1().build()), auth);
/// // As the method needs a request, you would usually fill it with the desired information
/// // into the respective structure. Some of the parts shown here might not be applicable !
/// // Values shown here are possibly random and not representative !
/// let mut req = PopSettings::default();
///
/// // You can configure optional parameters by calling the respective setters at will, and
/// // execute the final call using `doit()`.
/// // Values shown here are possibly random and not representative !
/// let result = hub.users().settings_update_pop(req, "userId")
///              .doit().await;
/// # }
/// ```
pub struct UserSettingUpdatePopCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: PopSettings,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingUpdatePopCall<'a, S> {}

impl<'a, S> UserSettingUpdatePopCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, PopSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.updatePop",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/pop";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: PopSettings) -> UserSettingUpdatePopCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingUpdatePopCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingUpdatePopCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserSettingUpdatePopCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::SettingBasic`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserSettingUpdatePopCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingUpdatePopCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingUpdatePopCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserSettingUpdateVacationCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: VacationSettings,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserSettingUpdateVacationCall<'a, S> {}

impl<'a, S> UserSettingUpdateVacationCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, VacationSettings)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.settings.updateVacation",
			http_method: hyper::Method::PUT,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/settings/vacation";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::SettingBasic.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::PUT)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: VacationSettings) -> UserSettingUpdateVacationCall<'a, S> {
		self._request = new_value;
		self
	}
	/// User's email address. The special value "me" can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserSettingUpdateVacationCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserSettingUpdateVacationCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserSettingUpdateVacationCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserSettingUpdateVacationCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserSettingUpdateVacationCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserSettingUpdateVacationCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadDeleteCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadDeleteCall<'a, S> {}

impl<'a, S> UserThreadDeleteCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.delete",
			http_method: hyper::Method::DELETE,
		});

		for &field in ["userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::DELETE)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn user_id(mut self, new_value: &str) -> UserThreadDeleteCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserThreadDeleteCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadDeleteCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadDeleteCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadDeleteCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadDeleteCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadDeleteCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadGetCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_metadata_headers: Vec<String>,
	_format: Option<String>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadGetCall<'a, S> {}

impl<'a, S> UserThreadGetCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Thread)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.get",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "id", "metadataHeaders", "format"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(6 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);
		if self._metadata_headers.len() > 0 {
			for f in self._metadata_headers.iter() {
				params.push("metadataHeaders", f);
			}
		}
		if let Some(value) = self._format.as_ref() {
			params.push("format", value);
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads/{id}";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::AddonCurrentMessageReadonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserThreadGetCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The ID of the thread to retrieve.
	///
	/// Sets the *id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn id(mut self, new_value: &str) -> UserThreadGetCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	/// When given and format is METADATA, only include headers specified.
	///
	/// Append the given value to the *metadata headers* query property.
	/// Each appended value will retain its original ordering and be '/'-separated in the URL's parameters.
	pub fn add_metadata_headers(mut self, new_value: &str) -> UserThreadGetCall<'a, S> {
		self._metadata_headers.push(new_value.to_string());
		self
	}
	/// The format to return the messages in.
	///
	/// Sets the *format* query property to the given value.
	pub fn format(mut self, new_value: &str) -> UserThreadGetCall<'a, S> {
		self._format = Some(new_value.to_string());
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadGetCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadGetCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadGetCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadGetCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadGetCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadListCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_q: Option<String>,
	_page_token: Option<String>,
	_max_results: Option<u32>,
	_label_ids: Vec<String>,
	_include_spam_trash: Option<bool>,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadListCall<'a, S> {}

impl<'a, S> UserThreadListCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, ListThreadsResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.list",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId", "q", "pageToken", "maxResults", "labelIds", "includeSpamTrash"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(8 + self._additional_params.len());
		params.push("userId", self._user_id);
		if let Some(value) = self._q.as_ref() {
			params.push("q", value);
		}
		if let Some(value) = self._page_token.as_ref() {
			params.push("pageToken", value);
		}
		if let Some(value) = self._max_results.as_ref() {
			params.push("maxResults", value.to_string());
		}
		if self._label_ids.len() > 0 {
			for f in self._label_ids.iter() {
				params.push("labelIds", f);
			}
		}
		if let Some(value) = self._include_spam_trash.as_ref() {
			params.push("includeSpamTrash", value.to_string());
		}

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn user_id(mut self, new_value: &str) -> UserThreadListCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// Only return threads matching the specified query. Supports the same query format as the Gmail search box. For example, `"from:someuser@example.com rfc822msgid: is:unread"`. Parameter cannot be used when accessing the api using the gmail.metadata scope.
	///
	/// Sets the *q* query property to the given value.
	pub fn q(mut self, new_value: &str) -> UserThreadListCall<'a, S> {
		self._q = Some(new_value.to_string());
		self
	}
	/// Page token to retrieve a specific page of results in the list.
	///
	/// Sets the *page token* query property to the given value.
	pub fn page_token(mut self, new_value: &str) -> UserThreadListCall<'a, S> {
		self._page_token = Some(new_value.to_string());
		self
	}
	/// Maximum number of threads to return. This field defaults to 100. The maximum allowed value for this field is 500.
	///
	/// Sets the *max results* query property to the given value.
	pub fn max_results(mut self, new_value: u32) -> UserThreadListCall<'a, S> {
		self._max_results = Some(new_value);
		self
	}
	pub fn add_label_ids(mut self, new_value: &str) -> UserThreadListCall<'a, S> {
		self._label_ids.push(new_value.to_string());
		self
	}
	/// Include threads from `SPAM` and `TRASH` in the results.
	///
	/// Sets the *include spam trash* query property to the given value.
	pub fn include_spam_trash(mut self, new_value: bool) -> UserThreadListCall<'a, S> {
		self._include_spam_trash = Some(new_value);
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadListCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadListCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadListCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadListCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadListCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadModifyCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: ModifyThreadRequest,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadModifyCall<'a, S> {}

impl<'a, S> UserThreadModifyCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Thread)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.modify",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(5 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads/{id}/modify";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn request(mut self, new_value: ModifyThreadRequest) -> UserThreadModifyCall<'a, S> {
		self._request = new_value;
		self
	}
	pub fn user_id(mut self, new_value: &str) -> UserThreadModifyCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn id(mut self, new_value: &str) -> UserThreadModifyCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadModifyCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadModifyCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadModifyCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadModifyCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadModifyCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadTrashCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadTrashCall<'a, S> {}

impl<'a, S> UserThreadTrashCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Thread)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.trash",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads/{id}/trash";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn user_id(mut self, new_value: &str) -> UserThreadTrashCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn id(mut self, new_value: &str) -> UserThreadTrashCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadTrashCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadTrashCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadTrashCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadTrashCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadTrashCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserThreadUntrashCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserThreadUntrashCall<'a, S> {}

impl<'a, S> UserThreadUntrashCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Thread)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.threads.untrash",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId", "id"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);
		params.push("id", self._id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/threads/{id}/untrash";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId"), ("{id}", "id")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["id", "userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn user_id(mut self, new_value: &str) -> UserThreadUntrashCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn id(mut self, new_value: &str) -> UserThreadUntrashCall<'a, S> {
		self._id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserThreadUntrashCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserThreadUntrashCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserThreadUntrashCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserThreadUntrashCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserThreadUntrashCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserGetProfileCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserGetProfileCall<'a, S> {}

impl<'a, S> UserGetProfileCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, Profile)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.getProfile",
			http_method: hyper::Method::GET,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(3 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/profile";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Readonly.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::GET)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	pub fn user_id(mut self, new_value: &str) -> UserGetProfileCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserGetProfileCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserGetProfileCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Readonly`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserGetProfileCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserGetProfileCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserGetProfileCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserStopCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserStopCall<'a, S> {}

impl<'a, S> UserStopCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<hyper::Response<hyper::body::Body>> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.stop",
			http_method: hyper::Method::POST,
		});

		for &field in ["userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(2 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/stop";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder.header(CONTENT_LENGTH, 0_u64).body(hyper::body::Body::empty());

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = res;

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	/// The user's email address. The special value `me` can be used to indicate the authenticated user.
	///
	/// Sets the *user id* path property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn user_id(mut self, new_value: &str) -> UserStopCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	/// The delegate implementation is consulted whenever there is an intermediate result, or if something goes wrong
	/// while executing the actual API request.
	///
	/// ````text
	///                   It should be used to handle progress information, and to implement a certain level of resilience.
	/// ````
	///
	/// Sets the *delegate* property to the given value.
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserStopCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserStopCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	/// Identifies the authorization scope for the method you are building.
	///
	/// Use this method to actively specify which scope should be used, instead of the default [`Scope`] variant
	/// [`Scope::Gmai`].
	///
	/// The `scope` will be added to a set of scopes. This is important as one can maintain access
	/// tokens for more than one scope.
	///
	/// Usually there is more than one suitable scope to authorize an operation, some of which may
	/// encompass more rights than others. For example, for listing resources, a *read-only* scope will be
	/// sufficient, a read-write scope will do as well.
	pub fn add_scope<St>(mut self, scope: St) -> UserStopCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserStopCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserStopCall<'a, S> {
		self._scopes.clear();
		self
	}
}

pub struct UserWatchCall<'a, S>
where
	S: 'a,
{
	hub: &'a Gmail<S>,
	_request: WatchRequest,
	_user_id: String,
	_delegate: Option<&'a mut dyn client::Delegate>,
	_additional_params: HashMap<String, String>,
	_scopes: BTreeSet<String>,
}

impl<'a, S> client::CallBuilder for UserWatchCall<'a, S> {}

impl<'a, S> UserWatchCall<'a, S>
where
	S: tower_service::Service<http::Uri> + Clone + Send + Sync + 'static,
	S::Response: hyper::client::connect::Connection + AsyncRead + AsyncWrite + Send + Unpin + 'static,
	S::Future: Send + Unpin + 'static,
	S::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	/// Perform the operation you have build so far.
	pub async fn doit(mut self) -> client::Result<(hyper::Response<hyper::body::Body>, WatchResponse)> {
		use client::{url::Params, ToParts};
		use hyper::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION, USER_AGENT};
		use std::borrow::Cow;
		use std::io::{Read, Seek};

		let mut dd = client::DefaultDelegate;
		let mut dlg: &mut dyn client::Delegate = self._delegate.unwrap_or(&mut dd);
		dlg.begin(client::MethodInfo {
			id: "gmail.users.watch",
			http_method: hyper::Method::POST,
		});

		for &field in ["alt", "userId"].iter() {
			if self._additional_params.contains_key(field) {
				dlg.finished(false);
				return Err(client::Error::FieldClash(field));
			}
		}

		let mut params = Params::with_capacity(4 + self._additional_params.len());
		params.push("userId", self._user_id);

		params.extend(self._additional_params.iter());

		params.push("alt", "json");
		let mut url = self.hub._base_url.clone() + "gmail/v1/users/{userId}/watch";
		if self._scopes.is_empty() {
			self._scopes.insert(Scope::Gmai.as_ref().to_string());
		}

		for &(find_this, param_name) in [("{userId}", "userId")].iter() {
			url = params.uri_replacement(url, param_name, find_this, false);
		}
		{
			let to_remove = ["userId"];
			params.remove_params(&to_remove);
		}

		let url = params.parse_with_url(&url);

		let mut json_mime_type = mime::APPLICATION_JSON;
		let mut request_value_reader = {
			let mut value = json::value::to_value(&self._request).expect("serde to work");
			client::remove_json_null_values(&mut value);
			let mut dst = io::Cursor::new(Vec::with_capacity(128));
			json::to_writer(&mut dst, &value).unwrap();
			dst
		};
		let request_size = request_value_reader.seek(io::SeekFrom::End(0)).unwrap();
		request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();

		loop {
			let token = match self.hub.auth.get_token(&self._scopes.iter().map(String::as_str).collect::<Vec<_>>()[..]).await {
				Ok(token) => token,
				Err(e) => match dlg.token(e) {
					Ok(token) => token,
					Err(e) => {
						dlg.finished(false);
						return Err(client::Error::MissingToken(e));
					}
				},
			};
			request_value_reader.seek(io::SeekFrom::Start(0)).unwrap();
			let mut req_result = {
				let client = &self.hub.client;
				dlg.pre_request();
				let mut req_builder = hyper::Request::builder()
					.method(hyper::Method::POST)
					.uri(url.as_str())
					.header(USER_AGENT, self.hub._user_agent.clone());

				if let Some(token) = token.as_ref() {
					req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", token));
				}

				let request = req_builder
					.header(CONTENT_TYPE, json_mime_type.to_string())
					.header(CONTENT_LENGTH, request_size as u64)
					.body(hyper::body::Body::from(request_value_reader.get_ref().clone()));

				client.request(request.unwrap()).await
			};

			match req_result {
				Err(err) => {
					if let client::Retry::After(d) = dlg.http_error(&err) {
						sleep(d).await;
						continue;
					}
					dlg.finished(false);
					return Err(client::Error::HttpError(err));
				}
				Ok(mut res) => {
					if !res.status().is_success() {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;
						let (parts, _) = res.into_parts();
						let body = hyper::Body::from(res_body_string.clone());
						let restored_response = hyper::Response::from_parts(parts, body);

						let server_response = json::from_str::<serde_json::Value>(&res_body_string).ok();

						if let client::Retry::After(d) = dlg.http_failure(&restored_response, server_response.clone()) {
							sleep(d).await;
							continue;
						}

						dlg.finished(false);

						return match server_response {
							Some(error_value) => Err(client::Error::BadRequest(error_value)),
							None => Err(client::Error::Failure(restored_response)),
						};
					}
					let result_value = {
						let res_body_string = client::get_body_as_string(res.body_mut()).await;

						match json::from_str(&res_body_string) {
							Ok(decoded) => (res, decoded),
							Err(err) => {
								dlg.response_json_decode_error(&res_body_string, &err);
								return Err(client::Error::JsonDecodeError(res_body_string, err));
							}
						}
					};

					dlg.finished(true);
					return Ok(result_value);
				}
			}
		}
	}

	///
	/// Sets the *request* property to the given value.
	///
	/// Even though the property as already been set when instantiating this call,
	/// we provide this method for API completeness.
	pub fn request(mut self, new_value: WatchRequest) -> UserWatchCall<'a, S> {
		self._request = new_value;
		self
	}
	pub fn user_id(mut self, new_value: &str) -> UserWatchCall<'a, S> {
		self._user_id = new_value.to_string();
		self
	}
	pub fn delegate(mut self, new_value: &'a mut dyn client::Delegate) -> UserWatchCall<'a, S> {
		self._delegate = Some(new_value);
		self
	}

	pub fn param<T>(mut self, name: T, value: T) -> UserWatchCall<'a, S>
	where
		T: AsRef<str>,
	{
		self._additional_params.insert(name.as_ref().to_string(), value.as_ref().to_string());
		self
	}

	pub fn add_scope<St>(mut self, scope: St) -> UserWatchCall<'a, S>
	where
		St: AsRef<str>,
	{
		self._scopes.insert(String::from(scope.as_ref()));
		self
	}
	/// Identifies the authorization scope(s) for the method you are building.
	///
	/// See [`Self::add_scope()`] for details.
	pub fn add_scopes<I, St>(mut self, scopes: I) -> UserWatchCall<'a, S>
	where
		I: IntoIterator<Item = St>,
		St: AsRef<str>,
	{
		self._scopes.extend(scopes.into_iter().map(|s| String::from(s.as_ref())));
		self
	}

	/// Removes all scopes, and no default scope will be used either.
	/// In this case, you have to specify your API-key using the `key` parameter (see [`Self::param()`]
	/// for details).
	pub fn clear_scopes(mut self) -> UserWatchCall<'a, S> {
		self._scopes.clear();
		self
	}
}
