/// All failure modes the crate can produce.
///
/// Design rule: every variant names a *distinct* situation.
/// Callers must handle them separately; we do not paper over
/// differences with a catch-all "something went wrong".
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PocketError {
	// ── I/O ──────────────────────────────────────────────────────────────────
	#[error("registry file not found at {path}")]
	RegistryNotFound { path: PathBuf },

	#[error("could not read registry at {path}: {source}")]
	RegistryRead {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},

	#[error("could not write registry at {path}: {source}")]
	RegistryWrite {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},

	#[error("could not create config directory {path}: {source}")]
	ConfigDirCreate {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},

	#[error("Could not create configuration directory at '{path}': {reason}")]
	InvalidRegistryPath { path: PathBuf, reason: String },

	// ── Serialisation ─────────────────────────────────────────────────────────
	#[error("registry is malformed JSON: {source}")]
	Deserialize {
		#[source]
		source: serde_json::Error,
	},

	#[error("could not serialise registry: {source}")]
	Serialize {
		#[source]
		source: serde_json::Error,
	},

	// ── Domain ────────────────────────────────────────────────────────────────
	#[error("registry is full ({max} entries); remove an entry before adding")]
	RegistryFull { max: usize },

	#[error("no register found with label: {label:?}")]
	LabelNotFound { label: String },

	#[error("label is empty")]
	EmptyLabel,

	#[error("value is empty")]
	EmptyValue,

	#[error("duplicate label: {label:?} already exists")]
	DuplicateLabel { label: String },

	// ── Interactive / picker ──────────────────────────────────────────────────
	#[error("picker was cancelled by the user")]
	PickerCancelled,

	#[error("picker produced no output")]
	PickerNoSelection,

	// ── Stdin ─────────────────────────────────────────────────────────────────
	#[error("could not read from stdin: {source}")]
	StdinRead {
		#[source]
		source: std::io::Error,
	},
}

pub type Result<T> = std::result::Result<T, PocketError>;
